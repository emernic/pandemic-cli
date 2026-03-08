use pandemic_cli_lib::snapshot::{render_to_string, run_snapshot};
use pandemic_cli_lib::state::GameState;

fn unlocked_state() -> GameState {
    let mut state = GameState::new_default(42);
    for med in &mut state.medicines {
        med.unlocked = true;
        med.tested_against = med.target_diseases.clone();
    }
    for disease in &mut state.diseases {
        disease.knowledge = 1.0;
    }
    state
}

#[test]
fn initial_screen() {
    let state = GameState::new_default(42);
    let output = render_to_string(&state);
    insta::assert_snapshot!(output);
}

#[test]
fn after_10_ticks() {
    let state = GameState::new_default(42);
    let result = run_snapshot(state, &[], Some(10)).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn threats_panel() {
    let state = GameState::new_default(42);
    let result = run_snapshot(state, &["t".to_string()], None).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn threats_panel_known() {
    let state = unlocked_state();
    let result = run_snapshot(state, &["t".to_string()], None).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn medicines_panel_browse() {
    let state = unlocked_state();
    let result = run_snapshot(state, &["m".to_string()], None).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn medicines_panel_select_region() {
    let state = unlocked_state();
    let result = run_snapshot(
        state,
        &["m".to_string(), "enter".to_string()],
        None,
    )
    .unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn medicines_panel_select_target() {
    let state = unlocked_state();
    let result = run_snapshot(
        state,
        &["m".to_string(), "enter".to_string(), "enter".to_string()],
        None,
    )
    .unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn research_panel_categories() {
    let state = GameState::new_default(42);
    let result = run_snapshot(state, &["r".to_string()], None).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn research_panel_field_projects() {
    let state = GameState::new_default(42);
    let result = run_snapshot(
        state,
        &["r".to_string(), "enter".to_string()],
        None,
    )
    .unwrap();
    insta::assert_snapshot!(result.screen);
}
