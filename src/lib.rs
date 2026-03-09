pub mod action;
pub mod engine;
pub mod snapshot;
pub mod state;
pub mod ui;

use action::Action;
use engine::execute_command;
use state::{GameCommand, GameOutcome, GameState, Panel, ResearchUiState, SimState};

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

    // When a crisis is active (Event state), only allow selecting options and confirming
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
                // Check if the selected option is affordable
                let option = if choice == 0 {
                    &new.active_crisis.as_ref().unwrap().option_a
                } else {
                    &new.active_crisis.as_ref().unwrap().option_b
                };
                if let Some(cost) = &option.cost {
                    if !cost.affordable(&new) {
                        new.ui.status_message = Some("Not enough resources".into());
                        return new;
                    }
                }
                let cmd = GameCommand::ResolveCrisis { choice };
                let result = execute_command(&mut new, &cmd);
                new.ui.status_message = result.message;
                new.ui.crisis_selection = 0;
                // Restore pre-event sim state
                if let SimState::Event { was_running } = new.sim_state {
                    new.sim_state = if was_running { SimState::Running } else { SimState::Paused };
                }
            }
            Action::Quit => {} // Still allow quit
            _ => {} // Block all other actions during crisis (including TogglePause)
        }
        return new;
    }

    match action {
        Action::TogglePause => {
            if new.outcome == GameOutcome::Playing {
                match new.sim_state {
                    SimState::Running => {
                        new.sim_state = SimState::Paused;
                        new.ui.speed_multiplier = 1;
                    }
                    SimState::Paused => new.sim_state = SimState::Running,
                    SimState::Event { .. } => {} // blocked during events
                }
            }
        }
        Action::SpeedUp => {
            if new.outcome == GameOutcome::Playing && new.sim_state.is_running() {
                new.ui.speed_multiplier = match new.ui.speed_multiplier {
                    1 => 2,
                    2 => 4,
                    4 => 6,
                    _ => 1,
                };
            }
        }
        Action::OpenThreats => new.ui.toggle_panel(Panel::Threats),
        Action::OpenResearch => new.ui.toggle_panel(Panel::Research),
        Action::OpenMedicines => new.ui.toggle_panel(Panel::Medicines),
        Action::OpenPolicy => {
            let was_open = new.ui.open_panel == Panel::Policy;
            new.ui.toggle_panel(Panel::Policy);
            if !was_open {
                // Pre-select the region matching the current map selection
                let order = state::grid_reading_order(new.regions.len());
                if let Some(pos) = order.iter().position(|&idx| idx == new.ui.map_selection) {
                    new.ui.panel_selection = pos;
                }
            }
        }
        Action::OpenHelp => new.ui.toggle_panel(Panel::Help),
        Action::ClosePanel => new.ui.close_panel(),
        Action::SelectNext => {
            // In ViewActive, up/down adjusts personnel assignment
            // Down = remove (fewer), Up = add (more)
            if let Some(ResearchUiState::ViewActive { bench }) = &new.ui.research_ui {
                let bench = *bench;
                let cmd = GameCommand::RemoveResearchPersonnel { bench };
                let result = execute_command(&mut new, &cmd);
                new.ui.status_message = result.message;
            } else {
                let max = new.ui.panel_selection_max(&new);
                new.ui.select_next(new.regions.len(), max);
            }
        }
        Action::SelectPrev => {
            if let Some(ResearchUiState::ViewActive { bench }) = &new.ui.research_ui {
                let bench = *bench;
                let cmd = GameCommand::AddResearchPersonnel { bench };
                let result = execute_command(&mut new, &cmd);
                new.ui.status_message = result.message;
            } else {
                new.ui.select_prev(new.regions.len());
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_cycles_through_multipliers() {
        let state = GameState::new_default(42);
        assert_eq!(state.ui.speed_multiplier, 1);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.ui.speed_multiplier, 2);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.ui.speed_multiplier, 4);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.ui.speed_multiplier, 6);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.ui.speed_multiplier, 1);
    }

    #[test]
    fn pause_resets_speed() {
        let state = GameState::new_default(42);
        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.ui.speed_multiplier, 2);

        // Pause should reset to 1x
        let state = apply_action(&state, &Action::TogglePause);
        assert_eq!(state.ui.speed_multiplier, 1);
        assert!(!state.sim_state.is_running());
    }

    #[test]
    fn speed_up_ignored_when_paused() {
        let state = GameState::new_default(42);
        let state = apply_action(&state, &Action::TogglePause); // pause
        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.ui.speed_multiplier, 1); // unchanged
    }
}
