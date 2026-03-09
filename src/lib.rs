pub mod action;
pub mod engine;
pub mod snapshot;
pub mod state;
pub mod ui;

use action::Action;
use engine::execute_command;
use state::{GameCommand, GameOutcome, GameState, MedicineUiState, Panel, ResearchUiState, SimState};

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
            Action::ToggleExtra => {
                new.ui.crisis_auto_resolve = !new.ui.crisis_auto_resolve;
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
                // Save or clear auto-resolve preference
                let tag = new.active_crisis.as_ref().unwrap().kind.tag().to_string();
                if new.ui.crisis_auto_resolve {
                    new.auto_resolve_crises.insert(tag, choice);
                } else {
                    // Manually handling a crisis clears any saved preference
                    new.auto_resolve_crises.remove(&tag);
                }
                let cmd = GameCommand::ResolveCrisis { choice };
                let result = execute_command(&mut new, &cmd);
                new.ui.status_message = result.message;
                new.ui.crisis_selection = 0;
                new.ui.crisis_auto_resolve = false;
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
        Action::OpenThreats => new.ui.toggle_panel(Panel::Threats, new.regions.len()),
        Action::OpenResearch => new.ui.toggle_panel(Panel::Research, new.regions.len()),
        Action::OpenMedicines => new.ui.toggle_panel(Panel::Medicines, new.regions.len()),
        Action::OpenPolicy => new.ui.toggle_panel(Panel::Policy, new.regions.len()),
        Action::OpenHelp => new.ui.toggle_panel(Panel::Help, new.regions.len()),
        Action::ClosePanel => new.ui.close_panel(&new.medicines, &new.diseases),
        Action::SelectNext => {
            // In ViewActive, up/down adjusts personnel assignment
            // Down = remove (fewer), Up = add (more)
            if let Some(ResearchUiState::ViewActive { track }) = &new.ui.research_ui {
                let track = *track;
                let cmd = GameCommand::RemoveResearchPersonnel { track };
                let result = execute_command(&mut new, &cmd);
                new.ui.status_message = result.message;
            } else {
                let max = new.ui.panel_selection_max(&new);
                new.ui.select_next(new.regions.len(), max);
            }
        }
        Action::SelectPrev => {
            if let Some(ResearchUiState::ViewActive { track }) = &new.ui.research_ui {
                let track = *track;
                let cmd = GameCommand::AddResearchPersonnel { track };
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
        Action::ToggleExtra => {
            // Toggle "Assign 2x personnel" on research confirm screen
            if let Some(ResearchUiState::ConfirmProject { double_personnel, .. }) = &mut new.ui.research_ui {
                *double_personnel = !*double_personnel;
            }
        }
        Action::Confirm => {
            if new.outcome == GameOutcome::Playing {
                let state_snapshot = new.clone();
                if let Some(cmd) = new.ui.handle_confirm(&state_snapshot) {
                    let result = execute_command(&mut new, &cmd);
                    // Map engine result to UI navigation (coordination logic)
                    match &cmd {
                        GameCommand::DeployMedicine { medicine_idx, .. } if result.success => {
                            let msg = result.message.clone().unwrap_or_default();
                            new.ui.medicine_ui = Some(MedicineUiState::DeployResult {
                                medicine_idx: *medicine_idx,
                                message: msg,
                                adverse: result.adverse,
                            });
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::StartResearch { track, .. } if result.success => {
                            new.ui.research_ui = Some(ResearchUiState::BrowseProjects { track: *track });
                            new.ui.panel_selection = 0;
                        }
                        _ => {}
                    }
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

    #[test]
    fn auto_resolve_toggle_during_crisis() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid Offer".into(),
            description: "Choose wisely".into(),
            option_a: CrisisOption { label: "Take funding".into(), description: "Get $500".into(), cost: None },
            option_b: CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            tick_created: 0,
        });

        // Toggle auto-resolve on
        let state = apply_action(&state, &Action::ToggleExtra);
        assert!(state.ui.crisis_auto_resolve);

        // Toggle it off
        let state = apply_action(&state, &Action::ToggleExtra);
        assert!(!state.ui.crisis_auto_resolve);
    }

    #[test]
    fn auto_resolve_saves_preference_on_confirm() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid Offer".into(),
            description: "Choose wisely".into(),
            option_a: CrisisOption { label: "Take funding".into(), description: "Get $500".into(), cost: None },
            option_b: CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            tick_created: 0,
        });

        // Toggle auto-resolve, select option B, confirm
        let state = apply_action(&state, &Action::ToggleExtra);
        let state = apply_action(&state, &Action::SelectNext); // select B
        let state = apply_action(&state, &Action::Confirm);

        // Preference should be saved
        assert_eq!(state.auto_resolve_crises.get("aid"), Some(&1));
        assert!(state.active_crisis.is_none());
        assert!(!state.ui.crisis_auto_resolve); // reset after confirm
    }

    #[test]
    fn auto_resolve_no_preference_without_toggle() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid Offer".into(),
            description: "Choose wisely".into(),
            option_a: CrisisOption { label: "Take funding".into(), description: "Get $500".into(), cost: None },
            option_b: CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            tick_created: 0,
        });

        // Confirm without toggling auto-resolve
        let state = apply_action(&state, &Action::Confirm);
        assert!(state.auto_resolve_crises.is_empty());
    }

    #[test]
    fn manual_confirm_clears_existing_preference() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        // Pre-existing preference for aid crises
        state.auto_resolve_crises.insert("aid".to_string(), 0);

        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid Offer".into(),
            description: "Choose wisely".into(),
            option_a: CrisisOption { label: "Take funding".into(), description: "Get $500".into(), cost: None },
            option_b: CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            tick_created: 0,
        });

        // Confirm WITHOUT [X] — should clear the existing preference
        let state = apply_action(&state, &Action::Confirm);
        assert!(!state.auto_resolve_crises.contains_key("aid"),
            "manually handling a crisis should clear saved preference");
    }
}
