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
use std::sync::mpsc;

use crate::payload::PingValue;

/// True when a usable `ping` binary is on PATH. Checked once per run.
/// `ping_program` overrides the binary name (tests inject a stub).
pub fn ping_available(ping_program: &str) -> bool {
    let _ = ping_program;
    todo!("M4I")
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
    let _ = (targets, timeout_secs, ping_program);
    todo!("M4I")
}

/// Parse iputils `ping -c 1` stdout: the `time=12.3 ms` value in whole ms
/// (rounded). None when absent (loss, error text, foreign format).
pub fn parse_ping_output(stdout: &str) -> Option<i64> {
    let _ = stdout;
    todo!("M4I")
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
