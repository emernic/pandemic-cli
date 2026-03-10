use pandemic_cli_lib::snapshot::{render_to_string, run_snapshot};
use pandemic_cli_lib::state::{GameOutcome, GameState, SimState};

/// Smoke test: initial screen renders without panicking and contains key UI elements.
#[test]
fn initial_screen() {
    let state = GameState::new_default(42);
    let output = render_to_string(&state);
    assert!(output.contains("Day:"), "missing day counter");
    assert!(output.contains("RUNNING"), "missing status");
    assert!(output.contains("World Map"), "missing map");
    assert!(output.contains("Funds:"), "missing funds");
    assert!(output.contains("Personnel:"), "missing personnel");
}

/// Smoke test: simulation advances and renders correctly after some ticks.
#[test]
fn after_ticks() {
    let state = GameState::new_default(42);
    let result = run_snapshot(state, &["t10".to_string()]).unwrap();
    assert_eq!(result.state.tick, 10);
    assert!(result.screen.contains("Day:"), "missing day counter");
    assert!(result.screen.contains("Infected:"), "missing infected count");
}

/// Smoke test: game over screen renders with defeat panel.
#[test]
fn game_over_defeat() {
    let mut state = GameState::new_default(42);
    state.outcome = GameOutcome::Lost;
    state.sim_state = SimState::Paused;
    let output = render_to_string(&state);
    assert!(output.contains("DEFEAT"), "missing defeat indicator");
    assert!(output.contains("collapsed") || output.contains("resources"), "missing defeat message");
    assert!(output.contains("Summary"), "missing summary section");
}
