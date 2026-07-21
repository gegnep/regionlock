//! The typed operation schema spoken between the CLI and regionlock-apply.
//!
//! SECURITY BOUNDARY. The applier reads exactly one [`Operation`] from
//! stdin, validates it with [`Operation::validate`], and acts only on
//! `table inet regionlock`. The schema cannot express raw nft syntax, a
//! table name, or a filesystem path. Every field the applier interpolates
//! into anything is validated here first. Keep this module dependency-free
//! beyond serde (the applier compiles core without default features).

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::Game;
use crate::plan::RulesetSpec;

/// Version of the operation wire format. The applier rejects mismatches
/// outright: an old applier must never guess at a newer schema.
pub const OPS_VERSION: u32 = 1;

/// Hard cap on POPs per operation; the live feeds carry ~60 codes, so 512
/// is generous headroom and still rejects absurd payloads.
pub const MAX_POPS: usize = 512;
/// Hard cap on relay IPs per POP (live feeds max at 14).
pub const MAX_IPS_PER_POP: usize = 64;
/// POP codes: short lowercase alphanumerics (live data: 3-4 chars, digits).
pub const MAX_POP_CODE_LEN: usize = 16;

/// One privileged request. Serialized as JSON with an `op` tag.
/// deny_unknown_fields: the boundary rejects payloads carrying anything
/// beyond the schema (e.g. a smuggled "ruleset" or "table" key), instead
/// of silently ignoring it. Tolerant parsing is for the SDR feed, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Replace the entire `table inet regionlock` with the spec's contents
    /// and journal the result. The applier renders the ruleset itself.
    ReplaceRuleset {
        ops_version: u32,
        game: Game,
        revision: u64,
        pops: BTreeMap<String, Vec<Ipv4Addr>>,
    },
    /// Delete `table inet regionlock` and the journal (teardown, orphan
    /// cleanup). Touches neither desired state nor the boot snapshot.
    DeleteTable { ops_version: u32 },
    /// Report the live table (normalized) plus journal state for
    /// `status --verify` and reconciliation. Read-only.
    Inspect { ops_version: u32 },
    // EnablePersist / DisablePersist land at M5; the tag namespace is
    // reserved here so the M3 wire format never shifts.
}

/// Why an operation was refused. The applier prints these verbatim; they
/// are also unit-tested as the boundary's rejection contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    VersionMismatch { got: u32 },
    TooManyPops { got: usize },
    EmptyPopCode,
    PopCodeTooLong { code: String },
    PopCodeBadChar { code: String },
    NoIps { code: String },
    TooManyIps { code: String, got: usize },
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejection::VersionMismatch { got } => {
                write!(f, "ops_version {got} unsupported (expected {OPS_VERSION})")
            }
            Rejection::TooManyPops { got } => {
                write!(f, "{got} POPs exceeds limit {MAX_POPS}")
            }
            Rejection::EmptyPopCode => write!(f, "empty POP code"),
            Rejection::PopCodeTooLong { code } => {
                write!(f, "POP code {code:?} exceeds {MAX_POP_CODE_LEN} chars")
            }
            Rejection::PopCodeBadChar { code } => {
                write!(f, "POP code {code:?} has characters outside [a-z0-9]")
            }
            Rejection::NoIps { code } => write!(f, "POP {code:?} has no IPs"),
            Rejection::TooManyIps { code, got } => {
                write!(f, "POP {code:?} has {got} IPs, limit {MAX_IPS_PER_POP}")
            }
        }
    }
}

impl Operation {
    pub fn ops_version(&self) -> u32 {
        match self {
            Operation::ReplaceRuleset { ops_version, .. }
            | Operation::DeleteTable { ops_version }
            | Operation::Inspect { ops_version } => *ops_version,
        }
    }

    /// The complete admission check the applier runs before acting.
    /// IPv4 syntax is enforced by the type (serde parses `Ipv4Addr`); this
    /// validates everything the type system cannot.
    pub fn validate(&self) -> Result<(), Rejection> {
        if self.ops_version() != OPS_VERSION {
            return Err(Rejection::VersionMismatch {
                got: self.ops_version(),
            });
        }
        let Operation::ReplaceRuleset { pops, .. } = self else {
            return Ok(());
        };
        if pops.len() > MAX_POPS {
            return Err(Rejection::TooManyPops { got: pops.len() });
        }
        for (code, ips) in pops {
            if code.is_empty() {
                return Err(Rejection::EmptyPopCode);
            }
            if code.len() > MAX_POP_CODE_LEN {
                return Err(Rejection::PopCodeTooLong { code: code.clone() });
            }
            if !code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            {
                return Err(Rejection::PopCodeBadChar { code: code.clone() });
            }
            if ips.is_empty() {
                return Err(Rejection::NoIps { code: code.clone() });
            }
            if ips.len() > MAX_IPS_PER_POP {
                return Err(Rejection::TooManyIps {
                    code: code.clone(),
                    got: ips.len(),
                });
            }
        }
        Ok(())
    }

    /// CLI-side constructor: the only sanctioned path from a plan to a
    /// privileged request.
    pub fn replace_from_spec(spec: &RulesetSpec) -> Operation {
        Operation::ReplaceRuleset {
            ops_version: OPS_VERSION,
            game: spec.game,
            revision: spec.revision,
            pops: spec.pops.clone(),
        }
    }
}

/// The applier's stdout reply, JSON, one object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    Applied {
        journal: crate::plan::AppliedState,
    },
    Deleted {
        /// False when there was no table to delete (idempotent success).
        existed: bool,
    },
    Inspected {
        /// Normalized live table state: POP code → sorted IPs. None when
        /// the table does not exist.
        live: Option<BTreeMap<String, Vec<Ipv4Addr>>>,
        journal: Option<crate::plan::AppliedState>,
        /// A pending (uncommitted) journal record was found and reconciled.
        reconciled_pending: bool,
    },
    Refused {
        reason: String,
    },
}
