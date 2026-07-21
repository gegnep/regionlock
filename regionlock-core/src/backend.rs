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
///         ip daddr @pop_<code> meta l4proto udp drop
///         ...one rule per POP, same order...
///     }
/// }
/// ```
///
/// The match is `ip daddr @set meta l4proto udp`: `daddr` is an IPv4-header
/// field, not a UDP field, so `udp daddr` is invalid nft syntax. `ip daddr`
/// selects the destination address from the ipv4_addr set and
/// `meta l4proto udp` restricts to UDP.
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
        let mut ruleset = String::from("table inet regionlock\ndelete table inet regionlock\n");
        if spec.pops.is_empty() {
            return ruleset;
        }

        ruleset.push_str("table inet regionlock {\n");
        for (code, ips) in &spec.pops {
            let elements = ips
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            ruleset.push_str(&format!(
                "    set pop_{code} {{\n        type ipv4_addr\n        elements = {{ {elements} }}\n    }}\n"
            ));
        }
        ruleset.push_str(
            "    chain out {\n        type filter hook output priority filter; policy accept;\n",
        );
        for code in spec.pops.keys() {
            ruleset.push_str(&format!(
                "        ip daddr @pop_{code} meta l4proto udp drop\n"
            ));
        }
        ruleset.push_str("    }\n}\n");
        ruleset
    }
}
