//! JSON payload shapes: the public API surface behind every `--json` flag.
//! These types ARE the contract documented in docs/json-api.md. Field
//! renames/removals are breaking changes and bump [`crate::SCHEMA_VERSION`].
//! Rendering of these types (tables, color, NDJSON framing) lives in the
//! binaries, never here.

use serde::Serialize;

use crate::regions::Region;

/// `list --json`.
#[derive(Debug, Serialize)]
pub struct ListPayload {
    pub schema_version: u32,
    pub game: String,
    pub revision: u64,
    pub pops: Vec<PopInfo>,
}

#[derive(Debug, Serialize)]
pub struct PopInfo {
    pub code: String,
    pub desc: Option<String>,
    /// Region alias names; empty for unclassified POPs.
    pub regions: Vec<Region>,
    /// False for relay-less POPs (cannot be blocked).
    pub blockable: bool,
    pub relay_count: usize,
    /// Valve's tier field, passed through (user decision Q5).
    pub tier: Option<i64>,
    pub blocked: bool,
    /// Latency in ms with provenance; None until --ping or estimates resolve.
    pub ping: Option<PingValue>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "source", content = "ms", rename_all = "lowercase")]
pub enum PingValue {
    /// Real ICMP measurement.
    Measured(i64),
    /// From the feed's sparse typical_pings table; labeled as estimate.
    Estimate(i64),
    /// No data: renders as "unknown" (user decision Q4). No synthesis.
    Unknown,
}

/// `list --regions --json`: the alias table so wrappers never hardcode it.
#[derive(Debug, Serialize)]
pub struct RegionsPayload {
    pub schema_version: u32,
    pub regions: Vec<RegionInfo>,
}

#[derive(Debug, Serialize)]
pub struct RegionInfo {
    pub alias: Region,
    pub pops: Vec<String>,
}

/// Printed by mutations (block/unblock/allow/reset) under --json.
#[derive(Debug, Serialize)]
pub struct DeltaPayload {
    pub schema_version: u32,
    pub game: String,
    pub now_blocked: Vec<String>,
    pub now_unblocked: Vec<String>,
    pub blocked_total: usize,
    /// True when staged mode left the change unapplied (hint: run apply).
    pub staged: bool,
}

/// `plan --json`: structured diff plus the rendered ruleset (SPEC).
#[derive(Debug, Serialize)]
pub struct PlanPayload {
    pub schema_version: u32,
    pub game: String,
    pub revision: u64,
    pub diff: crate::plan::PlanDiff,
    /// Desired POP codes absent from the current feed (revision drift);
    /// informational, not an error.
    pub missing_from_feed: Vec<String>,
    /// The exact nftables ruleset `apply` would submit.
    pub ruleset: String,
}

/// `status --json`: journaled applied state; `applied` is None when nothing
/// has been applied since boot. M3 extends this with verify/drift fields.
#[derive(Debug, Serialize)]
pub struct StatusPayload {
    pub schema_version: u32,
    pub applied: Option<crate::plan::AppliedState>,
}

/// Structured error on stderr when --json is active.
#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    pub schema_version: u32,
    pub error: String,
    pub kind: &'static str,
    pub exit_code: i32,
}
