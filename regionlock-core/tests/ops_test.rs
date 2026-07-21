use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use regionlock_core::Game;
use regionlock_core::ops::{MAX_IPS_PER_POP, MAX_POPS, OPS_VERSION, Operation, Rejection, Reply};
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
