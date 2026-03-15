use pandemic_cli_lib::snapshot::{render_to_string, run_snapshot};
use pandemic_cli_lib::state::{GameOutcome, AppState, GovernorPersonality, MANAGE_BARGAIN_POS};

/// Smoke test: initial screen renders without panicking and contains key UI elements.
#[test]
fn initial_screen() {
    let state = AppState::new_default(42);
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
    let state = AppState::new_default(42);
    let result = run_snapshot(state, &["t10".to_string()]).unwrap();
    assert_eq!(result.state.tick, 10);
    assert!(result.screen.contains("Day:"), "missing day counter");
    assert!(result.screen.contains("Infected:"), "missing infected count");
}

/// Smoke test: game over screen renders with defeat panel.
#[test]
fn game_over_defeat() {
    let mut state = AppState::new_default(42);
    state.outcome = GameOutcome::Lost;
    // Game is blocked via outcome != Playing — no need to change sim_state
    let output = render_to_string(&state);
    assert!(output.contains("DEFEAT"), "missing defeat indicator");
    assert!(output.contains("collapsed") || output.contains("resources"), "missing defeat message");
    assert!(output.contains("Summary"), "missing summary section");
}

/// Smoke test: dashboard Authority section renders correctly.
#[test]
fn dashboard_pol_breakdown() {
    let mut state = AppState::new_default(42);
    state.ui.home_splash_done = true;
    let output = render_to_string(&state);
    assert!(output.contains("AUTHORITY"), "missing Authority section header");
    assert!(output.contains("Level:"), "missing authority level");
    assert!(output.contains("Board:"), "missing board satisfaction breakdown");
}

/// Smoke test: bargain option appears when governor is hostile.
#[test]
fn bargain_shown_for_hostile_governor() {
    let mut state = AppState::new_default(42);
    // Force Hardliner governor to be hostile
    state.regions[0].governor.personality = GovernorPersonality::Hardliner;
    state.regions[0].governor.cooperation = 20.0;
    // Open policy panel for region 0, then scroll down to the bargain item
    // (panel viewport is small, need to move selection to bring bargain into view)
    let mut steps: Vec<String> = vec!["p".to_string(), "enter".to_string()];
    // Navigate to Bargain at display position MANAGE_BARGAIN_POS.
    // Layout: MANAGE_INFRA_BASE..MANAGE_NEGOTIATE_POS-1 = infra repair,
    //         MANAGE_NEGOTIATE_POS = Negotiate, MANAGE_BARGAIN_POS = Bargain.
    for _ in 0..MANAGE_BARGAIN_POS {
        steps.push("down".to_string());
    }
    let result = run_snapshot(state, &steps).unwrap();
    assert!(result.screen.contains("Bargain: Grant Authority"), "missing bargain option for hostile Hardliner");
    assert!(result.screen.contains("HOSTILE"), "missing HOSTILE label");
}
