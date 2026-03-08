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

/// Run snapshot mode: apply inputs, advance ticks, render.
/// Returns both the rendered screen and the updated state.
pub fn run_snapshot(
    mut state: GameState,
    keys: &[String],
    ticks: Option<u64>,
) -> Result<SnapshotResult, String> {
    // Apply key actions in order
    for key_str in keys {
        match string_to_action(key_str) {
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

    // Advance ticks if requested — unpause so simulation actually advances
    if let Some(n) = ticks {
        state.paused = false;
        for _ in 0..n {
            state = tick(&state);
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
        let result = run_snapshot(state, &[], Some(10)).unwrap();
        assert!(result.screen.contains("Tick: 10"));
        assert_eq!(result.state.tick, 10);
    }

    #[test]
    fn snapshot_with_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["t".to_string()], None).unwrap();
        assert!(result.screen.contains("Threats"));
        // Diseases start unknown — name is hidden until research reveals it
        assert!(result.screen.contains("Unknown Pathogen #1"));
    }

    #[test]
    fn snapshot_with_multiple_keys() {
        let state = GameState::new_default(42);
        // Navigate to Threats then press down to select second item
        let result = run_snapshot(state, &["t".to_string(), "down".to_string()], None).unwrap();
        assert!(result.screen.contains("Threats"));
    }

    #[test]
    fn snapshot_invalid_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["x".to_string()], None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown key"));
    }
}
