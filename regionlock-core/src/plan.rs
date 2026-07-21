//! Ruleset specification, plan/diff, and the applied-state journal schema.
//!
//! A [`RulesetSpec`] is the complete desired firewall state derived from
//! (desired POPs ∩ blockable POPs in the feed). The applier consumes specs,
//! never raw nft text. [`AppliedState`] is the journal the applier writes to
//! /run/regionlock/applied.json; `status` reads it unprivileged and
//! `status --verify` diffs it against the live table.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::feed::SdrFeed;
use crate::{Game, SCHEMA_VERSION};

/// Complete target state for one apply: every blocked POP with its relay IPs.
/// BTreeMap keys give deterministic set order for codegen and JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesetSpec {
    pub game: Game,
    /// Feed revision the IPs came from.
    pub revision: u64,
    /// POP code → relay IPv4s. Never contains an empty list: a desired POP
    /// with no relays in the current feed is dropped at spec-build time.
    pub pops: BTreeMap<String, Vec<Ipv4Addr>>,
}

impl RulesetSpec {
    /// Build the spec from config + feed: desired POPs that are blockable in
    /// this feed, with their current relay IPs. Desired codes missing from
    /// the feed (revision drift) are returned in the second tuple slot so
    /// callers can surface them; they are not an error.
    pub fn build(config: &Config, game: Game, feed: &SdrFeed) -> (RulesetSpec, Vec<String>) {
        let (_, _, _) = (config, game, feed);
        todo!("M2I")
    }
}

/// What the applier actually wrote, journaled at /run/regionlock/applied.json.
/// Field removals/renames are breaking: this file is read across versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedState {
    pub schema_version: u32,
    pub game: Game,
    pub revision: u64,
    pub pops: BTreeMap<String, Vec<Ipv4Addr>>,
    /// Unix seconds at apply time (applier clock).
    pub applied_at: u64,
}

impl AppliedState {
    pub const JOURNAL_PATH: &str = "/run/regionlock/applied.json";
    /// Written before nft runs; renamed onto JOURNAL_PATH on commit (M3).
    pub const PENDING_PATH: &str = "/run/regionlock/applied.json.pending";

    pub fn from_spec(spec: &RulesetSpec, applied_at: u64) -> AppliedState {
        AppliedState {
            schema_version: SCHEMA_VERSION,
            game: spec.game,
            revision: spec.revision,
            pops: spec.pops.clone(),
            applied_at,
        }
    }

    /// Read the journal (unprivileged). Ok(None) when absent (nothing
    /// applied since boot).
    pub fn read() -> crate::Result<Option<AppliedState>> {
        todo!("M2I: read + parse JOURNAL_PATH; NotFound → None")
    }
}

/// Diff between a target spec and the journaled applied state.
/// Set-level granularity: a POP appears in `to_update` when it is present in
/// both but its IP list changed (feed revision bump).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PlanDiff {
    pub to_block: Vec<String>,
    pub to_unblock: Vec<String>,
    pub to_update: Vec<String>,
    pub unchanged: Vec<String>,
}

impl PlanDiff {
    /// Compare target vs applied (None = clean slate). All four lists sorted.
    pub fn compute(target: &RulesetSpec, applied: Option<&AppliedState>) -> PlanDiff {
        let (_, _) = (target, applied);
        todo!("M2I")
    }

    pub fn is_empty(&self) -> bool {
        self.to_block.is_empty() && self.to_unblock.is_empty() && self.to_update.is_empty()
    }
}
