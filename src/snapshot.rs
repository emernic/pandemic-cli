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

/// Run snapshot mode: process an ordered sequence of steps, then render.
/// Each step is either a key action (e.g. "r", "enter") or ticks (e.g. "t10").
///
/// Mirrors interactive mode behavior: if a crisis event or game over occurs
/// during tick advancement, execution stops immediately and remaining steps
/// are dropped — just like a real player would be interrupted. This keeps
/// snapshot mode faithful to the actual gameplay experience.
///
/// Returns both the rendered screen and the updated state.
pub fn run_snapshot(
    mut state: GameState,
    steps: &[String],
) -> Result<SnapshotResult, String> {
    for (step_idx, step_str) in steps.iter().enumerate() {
        match parse_step(step_str)? {
            SnapshotStep::Key(key_str) => {
                match string_to_action(&key_str) {
                    Some(action) => {
                        state = apply_action(&state, &action);
                    }
                    None => {
                        return Err(format!(
                            "Unknown key: {:?}. Valid keys: space, t, r, m, p, ?, esc, up, down, left, right, h, l, enter, z, q",
                            key_str
                        ));
                    }
                }
            }
            SnapshotStep::Ticks(n) => {
                state.sim_state = SimState::Running;
                for tick_i in 0..n {
                    state = tick(&state);
                    ui::process_events(&mut state);

                    // Stop on crisis or game over — just like interactive mode.
                    if state.active_crisis.is_some() || state.outcome != GameOutcome::Playing {
                        let remaining_ticks = n - tick_i - 1;
                        let remaining_steps: Vec<&str> = steps[step_idx + 1..].iter()
                            .map(|s| s.as_str()).collect();

                        let reason = if state.active_crisis.is_some() {
                            "crisis event"
                        } else {
                            "game over"
                        };

                        // Log what was dropped so the caller knows
                        let mut dropped = Vec::new();
                        if remaining_ticks > 0 {
                            let remaining_days = remaining_ticks as f64
                                / crate::state::TICKS_PER_DAY;
                            dropped.push(format!("{:.1} days", remaining_days));
                        }
                        if !remaining_steps.is_empty() {
                            dropped.push(format!(
                                "steps: {}",
                                remaining_steps.join(", ")
                            ));
                        }

                        if !dropped.is_empty() {
                            eprintln!(
                                "Interrupted by {}: dropped {}",
                                reason,
                                dropped.join("; ")
                            );
                        }

                        let screen = render_to_string(&state);
                        return Ok(SnapshotResult { screen, state });
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
        let result = run_snapshot(state, &["x".to_string()]);
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
        // Advance far enough that a crisis fires (crises start after tick 360).
        // With seed 42, this should hit a crisis well before day 30.
        let result = run_snapshot(state, &["d30".to_string()]).unwrap();
        // Should have stopped early due to crisis
        assert!(result.state.active_crisis.is_some(),
            "should have hit a crisis during 30 days");
        assert!(result.state.tick < 30 * 120,
            "should have stopped before reaching 30 days (stopped at tick {})", result.state.tick);
        assert!(result.screen.contains("CRISIS"),
            "should show the crisis screen");
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
}
