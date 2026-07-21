use serde::{Deserialize, Serialize};

/// Supported games. Multi-game support is only an appid switch (SPEC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Game {
    Deadlock,
    Cs2,
    Dota2,
}

impl Game {
    pub const ALL: [Game; 3] = [Game::Deadlock, Game::Cs2, Game::Dota2];

    pub fn appid(self) -> u32 {
        match self {
            Game::Deadlock => 1_422_450,
            Game::Cs2 => 730,
            Game::Dota2 => 570,
        }
    }

    /// Canonical lowercase name, used in config, CLI values, and JSON.
    pub fn name(self) -> &'static str {
        match self {
            Game::Deadlock => "deadlock",
            Game::Cs2 => "cs2",
            Game::Dota2 => "dota2",
        }
    }
}

impl std::fmt::Display for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Game {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "deadlock" => Ok(Game::Deadlock),
            "cs2" => Ok(Game::Cs2),
            "dota2" => Ok(Game::Dota2),
            other => Err(format!("unknown game {other:?} (deadlock|cs2|dota2)")),
        }
    }
}

/// One resolver for the active game, threaded through every command:
/// CLI `--game` flag wins over the configured default.
pub fn resolve(flag: Option<Game>, config_default: Game) -> Game {
    flag.unwrap_or(config_default)
}
