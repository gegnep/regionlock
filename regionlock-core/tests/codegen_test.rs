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

/// Validate the rendered ruleset with `nft --check` wherever nft can run.
///
/// By default this runs automatically and only skips when nft is genuinely
/// unable to validate: absent from PATH, or unable to init its netlink cache
/// (the unprivileged dev sandbox). Skips print a visible note. Set
/// REGIONLOCK_NFT_CHECK=1 for strict mode, which turns those skips into hard
/// failures — use it in any environment where nft is expected to work (a real
/// host, the privileged e2e), so an invalid ruleset can never pass silently.
///
/// The earlier opt-in gate let invalid nft syntax (`udp daddr ...`) pass the
/// byte-golden tests because nobody set the env var; running by default
/// closes that gap.
fn validate_with_nft_when_enabled(name: &str, rendered: &str) {
    let strict = std::env::var("REGIONLOCK_NFT_CHECK").ok().as_deref() == Some("1");

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/.regionlock-{name}-{}-{}.nft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, rendered).expect("write nft validation input");
    let result = Command::new("nft")
        .arg("--check")
        .arg("-f")
        .arg(&path)
        .output();
    let _ = fs::remove_file(&path);

    let output = match result {
        Ok(output) => output,
        Err(e) => {
            let msg = format!("nft --check skipped for {name}: nft not runnable ({e})");
            assert!(!strict, "{msg}");
            eprintln!("{msg}");
            return;
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // netlink cache init needs privileges/namespaces; that is an
        // environment limit, not a ruleset error, so skip unless strict.
        if !strict && stderr.contains("cache initialization failed") {
            eprintln!("nft --check skipped for {name}: netlink unavailable in this environment");
            return;
        }
        panic!("nft --check failed for {name}: {stderr}");
    }
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
