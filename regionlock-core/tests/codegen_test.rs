use std::collections::BTreeSet;
use std::fs;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use regionlock_core::Game;
use regionlock_core::backend::{FirewallBackend, NftBackend};
use regionlock_core::config::Config;
use regionlock_core::feed::SdrFeed;
use regionlock_core::plan::RulesetSpec;

fn golden(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.nft"));
    fs::read(path).expect("golden file must exist")
}

fn compare_golden(name: &str, rendered: &str) {
    assert_eq!(
        rendered.as_bytes(),
        golden(name).as_slice(),
        "golden {name}"
    );
    validate_with_nft_when_enabled(name, rendered);
}

fn validate_with_nft_when_enabled(name: &str, rendered: &str) {
    if std::env::var("REGIONLOCK_NFT_CHECK").ok().as_deref() != Some("1") {
        eprintln!("nft --check skipped (set REGIONLOCK_NFT_CHECK=1)");
        return;
    }

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/.regionlock-{name}-{}-{}.nft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, rendered).expect("write nft validation input");
    let output = Command::new("nft")
        .arg("--check")
        .arg("-f")
        .arg(&path)
        .output()
        .expect("nft must be available when REGIONLOCK_NFT_CHECK=1");
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "nft --check failed for {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn empty_spec_matches_golden() {
    let spec = RulesetSpec {
        game: Game::Deadlock,
        revision: 1,
        pops: Default::default(),
    };

    compare_golden("empty", &NftBackend.render(&spec));
}

#[test]
fn one_pop_with_two_ips_matches_golden() {
    let spec = RulesetSpec {
        game: Game::Deadlock,
        revision: 1,
        pops: [(
            "fra".to_string(),
            vec![Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(5, 6, 7, 8)],
        )]
        .into_iter()
        .collect(),
    };

    compare_golden("one-pop", &NftBackend.render(&spec));
}

#[test]
fn three_real_fixture_pops_match_golden() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sdr-1422450.json");
    let feed = SdrFeed::parse(&fs::read(fixture).expect("Deadlock fixture must exist"))
        .expect("Deadlock fixture must parse");
    let mut config = Config::default();
    config.desired_mut(Game::Deadlock).blocked =
        BTreeSet::from(["ams".to_string(), "atl".to_string(), "dfw".to_string()]);
    let (spec, missing) = RulesetSpec::build(&config, Game::Deadlock, &feed);
    assert!(missing.is_empty());

    compare_golden("three-pops", &NftBackend.render(&spec));
}
