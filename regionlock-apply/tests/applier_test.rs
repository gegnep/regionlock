use std::collections::BTreeMap;
use std::io::Write;
use std::net::Ipv4Addr;
use std::process::{Command, Output, Stdio};

use regionlock_core::Game;
use regionlock_core::ops::{Operation, Reply};

const ROOT_REASON: &str = "must run as root";

fn run_applier(input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_regionlock-apply"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("regionlock-apply starts");
    // The applier runs the root check BEFORE reading stdin, so when it is
    // unprivileged it refuses and exits before consuming our input. A
    // BrokenPipe on this write is therefore expected, not a failure; the
    // refusal contract is the reply on stdout (checked by refused_reason).
    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(input) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(e) => panic!("unexpected stdin write error: {e}"),
        }
    }
    child.wait_with_output().expect("applier exits")
}

fn refused_reason(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(1));
    let stdout = std::str::from_utf8(&output.stdout).expect("stdout is UTF-8");
    let lines: Vec<&str> = stdout.split_terminator('\n').collect();
    assert_eq!(
        lines.len(),
        1,
        "stdout must contain one JSON line: {stdout:?}"
    );

    let value: serde_json::Value = serde_json::from_str(lines[0]).expect("stdout is JSON");
    assert_eq!(value["result"], "refused");
    let reply: Reply = serde_json::from_str(lines[0]).expect("stdout is a Reply");
    match reply {
        Reply::Refused { reason } => reason,
        _ => panic!("stdout is not a refused reply: {value}"),
    }
}

fn valid_operation_json() -> Vec<u8> {
    serde_json::to_vec(&Operation::ReplaceRuleset {
        ops_version: 1,
        game: Game::Deadlock,
        revision: 42,
        pops: BTreeMap::from([("fra".to_string(), vec![Ipv4Addr::new(192, 0, 2, 1)])]),
    })
    .expect("valid operation serializes")
}

#[test]
fn valid_operation_is_refused_before_privileged_work() {
    let reason = refused_reason(&run_applier(&valid_operation_json()));

    assert!(reason.contains(ROOT_REASON), "reason was: {reason}");
}

#[test]
fn malformed_json_is_refused_by_the_root_check_first() {
    let reason = refused_reason(&run_applier(b"{not json"));

    // run() checks geteuid before reading stdin, so the observed reason names
    // the privilege requirement rather than the malformed JSON.
    assert!(reason.contains(ROOT_REASON), "reason was: {reason}");
}

#[test]
fn empty_stdin_is_refused_by_the_root_check_first() {
    let reason = refused_reason(&run_applier(b""));

    assert!(reason.contains(ROOT_REASON), "reason was: {reason}");
}

#[test]
fn persist_operations_are_refused_unprivileged() {
    let operations = [
        Operation::EnablePersist {
            ops_version: 1,
            config_toml: "default_game = \"deadlock\"\n".to_string(),
            feed_filename: "feed-1422450-42.json".to_string(),
            feed_json: r#"{"revision":42,"pops":{}}"#.to_string(),
        },
        Operation::DisablePersist { ops_version: 1 },
    ];
    for operation in operations {
        let input = serde_json::to_vec(&operation).expect("operation serializes");
        let reason = refused_reason(&run_applier(&input));
        assert!(reason.contains(ROOT_REASON), "reason was: {reason}");
    }
}

#[test]
fn unsupported_version_is_refused() {
    let input = serde_json::to_vec(&Operation::Inspect { ops_version: 99 })
        .expect("unsupported operation serializes");
    let reason = refused_reason(&run_applier(&input));

    assert!(
        reason.contains(ROOT_REASON) || reason.contains("ops_version 99 unsupported"),
        "reason was neither the root check nor version validation: {reason}"
    );
}
