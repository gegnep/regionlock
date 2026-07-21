//! Tests for the POP→region classification table, selector parsing, and
//! selector expansion against the live-feed fixtures.

use regionlock_core::Error;
use regionlock_core::feed::SdrFeed;
use regionlock_core::regions::{
    CLASSIFICATION, Classification, Region, Selector, classify, expand, parse_selector,
};

const DEADLOCK: &[u8] = include_bytes!("fixtures/sdr-1422450.json");
const CS2: &[u8] = include_bytes!("fixtures/sdr-730.json");
const DOTA2: &[u8] = include_bytes!("fixtures/sdr-570.json");

fn deadlock() -> SdrFeed {
    SdrFeed::parse(DEADLOCK).expect("deadlock fixture parses")
}

/// Hard gate: every POP code in any of the three fixtures is classified,
/// and an unknown code falls through to Unclassified.
#[test]
fn classification_covers_every_fixture_pop() {
    for (name, bytes) in [("deadlock", DEADLOCK), ("cs2", CS2), ("dota2", DOTA2)] {
        let feed = SdrFeed::parse(bytes).unwrap_or_else(|e| panic!("{name} fixture parses: {e}"));
        for code in feed.pops.keys() {
            assert!(
                matches!(classify(code), Classification::Regions(_)),
                "{name} POP {code:?} missing from CLASSIFICATION"
            );
        }
    }
    assert!(matches!(classify("zzz"), Classification::Unclassified));
}

/// The table upholds the documented region invariants.
#[test]
fn classification_invariants_hold() {
    use Region::*;
    for &(code, regions) in CLASSIFICATION {
        if regions.contains(&Na) {
            assert!(
                regions.contains(&Nae) || regions.contains(&Naw),
                "{code}: na without nae/naw"
            );
        }
        if regions.contains(&Eu) {
            assert!(
                regions.contains(&Euw) || regions.contains(&Eue),
                "{code}: eu without euw/eue"
            );
        }
        if regions.contains(&Jp) || regions.contains(&Kr) {
            assert!(regions.contains(&Asia), "{code}: jp/kr without asia");
        }
        if regions.contains(&India) {
            assert!(!regions.contains(&Asia), "{code}: india must not be asia");
        }
        // Apac is a superset of asia/oce/india/jp/kr membership.
        if [Asia, Oce, India, Jp, Kr]
            .iter()
            .any(|r| regions.contains(r))
        {
            assert!(regions.contains(&Apac), "{code}: apac member missing apac");
        }
    }
    // Table stays sorted by code.
    assert!(
        CLASSIFICATION.windows(2).all(|w| w[0].0 < w[1].0),
        "CLASSIFICATION must be sorted by code"
    );
}

/// Deadlock: eu covers euw ∪ eue, contains fra, and skips the relay-less
/// eu-classified hel.
#[test]
fn expand_eu_covers_subregions_and_skips_relay_less() {
    let feed = deadlock();
    let eu = expand(&Selector::Region(Region::Eu), &feed);
    let euw = expand(&Selector::Region(Region::Euw), &feed);
    let eue = expand(&Selector::Region(Region::Eue), &feed);
    assert!(eu.contains(&"fra".to_string()), "eu contains fra");
    for code in euw.iter().chain(eue.iter()) {
        assert!(eu.contains(code), "{code} in euw/eue but missing from eu");
    }
    assert!(
        matches!(classify("hel"), Classification::Regions(rs) if rs.contains(&Region::Eu)),
        "hel classifies as eu"
    );
    assert!(!eu.contains(&"hel".to_string()), "relay-less hel excluded");
}

#[test]
fn parse_selector_resolution_order() {
    // A region alias wins even over a same-named POP code.
    let sel = parse_selector("eu", &["eu", "fra"]).expect("eu resolves");
    assert_eq!(sel, Selector::Region(Region::Eu));

    let sel = parse_selector("fra", &["fra"]).expect("fra resolves");
    assert_eq!(sel, Selector::Pop("fra".to_string()));

    let err = parse_selector("nope", &["fra"]).expect_err("nope is unknown");
    assert!(
        matches!(err, Error::UnknownSelector { ref selector } if selector == "nope"),
        "nope yields UnknownSelector, got {err:?}"
    );
}

#[test]
fn expand_relay_less_pop_is_empty() {
    let feed = deadlock();
    // eat is relay-less in the Deadlock feed.
    assert!(expand(&Selector::Pop("eat".to_string()), &feed).is_empty());
}
