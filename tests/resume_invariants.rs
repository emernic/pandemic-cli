//! Regression tests for save/load resume behavior.
//!
//! These tests protect two contracts:
//! 1. **Exact resume** — UI state that the player set up before saving must
//!    survive a JSON round-trip unchanged.
//! 2. **Session reset** — ephemeral runtime state must NOT leak into the save
//!    file (or must reset to a safe default on load).
//!
//! Assertions are written in terms of player-visible behavior, not raw field
//! layout, so the tests remain valid across the state-split refactor.

use pandemic_cli_lib::engine;
use pandemic_cli_lib::snapshot::run_snapshot;
use pandemic_cli_lib::state::{
    AppState, CrisisCost, CrisisEvent, CrisisKind, CrisisOption, MedicineUiState,
    Panel, LabUiState, SaveFile, SessionState,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Serialize then deserialize via SaveFile, simulating a save/load cycle.
/// Session state resets on load (speed, status message, etc.).
fn round_trip(state: &AppState) -> AppState {
    let sf = SaveFile {
        world: state.world.clone(),
        ui: state.ui.clone(),
    };
    let json = serde_json::to_string(&sf).expect("serialize");
    let restored: SaveFile = serde_json::from_str(&json).expect("deserialize");
    AppState {
        world: restored.world,
        ui: restored.ui,
        session: SessionState::default(),
    }
}

/// Build a minimal crisis event for testing.
fn fake_crisis() -> CrisisEvent {
    CrisisEvent {
        kind: CrisisKind::PersonnelCrisis { amount: 5 },
        title: "Test Crisis".into(),
        description: "A test crisis.".into(),
        options: vec![
            CrisisOption {
                label: "Option A".into(),
                description: "Do A.".into(),
                cost: None,
            },
            CrisisOption {
                label: "Option B".into(),
                description: "Do B.".into(),
                cost: Some(CrisisCost {
                    funding: 100.0,
                    ..Default::default()
                }),
            },
        ],
        tick_created: 100,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// EXACT-RESUME TESTS — state that MUST survive save/load
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn resume_preserves_open_panel_and_selection() {
    let mut state = engine::new_game(42);
    // Player opened the Threats panel and scrolled down
    state.ui.open_panel = Panel::Threats;
    state.ui.panel_selection = 2;

    let restored = round_trip(&state);

    assert_eq!(restored.ui.open_panel, Panel::Threats, "open panel lost on resume");
    assert_eq!(restored.ui.panel_selection, 2, "panel selection lost on resume");
}

#[test]
fn resume_preserves_medicine_wizard_state() {
    let mut state = engine::new_game(42);
    state.ui.open_panel = Panel::Medicines;
    state.ui.medicine_ui = Some(MedicineUiState::RegionFilter { medicine_idx: 1 });

    let restored = round_trip(&state);

    assert_eq!(
        restored.ui.medicine_ui,
        Some(MedicineUiState::RegionFilter { medicine_idx: 1 }),
        "medicine wizard step lost on resume"
    );
}

#[test]
fn resume_preserves_research_wizard_state() {
    let mut state = engine::new_game(42);
    state.ui.open_panel = Panel::Lab;
    state.ui.lab_ui = Some(LabUiState::ConfirmProject {
        tab: pandemic_cli_lib::state::LabTab::Sequencing,
        project_idx: 0,
        double_personnel: true,
    });

    let restored = round_trip(&state);

    assert_eq!(
        restored.ui.lab_ui,
        Some(LabUiState::ConfirmProject {
            tab: pandemic_cli_lib::state::LabTab::Sequencing,
            project_idx: 0,
            double_personnel: true,
        }),
        "research wizard step lost on resume"
    );
}

#[test]
fn resume_preserves_map_selection() {
    let mut state = engine::new_game(42);
    state.ui.map_selection = 4; // Africa

    let restored = round_trip(&state);

    assert_eq!(restored.ui.map_selection, 4, "map selection lost on resume");
}

#[test]
fn resume_preserves_active_crisis_with_selection_state() {
    let mut state = engine::new_game(42);
    state.active_crisis = Some(fake_crisis());
    // Player had scrolled to option B and toggled auto-resolve
    state.ui.crisis_selection = 1;
    state.ui.crisis_auto_resolve = true;

    let restored = round_trip(&state);

    assert!(
        restored.active_crisis.is_some(),
        "active crisis lost on resume"
    );
    assert_eq!(
        restored.active_crisis.as_ref().unwrap().title,
        "Test Crisis",
        "crisis content corrupted on resume"
    );
    assert_eq!(
        restored.ui.crisis_selection, 1,
        "crisis option selection lost on resume"
    );
    assert!(
        restored.ui.crisis_auto_resolve,
        "crisis auto-resolve toggle lost on resume"
    );
    // Game should be blocked due to active crisis
    assert!(
        restored.is_blocked(),
        "game should be blocked with active crisis on resume"
    );
}

#[test]
fn resume_preserves_splash_progression() {
    let mut state = engine::new_game(42);
    // Player has completed the splash animation
    state.ui.home_splash_done = true;
    state.ui.home_splash_revealed = true;

    let restored = round_trip(&state);

    assert!(
        restored.ui.home_splash_done,
        "splash completion flag lost on resume"
    );
    assert!(
        restored.ui.home_splash_revealed,
        "splash revealed flag lost on resume"
    );
}

#[test]
fn resume_preserves_event_log_continuity() {
    let mut state = engine::new_game(42);
    state.event_log.push_back((1.0, "First disease detected".into()));
    state.event_log.push_back((3.5, "Region collapsed".into()));

    let restored = round_trip(&state);

    assert_eq!(restored.event_log.len(), 2, "event log entries lost on resume");
    assert_eq!(restored.event_log[0].1, "First disease detected");
    assert_eq!(restored.event_log[1].1, "Region collapsed");
}

#[test]
fn resume_preserves_history_snapshots() {
    let state = engine::new_game(42);
    // Advance some ticks to generate history entries
    let result = run_snapshot(state, &["d5".to_string()]).unwrap();
    let pre_save_history_len = result.state.history.len();
    assert!(pre_save_history_len > 0, "need history entries for this test");

    let restored = round_trip(&result.state);

    assert_eq!(
        restored.history.len(),
        pre_save_history_len,
        "history snapshots lost on resume"
    );
}

#[test]
fn resume_preserves_event_notification() {
    let mut state = engine::new_game(42);
    state.ui.event_notification = Some("Disease mutated!".into());

    let restored = round_trip(&state);

    assert_eq!(
        restored.ui.event_notification.as_deref(),
        Some("Disease mutated!"),
        "sticky event notification lost on resume"
    );
}

#[test]
fn resume_snapshot_continues_same_playthrough() {
    // Simulate what snapshot autosave does: run some steps, serialize,
    // deserialize, run more steps. The game should continue from where it
    // left off (same tick, same diseases, same state).
    let state = engine::new_game(42);
    let after_day1 = run_snapshot(state, &["d1".to_string()]).unwrap();
    let tick_after_day1 = after_day1.state.tick;
    let diseases_after_day1 = after_day1.state.diseases.len();

    // Round-trip (simulating save file write + reload)
    let restored = round_trip(&after_day1.state);
    assert_eq!(restored.tick, tick_after_day1, "tick shifted on reload");
    assert_eq!(
        restored.diseases.len(),
        diseases_after_day1,
        "disease count changed on reload"
    );

    // Continue playing from restored state
    let after_day2 = run_snapshot(restored, &["d1".to_string()]).unwrap();
    assert!(
        after_day2.state.tick > tick_after_day1,
        "game should advance past the save point"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SESSION-RESET TESTS — state that must NOT persist across save/load
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn session_reset_status_message_not_persisted() {
    let mut state = engine::new_game(42);
    state.session.status_message = Some("Medicine deployed!".into());

    let restored = round_trip(&state);

    assert!(
        restored.session.status_message.is_none(),
        "status_message is transient command feedback — must not persist across sessions"
    );
}

#[test]
fn session_reset_speed_multiplier_resets() {
    let mut state = engine::new_game(42);
    state.session.speed_multiplier = 4;

    let restored = round_trip(&state);

    assert_eq!(
        restored.session.speed_multiplier, 1,
        "speed_multiplier is a runtime preference — must reset to 1× on reload"
    );
}

#[test]
fn session_reset_size_warning_dismissed() {
    let mut state = engine::new_game(42);
    state.session.size_warning_dismissed = true;

    let restored = round_trip(&state);

    assert!(
        !restored.session.size_warning_dismissed,
        "size_warning_dismissed must not persist — the warning should re-appear each session"
    );
}

// Transient events no longer live on AppState — they flow explicitly
// through tick/command return values. No save/load invariant to test.

#[test]
fn session_reset_crisis_dismissal_unblocks_game() {
    // When a crisis is active, the game is blocked (is_blocked() == true).
    // After the player dismisses the crisis, the game should no longer be blocked.
    // Pacing (Running/Paused) is independent of crises — it stays whatever
    // it was before the crisis fired. Blocking is derived from active_crisis.
    let mut state = engine::new_game(42);
    state.active_crisis = Some(fake_crisis());

    let restored = round_trip(&state);

    // Crisis should still be present after load
    assert!(restored.active_crisis.is_some());
    assert!(restored.is_blocked(), "game should be blocked with active crisis");

    // Dismiss the crisis by pressing enter (selects option A)
    let after_dismiss = run_snapshot(restored, &["enter".to_string()]).unwrap();

    // After dismissal, game should no longer be blocked
    assert!(
        !after_dismiss.state.is_blocked(),
        "game should not be blocked after crisis dismissal"
    );
    assert!(
        after_dismiss.state.active_crisis.is_none(),
        "active_crisis should be cleared after dismissal"
    );
}
