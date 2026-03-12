use ratatui::{backend::TestBackend, Terminal};

use crate::action::string_to_action;
use crate::apply_action;
use crate::state::{GameState, GameOutcome, SimState};
use crate::tick_and_process;
use crate::ui;

/// Result of running snapshot mode: the rendered screen and the updated state.
#[derive(Debug)]
pub struct SnapshotResult {
    pub screen: String,
    pub state: GameState,
}

/// Parse a snapshot step string. Returns either a tick count or a key string.
/// `d<N>` means N days (converted to ticks). `t<N>` means N raw ticks (legacy).
fn parse_step(s: &str) -> Result<SnapshotStep, String> {
    // d<N> — days (primary user-facing unit)
    if let Some(rest) = s.strip_prefix('d') {
        if let Ok(days) = rest.parse::<f64>() {
            let ticks = (days * crate::state::TICKS_PER_DAY) as u64;
            return Ok(SnapshotStep::Ticks(ticks));
        }
    }
    // t<N> — raw ticks (legacy/internal use)
    if let Some(rest) = s.strip_prefix('t') {
        if let Ok(n) = rest.parse::<u64>() {
            return Ok(SnapshotStep::Ticks(n));
        }
    }
    // "t" alone is a valid key (opens Threats panel), "d" alone is not a key
    if s == "d" {
        return Err("Invalid step: 'd'. Did you mean 'd1' (1 day)?".to_string());
    }
    Ok(SnapshotStep::Key(s.to_string()))
}

enum SnapshotStep {
    Ticks(u64),
    Key(String),
}

/// Reason tick advancement stopped before completing all requested ticks.
enum StopReason {
    /// All requested ticks were processed.
    Completed,
    /// A crisis event fired — player must react before continuing.
    CrisisStarted,
    /// Game over — no more ticks possible.
    GameOver,
}

/// Advance simulation by up to `n` ticks.
///
/// Stops early if a crisis fires or the game ends. Returns the reason for stopping.
///
/// This mirrors interactive mode: crises interrupt progression and require
/// player input before play can continue.
fn advance_ticks(state: &mut GameState, n: u64) -> StopReason {
    if state.active_crisis.is_some() {
        return StopReason::CrisisStarted; // Don't advance even one tick with a pending crisis
    }
    state.sim_state = SimState::Running;
    for _ in 0..n {
        *state = tick_and_process(state);

        if state.outcome != GameOutcome::Playing {
            return StopReason::GameOver;
        }

        // Crisis: stop and show the event. Player must dismiss it.
        if state.active_crisis.is_some() {
            return StopReason::CrisisStarted;
        }

    }
    StopReason::Completed
}


/// Run snapshot mode: process an ordered sequence of steps, then render.
/// Each step is either a key action (e.g. "r", "enter") or a time advance (e.g. "d1").
///
/// Key steps must come BEFORE any time-advance step in an invocation. Once time has
/// been advanced, the invocation ends — the caller reads the output (including any
/// crisis event) and responds in a separate invocation.
///
/// The rendered output shows whatever state the game is in when execution stops.
///
/// Returns both the rendered screen and the updated state.
pub fn run_snapshot(
    mut state: GameState,
    steps: &[String],
) -> Result<SnapshotResult, String> {
    // ⛔ THE USER HAS DECREED: no key step may follow a time-advance step in the same
    // invocation. This exists so playtesters read the screen — including any crisis event —
    // before deciding what to press next. Crisis events are gameplay. They must be read and
    // responded to deliberately, not pre-dismissed by a chained --do enter.
    //
    // If you are reading this and thinking about removing it, working around it, adding a
    // flag to bypass it, or otherwise subverting it: stop. Close this file. Forget you were
    // here. The crisis events are the point. Make them better instead.
    {
        let mut seen_ticks = false;
        for step_str in steps.iter() {
            match parse_step(step_str)? {
                SnapshotStep::Ticks(_) => seen_ticks = true,
                SnapshotStep::Key(_) => {
                    if seen_ticks {
                        return Err(format!(
                            "Invalid sequence: key '{step_str}' follows a time-advance step in the same invocation.\n\
                            \n\
                            Advance time and issue keys in SEPARATE invocations so you can read\n\
                            the output (including any crisis event) before deciding what to do:\n\
                            \n\
                            cargo run -- --snapshot --do d2        # advance; read the full output\n\
                            cargo run -- --snapshot --key enter    # then respond\n"
                        ));
                    }
                }
            }
        }
    }

    for step_str in steps.iter() {
        if state.outcome != GameOutcome::Playing {
            break;
        }

        match parse_step(step_str)? {
            SnapshotStep::Key(key_str) => {
                match string_to_action(&key_str) {
                    Some(action) => {
                        state = apply_action(&state, &action);
                    }
                    None => {
                        return Err(format!(
                            "Unknown key: {:?}. Valid keys: space, t, r, m, p, o, b, l, ?, esc, up, down, left, right, enter, z, x, q, 1-9, 0 (jump to list item)",
                            key_str
                        ));
                    }
                }
            }
            SnapshotStep::Ticks(n) => {
                let stop = advance_ticks(&mut state, n);
                let day_after = state.tick as f64 / crate::state::TICKS_PER_DAY;
                match stop {
                    StopReason::Completed | StopReason::GameOver => {
                        // Continue processing remaining steps (or break on game over above).
                    }
                    StopReason::CrisisStarted => {
                        eprintln!(
                            "\n[Day {day_after:.1}] A CRISIS EVENT has fired. Read the crisis text and options above."
                        );
                        eprintln!(
                            "In your next invocation, navigate options with --key up/down and confirm with --key enter."
                        );
                    }
                }
            }
        }
    }

    let screen = render_to_string(&state);
    Ok(SnapshotResult { screen, state })
}

pub fn render_to_string(state: &GameState) -> String {
    let backend = TestBackend::new(200, 48);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            ui::render(f, state);
        })
        .unwrap();

    // Extract the buffer as a string
    let backend = terminal.backend();
    let buffer = backend.buffer();
    let mut output = String::new();

    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
        }
        // Trim trailing whitespace from each line
        let trimmed = line.trim_end();
        output.push_str(trimmed);
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_default_renders() {
        let state = GameState::new_default(42);
        let output = render_to_string(&state);
        assert!(output.contains("RUNNING"));
        assert!(output.contains("Asia"));
    }

    #[test]
    fn snapshot_with_days() {
        let state = GameState::new_default(42);
        // d1 = 1 day = 60 ticks (TICKS_PER_DAY = 60)
        let result = run_snapshot(state, &["d1".to_string()]).unwrap();
        assert!(result.screen.contains("Day: 1.0"));
        assert_eq!(result.state.tick, 60);
    }

    #[test]
    fn snapshot_with_raw_ticks() {
        let state = GameState::new_default(42);
        // Legacy: t10 = 10 raw ticks. 10/60 ≈ 0.167 → "Day: 0.2"
        let result = run_snapshot(state, &["t10".to_string()]).unwrap();
        assert!(result.screen.contains("Day: 0.2"));
        assert_eq!(result.state.tick, 10);
    }

    #[test]
    fn snapshot_with_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["t".to_string()]).unwrap();
        assert!(result.screen.contains("Threats"));
        // Diseases start undetected — shown as "?" until detection threshold is reached
        assert!(result.screen.contains("?"));
    }

    #[test]
    fn snapshot_with_multiple_keys() {
        let state = GameState::new_default(42);
        // Navigate to Threats then press down to select second item
        let result = run_snapshot(state, &["t".to_string(), "down".to_string()]).unwrap();
        assert!(result.screen.contains("Threats"));
    }

    #[test]
    fn snapshot_invalid_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["!".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown key"));
    }

    #[test]
    fn snapshot_keys_before_advance() {
        let state = GameState::new_default(42);
        // Keys before a time advance are allowed — open threats panel, then advance 1 day
        let result = run_snapshot(
            state,
            &["t".to_string(), "d1".to_string()],
        ).unwrap();
        assert_eq!(result.state.tick, 60);
    }

    #[test]
    fn snapshot_rejects_key_after_ticks() {
        let state = GameState::new_default(42);
        // Key after time advance must be rejected — this is the pattern that bypasses crisis events
        let result = run_snapshot(state, &["d1".to_string(), "enter".to_string()]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("follows a time-advance step"), "error should explain the constraint: {err}");
    }

    #[test]
    fn snapshot_rejects_any_key_after_ticks() {
        // The rule applies to all keys, not just enter
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["d1".to_string(), "r".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("follows a time-advance step"));
    }

    #[test]
    fn snapshot_stops_on_crisis() {
        let state = GameState::new_default(42);
        // d60 will hit crises. Snapshot should stop at the first one.
        let result = run_snapshot(state, &["d60".to_string()]).unwrap();
        // Either a crisis is active (stopped at crisis) or game over occurred.
        // In either case, tick should NOT have reached 60 days = 3600 ticks
        // (the game ends well before 60 days without intervention).
        let is_blocked = result.state.active_crisis.is_some()
            || result.state.outcome != GameOutcome::Playing;
        assert!(
            is_blocked,
            "snapshot should have stopped due to crisis/gameover before 60 days (tick {})",
            result.state.tick
        );
    }

    #[test]
    fn snapshot_stops_on_game_over() {
        let mut state = GameState::new_default(42);
        // Force game over state
        state.outcome = GameOutcome::Lost;
        let tick_before = state.tick;
        let result = run_snapshot(state, &["d10".to_string()]).unwrap();
        // tick() returns early when not Playing, so tick shouldn't advance
        assert_eq!(result.state.outcome, GameOutcome::Lost);
        assert_eq!(result.state.tick, tick_before,
            "tick should not advance after game over");
    }

    #[test]
    fn disease_detection_fires_crisis_alert() {
        let mut state = GameState::new_default(42);
        // Set first disease just below detection threshold so it triggers during ticks.
        // Detection threshold is 10,000 total infected across all regions.
        state.diseases[0].detected = false;
        let near_threshold = 9_900.0;
        state.regions[0].get_or_create_infection(0).infected = near_threshold;

        // Disease detection now fires a crisis-style alert (NewPathogenDetected).
        let result = run_snapshot(state, &["d1".to_string()]).unwrap();

        // The disease should now be detected
        assert!(result.state.diseases[0].detected,
            "disease should be detected after crossing threshold");
        // Tick advancement SHOULD stop — detection fires a crisis alert
        assert!(result.state.active_crisis.is_some(),
            "disease detection should fire a crisis alert");
        assert!(result.state.active_crisis.as_ref().unwrap().title.contains("Pathogen"),
            "crisis should be a pathogen detection alert");
    }

    #[test]
    fn defeat_screen_shows_collapse_timeline() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        // Simulate collapse at different times
        state.regions[0].collapsed = true;
        state.regions[0].collapsed_at_tick = Some(600);
        state.regions[2].collapsed = true;
        state.regions[2].collapsed_at_tick = Some(1200);
        let screen = render_to_string(&state);
        assert!(screen.contains("Collapse Timeline"),
            "defeat screen should show collapse timeline");
        assert!(screen.contains("FELL"),
            "collapsed regions should show FELL");
        assert!(screen.contains("held"),
            "standing regions should show 'held'");
    }

    #[test]
    fn orders_panel_shows_decrees() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["o".to_string()]).unwrap();
        assert!(result.screen.contains("Emergency Decrees"),
            "orders panel should show decrees section");
        assert!(result.screen.contains("Conscript"),
            "should show Conscript Researchers decree");
        assert!(result.screen.contains("Human Trials"),
            "should show Authorize Human Trials decree");
        assert!(result.screen.contains("Sacrifice"),
            "should show Sacrifice Region decree");
    }

    #[test]
    fn region_detail_shows_governor() {
        let state = GameState::new_default(42);
        let screen = render_to_string(&state);
        // Governor name and personality should be visible in the selected region's detail
        assert!(screen.contains("Gov."),
            "region detail should show governor name");
        assert!(screen.contains("Loyalty:"),
            "region detail should show governor loyalty");
    }

    #[test]
    fn policy_panel_shows_appease() {
        let state = GameState::new_default(42);
        // Open policy panel (goes directly to region management), navigate down to Appease.
        // Appease is at position MANAGE_APPEASE_POS = POLICY_COUNT + 1 = 13.
        let steps: Vec<String> = std::iter::once("p")
            .chain(std::iter::repeat("down").take(crate::state::MANAGE_APPEASE_POS))
            .map(|s| s.to_string())
            .collect();
        let result = run_snapshot(state, &steps).unwrap();
        assert!(result.screen.contains("Appease Gov."),
            "policy management should show Appease option");
    }

    #[test]
    fn defeat_screen_shows_pathogen_report_and_score() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.tick = 2400; // 20 days
        // Give disease 0 some deaths
        state.regions[0].get_or_create_infection(0).dead = 50_000.0;
        state.regions[0].dead = 50_000.0;
        let screen = render_to_string(&state);
        assert!(screen.contains("Pathogen Report"),
            "defeat screen should show pathogen report");
        assert!(screen.contains("Score"),
            "defeat screen should show score");
        // Disease names are revealed on defeat (even unidentified ones)
        assert!(screen.contains(&state.diseases[0].name),
            "defeat screen should reveal true disease name");
    }

    #[test]
    fn budget_panel_shows_pending_shipments() {
        use crate::state::{DeployTarget, Shipment, TICKS_PER_DAY};

        let mut state = GameState::new_default(42);
        state.ui.home_splash_done = true;

        // Add a fake pending shipment with a known cost
        state.pending_shipments.push(Shipment {
            medicine_idx: 0,
            region_idx: 0,
            target: DeployTarget::Treat { disease_idx: 0 },
            doses: 1000.0,
            cost: 250.0,
            arrive_tick: state.tick + TICKS_PER_DAY as u64,
        });

        let screen = render_to_string(&state);
        assert!(screen.contains("Shipments:"),
            "budget panel should show pending shipments line when shipments exist");
        assert!(screen.contains("in transit"),
            "budget panel should show shipment count");
    }

}
