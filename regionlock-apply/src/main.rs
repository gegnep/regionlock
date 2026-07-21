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
use regionlock_core::ops::{Operation, Reply, parse_feed_snapshot_filename};
use regionlock_core::plan::{AppliedState, RulesetSpec};

const RUN_DIR: &str = "/run/regionlock";
const LOCK_PATH: &str = "/run/regionlock/lock";
const PENDING_DELETE_PATH: &str = "/run/regionlock/applied.json.pending-delete";
/// Boot snapshot directory: config.toml + feed-<appid>-<revision>.json,
/// written only here (EnablePersist), fixed filenames only.
const ETC_DIR: &str = "/etc/regionlock";
const ETC_CONFIG_NAME: &str = "config.toml";
/// The unit enable-persist/disable-persist manage.
const PERSIST_UNIT: &str = "regionlock.service";
/// Stdin cap: a maximal legitimate operation (EnablePersist carrying a
/// real ~30 KiB feed) stays under ~100 KiB; 1 MiB is generous.
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
    // The kernel ANDs every create mode with the umask. An inherited
    // restrictive umask (hardened sudo/pkexec setups use 0077) would
    // silently turn the 0644 journal into 0600 and break unprivileged
    // `status` reads. Pin it before any file creation so the explicit
    // .mode() calls below mean what they say.
    // SAFETY: umask has no preconditions and touches no memory.
    unsafe { libc::umask(0o022) };

    // Root check next: everything below assumes it.
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
        Operation::EnablePersist {
            config_toml,
            feed_filename,
            feed_json,
            ..
        } => enable_persist(&config_toml, &feed_filename, &feed_json),
        Operation::DisablePersist { .. } => disable_persist(),
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
    ensure_root_dir_0755(RUN_DIR)
}

/// Create/verify a root-owned 0755 directory at a fixed path. Refuses
/// symlinks and non-root ownership; repairs over-permissive modes.
/// Shared by /run/regionlock and /etc/regionlock (EnablePersist).
fn ensure_root_dir_0755(path: &str) -> Result<(), String> {
    let dir = Path::new(path);
    match fs::symlink_metadata(dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!("{path} exists and is not a plain directory"));
            }
            if meta.uid() != 0 {
                return Err(format!("{path} is not owned by root"));
            }
            // Repair a group/world-writable dir (bad tmpfiles.d rule, etc.)
            // before trusting files inside it, mirroring the lock file.
            if meta.permissions().mode() & 0o022 != 0 {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("could not chmod {path}: {e}"))?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Set the mode atomically at creation so there is no
            // world-writable window between create and chmod under a
            // permissive umask.
            std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o755)
                .create(dir)
                .map_err(|e| format!("could not create {path}: {e}"))?;
        }
        Err(e) => return Err(format!("could not stat {path}: {e}")),
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
    // Phase 2: apply atomically via nft -f -. On failure remove the pending
    // record: nothing crashed and the live table is untouched, so a later
    // Inspect must not report a spurious reconciliation.
    if let Err(e) = run_nft_stdin(&ruleset) {
        let _ = fs::remove_file(AppliedState::PENDING_PATH);
        return Err(e);
    }
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

/// A file /etc/regionlock recognizes as part of the boot snapshot. The
/// applier never touches any other name in that directory.
fn is_snapshot_file_name(name: &str) -> bool {
    name == ETC_CONFIG_NAME || parse_feed_snapshot_filename(name).is_some()
}

/// A trusted snapshot entry: a regular root-owned file, never a symlink.
/// Entries failing this are ignored by every persist path — the applier
/// neither backs up, restores, nor deletes anything it does not own.
/// `owner` is a test seam; production passes 0 (root).
fn is_trusted_snapshot_entry(path: &Path, owner: u32) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.uid() == owner)
}

/// Current snapshot files in `dir` (canonical names only, no .bak),
/// restricted to trusted entries. Missing directory reads as empty.
fn snapshot_file_names_in(dir: &Path, owner: u32) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(e) => return Err(format!("could not read {}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not read {}: {e}", dir.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if is_snapshot_file_name(&name) && is_trusted_snapshot_entry(&dir.join(&name), owner) {
            names.push(name);
        }
    }
    Ok(names)
}

/// Recovery pass for a crashed EnablePersist, run before any new mutation.
/// A `<name>.bak` whose canonical `<name>` is missing means the prior run
/// backed up but never wrote the replacement: restore it. A .bak whose
/// canonical exists means the prior run got past the writes: the .bak is
/// stale and safe to drop — never before that confirmation. Retrying
/// therefore converges from any interruption point.
fn recover_snapshot_backups_in(dir: &Path, owner: u32) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("could not read {}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not read {}: {e}", dir.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(canonical) = name.strip_suffix(".bak") else {
            continue;
        };
        if !is_snapshot_file_name(canonical) {
            continue;
        }
        let backup = dir.join(&name);
        if !is_trusted_snapshot_entry(&backup, owner) {
            continue;
        }
        let canonical_path = dir.join(canonical);
        if fs::symlink_metadata(&canonical_path).is_ok() {
            fs::remove_file(&backup)
                .map_err(|e| format!("could not drop stale {}: {e}", backup.display()))?;
        } else {
            fs::rename(&backup, &canonical_path)
                .map_err(|e| format!("could not recover {}: {e}", canonical_path.display()))?;
        }
    }
    Ok(())
}

/// EnablePersist: one idempotent transaction. A crashed prior run is
/// recovered first; then prior snapshot files move to `<name>.bak`
/// (prior-state capture) and the new snapshot is written. `systemctl
/// enable` runs last; on any failure the fresh writes are removed, the
/// backups restored, and a changed unit enable-state reverted — with
/// rollback failures reported, never swallowed.
fn enable_persist(
    config_toml: &str,
    feed_filename: &str,
    feed_json: &str,
) -> Result<Reply, String> {
    ensure_root_dir_0755(ETC_DIR)?;
    let etc = Path::new(ETC_DIR);
    // Converge any interrupted prior run before capturing new state.
    recover_snapshot_backups_in(etc, 0)?;
    // Detect before mutating anything: a transient systemctl failure must
    // abort here, never run enable on what might be a Nix-store unit. An
    // uninstalled unit reads as not managed; the enable step then reports
    // the real error.
    let managed_by_nixos = match unit_managed_by_nixos() {
        Ok(managed) => managed,
        Err(e) if is_missing_unit(&e) => false,
        Err(e) => return Err(e),
    };
    // Prior unit enable-state, captured for compensation. None means
    // undeterminable (compensation is skipped rather than guessed);
    // NixOS-managed units never change state here.
    let prior_enabled = if managed_by_nixos {
        None
    } else {
        unit_is_enabled()
    };

    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
    let mut ran_enable = false;
    let result = (|| -> Result<(), String> {
        for name in snapshot_file_names_in(etc, 0)? {
            let original = etc.join(&name);
            let backup = etc.join(format!("{name}.bak"));
            // The recovery pass cleared every snapshot .bak; rename would
            // atomically replace an untrusted leftover at this name anyway.
            fs::rename(&original, &backup)
                .map_err(|e| format!("could not back up {}: {e}", original.display()))?;
            backups.push((original, backup));
        }
        let config_path = etc.join(ETC_CONFIG_NAME);
        write_file_0644(&config_path, config_toml.as_bytes())?;
        written.push(config_path);
        let feed_path = etc.join(feed_filename);
        write_file_0644(&feed_path, feed_json.as_bytes())?;
        written.push(feed_path);
        if !managed_by_nixos {
            ran_enable = true;
            systemctl(&["enable", PERSIST_UNIT])
                .map_err(|stderr| format!("systemctl enable failed: {stderr}"))?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            // Committed. The .baks are now stale; removal failures are
            // non-fatal — the next run's recovery pass sweeps them (their
            // canonicals exist, so they read as stale there too).
            for (_, backup) in &backups {
                let _ = fs::remove_file(backup);
            }
            Ok(Reply::Persisted { managed_by_nixos })
        }
        Err(e) => {
            // Compensate everything, collecting failures instead of
            // stopping at the first: the caller must learn when rollback
            // itself failed. Fresh writes go first (one may share a
            // backup's original name), then the backups restore.
            let mut rollback_errors: Vec<String> = Vec::new();
            for path in &written {
                if let Err(err) = fs::remove_file(path)
                    && err.kind() != std::io::ErrorKind::NotFound
                {
                    rollback_errors.push(format!("remove {}: {err}", path.display()));
                }
            }
            for (original, backup) in backups.iter().rev() {
                if let Err(err) = fs::rename(backup, original) {
                    rollback_errors.push(format!("restore {}: {err}", original.display()));
                }
            }
            // A failed `systemctl enable` can still leave partial state;
            // restore a captured "disabled" prior state. Even without
            // this, enabled-with-no-snapshot is benign at boot — the
            // unit's ConditionPathExists= skips it — but compensate for
            // consistency.
            if ran_enable
                && prior_enabled == Some(false)
                && let Err(err) = systemctl(&["disable", PERSIST_UNIT])
            {
                rollback_errors.push(format!("restore prior disabled unit state: {err}"));
            }
            if rollback_errors.is_empty() {
                Err(e)
            } else {
                Err(format!(
                    "{e}; rollback also failed: {}",
                    rollback_errors.join("; ")
                ))
            }
        }
    }
}

/// DisablePersist: disable the unit (skipped when NixOS manages it) and
/// remove the snapshot files including stale .bak leftovers. Idempotent:
/// an absent unit, directory, or file is already the goal state. Removal
/// failures are collected and reported in aggregate, not swallowed.
fn disable_persist() -> Result<Reply, String> {
    // Same pre-mutation detection contract as enable_persist.
    let managed_by_nixos = match unit_managed_by_nixos() {
        Ok(managed) => managed,
        Err(e) if is_missing_unit(&e) => false,
        Err(e) => return Err(e),
    };
    if !managed_by_nixos {
        match systemctl(&["disable", PERSIST_UNIT]) {
            Ok(_) => {}
            Err(stderr) if is_missing_unit(&stderr) => {}
            Err(stderr) => return Err(format!("systemctl disable failed: {stderr}")),
        }
    }
    let etc = Path::new(ETC_DIR);
    let entries = match fs::read_dir(etc) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Reply::Unpersisted { managed_by_nixos });
        }
        Err(e) => return Err(format!("could not read {ETC_DIR}: {e}")),
    };
    let mut removal_errors: Vec<String> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                removal_errors.push(format!("read {ETC_DIR}: {e}"));
                continue;
            }
        };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let canonical = name.strip_suffix(".bak").unwrap_or(&name);
        if !is_snapshot_file_name(canonical) {
            continue;
        }
        let path = etc.join(&name);
        if !is_trusted_snapshot_entry(&path, 0) {
            continue;
        }
        if let Err(e) = fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            removal_errors.push(format!("remove {}: {e}", path.display()));
        }
    }
    if removal_errors.is_empty() {
        Ok(Reply::Unpersisted { managed_by_nixos })
    } else {
        Err(format!(
            "snapshot removal incomplete: {}",
            removal_errors.join("; ")
        ))
    }
}

/// NixOS-managed detection: a unit whose fragment path lives in /nix/store
/// belongs to the NixOS module, which owns enablement; systemctl
/// enable/disable is skipped for it. Detection failures propagate — acting
/// on a transient error could run systemctl against a module-owned unit.
fn unit_managed_by_nixos() -> Result<bool, String> {
    parse_unit_fragment_lookup(systemctl(&[
        "show",
        "--property=FragmentPath",
        "--value",
        PERSIST_UNIT,
    ]))
}

/// Pure half of [`unit_managed_by_nixos`], split out for unit tests.
fn parse_unit_fragment_lookup(lookup: Result<String, String>) -> Result<bool, String> {
    match lookup {
        Ok(stdout) => Ok(stdout.trim().starts_with("/nix/store/")),
        Err(stderr) => Err(format!(
            "could not resolve the {PERSIST_UNIT} unit path: {stderr}"
        )),
    }
}

/// Prior unit enable-state via `systemctl is-enabled` (its exit code
/// varies by state; only stdout matters). None = undeterminable.
fn unit_is_enabled() -> Option<bool> {
    let output = Command::new("systemctl")
        .args(["is-enabled", PERSIST_UNIT])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    match String::from_utf8_lossy(&output.stdout).trim() {
        "enabled" | "enabled-runtime" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
}

/// systemctl phrases a missing unit in a version-dependent way ("does not
/// exist", "could not be found", "No such file or directory"); match the
/// known phrasings so disabling an absent unit stays idempotent.
fn is_missing_unit(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("does not exist")
        || s.contains("not found")
        || s.contains("could not be found")
        || s.contains("no such file")
}

fn systemctl(args: &[&str]) -> Result<String, String> {
    let output = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run systemctl: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(stderr)
    }
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
        // Hard-error on any element we cannot parse rather than dropping it:
        // reconciliation compares against this map, so a silently-dropped IP
        // could make a genuinely-matching pending look mismatched and get
        // discarded, leaving the journal stale.
        let mut ips: Vec<Ipv4Addr> = Vec::new();
        for elem in set["elem"].as_array().into_iter().flatten() {
            let s = elem
                .as_str()
                .ok_or_else(|| format!("nft -j: non-string element in set {name}"))?;
            let ip = s
                .parse()
                .map_err(|_| format!("nft -j: unparseable IPv4 {s:?} in set {name}"))?;
            ips.push(ip);
        }
        ips.sort();
        live.insert(code.to_string(), ips);
    }
    Ok(Some(live))
}

/// nft signals a missing table in a version-dependent way; match the known
/// phrasings so teardown/inspect of an absent table stay idempotent rather
/// than erroring on a wording change.
fn is_missing_table(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("no such file or directory") || s.contains("does not exist")
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
    // pid + counter, not pid alone: a stale tmp left by a crashed run whose
    // pid is later reused would otherwise make create_new fail (EEXIST) and
    // abort a legitimate apply.
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = PathBuf::from(format!(
        "{}.tmp.{}.{seq}",
        path.display(),
        std::process::id()
    ));
    // Defensively clear a stale tmp at this exact name before create_new.
    let _ = fs::remove_file(&tmp);
    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .map_err(|e| format!("could not create {}: {e}", tmp.display()))?;
        // Clean up the tmp on every post-creation failure, not only on
        // rename: a failed write must not litter the directory either.
        if let Err(e) = file.write_all(bytes) {
            drop(file);
            let _ = fs::remove_file(&tmp);
            return Err(format!("could not write {}: {e}", tmp.display()));
        }
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("could not commit {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Unique tempdir per test; no env mutation, never real /etc.
    fn tempdir(tag: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "regionlock-apply-test-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    /// The uid this test process creates files as: the trusted-entry
    /// owner seam (production passes 0).
    fn own_uid() -> u32 {
        // SAFETY: geteuid has no preconditions and touches no memory.
        unsafe { libc::geteuid() }
    }

    #[test]
    fn recovery_restores_backup_whose_canonical_is_missing() {
        let dir = tempdir("recover-restore");
        // Crash profile: the prior run backed up config.toml and the feed
        // but never wrote their replacements.
        fs::write(dir.join("config.toml.bak"), b"prior config").unwrap();
        fs::write(dir.join("feed-1422450-1.json.bak"), b"prior feed").unwrap();

        recover_snapshot_backups_in(&dir, own_uid()).expect("recovery succeeds");

        assert_eq!(fs::read(dir.join("config.toml")).unwrap(), b"prior config");
        assert_eq!(
            fs::read(dir.join("feed-1422450-1.json")).unwrap(),
            b"prior feed"
        );
        assert!(!dir.join("config.toml.bak").exists());
        assert!(!dir.join("feed-1422450-1.json.bak").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_drops_stale_backup_only_when_canonical_exists() {
        let dir = tempdir("recover-stale");
        // Crash profile: the prior run wrote the new snapshot but died
        // before sweeping its .baks. The canonical is the newer state and
        // must win; the stale .bak goes.
        fs::write(dir.join("config.toml"), b"new config").unwrap();
        fs::write(dir.join("config.toml.bak"), b"old config").unwrap();

        recover_snapshot_backups_in(&dir, own_uid()).expect("recovery succeeds");

        assert_eq!(fs::read(dir.join("config.toml")).unwrap(), b"new config");
        assert!(!dir.join("config.toml.bak").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_is_idempotent_and_tolerates_missing_dir() {
        let dir = tempdir("recover-idem");
        fs::write(dir.join("feed-1422450-1.json.bak"), b"prior feed").unwrap();
        recover_snapshot_backups_in(&dir, own_uid()).expect("first pass");
        recover_snapshot_backups_in(&dir, own_uid()).expect("second pass is a no-op");
        assert_eq!(
            fs::read(dir.join("feed-1422450-1.json")).unwrap(),
            b"prior feed"
        );
        recover_snapshot_backups_in(&dir.join("missing"), own_uid())
            .expect("missing dir reads as nothing to recover");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_ignores_untrusted_and_foreign_entries() {
        let dir = tempdir("recover-untrusted");
        // A symlink posing as a snapshot backup is not a trusted entry.
        fs::write(dir.join("target"), b"elsewhere").unwrap();
        std::os::unix::fs::symlink(dir.join("target"), dir.join("config.toml.bak")).unwrap();
        // Foreign names are never touched, .bak or not.
        fs::write(dir.join("notes.txt.bak"), b"foreign").unwrap();
        // A wrong owner (seam: demand a uid this process is not) is skipped.
        fs::write(dir.join("feed-1422450-1.json.bak"), b"prior feed").unwrap();

        recover_snapshot_backups_in(&dir, own_uid().wrapping_add(1)).expect("recovery succeeds");
        assert!(
            !dir.join("feed-1422450-1.json").exists(),
            "wrong-owner backup must not be restored"
        );

        recover_snapshot_backups_in(&dir, own_uid()).expect("recovery succeeds");
        assert!(
            !dir.join("config.toml").exists(),
            "symlink backup must not be restored"
        );
        assert!(dir.join("config.toml.bak").exists(), "symlink left alone");
        assert!(dir.join("notes.txt.bak").exists(), "foreign name untouched");
        assert_eq!(
            fs::read(dir.join("feed-1422450-1.json")).unwrap(),
            b"prior feed",
            "trusted backup restored once the owner matches"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn snapshot_listing_requires_trusted_regular_files() {
        let dir = tempdir("listing-trust");
        fs::write(dir.join("config.toml"), b"c").unwrap();
        fs::write(dir.join("feed-1422450-1.json"), b"f").unwrap();
        fs::write(dir.join("notes.txt"), b"junk").unwrap();
        fs::write(dir.join("feed-1422450-1.json.bak"), b"bak").unwrap();
        std::os::unix::fs::symlink(dir.join("notes.txt"), dir.join("feed-570-1.json")).unwrap();

        let mut names = snapshot_file_names_in(&dir, own_uid()).expect("listing succeeds");
        names.sort();
        assert_eq!(names, ["config.toml", "feed-1422450-1.json"]);

        // Wrong owner: nothing is trusted.
        assert_eq!(
            snapshot_file_names_in(&dir, own_uid().wrapping_add(1)).expect("listing succeeds"),
            Vec::<String>::new()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nixos_detection_propagates_failure_and_classifies_paths() {
        // The Result contract behind fix "abort before mutation": a failed
        // lookup is an error, never silently "not managed".
        assert_eq!(
            parse_unit_fragment_lookup(Ok(
                "/nix/store/abc123-unit-files/regionlock.service\n".to_string()
            )),
            Ok(true)
        );
        assert_eq!(
            parse_unit_fragment_lookup(Ok("/etc/systemd/system/regionlock.service\n".to_string())),
            Ok(false)
        );
        assert_eq!(parse_unit_fragment_lookup(Ok(String::new())), Ok(false));
        let err = parse_unit_fragment_lookup(Err("transient dbus error".to_string()))
            .expect_err("lookup failure propagates");
        assert!(err.contains("transient dbus error"), "err was: {err}");

        // The tolerated escape hatch is exactly the missing-unit phrasing.
        assert!(is_missing_unit("Unit regionlock.service does not exist."));
        assert!(is_missing_unit(
            "Unit regionlock.service could not be found."
        ));
        assert!(!is_missing_unit("Connection timed out"));
    }
}
