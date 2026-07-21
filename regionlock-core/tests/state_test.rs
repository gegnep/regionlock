//! M1c: desired-state set operations (block/unblock/allow/reset).

use regionlock_core::state::DesiredState;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn block_dedups_and_is_idempotent() {
    let mut state = DesiredState::default();

    let delta = state.block(&strings(&["fra", "ams", "fra"]));
    assert_eq!(delta.now_blocked, strings(&["ams", "fra"]));
    assert!(delta.now_unblocked.is_empty());
    assert_eq!(
        state.blocked,
        strings(&["ams", "fra"]).into_iter().collect()
    );

    // Re-applying the same block is idempotent: nothing new, empty delta.
    let delta2 = state.block(&strings(&["fra", "ams"]));
    assert!(delta2.now_blocked.is_empty());
    assert!(delta2.now_unblocked.is_empty());
}

#[test]
fn unblock_lists_only_previously_blocked_codes() {
    let mut state = DesiredState::default();
    state.block(&strings(&["fra", "ams"]));

    // "par" was never blocked, so it must not show up in the delta.
    let delta = state.unblock(&strings(&["fra", "par"]));
    assert_eq!(delta.now_unblocked, strings(&["fra"]));
    assert!(delta.now_blocked.is_empty());
    assert_eq!(state.blocked, strings(&["ams"]).into_iter().collect());
}

#[test]
fn allow_blocks_everything_except_keep_and_preserves_already_blocked() {
    let mut state = DesiredState::default();
    state.block(&strings(&["b"])); // "b" is already blocked before allow().

    let delta = state.allow(&strings(&["a"]), &strings(&["a", "b", "c"]));

    // Final state blocks b and c, keeps a unblocked.
    assert_eq!(state.blocked, strings(&["b", "c"]).into_iter().collect());
    // b was already blocked, so only c is newly blocked in the delta.
    assert_eq!(delta.now_blocked, strings(&["c"]));
    assert!(delta.now_unblocked.is_empty());
}

#[test]
fn allow_with_empty_keep_blocks_everything() {
    let mut state = DesiredState::default();
    let delta = state.allow(&[], &strings(&["a", "b", "c"]));

    assert_eq!(
        state.blocked,
        strings(&["a", "b", "c"]).into_iter().collect()
    );
    assert_eq!(delta.now_blocked, strings(&["a", "b", "c"]));
    assert!(delta.now_unblocked.is_empty());
}

#[test]
fn allow_unblocks_previously_blocked_pops_not_in_target() {
    let mut state = DesiredState::default();
    state.block(&strings(&["x", "y"]));

    // New target only wants "y" blocked; "x" must be lifted.
    let delta = state.allow(&strings(&["z"]), &strings(&["y"]));

    assert_eq!(state.blocked, strings(&["y"]).into_iter().collect());
    assert_eq!(delta.now_unblocked, strings(&["x"]));
    assert!(delta.now_blocked.is_empty());
}

#[test]
fn reset_delta_lists_everything_previously_blocked() {
    let mut state = DesiredState::default();
    state.block(&strings(&["fra", "ams", "par"]));

    let delta = state.reset();
    assert_eq!(delta.now_unblocked, strings(&["ams", "fra", "par"]));
    assert!(delta.now_blocked.is_empty());
    assert!(state.blocked.is_empty());
}

#[test]
fn reset_on_empty_state_yields_empty_delta() {
    let mut state = DesiredState::default();
    let delta = state.reset();
    assert!(delta.now_blocked.is_empty());
    assert!(delta.now_unblocked.is_empty());
}
