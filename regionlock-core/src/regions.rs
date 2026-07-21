//! Region aliases and the static POP→region classification table.
//!
//! The feed carries no region field; classification is ours. Every POP code
//! present in the fixtures (all three games) must appear in [`CLASSIFICATION`]
//! — a test enforces completeness. Unknown future codes classify as
//! [`Classification::Unclassified`]: region aliases never match them and
//! `list` labels them visibly, so they can never be silently unblockable.

use serde::{Deserialize, Serialize};

/// Region aliases fixed by the SPEC. Wrappers read these via
/// `list --regions --json`; never hardcode them elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Na,
    Nae,
    Naw,
    Sa,
    Eu,
    Euw,
    Eue,
    Asia,
    Apac,
    India,
    Jp,
    Kr,
    Oce,
    Me,
    Af,
}

impl Region {
    pub const ALL: [Region; 15] = [
        Region::Na,
        Region::Nae,
        Region::Naw,
        Region::Sa,
        Region::Eu,
        Region::Euw,
        Region::Eue,
        Region::Asia,
        Region::Apac,
        Region::India,
        Region::Jp,
        Region::Kr,
        Region::Oce,
        Region::Me,
        Region::Af,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Region::Na => "na",
            Region::Nae => "nae",
            Region::Naw => "naw",
            Region::Sa => "sa",
            Region::Eu => "eu",
            Region::Euw => "euw",
            Region::Eue => "eue",
            Region::Asia => "asia",
            Region::Apac => "apac",
            Region::India => "india",
            Region::Jp => "jp",
            Region::Kr => "kr",
            Region::Oce => "oce",
            Region::Me => "me",
            Region::Af => "af",
        }
    }
}

impl std::str::FromStr for Region {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Region::ALL.into_iter().find(|r| r.name() == s).ok_or(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    /// Regions this POP belongs to. A POP can be in several
    /// (fra is eu + euw). Broad aliases (eu) contain narrow ones (euw).
    Regions(&'static [Region]),
    /// POP code not in the static table (new Valve POP since this build).
    Unclassified,
}

/// Static classification for every known POP code across all three games.
/// The completeness test walks all three fixture files and fails on any
/// code missing here.
///
/// Sub-region split rules:
/// - nae/naw split at the 100th meridian west (lon -100.0). Central POPs
///   east of it count as nae. Borderline entries carry a comment.
/// - euw/eue split at 15°E. Borderline entries carry a comment.
/// - apac always accompanies asia/oce/india/jp/kr (superset invariant).
/// - India POPs are india, not asia. China partner POPs are asia.
pub const CLASSIFICATION: &[(&str, &[Region])] = &[
    ("ams", &[Region::Eu, Region::Euw]),
    ("ams4", &[Region::Eu, Region::Euw]),
    ("atl", &[Region::Na, Region::Nae]),
    ("bom2", &[Region::Apac, Region::India]),
    ("ctu", &[Region::Asia, Region::Apac]),
    ("ctum", &[Region::Asia, Region::Apac]),
    ("ctut", &[Region::Asia, Region::Apac]),
    ("ctuu", &[Region::Asia, Region::Apac]),
    ("dfw", &[Region::Na, Region::Nae]), // borderline: lon -96.8, central TX east of -100
    ("dfwm", &[Region::Na, Region::Nae]), // borderline: same site as dfw
    ("dxb", &[Region::Me]),
    ("eat", &[Region::Na, Region::Naw]),
    ("eze", &[Region::Sa]),
    ("fra", &[Region::Eu, Region::Euw]),
    ("fsn", &[Region::Eu, Region::Euw]), // borderline: lon 12.4 (Saxony) stays west of 15E
    ("gru", &[Region::Sa]),
    ("gum", &[Region::Apac, Region::Oce]), // Guam: Micronesia, grouped with Oceania
    ("hel", &[Region::Eu, Region::Eue]),
    ("hkg", &[Region::Asia, Region::Apac]),
    ("hkg4", &[Region::Asia, Region::Apac]),
    ("iad", &[Region::Na, Region::Nae]),
    ("jnb", &[Region::Af]),
    ("lax", &[Region::Na, Region::Naw]),
    ("lhr", &[Region::Eu, Region::Euw]),
    ("lim", &[Region::Sa]),
    ("maa2", &[Region::Apac, Region::India]),
    ("mad", &[Region::Eu, Region::Euw]),
    ("mam1", &[Region::Eu, Region::Euw]),
    ("mas1", &[Region::Na, Region::Nae]),
    ("mat1", &[Region::Na, Region::Nae]),
    ("mch1", &[Region::Na, Region::Nae]),
    ("mdb1", &[Region::Me]),
    ("mdc1", &[Region::Na, Region::Nae]),
    ("mdf1", &[Region::Na, Region::Nae]), // borderline: lon -96.8, central TX east of -100
    ("mfr1", &[Region::Eu, Region::Euw]),
    ("mhk1", &[Region::Asia, Region::Apac]),
    ("mla1", &[Region::Na, Region::Naw]),
    ("mln1", &[Region::Eu, Region::Euw]),
    ("mlx1", &[Region::Eu, Region::Euw]),
    ("mmi1", &[Region::Na, Region::Nae]),
    ("mmo1", &[Region::Na, Region::Nae]),
    ("mny1", &[Region::Na, Region::Nae]),
    ("mpx1", &[Region::Na, Region::Naw]),
    ("msa1", &[Region::Na, Region::Naw]),
    ("msg1", &[Region::Asia, Region::Apac]),
    ("msj1", &[Region::Na, Region::Naw]),
    ("msl1", &[Region::Na, Region::Nae]), // borderline: lon -90.2, central MO east of -100
    ("msp1", &[Region::Sa]),
    ("mst1", &[Region::Eu, Region::Euw]),
    ("msy1", &[Region::Apac, Region::Oce]),
    ("mto1", &[Region::Na, Region::Nae]),
    ("mtp1", &[Region::Asia, Region::Apac]),
    ("mty1", &[Region::Asia, Region::Apac, Region::Jp]),
    ("ord", &[Region::Na, Region::Nae]),
    ("par", &[Region::Eu, Region::Euw]),
    ("pek", &[Region::Asia, Region::Apac]),
    ("pekm", &[Region::Asia, Region::Apac]),
    ("pekt", &[Region::Asia, Region::Apac]),
    ("peku", &[Region::Asia, Region::Apac]),
    ("pvg", &[Region::Asia, Region::Apac]),
    ("pvgm", &[Region::Asia, Region::Apac]),
    ("pvgt", &[Region::Asia, Region::Apac]),
    ("pvgu", &[Region::Asia, Region::Apac]),
    ("scl", &[Region::Sa]),
    ("sea", &[Region::Na, Region::Naw]),
    ("seo", &[Region::Asia, Region::Apac, Region::Kr]),
    ("sgp", &[Region::Asia, Region::Apac]),
    ("sto", &[Region::Eu, Region::Eue]), // borderline: lon 17.9, grouped east with waw/hel
    ("sto2", &[Region::Eu, Region::Eue]), // borderline: same site as sto
    ("syd", &[Region::Apac, Region::Oce]),
    ("tgd", &[Region::Asia, Region::Apac]),
    ("tgdm", &[Region::Asia, Region::Apac]),
    ("tgdt", &[Region::Asia, Region::Apac]),
    ("tgdu", &[Region::Asia, Region::Apac]),
    ("tyo", &[Region::Asia, Region::Apac, Region::Jp]),
    ("vie", &[Region::Eu, Region::Eue]), // borderline: lon 16.2, just east of 15E
    ("waw", &[Region::Eu, Region::Eue]),
];

pub fn classify(pop_code: &str) -> Classification {
    CLASSIFICATION
        .iter()
        .find(|(code, _)| *code == pop_code)
        .map(|(_, regions)| Classification::Regions(regions))
        .unwrap_or(Classification::Unclassified)
}

/// A CLI selector: either a POP code or a region alias.
/// `block fra eu` parses as [Pop("fra"), Region(Eu)].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Pop(String),
    Region(Region),
}

/// Parse one selector against the known POP codes of the active feed.
/// Region aliases win over (hypothetical) same-named POPs; unknown input
/// is [`crate::Error::UnknownSelector`].
pub fn parse_selector(input: &str, known_pops: &[&str]) -> crate::Result<Selector> {
    if let Ok(region) = input.parse::<Region>() {
        return Ok(Selector::Region(region));
    }
    if known_pops.contains(&input) {
        return Ok(Selector::Pop(input.to_string()));
    }
    Err(crate::Error::UnknownSelector {
        selector: input.to_string(),
    })
}

/// Expand a selector to blockable POP codes for the active feed.
/// Regions expand through [`classify`]; relay-less and unclassified POPs are
/// never included.
pub fn expand(selector: &Selector, feed: &crate::feed::SdrFeed) -> Vec<String> {
    let mut out = Vec::new();
    match selector {
        Selector::Pop(code) => {
            if feed.blockable_pops().any(|(c, _)| c == code) {
                out.push(code.clone());
            }
        }
        Selector::Region(region) => {
            for (code, _) in feed.blockable_pops() {
                if let Classification::Regions(regions) = classify(code)
                    && regions.contains(region)
                {
                    out.push(code.to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}
