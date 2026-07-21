//! regionlock-apply: the only privileged component.
//!
//! Contract (SPEC, privilege model):
//! - Reads exactly ONE typed [`Operation`] as JSON from stdin (never env,
//!   never argv paths). Input is size-capped.
//! - Validates it via [`Operation::validate`] before acting; refusals are
//!   reported as a [`Reply::Refused`] on stdout with exit code 1.
//! - Acts ONLY on `table inet regionlock` and the fixed paths under
//!   /run/regionlock. It constructs the ruleset itself from the operation;
//!   raw nft text from the caller is inexpressible in the schema.
//! - All operations serialize on an exclusive flock of a 0600 lock file.
//! - The applied-state journal is written two-phase: pending record →
//!   nft → commit rename. Inspect reconciles leftovers from crashes.
//!
//! Keep this file small and boring. Every line runs as root.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use regionlock_core::backend::{FirewallBackend, NftBackend};
use regionlock_core::ops::{Operation, Reply};
use regionlock_core::plan::{AppliedState, RulesetSpec};

const RUN_DIR: &str = "/run/regionlock";
const LOCK_PATH: &str = "/run/regionlock/lock";
const PENDING_DELETE_PATH: &str = "/run/regionlock/applied.json.pending-delete";
/// Stdin cap: a maximal legitimate operation is ~50 KiB; 1 MiB is generous.
const MAX_INPUT: u64 = 1024 * 1024;

fn main() -> ExitCode {
    match run() {
        Ok(reply) => {
            print_reply(&reply);
            match reply {
                Reply::Refused { .. } => ExitCode::from(1),
                _ => ExitCode::SUCCESS,
            }
        }
        Err(message) => {
            print_reply(&Reply::Refused { reason: message });
            ExitCode::from(1)
        }
    }
}

fn print_reply(reply: &Reply) {
    // Reply serialization cannot fail (no maps with non-string keys, no
    // non-finite floats); if it somehow does, exit code still reports it.
    if let Ok(json) = serde_json::to_string(reply) {
        println!("{json}");
    }
}

fn run() -> Result<Reply, String> {
    // Root check first: everything below assumes it.
    // SAFETY: geteuid has no preconditions and touches no memory.
    if unsafe { libc::geteuid() } != 0 {
        return Err("regionlock-apply must run as root (via pkexec/sudo/doas/run0)".into());
    }

    let operation = read_operation()?;
    operation
        .validate()
        .map_err(|rejection| format!("operation refused: {rejection}"))?;

    ensure_run_dir()?;
    let _lock = ExclusiveLock::acquire()?;

    match operation {
        Operation::ReplaceRuleset {
            game,
            revision,
            pops,
            ..
        } => replace_ruleset(game, revision, pops),
        Operation::DeleteTable { .. } => delete_table(),
        Operation::Inspect { .. } => inspect(),
    }
}

fn read_operation() -> Result<Operation, String> {
    let mut input = String::new();
    std::io::stdin()
        .lock()
        .take(MAX_INPUT)
        .read_to_string(&mut input)
        .map_err(|e| format!("could not read stdin: {e}"))?;
    if input.len() as u64 >= MAX_INPUT {
        return Err(format!("input exceeds {MAX_INPUT} bytes"));
    }
    serde_json::from_str(&input).map_err(|e| format!("malformed operation: {e}"))
}

/// Create/verify /run/regionlock. Refuses symlinks and non-root ownership:
/// a pre-existing attacker-owned directory must never be trusted.
fn ensure_run_dir() -> Result<(), String> {
    let dir = Path::new(RUN_DIR);
    match fs::symlink_metadata(dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!("{RUN_DIR} exists and is not a plain directory"));
            }
            if meta.uid() != 0 {
                return Err(format!("{RUN_DIR} is not owned by root"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(dir).map_err(|e| format!("could not create {RUN_DIR}: {e}"))?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("could not chmod {RUN_DIR}: {e}"))?;
        }
        Err(e) => return Err(format!("could not stat {RUN_DIR}: {e}")),
    }
    Ok(())
}

/// Exclusive flock on /run/regionlock/lock (0600: a world-readable lock
/// would let any user flock it and block privileged operations). Held for
/// the process lifetime; the kernel releases it on any exit path.
struct ExclusiveLock {
    _file: fs::File,
}

impl ExclusiveLock {
    fn acquire() -> Result<ExclusiveLock, String> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(LOCK_PATH)
            .map_err(|e| format!("could not open {LOCK_PATH}: {e}"))?;
        // Pre-existing file: O_NOFOLLOW blocked symlinks; still refuse a
        // non-root-owned or over-permissive lock left by someone else.
        let meta = file
            .metadata()
            .map_err(|e| format!("could not stat {LOCK_PATH}: {e}"))?;
        if meta.uid() != 0 {
            return Err(format!("{LOCK_PATH} is not owned by root"));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(LOCK_PATH, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("could not chmod {LOCK_PATH}: {e}"))?;
        }
        // SAFETY: flock on an owned, open fd; blocks until acquired.
        let rc =
            unsafe { libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) };
        if rc != 0 {
            return Err(format!(
                "could not lock {LOCK_PATH}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(ExclusiveLock { _file: file })
    }
}

fn replace_ruleset(
    game: regionlock_core::Game,
    revision: u64,
    pops: BTreeMap<String, Vec<Ipv4Addr>>,
) -> Result<Reply, String> {
    let spec = RulesetSpec {
        game,
        revision,
        pops,
    };
    let ruleset = NftBackend.render(&spec);
    let applied_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock before epoch".to_string())?
        .as_secs();
    let journal = AppliedState::from_spec(&spec, applied_at);
    let journal_bytes = serde_json::to_vec_pretty(&journal)
        .map_err(|e| format!("could not serialize journal: {e}"))?;

    // Phase 1: pending intent record.
    write_file_0644(Path::new(AppliedState::PENDING_PATH), &journal_bytes)?;
    // Phase 2: apply atomically via nft -f -.
    run_nft_stdin(&ruleset)?;
    // Phase 3: commit.
    fs::rename(AppliedState::PENDING_PATH, AppliedState::JOURNAL_PATH)
        .map_err(|e| format!("applied but could not commit journal: {e}"))?;
    Ok(Reply::Applied { journal })
}

fn delete_table() -> Result<Reply, String> {
    // Mark intent when a journal exists: rename journal → pending-delete.
    let had_journal = match fs::rename(AppliedState::JOURNAL_PATH, PENDING_DELETE_PATH) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("could not mark pending delete: {e}")),
    };

    let existed = match nft(&["delete", "table", "inet", "regionlock"]) {
        Ok(_) => true,
        Err(stderr) if is_missing_table(&stderr) => false,
        Err(stderr) => {
            // Delete failed with the table possibly still present: restore
            // the journal so state stays truthful.
            if had_journal {
                let _ = fs::rename(PENDING_DELETE_PATH, AppliedState::JOURNAL_PATH);
            }
            return Err(format!("nft delete failed: {stderr}"));
        }
    };

    let _ = fs::remove_file(PENDING_DELETE_PATH);
    let _ = fs::remove_file(AppliedState::PENDING_PATH);
    Ok(Reply::Deleted { existed })
}

fn inspect() -> Result<Reply, String> {
    let live = read_live_table()?;
    let mut reconciled_pending = false;

    // Reconcile a crashed replace: pending record present.
    if let Ok(bytes) = fs::read(AppliedState::PENDING_PATH) {
        reconciled_pending = true;
        let pending_matches_live = AppliedState::parse(&bytes)
            .map(|pending| Some(&pending.pops) == live.as_ref())
            .unwrap_or(false);
        if pending_matches_live {
            fs::rename(AppliedState::PENDING_PATH, AppliedState::JOURNAL_PATH)
                .map_err(|e| format!("could not commit reconciled journal: {e}"))?;
        } else {
            let _ = fs::remove_file(AppliedState::PENDING_PATH);
        }
    }
    // Reconcile a crashed delete: marker present.
    if Path::new(PENDING_DELETE_PATH).exists() {
        reconciled_pending = true;
        if live.is_none() {
            let _ = fs::remove_file(PENDING_DELETE_PATH);
            let _ = fs::remove_file(AppliedState::JOURNAL_PATH);
        } else {
            let _ = fs::rename(PENDING_DELETE_PATH, AppliedState::JOURNAL_PATH);
        }
    }

    let journal = match fs::read(AppliedState::JOURNAL_PATH) {
        Ok(bytes) => AppliedState::parse(&bytes).ok(),
        Err(_) => None,
    };
    Ok(Reply::Inspected {
        live,
        journal,
        reconciled_pending,
    })
}

/// `nft -j list table inet regionlock`, normalized to POP → sorted IPs.
/// None when the table does not exist.
fn read_live_table() -> Result<Option<BTreeMap<String, Vec<Ipv4Addr>>>, String> {
    let stdout = match nft(&["-j", "list", "table", "inet", "regionlock"]) {
        Ok(stdout) => stdout,
        Err(stderr) if is_missing_table(&stderr) => return Ok(None),
        Err(stderr) => return Err(format!("nft list failed: {stderr}")),
    };
    let value: serde_json::Value =
        serde_json::from_str(&stdout).map_err(|e| format!("nft -j output unparseable: {e}"))?;
    let mut live: BTreeMap<String, Vec<Ipv4Addr>> = BTreeMap::new();
    for entry in value["nftables"].as_array().into_iter().flatten() {
        let Some(set) = entry.get("set") else {
            continue;
        };
        let Some(name) = set["name"].as_str() else {
            continue;
        };
        let Some(code) = name.strip_prefix("pop_") else {
            continue;
        };
        let mut ips: Vec<Ipv4Addr> = set["elem"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str())
            .filter_map(|s| s.parse().ok())
            .collect();
        ips.sort();
        live.insert(code.to_string(), ips);
    }
    Ok(Some(live))
}

fn is_missing_table(stderr: &str) -> bool {
    stderr.contains("No such file or directory")
}

fn nft(args: &[&str]) -> Result<String, String> {
    let output = Command::new("nft")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run nft (is nftables installed?): {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(stderr)
    }
}

fn run_nft_stdin(ruleset: &str) -> Result<(), String> {
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run nft (is nftables installed?): {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(ruleset.as_bytes())
        .map_err(|e| format!("could not feed nft: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("nft did not exit cleanly: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "nft rejected the ruleset: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// 0644 atomic write within /run/regionlock (tmp + rename, O_NOFOLLOW).
fn write_file_0644(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = PathBuf::from(format!("{}.tmp.{}", path.display(), std::process::id()));
    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .map_err(|e| format!("could not create {}: {e}", tmp.display()))?;
        file.write_all(bytes)
            .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("could not commit {}: {e}", path.display())
    })
}
