pub mod action;
pub mod engine;
pub mod snapshot;
pub mod state;
pub mod ui;

use action::Action;
use engine::execute_command;
use state::{GameCommand, GameOutcome, GameState, Panel};

/// Route a player action to the appropriate handler.
///
/// This is the bridge between raw input (Action) and game logic (engine).
/// UI-only actions (panel navigation, selection) are handled directly via
/// UiState methods. Game commands (deploy, research) go through
/// engine::execute_command(). This function does NOT live in engine.rs
/// because it's coordination logic, not game simulation.
pub fn apply_action(state: &GameState, action: &Action) -> GameState {
    let mut new = state.clone();
    new.ui.status_message = None;

    // When a crisis is active, only allow selecting options and confirming
    if new.active_crisis.is_some() {
        match action {
            Action::SelectNext | Action::SelectRight => {
                new.ui.crisis_selection = 1;
            }
            Action::SelectPrev | Action::SelectLeft => {
                new.ui.crisis_selection = 0;
            }
            Action::Confirm => {
                let choice = new.ui.crisis_selection;
                let cmd = GameCommand::ResolveCrisis { choice };
                let result = execute_command(&mut new, &cmd);
                new.ui.status_message = result.message;
                new.ui.crisis_selection = 0;
            }
            Action::Quit => {} // Still allow quit
            _ => {} // Block all other actions during crisis
        }
        return new;
    }

    match action {
        Action::TogglePause => {
            // Can't unpause after game over
            if new.outcome == GameOutcome::Playing {
                new.paused = !new.paused;
            }
        }
        Action::OpenThreats => new.ui.toggle_panel(Panel::Threats),
        Action::OpenResearch => new.ui.toggle_panel(Panel::Research),
        Action::OpenMedicines => new.ui.toggle_panel(Panel::Medicines),
        Action::OpenPolicy => new.ui.toggle_panel(Panel::Policy),
        Action::OpenHelp => new.ui.toggle_panel(Panel::Help),
        Action::ClosePanel => new.ui.close_panel(),
        Action::SelectNext => {
            let max = new.ui.panel_selection_max(&new);
            new.ui.select_next(new.regions.len(), max);
        }
        Action::SelectPrev => {
            new.ui.select_prev(new.regions.len());
        }
        Action::SelectLeft => {
            new.ui.select_left(new.regions.len());
        }
        Action::SelectRight => {
            new.ui.select_right(new.regions.len());
        }
        Action::Confirm => {
            if new.outcome == GameOutcome::Playing {
                let state_snapshot = new.clone();
                if let Some(cmd) = new.ui.handle_confirm(&state_snapshot) {
                    let result = execute_command(&mut new, &cmd);
                    new.ui.apply_command_result(&cmd, result.success);
                    if new.ui.status_message.is_none() {
                        new.ui.status_message = result.message;
                    }
                }
            }
        }
        Action::Quit => {} // Handled by the caller
    }

    new
}

/// Format a number with human-readable suffix (K, M, B).
pub fn format_number(n: f64) -> String {
    let abs = n.abs();
    if abs < 0.5 {
        return "0".to_string();
    }
    if abs >= 999_999_500.0 {
        format!("{:.1}B", n / 1_000_000_000.0)
    } else if abs >= 999_950.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if abs >= 999.5 {
        format!("{:.1}K", n / 1_000.0)
    } else {
        format!("{:.0}", n)
    }
}
