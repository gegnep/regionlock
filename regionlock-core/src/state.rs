//! Desired-state model: which POPs the user wants blocked, per game.
//! Mutations edit this; `apply` reconciles it against the live table.
//!
//! Mutations take POP code lists that the caller already expanded and
//! validated through [`crate::regions::expand`]. This keeps the state model
//! pure set logic with no feed or selector dependency.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The user's intent for one game: an explicit set of blocked POP codes.
/// Stored in config.toml, so it must stay human-editable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DesiredState {
    pub blocked: BTreeSet<String>,
}

/// What a mutation changed; mutations print this and hint `regionlock apply`.
/// Both lists stay sorted (BTreeSet iteration order).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Delta {
    pub now_blocked: Vec<String>,
    pub now_unblocked: Vec<String>,
}

impl DesiredState {
    /// `block`: add the given POPs. The delta lists only POPs that were not
    /// already blocked.
    pub fn block(&mut self, pops: &[String]) -> Delta {
        let _ = pops;
        todo!("M1c")
    }

    /// `unblock`: remove the given POPs. The delta lists only POPs that were
    /// actually blocked.
    pub fn unblock(&mut self, pops: &[String]) -> Delta {
        let _ = pops;
        todo!("M1c")
    }

    /// `allow` (exclusive): block every POP in `all_blockable` except those
    /// in `keep`. The delta reflects the difference from the previous state.
    pub fn allow(&mut self, keep: &[String], all_blockable: &[String]) -> Delta {
        let _ = (keep, all_blockable);
        todo!("M1c")
    }

    /// `reset`: clear desired state (never touches the firewall; that is
    /// `teardown`).
    pub fn reset(&mut self) -> Delta {
        todo!("M1c")
    }
}
