use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use regionlock_core::Game;
use regionlock_core::ops::{
    MAX_IPS_PER_POP, MAX_POPS, MAX_SNAPSHOT_BYTES, OPS_VERSION, Operation, Rejection, Reply,
    parse_feed_snapshot_filename,
};
use regionlock_core::plan::{AppliedState, RulesetSpec};

fn valid_pops() -> BTreeMap<String, Vec<Ipv4Addr>> {
    BTreeMap::from([
        (
            "fra".to_string(),
            vec![Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(192, 0, 2, 2)],
        ),
        ("ams".to_string(), vec![Ipv4Addr::new(198, 51, 100, 1)]),
    ])
}

fn replace_with_pops(pops: BTreeMap<String, Vec<Ipv4Addr>>) -> Operation {
    Operation::ReplaceRuleset {
        ops_version: OPS_VERSION,
        game: Game::Deadlock,
        revision: 42,
        pops,
    }
}

fn replace_with_code(code: &str, ips: Vec<Ipv4Addr>) -> Operation {
    replace_with_pops(BTreeMap::from([(code.to_string(), ips)]))
}

fn valid_enable_persist() -> Operation {
    Operation::EnablePersist {
        ops_version: OPS_VERSION,
        config_toml: "default_game = \"deadlock\"\n".to_string(),
        feed_filename: "feed-1422450-1784582254.json".to_string(),
        feed_json: r#"{"revision":1784582254,"pops":{}}"#.to_string(),
    }
}

fn enable_persist_with_filename(name: &str) -> Operation {
    let Operation::EnablePersist {
        ops_version,
        config_toml,
        feed_json,
        ..
    } = valid_enable_persist()
    else {
        unreachable!()
    };
    Operation::EnablePersist {
        ops_version,
        config_toml,
        feed_filename: name.to_string(),
        feed_json,
    }
}

#[test]
fn valid_replace_ruleset_round_trips_and_validates() {
    let operation = replace_with_pops(valid_pops());
    let json = serde_json::to_string(&operation).expect("operation serializes");
    let parsed: Operation = serde_json::from_str(&json).expect("operation parses");

    assert_eq!(parsed, operation);
    assert_eq!(parsed.validate(), Ok(()));
}

#[test]
fn every_operation_kind_rejects_an_unsupported_version() {
    let operations = [
        Operation::ReplaceRuleset {
            ops_version: 99,
            game: Game::Deadlock,
            revision: 42,
            pops: valid_pops(),
        },
        Operation::DeleteTable { ops_version: 99 },
        Operation::Inspect { ops_version: 99 },
        Operation::EnablePersist {
            ops_version: 99,
            config_toml: String::new(),
            feed_filename: "feed-1-1.json".to_string(),
            feed_json: String::new(),
        },
        Operation::DisablePersist { ops_version: 99 },
    ];

    for operation in operations {
        assert_eq!(
            operation.validate(),
            Err(Rejection::VersionMismatch { got: 99 })
        );
    }
}

#[test]
fn validation_rejects_too_many_pops() {
    let pops = (0..=MAX_POPS)
        .map(|index| (format!("p{index}"), vec![Ipv4Addr::new(192, 0, 2, 1)]))
        .collect();

    assert_eq!(
        replace_with_pops(pops).validate(),
        Err(Rejection::TooManyPops { got: MAX_POPS + 1 })
    );
}

#[test]
fn validation_rejects_invalid_pop_codes_with_specific_reasons() {
    let cases = [
        ("", Rejection::EmptyPopCode),
        (
            "abcdefghijklmnopq",
            Rejection::PopCodeTooLong {
                code: "abcdefghijklmnopq".to_string(),
            },
        ),
        (
            "FRA",
            Rejection::PopCodeBadChar {
                code: "FRA".to_string(),
            },
        ),
        (
            "fra;drop",
            Rejection::PopCodeBadChar {
                code: "fra;drop".to_string(),
            },
        ),
        (
            "pop fra",
            Rejection::PopCodeBadChar {
                code: "pop fra".to_string(),
            },
        ),
        (
            "fr\u{e9}",
            Rejection::PopCodeBadChar {
                code: "fr\u{e9}".to_string(),
            },
        ),
    ];

    for (code, expected) in cases {
        assert_eq!(
            replace_with_code(code, vec![Ipv4Addr::new(192, 0, 2, 1)]).validate(),
            Err(expected)
        );
    }
}

#[test]
fn validation_rejects_empty_and_oversized_ip_lists() {
    assert_eq!(
        replace_with_code("fra", Vec::new()).validate(),
        Err(Rejection::NoIps {
            code: "fra".to_string(),
        })
    );

    let ips = vec![Ipv4Addr::new(192, 0, 2, 1); MAX_IPS_PER_POP + 1];
    assert_eq!(
        replace_with_code("fra", ips).validate(),
        Err(Rejection::TooManyIps {
            code: "fra".to_string(),
            got: MAX_IPS_PER_POP + 1,
        })
    );
}

#[test]
fn raw_nft_fields_are_rejected_by_the_operation_wire_format() {
    for field in ["ruleset", "table"] {
        let mut value = serde_json::json!({
            "op": "replace_ruleset",
            "ops_version": OPS_VERSION,
            "game": "deadlock",
            "revision": 42,
            "pops": {"fra": ["192.0.2.1"]}
        });
        value[field] = serde_json::Value::String("table inet regionlock {}".to_string());

        assert!(
            serde_json::from_value::<Operation>(value).is_err(),
            "unknown {field} field must not enter the operation schema"
        );
    }
}

#[test]
fn dangerous_pop_code_characters_fail_validation() {
    for code in ["fra\n", "fra\""] {
        assert_eq!(
            replace_with_code(code, vec![Ipv4Addr::new(192, 0, 2, 1)]).validate(),
            Err(Rejection::PopCodeBadChar {
                code: code.to_string(),
            })
        );
    }
}

#[test]
fn persist_operations_round_trip_and_validate() {
    for operation in [
        valid_enable_persist(),
        Operation::DisablePersist {
            ops_version: OPS_VERSION,
        },
    ] {
        let json = serde_json::to_string(&operation).expect("operation serializes");
        let parsed: Operation = serde_json::from_str(&json).expect("operation parses");
        assert_eq!(parsed, operation);
        assert_eq!(parsed.validate(), Ok(()));
    }
}

#[test]
fn bad_feed_filenames_are_rejected() {
    let cases = [
        "sdr-1422450-1.json",            // wrong prefix
        "feed-1422450-1.txt",            // wrong suffix
        "feed-1422450.json",             // missing revision
        "feed--1.json",                  // empty appid
        "feed-1-.json",                  // empty revision
        "feed-a-1.json",                 // non-digit appid
        "feed-1-1x.json",                // non-digit revision
        "feed-+1-1.json",                // sign smuggled past parse()
        "feed-1-1.json.bak",             // backup name, not canonical
        "feed-1-2/../evil.json",         // path traversal
        "feed-1422450-1/x.json",         // path separator
        "../feed-1422450-1.json",        // leading traversal
        "feed-99999999999999999-1.json", // appid overflows u32
        "FEED-1-1.json",                 // case matters
    ];
    for name in cases {
        assert_eq!(parse_feed_snapshot_filename(name), None, "name: {name:?}");
        assert_eq!(
            enable_persist_with_filename(name).validate(),
            Err(Rejection::BadFeedFilename {
                name: name.to_string(),
            }),
            "name: {name:?}"
        );
    }
    assert_eq!(
        parse_feed_snapshot_filename("feed-1422450-1784582254.json"),
        Some((1_422_450, 1_784_582_254))
    );
}

#[test]
fn oversize_snapshot_files_are_rejected() {
    let big = "x".repeat(MAX_SNAPSHOT_BYTES + 1);

    let Operation::EnablePersist {
        ops_version,
        feed_filename,
        feed_json,
        ..
    } = valid_enable_persist()
    else {
        unreachable!()
    };
    let oversized_config = Operation::EnablePersist {
        ops_version,
        config_toml: big.clone(),
        feed_filename: feed_filename.clone(),
        feed_json,
    };
    assert_eq!(
        oversized_config.validate(),
        Err(Rejection::SnapshotTooLarge {
            file: "config_toml",
            got: MAX_SNAPSHOT_BYTES + 1,
        })
    );

    let Operation::EnablePersist {
        ops_version,
        config_toml,
        feed_filename,
        ..
    } = valid_enable_persist()
    else {
        unreachable!()
    };
    let oversized_feed = Operation::EnablePersist {
        ops_version,
        config_toml,
        feed_filename,
        feed_json: big,
    };
    assert_eq!(
        oversized_feed.validate(),
        Err(Rejection::SnapshotTooLarge {
            file: "feed_json",
            got: MAX_SNAPSHOT_BYTES + 1,
        })
    );
}

#[test]
fn reply_variants_round_trip_through_json() {
    let spec = RulesetSpec {
        game: Game::Deadlock,
        revision: 42,
        pops: valid_pops(),
    };
    let journal = AppliedState::from_spec(&spec, 123);
    let replies = [
        Reply::Applied {
            journal: journal.clone(),
        },
        Reply::Deleted { existed: true },
        Reply::Inspected {
            live: Some(journal.pops.clone()),
            journal: Some(journal.clone()),
            reconciled_pending: true,
        },
        Reply::Persisted {
            managed_by_nixos: false,
        },
        Reply::Unpersisted {
            managed_by_nixos: true,
        },
        Reply::Refused {
            reason: "test refusal".to_string(),
        },
    ];

    for reply in replies {
        let json = serde_json::to_string(&reply).expect("reply serializes");
        let parsed: Reply = serde_json::from_str(&json).expect("reply parses");
        let reparsed = serde_json::to_value(parsed).expect("parsed reply serializes");
        assert_eq!(
            reparsed,
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
    }
}

#[test]
fn replace_from_spec_copies_spec_fields() {
    let spec = RulesetSpec {
        game: Game::Dota2,
        revision: 7,
        pops: BTreeMap::from([
            ("fra".to_string(), vec![Ipv4Addr::new(192, 0, 2, 1)]),
            ("ams".to_string(), vec![Ipv4Addr::new(198, 51, 100, 1)]),
        ]),
    };

    assert_eq!(
        Operation::replace_from_spec(&spec),
        Operation::ReplaceRuleset {
            ops_version: OPS_VERSION,
            game: spec.game,
            revision: spec.revision,
            pops: spec.pops.clone(),
        }
    );
}
