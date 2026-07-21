//! Fixture-driven tests for SDR feed parsing and the on-disk cache.
//! Fixtures are real GetSDRConfig captures under tests/fixtures/.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use regionlock_core::Game;
use regionlock_core::feed::{SdrFeed, cache};

const DEADLOCK: u32 = 1_422_450;
const CS2: u32 = 730;
const DOTA2: u32 = 570;

fn fixture_bytes(appid: u32) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("sdr-{appid}.json"));
    fs::read(path).expect("fixture must exist")
}

fn fixture(appid: u32) -> SdrFeed {
    SdrFeed::parse(&fixture_bytes(appid)).expect("fixture must parse")
}

fn total_relay_ips(feed: &SdrFeed) -> usize {
    feed.pops
        .values()
        .filter_map(|p| p.relays.as_ref())
        .map(Vec::len)
        .sum()
}

fn relayless_codes(feed: &SdrFeed) -> BTreeSet<&str> {
    feed.pops
        .iter()
        .filter(|(_, p)| p.relays.as_ref().is_none_or(Vec::is_empty))
        .map(|(code, _)| code.as_str())
        .collect()
}

/// Unique tempdir per test. No env mutation (unsafe in edition 2024).
fn tempdir(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "regionlock-feed-test-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn all_three_fixtures_parse() {
    for appid in [DEADLOCK, CS2, DOTA2] {
        let feed = fixture(appid);
        assert!(feed.revision > 0, "appid {appid}: revision missing");
        assert!(!feed.pops.is_empty(), "appid {appid}: no POPs parsed");
    }
}

#[test]
fn deadlock_pop_and_relay_counts() {
    let feed = fixture(DEADLOCK);
    assert_eq!(feed.pops.len(), 32);
    assert_eq!(feed.blockable_pops().count(), 29);
    assert_eq!(
        relayless_codes(&feed),
        BTreeSet::from(["eat", "fsn", "hel"])
    );
    assert_eq!(total_relay_ips(&feed), 141);
}

#[test]
fn cs2_pop_and_relay_counts() {
    let feed = fixture(CS2);
    assert_eq!(feed.pops.len(), 48);
    assert_eq!(total_relay_ips(&feed), 210);
}

#[test]
fn dota2_pop_and_relay_counts() {
    let feed = fixture(DOTA2);
    assert_eq!(feed.pops.len(), 61);
    assert_eq!(total_relay_ips(&feed), 141);
    assert_eq!(relayless_codes(&feed).len(), 32);
}

#[test]
fn typical_pings_sparse_lookup() {
    let feed = fixture(DEADLOCK);
    assert_eq!(feed.estimate_ms("ams", "fra"), Some(6));
    // Reversed lookup matches the same entry.
    assert_eq!(feed.estimate_ms("fra", "ams"), Some(6));
    // ams↔atl is absent from the sparse table (verified in the capture).
    assert_eq!(feed.estimate_ms("ams", "atl"), None);
    assert_eq!(feed.estimate_ms("atl", "ams"), None);
}

#[test]
fn pop_geo_is_lon_lat_and_tier_parses() {
    let feed = fixture(DEADLOCK);
    let fra = feed.pops.get("fra").expect("fra POP");
    let [lon, lat] = fra.geo.expect("fra geo");
    assert!((lon - 8.68).abs() < 0.01, "fra lon {lon}");
    assert!((lat - 50.12).abs() < 0.01, "fra lat {lat}");
    assert_eq!(fra.tier, Some(0));
    assert_eq!(feed.pops.get("ams").and_then(|p| p.tier), Some(1));
}

#[test]
fn cache_round_trip_picks_highest_revision() {
    let dir = tempdir("roundtrip");
    // In production the filename revision always matches the body's
    // `revision` field (acquire stores under feed.revision); keep that
    // invariant here. The Deadlock capture has revision 1784582254.
    let rev1: u64 = 1_784_582_254;
    let rev2: u64 = 1_784_582_255;

    let stored =
        cache::store_in(&dir, Game::Deadlock, rev1, &fixture_bytes(DEADLOCK)).expect("store rev1");
    assert_eq!(stored, dir.join(format!("{DEADLOCK}-{rev1}.json")));
    let latest = cache::load_latest_in(&dir, Game::Deadlock)
        .expect("load_latest rev1")
        .expect("deadlock feed cached");
    assert_eq!(latest.revision, rev1);
    assert_eq!(latest.pops.len(), 32, "fixture body must round-trip intact");

    // A higher revision wins even when it is a different, smaller body.
    let body2 = format!(r#"{{"revision":{rev2},"pops":{{}}}}"#);
    cache::store_in(&dir, Game::Deadlock, rev2, body2.as_bytes()).expect("store rev2");
    let latest = cache::load_latest_in(&dir, Game::Deadlock)
        .expect("load_latest rev2")
        .expect("deadlock feed cached");
    assert_eq!(latest.revision, rev2);

    // Unrelated files are tolerated and other games stay separate.
    fs::write(dir.join("notes.txt"), b"not a feed").unwrap();
    fs::write(dir.join("1422450-abc.json"), b"{}").unwrap();
    cache::store_in(&dir, Game::Cs2, 50, br#"{"revision":50,"pops":{}}"#)
        .expect("store cs2 rev 50");
    let latest = cache::load_latest_in(&dir, Game::Deadlock)
        .expect("load_latest with junk")
        .expect("deadlock feed cached");
    assert_eq!(latest.revision, rev2);
    let cs2 = cache::load_latest_in(&dir, Game::Cs2)
        .expect("load_latest cs2")
        .expect("cs2 feed cached");
    assert_eq!(cs2.revision, 50);
    assert!(
        cache::load_latest_in(&dir, Game::Dota2)
            .expect("load_latest dota2")
            .is_none()
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cache_empty_dir_is_none() {
    let dir = tempdir("empty");
    assert!(
        cache::load_latest_in(&dir, Game::Deadlock)
            .expect("load_latest on empty dir")
            .is_none()
    );
    fs::remove_dir_all(&dir).unwrap();
}
