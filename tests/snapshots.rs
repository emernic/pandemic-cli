use pandemic_cli_lib::snapshot::{render_to_string, run_snapshot};
use pandemic_cli_lib::state::{GameOutcome, GameState};

/// Smoke test: initial screen renders without panicking and contains key elements.
#[test]
fn initial_screen() {
    let state = GameState::new_default(42);
    let output = render_to_string(&state);
    insta::assert_snapshot!(output);
}

/// Smoke test: simulation advances and renders correctly after 10 ticks.
#[test]
fn after_10_ticks() {
    let state = GameState::new_default(42);
    let result = run_snapshot(state, &[], Some(10)).unwrap();
    insta::assert_snapshot!(result.screen);
}

/// Smoke test: game over screen renders.
#[test]
fn game_over_defeat() {
    let mut state = GameState::new_default(42);
    state.outcome = GameOutcome::Lost;
    state.paused = true;
    let output = render_to_string(&state);
    insta::assert_snapshot!(output);
}
