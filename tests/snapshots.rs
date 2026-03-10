use pandemic_cli_lib::snapshot::{render_to_string, run_snapshot};
use pandemic_cli_lib::state::{GameOutcome, GameState, GovernorPersonality, POLICY_COUNT, SimState};

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

/// Smoke test: bargain option appears when governor is defiant.
#[test]
fn bargain_shown_for_defiant_governor() {
    let mut state = GameState::new_default(42);
    // Force Nationalist governor to be defiant
    state.regions[0].governor.personality = GovernorPersonality::Nationalist;
    state.regions[0].governor.loyalty = 20.0;
    // Open policy panel for region 0, then scroll down to the bargain item
    // (panel viewport is small, need to move selection to bring bargain into view)
    let mut steps: Vec<String> = vec!["p".to_string(), "enter".to_string()];
    // Navigate down past all policies + Appease to reach Bargain (POLICY_COUNT + 1)
    for _ in 0..=POLICY_COUNT {
        steps.push("down".to_string());
    }
    let result = run_snapshot(state, &steps).unwrap();
    assert!(result.screen.contains("Bargain: Regional Priority"), "missing bargain option for defiant Nationalist");
    assert!(result.screen.contains("DEFIANT"), "missing DEFIANT label");
}
