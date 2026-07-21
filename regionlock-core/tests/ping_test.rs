//! Ping module tests: iputils output parsing, the estimate ladder, and
//! end-to-end probing against a stub executable (a shell script echoing a
//! canned iputils reply, passed to `probe` as `ping_program`).

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use regionlock_core::feed::{SdrFeed, TypicalPing};
use regionlock_core::payload::PingValue;
use regionlock_core::ping;

fn feed_with_pairs(pairs: &[(&str, &str, i64)]) -> SdrFeed {
    SdrFeed {
        revision: 1,
        pops: BTreeMap::new(),
        typical_pings: pairs
            .iter()
            .map(|(from, to, ms)| TypicalPing(from.to_string(), to.to_string(), *ms))
            .collect(),
    }
}

#[test]
fn parse_success_sub_ms_rounds_to_zero() {
    let stdout = "PING 155.133.226.68 (155.133.226.68) 56(84) bytes of data.\n\
                  64 bytes from 155.133.226.68: icmp_seq=1 ttl=56 time=0.4 ms\n\
                  \n\
                  --- 155.133.226.68 ping statistics ---\n\
                  1 packets transmitted, 1 received, 0% packet loss, time 0ms\n\
                  rtt min/avg/max/mdev = 0.4/0.4/0.4/0.0 ms\n";
    assert_eq!(ping::parse_ping_output(stdout), Some(0));
}

#[test]
fn parse_success_rounds_half_up() {
    let stdout = "64 bytes from 155.133.226.68: icmp_seq=1 ttl=56 time=23.7 ms\n";
    assert_eq!(ping::parse_ping_output(stdout), Some(24));
}

#[test]
fn parse_success_whole_ms_form() {
    let stdout = "64 bytes from 155.133.226.68: icmp_seq=1 ttl=56 time=12 ms\n";
    assert_eq!(ping::parse_ping_output(stdout), Some(12));
}

#[test]
fn parse_total_loss_is_none() {
    let stdout = "PING 10.0.0.1 (10.0.0.1) 56(84) bytes of data.\n\
                  \n\
                  --- 10.0.0.1 ping statistics ---\n\
                  1 packets transmitted, 0 received, 100% packet loss, time 0ms\n";
    assert_eq!(ping::parse_ping_output(stdout), None);
}

#[test]
fn parse_garbage_is_none() {
    assert_eq!(ping::parse_ping_output("definitely not ping output"), None);
    assert_eq!(ping::parse_ping_output(""), None);
}

#[test]
fn estimate_home_set_and_pair_present() {
    let feed = feed_with_pairs(&[("fra", "ams", 6)]);
    assert!(matches!(
        ping::estimate(&feed, Some("fra"), "ams"),
        PingValue::Estimate(6)
    ));
    // The pair resolves in either direction.
    assert!(matches!(
        ping::estimate(&feed, Some("ams"), "fra"),
        PingValue::Estimate(6)
    ));
}

#[test]
fn estimate_pair_absent_is_unknown() {
    let feed = feed_with_pairs(&[("fra", "ams", 6)]);
    assert!(matches!(
        ping::estimate(&feed, Some("fra"), "syd"),
        PingValue::Unknown
    ));
}

#[test]
fn estimate_no_home_is_unknown() {
    let feed = feed_with_pairs(&[("fra", "ams", 6)]);
    assert!(matches!(
        ping::estimate(&feed, None, "ams"),
        PingValue::Unknown
    ));
}

/// A stub `ping` executable in a temp dir: a shell script echoing canned
/// output. Removed on drop.
struct StubPing {
    dir: PathBuf,
    program: PathBuf,
}

impl StubPing {
    fn new(script: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("regionlock-ping-core-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("ping");
        std::fs::write(&program, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        StubPing { dir, program }
    }

    fn path(&self) -> &std::path::Path {
        &self.program
    }
}

impl Drop for StubPing {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A canned iputils `ping -c 1` reply reporting 42.6 ms.
const CANNED_OK: &str = "#!/bin/sh
echo 'PING 10.0.0.1 (10.0.0.1) 56(84) bytes of data.'
echo '64 bytes from 10.0.0.1: icmp_seq=1 ttl=64 time=42.6 ms'
echo ''
echo '--- 10.0.0.1 ping statistics ---'
echo '1 packets transmitted, 1 received, 0% packet loss, time 0ms'
";

#[test]
fn probe_reports_every_target_measured() {
    let stub = StubPing::new(CANNED_OK);
    // 12 targets > 8 workers, so the pool must cycle the work queue.
    let targets: Vec<(String, Ipv4Addr)> = (0..12)
        .map(|i| (format!("pop{i:02}"), Ipv4Addr::new(10, 0, 0, i + 1)))
        .collect();
    let rx = ping::probe(targets.clone(), 1, stub.path().to_str().unwrap());
    let results: Vec<(String, PingValue)> = rx.iter().collect();
    assert_eq!(
        results.len(),
        targets.len(),
        "every target reports exactly once: {results:?}"
    );
    let mut pops: Vec<&str> = results.iter().map(|(pop, _)| pop.as_str()).collect();
    pops.sort_unstable();
    let mut expected: Vec<&str> = targets.iter().map(|(pop, _)| pop.as_str()).collect();
    expected.sort_unstable();
    assert_eq!(pops, expected, "results cover every target");
    for (pop, value) in &results {
        assert!(
            matches!(value, PingValue::Measured(43)),
            "{pop}: canned 42.6 rounds half-up to 43, got {value:?}"
        );
    }
}

#[test]
fn probe_spawn_failure_yields_unknown_not_panic() {
    let targets = vec![("fra".to_string(), Ipv4Addr::new(127, 0, 0, 1))];
    let rx = ping::probe(targets, 1, "/nonexistent/ping");
    let results: Vec<(String, PingValue)> = rx.iter().collect();
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0].1, PingValue::Unknown));
}

#[test]
fn probe_no_targets_terminates_immediately() {
    let rx = ping::probe(Vec::new(), 1, "ping");
    assert_eq!(rx.iter().count(), 0);
}

#[test]
fn availability_checks_spawn_and_exit_success() {
    let stub = StubPing::new(CANNED_OK);
    assert!(ping::ping_available(stub.path().to_str().unwrap()));
    assert!(!ping::ping_available("/nonexistent/ping"));
    // Runs but fails every invocation: must read as unavailable so the
    // CLI drops to the estimate ladder.
    let failing = StubPing::new("#!/bin/sh\nexit 1\n");
    assert!(!ping::ping_available(failing.path().to_str().unwrap()));
}

#[test]
fn parse_requires_ms_suffix() {
    // Foreign formats without the iputils ` ms` unit must not be mistaken
    // for a measurement.
    assert_eq!(ping::parse_ping_output("time=12"), None);
    assert_eq!(ping::parse_ping_output("time=12 garbage"), None);
    assert_eq!(ping::parse_ping_output("rtt time=5.0 msec-ish"), Some(5));
}
