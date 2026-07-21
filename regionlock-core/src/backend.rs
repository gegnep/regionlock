//! Firewall backend seam (SPEC requirement). nftables is the only v1
//! implementation; the trait exists so iptables can be added without core
//! surgery. Do NOT implement a second backend yet.

use crate::plan::RulesetSpec;

/// Renders a [`RulesetSpec`] to backend-native ruleset text. Rendering is
/// pure string generation: no privileges, no process execution. Applying the
/// text is the applier's job (M3).
pub trait FirewallBackend {
    /// Human-visible backend name ("nftables").
    fn name(&self) -> &'static str;

    /// Full replacement ruleset for the spec. Deterministic: equal specs
    /// render byte-identical output (golden-file tested).
    fn render(&self, spec: &RulesetSpec) -> String;
}

/// The nftables backend. Output contract (golden-file tested, M2I):
///
/// ```text
/// table inet regionlock
/// delete table inet regionlock
/// table inet regionlock {
///     set pop_<code> {
///         type ipv4_addr
///         elements = { <ip>, <ip>, ... }
///     }
///     ...one set per POP, feed order (BTreeMap iteration)...
///     chain out {
///         type filter hook output priority filter; policy accept;
///         udp daddr @pop_<code> drop
///         ...one rule per POP, same order...
///     }
/// }
/// ```
///
/// The leading `table` + `delete table` pair makes the script idempotent
/// under `nft -f -` whether or not the table exists (nft cannot delete a
/// nonexistent table without the preceding declaration). Empty spec (no
/// blocked POPs) renders only the declare+delete pair: applying it removes
/// the table entirely.
pub struct NftBackend;

impl FirewallBackend for NftBackend {
    fn name(&self) -> &'static str {
        "nftables"
    }

    fn render(&self, spec: &RulesetSpec) -> String {
        let _ = spec;
        todo!("M2I")
    }
}
