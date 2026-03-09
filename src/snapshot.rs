use ratatui::{backend::TestBackend, Terminal};

use crate::action::string_to_action;
use crate::apply_action;
use crate::engine::tick;
use crate::state::{GameState, GameOutcome, SimState};
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
        return Err("Invalid step: 'd' — did you mean 'd1' (1 day)?".to_string());
    }
    Ok(SnapshotStep::Key(s.to_string()))
}

enum SnapshotStep {
    Ticks(u64),
    Key(String),
}

/// Advance simulation by up to `n` ticks. Returns the number of ticks NOT
/// executed (remaining after an interruption). Returns 0 if all ticks ran.
///
/// Stops early on:
/// - Crisis event → returns remaining ticks (caller can resume after resolution)
/// - Game over → returns remaining ticks (game is done)
fn advance_ticks(state: &mut GameState, n: u64) -> u64 {
    state.sim_state = SimState::Running;
    for tick_i in 0..n {
        *state = tick(state);
        ui::process_events(state);
        if state.active_crisis.is_some() || state.outcome != GameOutcome::Playing {
            return n - tick_i - 1;
        }
    }
    0
}

/// Run snapshot mode: process an ordered sequence of steps, then render.
/// Each step is either a key action (e.g. "r", "enter") or ticks (e.g. "t10").
///
/// Crisis events interrupt tick advancement but do NOT drop remaining steps.
/// Subsequent key steps can resolve the crisis, after which any remaining
/// ticks from the interrupted step automatically resume. This mirrors the
/// interactive experience: the player sees the crisis, responds, and time
/// continues from where it left off.
///
/// Game over always stops execution immediately.
///
/// Returns both the rendered screen and the updated state.
pub fn run_snapshot(
    mut state: GameState,
    steps: &[String],
) -> Result<SnapshotResult, String> {
    // Ticks remaining from an interrupted days step (saved across crisis resolution).
    let mut pending_ticks: u64 = 0;

    for step_str in steps.iter() {
        // Game over stops everything.
        if state.outcome != GameOutcome::Playing {
            break;
        }

        match parse_step(step_str)? {
            SnapshotStep::Key(key_str) => {
                match string_to_action(&key_str) {
                    Some(action) => {
                        let had_crisis = state.active_crisis.is_some();
                        state = apply_action(&state, &action);

                        // If this key resolved a crisis, resume pending ticks.
                        if had_crisis && state.active_crisis.is_none() && pending_ticks > 0 {
                            let remaining = advance_ticks(&mut state, pending_ticks);
                            pending_ticks = remaining;
                        }
                    }
                    None => {
                        return Err(format!(
                            "Unknown key: {:?}. Valid keys: space, t, r, m, p, ?, esc, up, down, left, right, h, l, enter, z, x, q",
                            key_str
                        ));
                    }
                }
            }
            SnapshotStep::Ticks(n) => {
                // Can't advance time during a crisis — skip tick steps.
                if state.active_crisis.is_some() {
                    let days = n as f64 / crate::state::TICKS_PER_DAY;
                    eprintln!("Skipped d{:.1}: crisis active (resolve with enter first)", days);
                    continue;
                }
                let total = n + pending_ticks;
                pending_ticks = advance_ticks(&mut state, total);
            }
        }
    }

    // Log any time that was ultimately lost (crisis not resolved by end of steps).
    if pending_ticks > 0 {
        let days = pending_ticks as f64 / crate::state::TICKS_PER_DAY;
        if state.active_crisis.is_some() {
            eprintln!("Crisis unresolved: {:.1} days pending (resolve with enter)", days);
        }
        // If game over, the remaining ticks are expected — no need to log.
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
        // d1 = 1 day = 120 ticks
        let result = run_snapshot(state, &["d1".to_string()]).unwrap();
        assert!(result.screen.contains("Day: 1.0"));
        assert_eq!(result.state.tick, 120);
    }

    #[test]
    fn snapshot_with_raw_ticks() {
        let state = GameState::new_default(42);
        // Legacy: t10 = 10 raw ticks
        let result = run_snapshot(state, &["t10".to_string()]).unwrap();
        assert!(result.screen.contains("Day: 0.1"));
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
    fn snapshot_interleaved_days_and_keys() {
        let state = GameState::new_default(42);
        // Advance 0.5 days (60 ticks), open threats panel, advance 0.5 more days
        let result = run_snapshot(
            state,
            &["d0.5".to_string(), "t".to_string(), "d0.5".to_string()],
        )
        .unwrap();
        assert_eq!(result.state.tick, 120);
        assert!(result.screen.contains("Threats"));
        assert!(result.screen.contains("Day: 1.0"));
    }

    #[test]
    fn snapshot_stops_on_crisis() {
        let state = GameState::new_default(42);
        // Advance far enough that a crisis fires (crises start after tick 360,
        // average interval ~840 ticks). 60 days gives P(crisis) > 99%.
        let result = run_snapshot(state, &["d60".to_string()]).unwrap();
        // Should have stopped early due to crisis
        assert!(result.state.active_crisis.is_some(),
            "should have hit a crisis during 60 days");
        assert!(result.state.tick < 60 * 120,
            "should have stopped before reaching 60 days (stopped at tick {})", result.state.tick);
        assert!(result.screen.contains("CRISIS"),
            "should show the crisis screen");
    }

    #[test]
    fn snapshot_crisis_resume_after_enter() {
        let state = GameState::new_default(42);
        // Request 60 days. A crisis will fire partway through.
        // Then press enter to resolve it. Time should resume (and may hit
        // another crisis during the resumed ticks — that's correct).
        let crisis_only = run_snapshot(
            state.clone(),
            &["d60".to_string()],
        ).unwrap();
        let tick_at_first_crisis = crisis_only.state.tick;
        assert!(crisis_only.state.active_crisis.is_some(),
            "should hit a crisis");

        // Now resolve with enter — should advance beyond the first crisis point
        let resumed = run_snapshot(
            state,
            &["d60".to_string(), "enter".to_string()],
        ).unwrap();
        assert!(resumed.state.tick > tick_at_first_crisis,
            "should advance past first crisis (was {}, now {})",
            tick_at_first_crisis, resumed.state.tick);
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
    fn stalemate_screen_renders() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Stalemate;
        // Simulate some regions collapsed during the epidemic
        state.regions[0].collapsed = true;
        state.regions[0].collapsed_at_tick = Some(600);
        let screen = render_to_string(&state);
        assert!(screen.contains("STALEMATE"),
            "should show STALEMATE header");
        assert!(screen.contains("burned itself out"),
            "should show stalemate headline");
        assert!(screen.contains("Collapse Timeline"),
            "should show collapse timeline for partial collapse");
    }
}
