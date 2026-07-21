//! Privilege escalation: run regionlock-apply as root, once per apply.
//!
//! Backends per SPEC: pkexec (with a pkttyagent fallback), sudo, doas,
//! run0. Auto-detection tries them in that order; config overrides. The
//! operation travels over the child's stdin (never env, never argv), and
//! the applier's single JSON reply comes back on stdout.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::Escalator;
use crate::ops::{Operation, Reply};
use crate::{Error, Result};

/// Locate regionlock-apply: next to the current executable first (the
/// packaged layout), then $PATH.
pub fn applier_path() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("regionlock-apply");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    find_in_path("regionlock-apply").ok_or_else(|| Error::Escalation {
        attempted: "locating regionlock-apply".into(),
        reason: "not found next to the binary or on PATH".into(),
    })
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The concrete escalation commands, in auto-detection order.
fn backends(preference: Escalator) -> Vec<&'static str> {
    match preference {
        Escalator::Auto => vec!["pkexec", "sudo", "doas", "run0"],
        Escalator::Pkexec => vec!["pkexec"],
        Escalator::Sudo => vec!["sudo"],
        Escalator::Doas => vec!["doas"],
        Escalator::Run0 => vec!["run0"],
    }
}

/// Escalate once and run the operation through regionlock-apply.
/// Tries the preferred backend(s); reports every attempt on failure so the
/// error names what was tried (SPEC polish requirement).
pub fn run_applier(preference: Escalator, operation: &Operation) -> Result<Reply> {
    let applier = applier_path()?;
    let payload = serde_json::to_string(operation).map_err(|e| Error::Escalation {
        attempted: "serializing operation".into(),
        reason: e.to_string(),
    })?;

    // Already root (a systemd boot unit runs the CLI as root): run the
    // applier directly. An escalator like pkexec needs an auth agent that
    // does not exist at boot; SPEC: "runs as root, no escalation inside the
    // unit". SAFETY: geteuid has no preconditions and touches no memory.
    if unsafe { libc::geteuid() } == 0 {
        return drive(Command::new(&applier), &payload).map_err(|reason| Error::Escalation {
            attempted: "direct exec (already root)".into(),
            reason,
        });
    }

    let mut attempts: Vec<String> = Vec::new();
    for backend in backends(preference) {
        if find_in_path(backend).is_none() {
            attempts.push(format!("{backend} (not installed)"));
            continue;
        }
        match run_via(backend, &applier, &payload) {
            Ok(reply) => return Ok(reply),
            Err(reason) => {
                // pkexec without a polkit agent: start pkttyagent for this
                // process and retry once before moving on (SPEC).
                if backend == "pkexec" && find_in_path("pkttyagent").is_some() {
                    let agent = Command::new("pkttyagent")
                        .arg("--process")
                        .arg(std::process::id().to_string())
                        .stdin(Stdio::null())
                        .spawn();
                    if let Ok(mut agent) = agent {
                        let retry = run_via(backend, &applier, &payload);
                        let _ = agent.kill();
                        let _ = agent.wait();
                        match retry {
                            Ok(reply) => return Ok(reply),
                            Err(retry_reason) => {
                                attempts
                                    .push(format!("{backend} (with pkttyagent: {retry_reason})"));
                                continue;
                            }
                        }
                    }
                }
                attempts.push(format!("{backend} ({reason})"));
            }
        }
    }
    Err(Error::Escalation {
        attempted: attempts.join(", "),
        reason: "no escalation backend succeeded".into(),
    })
}

fn run_via(
    backend: &str,
    applier: &std::path::Path,
    payload: &str,
) -> std::result::Result<Reply, String> {
    let mut cmd = Command::new(backend);
    cmd.arg(applier);
    drive(cmd, payload)
}

/// Send the operation on the child's stdin and read its single JSON reply
/// from stdout. Shared by the escalated path (backend + applier) and the
/// direct-root path (applier alone).
fn drive(mut cmd: Command, payload: &str) -> std::result::Result<Reply, String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()) // auth prompts (sudo/doas) talk to the tty
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(payload.as_bytes())
        .map_err(|e| format!("could not send operation: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("did not exit: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reply: Option<Reply> = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok());
    match reply {
        Some(reply) => Ok(reply),
        None if output.status.success() => Err("applier produced no reply".into()),
        None => Err(format!(
            "exit {}",
            output
                .status
                .code()
                .map_or_else(|| "by signal".into(), |c| c.to_string())
        )),
    }
}
