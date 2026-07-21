//! Tests for the hidden `generate` subcommand: packaging-facing completion
//! and man page output, rendered from the live grammar. The command needs
//! no config or cache, so these tests drive the bare binary.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_regionlock"))
        .args(args)
        // Never let a real REGIONLOCK_CONFIG in the ambient environment
        // influence the invocation.
        .env_remove("REGIONLOCK_CONFIG")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn completions_for_all_shells() {
    for shell in ["bash", "zsh", "fish", "nu"] {
        let out = run(&["generate", "completions", shell]);
        assert!(
            out.status.success(),
            "completions {shell} failed: {}",
            stderr(&out)
        );
        let text = stdout(&out);
        assert!(!text.is_empty(), "completions {shell} is non-empty");
        assert!(
            text.contains("regionlock"),
            "completions {shell} mention regionlock"
        );
        if shell == "bash" {
            assert!(
                text.contains("_regionlock"),
                "bash completions define a completion function"
            );
        }
    }
}

#[test]
fn man_page_covers_full_grammar() {
    let out = run(&["generate", "man"]);
    assert!(
        out.status.success(),
        "generate man failed: {}",
        stderr(&out)
    );
    let text = stdout(&out);
    assert!(
        text.starts_with(".TH") || text.contains(".SH SYNOPSIS"),
        "output is roff: {}",
        &text[..text.len().min(200)]
    );
    assert!(
        text.contains("teardown"),
        "man page covers the full grammar"
    );
}

#[test]
fn unknown_shell_lists_supported_shells() {
    let out = run(&["generate", "completions", "powershell"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stderr(&out);
    for shell in ["bash", "zsh", "fish", "nu"] {
        assert!(text.contains(shell), "error lists {shell}: {text}");
    }
}

#[test]
fn generate_is_hidden_from_help() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "--help failed: {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        !text.contains("generate"),
        "--help must not mention the hidden subcommand: {text}"
    );
}
