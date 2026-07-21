//! Privileged applier. Reads ONE typed operation from stdin (never env,
//! never argv paths), validates it, acts on `table inet regionlock` only.
//! The operation schema is designed and frozen at M3; until then this
//! binary refuses to run.

fn main() -> std::process::ExitCode {
    eprintln!("regionlock-apply: operation schema lands at M3; refusing to run");
    std::process::ExitCode::FAILURE
}
