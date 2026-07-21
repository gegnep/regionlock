//! End-to-end tests for `ping` and `list --ping`, driving the real
//! `regionlock` binary (hermetic pattern from cli_test.rs: temp config +
//! seeded cache via child env). The system `ping` is never invoked: PATH is
//! prepended with a temp dir holding a stub executable named `ping`, so
//! every probe here is fully offline and deterministic.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../regionlock-core/tests/fixtures/sdr-1422450.json"
);
const REVISION: u64 = 1_784_582_254;

/// A stub `ping` that always succeeds and echoes a canned 12.3ms reply,
/// regardless of its arguments. Covers both invocations the CLI makes:
/// the `-c 1 ... 127.0.0.1` availability probe and the real
/// `-n -c 1 -W ... <ip>` per-target probe.
const SUCCESS_STUB: &str = "#!/bin/sh\n\
echo 'PING 127.0.0.1 (127.0.0.1) 56(84) bytes of data.'\n\
echo '64 bytes from 127.0.0.1: icmp_seq=1 ttl=64 time=12.3 ms'\n\
echo ''\n\
echo '--- 127.0.0.1 ping statistics ---'\n\
echo '1 packets transmitted, 1 received, 0% packet loss, time 0ms'\n\
echo 'rtt min/avg/max/mdev = 12.3/12.3/12.3/0.000 ms'\n\
exit 0\n";

/// A stub `ping` that always fails, simulating a missing binary or one
/// without CAP_NET_RAW: both the availability probe and any per-target
/// probe exit non-zero.
const FAIL_STUB: &str = "#!/bin/sh\n\
echo 'ping: socket: Operation not permitted' 1>&2\n\
exit 1\n";

struct TestEnv {
    dir: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    stub_dir: PathBuf,
}

impl TestEnv {
    fn new(tag: &str, stub_script: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "regionlock-ping-cli-{tag}-{}-{seq}",
            std::process::id()
        ));
        let config = dir.join("config.toml");
        let cache = dir.join("cache");
        let stub_dir = dir.join("stub");
        let feed_dir = cache.join("regionlock");
        std::fs::create_dir_all(&feed_dir).unwrap();
        std::fs::create_dir_all(&stub_dir).unwrap();
        std::fs::copy(FIXTURE, feed_dir.join(format!("1422450-{REVISION}.json"))).unwrap();

        let stub_path = stub_dir.join("ping");
        std::fs::write(&stub_path, stub_script).unwrap();
        set_executable(&stub_path);

        TestEnv {
            dir,
            config,
            cache,
            stub_dir,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        // Prepend the stub dir so a bare "ping" (what the CLI always
        // spawns) resolves to the stub, never a real system binary.
        let path_var = format!(
            "{}:{}",
            self.stub_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_regionlock"))
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .env("XDG_CACHE_HOME", &self.cache)
            .env_remove("REGIONLOCK_CONFIG")
            .env("TERM", "xterm")
            .env_remove("NO_COLOR")
            .env("PATH", path_var)
            .output()
            .unwrap()
    }

    fn run_ok(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn ndjson_lines(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{line:?}: {e}")))
        .collect()
}

/// Every blockable POP code from the fixture (relay-less POPs excluded),
/// computed from the feed itself rather than hardcoded, so this stays
/// correct if the fixture ever changes.
fn blockable_pop_codes() -> std::collections::BTreeSet<String> {
    let bytes = std::fs::read(FIXTURE).unwrap();
    let feed = regionlock_core::feed::SdrFeed::parse(&bytes).unwrap();
    feed.blockable_pops()
        .map(|(code, _)| code.to_string())
        .collect()
}

#[test]
fn ping_json_streams_one_measured_line_per_blockable_pop() {
    let env = TestEnv::new("ok", SUCCESS_STUB);
    let out = env.run_ok(&["ping", "--json"]);
    let lines = ndjson_lines(&stdout(&out));
    let expected = blockable_pop_codes();
    assert_eq!(
        lines.len(),
        expected.len(),
        "one NDJSON line per blockable POP"
    );

    let mut seen = std::collections::BTreeSet::new();
    for line in &lines {
        assert_eq!(line["schema_version"], 1);
        assert_eq!(line["ping"]["source"], "measured");
        assert_eq!(line["ping"]["ms"], 12, "canned 12.3ms rounds to 12");
        seen.insert(line["pop"].as_str().unwrap().to_string());
    }
    assert_eq!(seen, expected, "every blockable POP is represented");
}

#[test]
fn list_ping_json_fills_ping_for_blockable_pops_only() {
    let env = TestEnv::new("list-ok", SUCCESS_STUB);
    let out = env.run_ok(&["list", "--ping", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let pops = value["pops"].as_array().unwrap();

    let fra = pops.iter().find(|p| p["code"] == "fra").unwrap();
    assert_eq!(fra["blockable"], true);
    assert_eq!(fra["ping"]["source"], "measured");
    assert_eq!(fra["ping"]["ms"], 12);

    let hel = pops.iter().find(|p| p["code"] == "hel").unwrap();
    assert_eq!(hel["blockable"], false, "hel is relay-less");
    assert!(
        hel["ping"].is_null(),
        "relay-less POPs have no ping target: {hel}"
    );
}

#[test]
fn ping_json_yields_unknown_without_home_pop_when_ping_unavailable() {
    let env = TestEnv::new("fail-nohome", FAIL_STUB);
    let out = env.run_ok(&["ping", "--json"]);
    let lines = ndjson_lines(&stdout(&out));
    assert!(!lines.is_empty());
    for line in &lines {
        assert_eq!(line["schema_version"], 1);
        assert_eq!(
            line["ping"]["source"], "unknown",
            "no home_pop configured: {line}"
        );
    }
    assert!(
        stderr(&out).is_empty(),
        "the fallback note is human-mode only, --json stays silent on stderr: {}",
        stderr(&out)
    );
}

#[test]
fn ping_json_uses_estimates_when_home_pop_set_and_ping_unavailable() {
    let env = TestEnv::new("fail-home", FAIL_STUB);
    std::fs::write(&env.config, "home_pop = \"fra\"\n").unwrap();
    let out = env.run_ok(&["ping", "--json"]);
    let lines = ndjson_lines(&stdout(&out));

    let mut sources = std::collections::BTreeSet::new();
    for line in &lines {
        let source = line["ping"]["source"].as_str().unwrap().to_string();
        assert!(
            source == "estimate" || source == "unknown",
            "unexpected source {source} in {line}"
        );
        sources.insert(source);
    }
    assert!(
        sources.contains("estimate"),
        "typical_pings has fra pairs for some blockable POPs: {lines:?}"
    );
    assert!(
        sources.contains("unknown"),
        "typical_pings lacks a fra pair for some blockable POPs: {lines:?}"
    );
}

#[test]
fn ping_human_mode_prints_exactly_one_fallback_note() {
    let env = TestEnv::new("fail-human", FAIL_STUB);
    let out = env.run_ok(&["ping"]);
    let err = stderr(&out);
    let note_lines: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(note_lines.len(), 1, "exactly one stderr note: {err:?}");
    assert!(
        note_lines[0].contains("estimate"),
        "note explains the fallback: {err}"
    );
}
