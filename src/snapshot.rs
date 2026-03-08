use ratatui::{backend::TestBackend, Terminal};

use crate::action::string_to_action;
use crate::engine::{apply_action, tick};
use crate::state::GameState;
use crate::ui;

/// Result of running snapshot mode: the rendered screen and the updated state.
#[derive(Debug)]
pub struct SnapshotResult {
    pub screen: String,
    pub state: GameState,
}

/// Parse a snapshot step string. Returns either a tick count or a key string.
fn parse_step(s: &str) -> Result<SnapshotStep, String> {
    if let Some(rest) = s.strip_prefix('t') {
        if let Ok(n) = rest.parse::<u64>() {
            return Ok(SnapshotStep::Ticks(n));
        }
    }
    // "t" alone is a valid key (opens Threats panel)
    Ok(SnapshotStep::Key(s.to_string()))
}

enum SnapshotStep {
    Ticks(u64),
    Key(String),
}

/// Run snapshot mode: process an ordered sequence of steps, then render.
/// Each step is either a key action (e.g. "r", "enter") or ticks (e.g. "t10").
/// Returns both the rendered screen and the updated state.
pub fn run_snapshot(
    mut state: GameState,
    steps: &[String],
) -> Result<SnapshotResult, String> {
    for step_str in steps {
        match parse_step(step_str)? {
            SnapshotStep::Key(key_str) => {
                match string_to_action(&key_str) {
                    Some(action) => {
                        state = apply_action(&state, &action);
                    }
                    None => {
                        return Err(format!(
                            "Unknown key: {:?}. Valid keys: space, t, r, m, p, ?, esc, up, down, left, right, h, l, enter, q",
                            key_str
                        ));
                    }
                }
            }
            SnapshotStep::Ticks(n) => {
                state.paused = false;
                for _ in 0..n {
                    state = tick(&state);
                    ui::process_events(&mut state);
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
        assert!(output.contains("PANDEMIC DEFENSE"));
        assert!(output.contains("RUNNING"));
        assert!(output.contains("Asia"));
    }

    #[test]
    fn snapshot_with_ticks() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["t10".to_string()]).unwrap();
        assert!(result.screen.contains("Tick: 10"));
        assert_eq!(result.state.tick, 10);
    }

    #[test]
    fn snapshot_with_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["t".to_string()]).unwrap();
        assert!(result.screen.contains("Threats"));
        // Diseases start unknown — name is hidden until research reveals it
        assert!(result.screen.contains("Unknown Pathogen #1"));
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
    fn snapshot_interleaved_ticks_and_keys() {
        let state = GameState::new_default(42);
        // Advance 5 ticks, open threats panel, advance 5 more ticks
        let result = run_snapshot(
            state,
            &["t5".to_string(), "t".to_string(), "t5".to_string()],
        )
        .unwrap();
        assert_eq!(result.state.tick, 10);
        assert!(result.screen.contains("Threats"));
        assert!(result.screen.contains("Tick: 10"));
    }
}
