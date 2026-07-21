//! M1c: config load/save round-trips, XDG resolution precedence.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use regionlock_core::Error;
use regionlock_core::Game;
use regionlock_core::config::{ApplyMode, Config, Escalator, GameConfig};
use regionlock_core::state::DesiredState;

/// A fresh, unique scratch directory under `std::env::temp_dir()`. No
/// tempfile crate available (M1c constraint); a per-process id plus a
/// monotonic counter keeps parallel test threads from colliding.
fn unique_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "regionlock-config-test-{}-{label}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn default_round_trips_through_save_and_load() {
    let dir = unique_dir("default");
    let path = dir.join("config.toml");

    let config = Config::default();
    config.save(&path).expect("save");
    let loaded = Config::load(&path).expect("load");

    assert_eq!(loaded.default_game, config.default_game);
    assert_eq!(loaded.apply_mode, config.apply_mode);
    assert_eq!(loaded.escalator, config.escalator);
    assert!(loaded.games.is_empty());
}

#[test]
fn populated_config_survives_save_and_load_exactly() {
    let dir = unique_dir("populated");
    let path = dir.join("config.toml");

    let mut config = Config {
        default_game: Game::Cs2,
        apply_mode: ApplyMode::Auto,
        escalator: Escalator::Doas,
        ..Config::default()
    };

    let mut deadlock = GameConfig {
        desired: DesiredState {
            blocked: BTreeSet::from(["fra".to_string(), "ams".to_string()]),
        },
        ..GameConfig::default()
    };
    deadlock.presets.insert(
        "eu-only".to_string(),
        DesiredState {
            blocked: BTreeSet::from(["par".to_string()]),
        },
    );
    config.games.insert(Game::Deadlock, deadlock);

    let dota2 = GameConfig {
        desired: DesiredState {
            blocked: BTreeSet::from(["sin".to_string()]),
        },
        ..GameConfig::default()
    };
    config.games.insert(Game::Dota2, dota2);

    config.save(&path).expect("save");
    let loaded = Config::load(&path).expect("load");

    // Debug output covers every field exactly without adding PartialEq to
    // the frozen public API.
    assert_eq!(format!("{loaded:?}"), format!("{config:?}"));
}

#[test]
fn load_of_missing_path_yields_default() {
    let dir = unique_dir("missing");
    let path = dir.join("does-not-exist.toml");

    let loaded = Config::load(&path).expect("missing file loads as default");
    assert_eq!(loaded.default_game, Config::default().default_game);
    assert!(loaded.games.is_empty());
}

#[test]
fn malformed_toml_is_a_config_error() {
    let dir = unique_dir("malformed");
    let path = dir.join("config.toml");
    std::fs::write(&path, "this is not [ valid toml").expect("write malformed file");

    let err = Config::load(&path).expect_err("malformed toml must error");
    match err {
        Error::Config {
            path: err_path,
            reason,
        } => {
            assert_eq!(err_path, path);
            assert!(!reason.is_empty());
        }
        other => panic!("expected Error::Config, got {other:?}"),
    }
}

#[test]
fn save_is_atomic_no_leftover_tmp_file() {
    let dir = unique_dir("atomic");
    let path = dir.join("config.toml");

    Config::default().save(&path).expect("save");

    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .map(|e| e.expect("dir entry").file_name())
        .collect();
    assert_eq!(entries, vec![std::ffi::OsString::from("config.toml")]);
}

#[test]
fn resolve_path_with_precedence() {
    let dir = unique_dir("resolve");

    let flag_path = dir.join("flag.toml");
    let env_path = dir.join("env.toml");
    let xdg_path = dir.join("xdg.toml");
    let etc_path = dir.join("etc.toml");

    std::fs::write(&flag_path, "").unwrap();
    std::fs::write(&env_path, "").unwrap();
    std::fs::write(&xdg_path, "").unwrap();
    std::fs::write(&etc_path, "").unwrap();

    // flag wins over everything else, even when all exist.
    assert_eq!(
        Config::resolve_path_with(Some(&flag_path), Some(&env_path), &xdg_path, &etc_path),
        flag_path
    );

    // Without a flag, env wins over xdg.
    assert_eq!(
        Config::resolve_path_with(None, Some(&env_path), &xdg_path, &etc_path),
        env_path
    );

    // Without flag or env, xdg wins (when it exists).
    assert_eq!(
        Config::resolve_path_with(None, None, &xdg_path, &etc_path),
        xdg_path
    );

    // xdg missing, etc present: etc wins.
    let missing_xdg = dir.join("no-such-xdg.toml");
    assert_eq!(
        Config::resolve_path_with(None, None, &missing_xdg, &etc_path),
        etc_path
    );

    // Explicit overrides win even when the file does not exist yet: a first
    // run with --config (or $REGIONLOCK_CONFIG) must write there, never
    // fall through to XDG.
    let missing_flag = dir.join("no-such-flag.toml");
    let missing_env = dir.join("no-such-env.toml");
    let missing_etc = dir.join("no-such-etc.toml");
    assert_eq!(
        Config::resolve_path_with(
            Some(&missing_flag),
            Some(&missing_env),
            &xdg_path,
            &etc_path
        ),
        missing_flag
    );
    assert_eq!(
        Config::resolve_path_with(None, Some(&missing_env), &xdg_path, &etc_path),
        missing_env
    );

    // No overrides and nothing exists: the xdg path is the write target.
    assert_eq!(
        Config::resolve_path_with(None, None, &missing_xdg, &missing_etc),
        missing_xdg
    );
}

#[test]
fn save_creates_nested_parent_directories() {
    let dir = unique_dir("nested");
    let path = dir.join("a").join("b").join("config.toml");

    Config::default()
        .save(&path)
        .expect("save into missing parents");
    assert!(path.is_file());
}

#[test]
fn resolve_path_wrapper_returns_a_usable_write_target() {
    // Smoke test of the real wrapper: no flag, and whatever the host env
    // holds, the result must end in config.toml and be absolute.
    let path = Config::resolve_path(None).expect("resolve");
    assert!(path.is_absolute());
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some("config.toml")
    );
}
