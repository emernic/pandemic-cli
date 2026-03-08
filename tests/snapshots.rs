use pandemic_cli_lib::snapshot::{render_to_string, run_snapshot};
use pandemic_cli_lib::state::GameState;

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
fn medicines_panel_browse() {
    let state = GameState::new_default(42);
    let result = run_snapshot(state, &["m".to_string()], None).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn medicines_panel_select_region() {
    let state = GameState::new_default(42);
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
    let state = GameState::new_default(42);
    let result = run_snapshot(
        state,
        &["m".to_string(), "enter".to_string(), "enter".to_string()],
        None,
    )
    .unwrap();
    insta::assert_snapshot!(result.screen);
}
