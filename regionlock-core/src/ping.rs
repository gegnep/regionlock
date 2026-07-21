//! Live latency probing.
//!
//! Design: shell out to the system `ping` binary (iputils). It carries its
//! own capabilities, so probing works unprivileged; core takes on no
//! raw-socket code, CAP_NET_RAW handling, or async runtime. Degradation
//! ladder per resolved decisions (Q4):
//! 1. `ping` available → Measured per POP (failed probe → Unknown).
//! 2. `ping` unavailable → Estimate = typical_pings(home_pop, X) when the
//!    user configured `home_pop` and the pair exists; otherwise Unknown.
//!    Never synthesize through intermediate POPs.

use std::net::Ipv4Addr;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::payload::PingValue;

/// True when a usable `ping` binary is on PATH. Checked once per run.
/// `ping_program` overrides the binary name (tests inject a stub).
///
/// Spawns `<program> -c 1 -W 1 127.0.0.1` and requires a successful exit,
/// not just a successful spawn: an unprivileged iputils binary can spawn
/// fine and still fail (no CAP_NET_RAW), and that must count as
/// unavailable so the CLI falls back to estimates.
pub fn ping_available(ping_program: &str) -> bool {
    Command::new(ping_program)
        .args(["-c", "1", "-W", "1", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Probe each (pop, ip) concurrently with `<ping_program> -n -c 1 -W <sec>`.
/// Returns a receiver yielding (pop, PingValue) AS RESULTS ARRIVE (the CLI
/// streams NDJSON from it). Worker pool: min(targets, 8) threads. A POP's
/// first relay IP is representative: relays in one POP share a site.
/// Probe failure or unparseable output → PingValue::Unknown, never an error.
pub fn probe(
    targets: Vec<(String, Ipv4Addr)>,
    timeout_secs: u32,
    ping_program: &str,
) -> mpsc::Receiver<(String, PingValue)> {
    let (results_tx, results_rx) = mpsc::channel();
    let worker_count = targets.len().min(8);

    // Shared mpsc work queue: feed every target in, then close it so
    // workers see the queue drain and exit on their own.
    let (work_tx, work_rx) = mpsc::channel::<(String, Ipv4Addr)>();
    for target in targets {
        work_tx
            .send(target)
            .expect("work_rx outlives this send loop");
    }
    drop(work_tx);
    let work_rx = Arc::new(Mutex::new(work_rx));

    for _ in 0..worker_count {
        let work_rx = Arc::clone(&work_rx);
        let results_tx = results_tx.clone();
        let ping_program = ping_program.to_string();
        std::thread::spawn(move || {
            loop {
                let next = {
                    let queue = work_rx.lock().expect("work queue mutex not poisoned");
                    queue.recv()
                };
                let Ok((pop, ip)) = next else { break };
                let value = probe_one(&ping_program, ip, timeout_secs);
                if results_tx.send((pop, value)).is_err() {
                    break;
                }
            }
            // `results_tx` drops here; once every worker has exited, the
            // last clone drops and `results_rx`'s iterator terminates.
        });
    }
    // Drop our own clone: only worker threads must hold the sender alive.
    drop(results_tx);

    results_rx
}

/// Run one `<ping_program> -n -c 1 -W <timeout_secs> <ip>` probe. Never
/// panics: a spawn failure or unparseable reply both yield `Unknown`.
fn probe_one(ping_program: &str, ip: Ipv4Addr, timeout_secs: u32) -> PingValue {
    let output = Command::new(ping_program)
        .args(["-n", "-c", "1", "-W"])
        .arg(timeout_secs.to_string())
        .arg(ip.to_string())
        .output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            match parse_ping_output(&stdout) {
                Some(ms) => PingValue::Measured(ms),
                None => PingValue::Unknown,
            }
        }
        Err(_) => PingValue::Unknown,
    }
}

/// Parse iputils `ping -c 1` stdout: the `time=12.3 ms` value in whole ms
/// (rounded). None when absent (loss, error text, foreign format). The
/// ` ms` unit suffix is required, so `time=12` or `time=12 garbage` (foreign
/// formats, not iputils) parse as None rather than a bogus measurement.
pub fn parse_ping_output(stdout: &str) -> Option<i64> {
    let start = stdout.find("time=")? + "time=".len();
    let rest = &stdout[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    if !rest[end..].trim_start().starts_with("ms") {
        return None;
    }
    let value: f64 = rest[..end].parse().ok()?;
    Some(round_half_up(value))
}

/// Round-half-up to the nearest whole ms (ping times are never negative).
fn round_half_up(value: f64) -> i64 {
    (value + 0.5).floor() as i64
}

/// Estimate ladder step 2: typical_pings(home_pop → pop), labeled Estimate;
/// Unknown when home_pop is None or the sparse table lacks the pair.
pub fn estimate(feed: &crate::feed::SdrFeed, home_pop: Option<&str>, pop: &str) -> PingValue {
    match home_pop {
        Some(home) => match feed.estimate_ms(home, pop) {
            Some(ms) => PingValue::Estimate(ms),
            None => PingValue::Unknown,
        },
        None => PingValue::Unknown,
    }
}
