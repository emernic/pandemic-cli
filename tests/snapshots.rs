use pandemic_cli_lib::snapshot::render_to_string;
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
    let result = pandemic_cli_lib::snapshot::run_snapshot(state, None, Some(10)).unwrap();
    insta::assert_snapshot!(result.screen);
}

#[test]
fn threats_panel() {
    let state = GameState::new_default(42);
    let result = pandemic_cli_lib::snapshot::run_snapshot(state, Some("t"), None).unwrap();
    insta::assert_snapshot!(result.screen);
}
