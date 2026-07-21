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
/// M1b fills this from the fixtures; the completeness test walks all three
/// fixture files and fails on any code missing here.
pub const CLASSIFICATION: &[(&str, &[Region])] = &[
    // M1b: ("fra", &[Region::Eu, Region::Euw]), ... every fixture POP code.
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
    let _ = (input, known_pops);
    todo!("M1b")
}

/// Expand a selector to blockable POP codes for the active feed.
/// Regions expand through [`classify`]; relay-less and unclassified POPs are
/// never included.
pub fn expand(selector: &Selector, feed: &crate::feed::SdrFeed) -> Vec<String> {
    let _ = (selector, feed);
    todo!("M1b")
}
