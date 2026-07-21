//! User config: declarative desired state, dotfile-able.
//! Resolution order: `--config` > `$REGIONLOCK_CONFIG` > user XDG >
//! /etc/regionlock/config.toml.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::DesiredState;
use crate::{Game, Result};

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
    /// Per-game desired state and presets (user decision Q3: per-game).
    pub games: BTreeMap<Game, GameConfig>,
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
        let _ = flag;
        todo!("M1c")
    }

    /// Load from the resolved path; a missing file yields `Config::default()`.
    pub fn load(path: &Path) -> Result<Config> {
        let _ = path;
        todo!("M1c")
    }

    /// Atomic write (tmp + rename) with a stable field order.
    pub fn save(&self, path: &Path) -> Result<()> {
        let _ = path;
        todo!("M1c")
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
