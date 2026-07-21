//! User config: declarative desired state, dotfile-able.
//! Resolution order: `--config` > `$REGIONLOCK_CONFIG` > user XDG >
//! /etc/regionlock/config.toml.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use etcetera::base_strategy::{BaseStrategy, choose_base_strategy};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::state::DesiredState;
use crate::{Game, Result};

/// Last resort in the resolution order (SPEC: XDG layout).
const ETC_CONFIG_PATH: &str = "/etc/regionlock/config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyMode {
    #[default]
    Staged,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Escalator {
    /// Try pkexec (with pkttyagent fallback), then sudo, then doas, then run0.
    #[default]
    Auto,
    Pkexec,
    Sudo,
    Doas,
    Run0,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_game: Game,
    pub apply_mode: ApplyMode,
    pub escalator: Escalator,
    /// Reference POP for latency estimates when live probing is unavailable
    /// (see ping module). None = no estimates, values show as unknown.
    pub home_pop: Option<String>,
    /// Per-game desired state and presets (user decision Q3: per-game).
    /// TOML tables require string keys; `Game` is an enum, so this field
    /// round-trips through `Game::name()` / `Game::from_str` instead of
    /// serde's default enum-key encoding (see `game_map` below).
    #[serde(with = "game_map")]
    pub games: BTreeMap<Game, GameConfig>,
}

/// (De)serializes `BTreeMap<Game, GameConfig>` as a string-keyed TOML table,
/// since `toml` rejects non-string map keys. The public field type stays
/// `BTreeMap<Game, GameConfig>`; only the wire encoding changes.
mod game_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::GameConfig;
    use crate::Game;

    pub fn serialize<S>(map: &BTreeMap<Game, GameConfig>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let by_name: BTreeMap<&'static str, &GameConfig> =
            map.iter().map(|(game, cfg)| (game.name(), cfg)).collect();
        by_name.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<Game, GameConfig>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let by_name: BTreeMap<String, GameConfig> = BTreeMap::deserialize(deserializer)?;
        by_name
            .into_iter()
            .map(|(name, cfg)| {
                name.parse::<Game>()
                    .map(|game| (game, cfg))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GameConfig {
    pub desired: DesiredState,
    /// Preset name → saved desired state.
    pub presets: BTreeMap<String, DesiredState>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            default_game: Game::Deadlock,
            apply_mode: ApplyMode::default(),
            escalator: Escalator::default(),
            home_pop: None,
            games: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Resolve the config path: explicit flag > $REGIONLOCK_CONFIG >
    /// ~/.config/regionlock/config.toml > /etc/regionlock/config.toml.
    /// Returns the first path that exists, or the user XDG path (for writes)
    /// when none do.
    pub fn resolve_path(flag: Option<&Path>) -> Result<PathBuf> {
        let env_value = std::env::var_os("REGIONLOCK_CONFIG").map(PathBuf::from);
        // Explicit overrides win unconditionally, existing or not: a first
        // run with --config must write there, not fall through to XDG. This
        // also keeps XDG discovery failure from defeating an override.
        if let Some(explicit) = flag.or(env_value.as_deref()) {
            return Ok(explicit.to_path_buf());
        }
        let xdg_path = Self::user_xdg_path()?;
        Ok(Self::resolve_path_with(
            flag,
            env_value.as_deref(),
            &xdg_path,
            Path::new(ETC_CONFIG_PATH),
        ))
    }

    /// The user XDG config path (`<XDG config>/regionlock/config.toml`),
    /// via etcetera's base strategy.
    fn user_xdg_path() -> Result<PathBuf> {
        let strategy = choose_base_strategy().map_err(|e| Error::Config {
            path: PathBuf::from("<home dir>"),
            reason: e.to_string(),
        })?;
        Ok(strategy.config_dir().join("regionlock").join("config.toml"))
    }

    /// Testability exception (M1c): resolution logic factored out of
    /// [`Config::resolve_path`] so tests can inject `env_value` and
    /// `xdg_path` directly instead of mutating process env (unsafe on
    /// edition 2024) or the real home directory.
    ///
    /// Precedence: `flag` and `env_value` are explicit overrides and win
    /// unconditionally (existing or not — a first run must write to them).
    /// Otherwise `xdg_path` wins when it exists, then `etc_path` when it
    /// exists (production passes `/etc/regionlock/config.toml`; tests inject
    /// their own so results never depend on host state), else `xdg_path` as
    /// the write target.
    pub fn resolve_path_with(
        flag: Option<&Path>,
        env_value: Option<&Path>,
        xdg_path: &Path,
        etc_path: &Path,
    ) -> PathBuf {
        if let Some(explicit) = flag.or(env_value) {
            return explicit.to_path_buf();
        }
        for candidate in [xdg_path, etc_path] {
            if candidate.exists() {
                return candidate.to_path_buf();
            }
        }
        xdg_path.to_path_buf()
    }

    /// Load from the resolved path; a missing file yields `Config::default()`.
    pub fn load(path: &Path) -> Result<Config> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(e) => {
                return Err(Error::Io {
                    path: path.to_path_buf(),
                    source: e,
                });
            }
        };
        toml::from_str(&contents).map_err(|e| Error::Config {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })
    }

    /// Atomic write (tmp + rename) with a stable field order.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        let serialized = toml::to_string_pretty(self).map_err(|e| Error::Config {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        // pid + counter keeps concurrent saves (across and within processes)
        // off each other's temp file; failed renames clean up after
        // themselves so no `.tmp` litter survives.
        static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.toml".to_string());
        let tmp_path = path.with_file_name(format!("{file_name}.{}.{seq}.tmp", std::process::id()));

        std::fs::write(&tmp_path, serialized).map_err(|e| Error::Io {
            path: tmp_path.clone(),
            source: e,
        })?;
        std::fs::rename(&tmp_path, path)
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp_path);
            })
            .map_err(|e| Error::Io {
                path: path.to_path_buf(),
                source: e,
            })?;
        Ok(())
    }

    /// Mutable desired state for a game, creating the section on first use.
    pub fn desired_mut(&mut self, game: Game) -> &mut DesiredState {
        &mut self.games.entry(game).or_default().desired
    }

    pub fn desired(&self, game: Game) -> DesiredState {
        self.games
            .get(&game)
            .map(|g| g.desired.clone())
            .unwrap_or_default()
    }
}
