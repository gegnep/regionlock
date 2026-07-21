//! End-to-end tests driving the real `regionlock` binary. Each test gets a
//! hermetic temp dir: an (existing, empty) config file passed via --config
//! and a fake XDG cache seeded with the Deadlock feed fixture. The child
//! env carries XDG_CACHE_HOME; the test process env is never touched
//! (std::env::set_var is unsafe on edition 2024).

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use regionlock_core::Game;
use regionlock_core::config::Config;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../regionlock-core/tests/fixtures/sdr-1422450.json"
);
const REVISION: u64 = 1_784_582_254;

struct TestEnv {
    dir: PathBuf,
    config: PathBuf,
    cache: PathBuf,
}

impl TestEnv {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("regionlock-cli-{tag}-{}-{seq}", std::process::id()));
        let config = dir.join("config.toml");
        let cache = dir.join("cache");
        let feed_dir = cache.join("regionlock");
        std::fs::create_dir_all(&feed_dir).unwrap();
        // No config pre-touch: an explicit --config path is honored as the
        // write target even before the file exists (first-run behavior).
        std::fs::copy(FIXTURE, feed_dir.join(format!("1422450-{REVISION}.json"))).unwrap();
        TestEnv { dir, config, cache }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_regionlock"))
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .env("XDG_CACHE_HOME", &self.cache)
            // Never let a real REGIONLOCK_CONFIG in the ambient environment
            // shadow the --config path under test.
            .env_remove("REGIONLOCK_CONFIG")
            // Prove tty-independent color gating: even with a real TERM and
            // no NO_COLOR, piped stdout must stay plain.
            .env("TERM", "xterm")
            .env_remove("NO_COLOR")
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

    /// Desired blocked POPs for Deadlock, read back from the config file.
    fn desired(&self) -> std::collections::BTreeSet<String> {
        Config::load(&self.config)
            .unwrap()
            .desired(Game::Deadlock)
            .blocked
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn block_selectors_persist_and_report() {
    let env = TestEnv::new("block");
    let out = env.run_ok(&["block", "fra", "ams"]);
    let text = stdout(&out);
    assert!(
        text.contains("fra") && text.contains("ams"),
        "delta mentions both: {text}"
    );
    let desired = env.desired();
    assert!(
        desired.contains("fra") && desired.contains("ams"),
        "config on disk lists them"
    );

    // Idempotent: a second identical block reports no changes.
    let out = env.run_ok(&["block", "fra", "ams"]);
    assert!(stdout(&out).contains("no changes"));

    let out = env.run_ok(&["unblock", "fra"]);
    assert!(stdout(&out).contains("unblocked"));
    assert!(!env.desired().contains("fra"));
}

#[test]
fn block_region_expands_without_relay_less_pops() {
    let env = TestEnv::new("block-eu");
    env.run_ok(&["block", "eu"]);
    let desired = env.desired();
    assert!(
        desired.contains("fra") && desired.contains("ams") && desired.contains("waw"),
        "eu expands to member POPs: {desired:?}"
    );
    assert!(desired.len() > 2, "eu expands broadly: {desired:?}");
    assert!(
        !desired.contains("hel"),
        "relay-less POPs never land in desired state"
    );
}

#[test]
fn list_json_shape() {
    let env = TestEnv::new("list-json");
    env.run_ok(&["block", "fra"]);
    let out = env.run_ok(&["list", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["game"], "deadlock");
    assert_eq!(value["revision"], REVISION);
    let pops = value["pops"].as_array().unwrap();
    let fra = pops.iter().find(|pop| pop["code"] == "fra").unwrap();
    assert_eq!(fra["blocked"], true);
    assert!(fra["tier"].is_number(), "tier is exposed: {fra}");
    assert_eq!(fra["blockable"], true);
    assert!(fra["ping"].is_null(), "ping is not wired until M4");
    let hel = pops.iter().find(|pop| pop["code"] == "hel").unwrap();
    assert_eq!(
        hel["blockable"], false,
        "relay-less POPs are marked unblockable"
    );
}

#[test]
fn unknown_selector_human_and_json() {
    let env = TestEnv::new("unknown-sel");
    let out = env.run(&["block", "zzz"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("zzz"));

    let out = env.run(&["block", "zzz", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_str(&stderr(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["kind"], "unknown_selector");
    assert_eq!(value["exit_code"], 1);
}

#[test]
fn allow_keeps_only_selected() {
    let env = TestEnv::new("allow");
    env.run_ok(&["allow", "fra"]);
    let desired = env.desired();
    assert!(!desired.contains("fra"), "the keep-set stays unblocked");
    assert!(desired.contains("ams") && desired.contains("lhr") && desired.contains("sea"));
    assert!(!desired.contains("hel") && !desired.contains("eat") && !desired.contains("fsn"));
    // 32 fixture POPs - 3 relay-less - fra = 28.
    assert_eq!(
        desired.len(),
        28,
        "everything blockable except fra: {desired:?}"
    );
}

#[test]
fn game_get_set_roundtrip() {
    let env = TestEnv::new("game");
    assert_eq!(stdout(&env.run_ok(&["game"])).trim(), "deadlock");
    env.run_ok(&["game", "cs2"]);
    assert_eq!(stdout(&env.run_ok(&["game"])).trim(), "cs2");
    let out = env.run_ok(&["game", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["default_game"], "cs2");
    assert_eq!(Config::load(&env.config).unwrap().default_game, Game::Cs2);
}

#[test]
fn preset_roundtrip() {
    let env = TestEnv::new("preset");
    env.run_ok(&["block", "fra", "ams"]);
    env.run_ok(&["preset", "save", "faves"]);
    env.run_ok(&["reset"]);
    assert!(env.desired().is_empty(), "reset clears desired state");
    env.run_ok(&["preset", "load", "faves"]);
    let desired = env.desired();
    assert!(desired.contains("fra") && desired.contains("ams") && desired.len() == 2);

    let out = env.run_ok(&["preset", "list", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    let presets = value["presets"].as_array().unwrap();
    assert_eq!(presets[0]["name"], "faves");
    assert_eq!(presets[0]["blocked"], 2);

    env.run_ok(&["preset", "rm", "faves"]);
    let out = env.run(&["preset", "rm", "faves"]);
    assert_eq!(out.status.code(), Some(1), "rm of a missing name fails");
    let out = env.run(&["preset", "load", "missing"]);
    assert_eq!(out.status.code(), Some(1), "load of a missing name fails");
}

#[test]
fn still_unwired_commands_report_milestone() {
    let env = TestEnv::new("unwired");
    let out = env.run(&["teardown"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not yet wired: teardown lands at M2-M3"));

    let out = env.run(&["apply", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_str(&stderr(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("not yet wired: apply")
    );
    assert_eq!(value["exit_code"], 1);
    assert!(
        value.get("kind").is_none(),
        "not-wired errors are CLI-composed, no core kind"
    );
}

#[test]
fn plan_json_reports_diff_and_ruleset() {
    let env = TestEnv::new("plan-json");
    env.run_ok(&["block", "fra"]);

    let out = env.run_ok(&["plan", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["game"], "deadlock");
    assert!(
        value["diff"]["to_block"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "fra")
    );
    assert!(
        value["ruleset"]
            .as_str()
            .unwrap()
            .contains("table inet regionlock")
    );
    assert!(
        value["ruleset"]
            .as_str()
            .unwrap()
            .contains("udp daddr @pop_fra drop")
    );
}

#[test]
fn plan_human_output_contains_rendered_ruleset() {
    let env = TestEnv::new("plan-human");
    env.run_ok(&["block", "fra"]);

    let out = env.run_ok(&["plan"]);
    let text = stdout(&out);
    assert!(
        text.contains("to block: 1 (fra)"),
        "summary mentions fra: {text}"
    );
    assert!(
        text.contains("table inet regionlock"),
        "ruleset is rendered: {text}"
    );
    assert!(
        text.contains("udp daddr @pop_fra drop"),
        "fra rule is rendered: {text}"
    );
}

#[test]
fn status_json_reports_no_applied_state() {
    let env = TestEnv::new("status-json");

    let out = env.run_ok(&["status", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(value["applied"].is_null());
}

#[test]
fn status_verify_remains_unwired() {
    let env = TestEnv::new("status-verify");

    let out = env.run(&["status", "--verify"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not yet wired: status --verify lands at M3"));
}

#[test]
fn apply_flag_persists_state_before_failing() {
    let env = TestEnv::new("apply-flag");
    let out = env.run(&["block", "fra", "--apply"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not yet wired: apply lands at M2-M3"));
    assert!(
        env.desired().contains("fra"),
        "state persists even on the not-wired exit"
    );
}

#[test]
fn piped_output_has_no_ansi_escapes() {
    let env = TestEnv::new("no-esc");
    env.run_ok(&["block", "fra"]);
    let commands: [&[&str]; 4] = [
        &["list"],
        &["list", "--regions"],
        &["block", "ams"],
        &["preset", "list"],
    ];
    for args in commands {
        let out = env.run_ok(args);
        assert!(
            !out.stdout.contains(&0x1b),
            "ESC byte in piped stdout of {args:?}"
        );
    }
}

#[test]
fn list_regions_serves_the_full_static_table() {
    let env = TestEnv::new("regions");
    let out = env.run_ok(&["list", "--regions", "--json"]);
    let value: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(value["schema_version"], 1);
    let regions = value["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 15);
    let pops_of = |alias: &str| -> Vec<String> {
        regions
            .iter()
            .find(|r| r["alias"] == alias)
            .unwrap_or_else(|| panic!("alias {alias} missing"))["pops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect()
    };
    assert!(pops_of("eu").contains(&"fra".to_string()));
    // mny1 exists only in the Dota feed: proves the table is the static
    // classification, not derived from the active (Deadlock) feed.
    assert!(pops_of("na").contains(&"mny1".to_string()));
}

#[test]
fn apply_mode_auto_persists_state_then_fails_not_wired() {
    let env = TestEnv::new("automode");
    std::fs::write(&env.config, "apply_mode = \"auto\"\n").unwrap();
    let out = env.run(&["block", "fra"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "auto mode apply is not wired yet"
    );
    assert!(
        env.desired().contains("fra"),
        "state must persist even though the auto-apply step failed"
    );
    let loaded = Config::load(&env.config).unwrap();
    assert!(matches!(
        loaded.apply_mode,
        regionlock_core::config::ApplyMode::Auto
    ));
}
