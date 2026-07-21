use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv4Addr;
use std::path::PathBuf;

use regionlock_core::Game;
use regionlock_core::config::Config;
use regionlock_core::feed::SdrFeed;
use regionlock_core::plan::{AppliedState, PlanDiff, RulesetSpec};

fn fixture() -> SdrFeed {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sdr-1422450.json");
    SdrFeed::parse(&fs::read(path).expect("Deadlock fixture must exist"))
        .expect("Deadlock fixture must parse")
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

fn spec(pops: &[(&str, &[&str])]) -> RulesetSpec {
    RulesetSpec {
        game: Game::Deadlock,
        revision: 1,
        pops: pops
            .iter()
            .map(|(code, ips)| {
                (
                    (*code).to_string(),
                    ips.iter()
                        .map(|ip| ip.parse::<Ipv4Addr>().expect("valid test IP"))
                        .collect(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn build_filters_relayless_and_missing_desired_pops() {
    let feed = fixture();
    let mut config = Config::default();
    config.desired_mut(Game::Deadlock).blocked = ["fra", "hel", "missing"]
        .into_iter()
        .map(str::to_string)
        .collect();

    let (target, missing) = RulesetSpec::build(&config, Game::Deadlock, &feed);

    assert_eq!(target.game, Game::Deadlock);
    assert_eq!(target.revision, feed.revision);
    assert_eq!(target.pops.len(), 1);
    assert_eq!(target.pops["fra"], feed.relay_ips("fra"));
    assert_eq!(missing, strings(&["hel", "missing"]));
}

#[test]
fn clean_slate_blocks_every_target_pop() {
    let target = spec(&[("fra", &["1.2.3.4"]), ("ams", &["5.6.7.8"])]);

    let diff = PlanDiff::compute(&target, None);

    assert_eq!(diff.to_block, strings(&["ams", "fra"]));
    assert!(diff.to_unblock.is_empty());
    assert!(diff.to_update.is_empty());
    assert!(diff.unchanged.is_empty());
}

#[test]
fn identical_applied_state_is_a_noop() {
    let target = spec(&[("fra", &["1.2.3.4"]), ("ams", &["5.6.7.8"])]);
    let applied = AppliedState::from_spec(&target, 42);

    let diff = PlanDiff::compute(&target, Some(&applied));

    assert!(diff.is_empty());
    assert_eq!(diff.unchanged, strings(&["ams", "fra"]));
}

#[test]
fn diff_reports_block_and_unblock_mix() {
    let target = spec(&[("ams", &["5.6.7.8"]), ("fra", &["1.2.3.4"])]);
    let applied = spec(&[("ams", &["5.6.7.8"]), ("par", &["9.10.11.12"])]);
    let applied = AppliedState::from_spec(&applied, 42);

    let diff = PlanDiff::compute(&target, Some(&applied));

    assert_eq!(diff.to_block, strings(&["fra"]));
    assert_eq!(diff.to_unblock, strings(&["par"]));
    assert_eq!(diff.unchanged, strings(&["ams"]));
    assert!(diff.to_update.is_empty());
}

#[test]
fn changed_ip_list_is_an_update() {
    let target = spec(&[("ams", &["5.6.7.8"]), ("fra", &["1.2.3.5"])]);
    let applied = spec(&[("ams", &["5.6.7.8"]), ("fra", &["1.2.3.4"])]);
    let applied = AppliedState::from_spec(&applied, 42);

    let diff = PlanDiff::compute(&target, Some(&applied));

    assert_eq!(diff.to_update, strings(&["fra"]));
    assert_eq!(diff.unchanged, strings(&["ams"]));
    assert!(diff.to_block.is_empty());
    assert!(diff.to_unblock.is_empty());
}

#[test]
fn applied_state_serializes_and_parses_round_trip() {
    let target = spec(&[("fra", &["1.2.3.4", "5.6.7.8"])]);
    let applied = AppliedState::from_spec(&target, 1_784_582_254);
    let bytes = serde_json::to_vec(&applied).expect("AppliedState serializes");

    let parsed = AppliedState::parse(&bytes).expect("AppliedState parses");

    assert_eq!(parsed, applied);
}

#[test]
fn applied_state_parse_error_names_journal_path() {
    let err = AppliedState::parse(b"{not json").unwrap_err();
    assert_eq!(err.kind(), "journal_parse");
    assert!(
        err.to_string().contains(AppliedState::JOURNAL_PATH),
        "parse errors name the journal path: {err}"
    );
}
