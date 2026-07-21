//! Desired-state model: which POPs the user wants blocked, per game.
//! Mutations edit this; `apply` reconciles it against the live table.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::feed::SdrFeed;
use crate::regions::Selector;

/// The user's intent for one game: an explicit set of blocked POP codes.
/// Stored in config.toml, so it must stay human-editable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DesiredState {
    pub blocked: BTreeSet<String>,
}

/// What a mutation changed; mutations print this and hint `regionlock apply`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Delta {
    pub now_blocked: Vec<String>,
    pub now_unblocked: Vec<String>,
}

impl DesiredState {
    /// `block <selectors>`: add every blockable POP the selectors expand to.
    pub fn block(&mut self, selectors: &[Selector], feed: &SdrFeed) -> Result<Delta> {
        let _ = (selectors, feed);
        todo!("M1c")
    }

    /// `unblock <selectors>`.
    pub fn unblock(&mut self, selectors: &[Selector], feed: &SdrFeed) -> Result<Delta> {
        let _ = (selectors, feed);
        todo!("M1c")
    }

    /// `allow <selectors>`: exclusive — block every blockable POP EXCEPT the
    /// expansion of the selectors.
    pub fn allow(&mut self, selectors: &[Selector], feed: &SdrFeed) -> Result<Delta> {
        let _ = (selectors, feed);
        todo!("M1c")
    }

    /// `reset`: clear desired state (never touches the firewall; that is
    /// `teardown`).
    pub fn reset(&mut self) -> Delta {
        todo!("M1c")
    }
}
