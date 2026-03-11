pub mod action;
pub mod engine;
pub mod snapshot;
pub mod state;
pub mod ui;

use action::Action;
use engine::execute_command;
use state::{
    DeployTarget, DECREE_COUNT, FIELD_OP_TYPE_COUNT, FieldOpKind, GameCommand, GameOutcome, GameState,
    KNOWLEDGE_NAME, MANAGE_APPEASE_POS, MANAGE_BARGAIN_POS, MANAGE_PRIORITY_POS,
    MedicineUiState, OpsUiState, Panel, PolicyUiState, RESEARCH_TRACK_COUNT, ResearchTrack, ResearchUiState, SimState,
    STANDING_ORDER_COUNT, UiState, grid_reading_order, policy_display_order,
};

/// Route a player action to the appropriate handler.
///
/// This is the bridge between raw input (Action) and game logic (engine).
/// UI-only actions (panel navigation, selection) are handled directly via
/// UiState methods. Game commands (deploy, research) go through
/// engine::execute_command(). This function does NOT live in engine.rs
/// because it's coordination logic, not game simulation.
///
/// Wizard confirm logic (handle_confirm and friends below) lives here too —
/// it's the "what happens when you press Enter" half of the coordination layer,
/// complementing the post-command UI navigation done inline after execute_command.
pub fn apply_action(state: &GameState, action: &Action) -> GameState {
    let mut new = state.clone();
    new.ui.status_message = None;

    // When a crisis is active (Event state), only allow selecting options and confirming
    if new.active_crisis.is_some() {
        match action {
            Action::SelectNext | Action::SelectRight => {
                let max = new.active_crisis.as_ref()
                    .map(|c| c.options.len().saturating_sub(1))
                    .unwrap_or(0);
                if new.ui.crisis_selection < max {
                    new.ui.crisis_selection += 1;
                }
            }
            Action::SelectPrev | Action::SelectLeft => {
                if new.ui.crisis_selection > 0 {
                    new.ui.crisis_selection -= 1;
                }
            }
            Action::ToggleExtra => {
                new.ui.crisis_auto_resolve = !new.ui.crisis_auto_resolve;
            }
            Action::Confirm => {
                let choice = new.ui.crisis_selection;
                // Check if the selected option is affordable
                let option = &new.active_crisis.as_ref().unwrap().options[choice];
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
                ui::process_events(&mut new);
                new.ui.status_message = result.message;
                new.ui.crisis_selection = 0;
                new.ui.crisis_auto_resolve = false;
                // Close the policy panel on crisis dismiss — there is no safe
                // "browse" intermediate state to reset to (ManagePolicies is the
                // top level and would allow a stray Enter to toggle a policy).
                // The player can press P again to reopen.
                if new.ui.policy_ui.is_some() {
                    new.ui.open_panel = Panel::None;
                    new.ui.policy_ui = None;
                    new.ui.panel_selection = 0;
                }
                if new.ui.medicine_ui.is_some() {
                    new.ui.medicine_ui = Some(MedicineUiState::BrowseMedicines);
                    new.ui.panel_selection = 0;
                }
                if new.ui.research_ui.is_some() {
                    new.ui.research_ui = Some(ResearchUiState::BrowseCategories);
                    new.ui.panel_selection = 0;
                }
                if new.ui.operations_ui.is_some() {
                    new.ui.operations_ui = Some(OpsUiState::BrowseOps);
                    new.ui.panel_selection = 0;
                }
                // sim_state restoration (Event → Running/Paused) happens inside
                // crisis::resolve_crisis() — no post-processing needed here.
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
        Action::OpenOperations => new.ui.toggle_panel(Panel::Operations, new.regions.len()),
        Action::OpenHelp => new.ui.toggle_panel(Panel::Help, new.regions.len()),
        Action::ClosePanel => new.ui.close_panel(&new.medicines, &new.diseases),
        Action::GoHome => new.ui.go_home(),
        Action::SelectNext => {
            let max = new.ui.panel_selection_max(&new);
            new.ui.select_next(new.regions.len(), max);
        }
        Action::SelectPrev => {
            let max = new.ui.panel_selection_max(&new);
            new.ui.select_prev(new.regions.len(), max);
        }
        Action::SelectLeft => {
            new.ui.select_left(new.regions.len());
        }
        Action::SelectRight => {
            new.ui.select_right(new.regions.len());
        }
        Action::JumpToItem { index } => {
            // Jump directly to item N in the current panel list (only when a panel is open).
            if new.ui.open_panel != Panel::None {
                let max = new.ui.panel_selection_max(&new);
                new.ui.panel_selection = (*index).min(max);
            }
        }
        Action::ToggleExtra => {
            // Toggle "Assign 2x personnel" on research confirm screen (pure UI state)
            if let Some(ResearchUiState::ConfirmProject { double_personnel, .. }) = &mut new.ui.research_ui {
                *double_personnel = !*double_personnel;
            }
            // Toggle auto-deploy when browsing medicines
            else if new.ui.open_panel == Panel::Medicines
                && matches!(new.ui.medicine_ui, None | Some(MedicineUiState::BrowseMedicines))
            {
                let unlocked: Vec<usize> = new.medicines.iter().enumerate()
                    .filter(|(_, m)| m.unlocked)
                    .map(|(i, _)| i)
                    .collect();
                if let Some(&med_idx) = unlocked.get(new.ui.panel_selection) {
                    execute_command(&mut new, &GameCommand::ToggleAutoDeploy { med_idx });
                }
            }
            // Toggle auto-research when browsing categories or projects
            else {
                let track = match &new.ui.research_ui {
                    Some(ResearchUiState::BrowseCategories) => match new.ui.panel_selection {
                        0 => Some(ResearchTrack::Field),
                        1 => Some(ResearchTrack::Applied),
                        2 => Some(ResearchTrack::Basic),
                        _ => None,
                    },
                    Some(ResearchUiState::BrowseProjects { track }) => Some(*track),
                    Some(ResearchUiState::ViewActive { track, .. }) => Some(*track),
                    _ => None,
                };
                if let Some(track) = track {
                    execute_command(&mut new, &GameCommand::ToggleAutoResearch { track });
                }
            }
        }
        Action::Confirm => {
            if new.outcome == GameOutcome::Playing {
                let state_snapshot = new.clone();
                if let Some(cmd) = handle_confirm(&mut new.ui, &state_snapshot) {
                    let result = execute_command(&mut new, &cmd);
                    ui::process_events(&mut new);
                    // Map engine result to UI navigation (coordination logic)
                    match &cmd {
                        GameCommand::DeployMedicine { medicine_idx, .. } if result.success => {
                            let msg = result.message.clone().unwrap_or_default();
                            new.ui.medicine_ui = Some(MedicineUiState::DeployResult {
                                medicine_idx: *medicine_idx,
                                message: msg,
                            });
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::StartResearch { track, .. } if result.success => {
                            new.ui.research_ui = Some(ResearchUiState::BrowseProjects { track: *track });
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::EnactDecree { .. } if result.success => {
                            // Return to BrowseOps after enacting (decrees are in the Orders panel)
                            new.ui.operations_ui = Some(OpsUiState::BrowseOps);
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

/// Advance the simulation by one tick and process the resulting events into
/// UI state (event log, notifications, panel resets).
///
/// This is the canonical way to advance the game simulation. Both
/// `engine::tick()` and `ui::process_events()` are `pub(crate)`, so external
/// callers must go through this function and cannot split the pairing.
/// Engine unit tests may call `engine::tick()` directly to test game logic
/// in isolation without UI state updates.
pub fn tick_and_process(state: &GameState) -> GameState {
    let mut new = engine::tick(state);
    ui::process_events(&mut new);
    new
}

/// Format a number with human-readable suffix (K, M, B).
// Re-export from state.rs so all existing `crate::format_number` references work.
pub use state::format_number;

// ── Wizard confirm handlers ───────────────────────────────────────────────────
//
// These translate a Confirm keypress (Enter) into an optional GameCommand.
// They live here — the coordination layer — not in state.rs, because they're
// behavioral logic (decision-making, command synthesis, UX pre-validation),
// not data or simple state mutations. state.rs owns the UiState struct and its
// navigation/panel methods; lib.rs owns what happens when the player acts.

/// Dispatch Confirm to the active panel's wizard handler.
fn handle_confirm(ui: &mut UiState, state: &GameState) -> Option<GameCommand> {
    match ui.open_panel {
        Panel::Medicines => handle_medicine_confirm(ui, state),
        Panel::Research => handle_research_confirm(ui, state),
        Panel::Policy => handle_policy_confirm(ui, state),
        Panel::Operations => handle_operations_confirm(ui, state),
        _ => None,
    }
}

fn handle_medicine_confirm(ui: &mut UiState, state: &GameState) -> Option<GameCommand> {
    match ui.medicine_ui.clone() {
        Some(MedicineUiState::BrowseMedicines) => {
            let unlocked: Vec<usize> = state
                .medicines
                .iter()
                .enumerate()
                .filter(|(_, m)| m.unlocked)
                .map(|(i, _)| i)
                .collect();
            if let Some(&med_idx) = unlocked.get(ui.panel_selection) {
                ui.medicine_ui =
                    Some(MedicineUiState::SelectRegion { medicine_idx: med_idx });
                // Pre-select the region matching the current map selection
                let order = grid_reading_order(state.regions.len());
                ui.panel_selection = order.iter()
                    .position(|&idx| idx == ui.map_selection)
                    .unwrap_or(0);
            }
            None
        }
        Some(MedicineUiState::SelectRegion { medicine_idx }) => {
            let order = grid_reading_order(state.regions.len());
            let region_idx = order.get(ui.panel_selection).copied().unwrap_or(0);
            if region_idx < state.regions.len() {
                let med = &state.medicines[medicine_idx];
                let deployable = med.deployable_diseases(&state.diseases);
                let has_known_incompatible = state.diseases.iter().enumerate()
                    .any(|(i, d)| d.detected && d.knowledge >= KNOWLEDGE_NAME && !deployable.contains(&i));
                if deployable.len() == 1 && !has_known_incompatible {
                    // Only one deployable disease and no incompatible ones to explain:
                    // skip disease selection to save the player a step.
                    ui.medicine_ui = Some(MedicineUiState::SelectTarget {
                        medicine_idx,
                        region_idx,
                        disease_idx: deployable[0],
                    });
                } else {
                    ui.medicine_ui = Some(MedicineUiState::SelectDisease {
                        medicine_idx,
                        region_idx,
                    });
                }
                ui.panel_selection = 0;
            }
            None
        }
        Some(MedicineUiState::SelectDisease { medicine_idx, region_idx }) => {
            let med = &state.medicines[medicine_idx];
            let deployable = med.deployable_diseases(&state.diseases);
            if let Some(&disease_idx) = deployable.get(ui.panel_selection) {
                ui.medicine_ui = Some(MedicineUiState::SelectTarget {
                    medicine_idx,
                    region_idx,
                    disease_idx,
                });
                ui.panel_selection = 0;
            }
            None
        }
        Some(MedicineUiState::SelectTarget {
            medicine_idx,
            region_idx,
            disease_idx,
        }) => {
            let med = &state.medicines[medicine_idx];
            // panel_selection: 0 = vaccinate, 1 = treat
            let target = if ui.panel_selection == 0 {
                DeployTarget::Vaccinate { disease_idx }
            } else {
                DeployTarget::Treat { disease_idx }
            };
            // UX pre-check: show error early so the wizard doesn't advance to ConfirmDeploy
            // when the player can't afford it. Uses medicine_deploy_cost() — the same calculation
            // as the engine — so this preview can never drift from the authoritative check.
            let deploy_cost = state.medicine_deploy_cost(medicine_idx, region_idx);
            if state.resources.funding < deploy_cost {
                ui.status_message = Some(
                    format!("Insufficient funds! Need ¥{:.0}, have ¥{:.0}",
                        deploy_cost, state.resources.funding),
                );
                None
            } else {
                let is_tested = med.tested_against.contains(&disease_idx);
                if !is_tested {
                    ui.medicine_ui = Some(MedicineUiState::ConfirmDeploy {
                        medicine_idx,
                        region_idx,
                        target: target.clone(),
                    });
                    None
                } else {
                    Some(GameCommand::DeployMedicine {
                        medicine_idx,
                        region_idx,
                        target,
                    })
                }
            }
        }
        Some(MedicineUiState::ConfirmDeploy {
            medicine_idx,
            region_idx,
            target,
        }) => {
            Some(GameCommand::DeployMedicine {
                medicine_idx,
                region_idx,
                target,
            })
        }
        Some(MedicineUiState::DeployResult { medicine_idx, .. }) => {
            ui.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
            ui.panel_selection = 0;
            None
        }
        None => None,
    }
}

fn handle_research_confirm(ui: &mut UiState, state: &GameState) -> Option<GameCommand> {
    match ui.research_ui.clone() {
        Some(ResearchUiState::BrowseCategories) => {
            if ui.panel_selection == RESEARCH_TRACK_COUNT {
                // Upgrade Lab
                return Some(GameCommand::UpgradeLab);
            }
            let track = match ui.panel_selection {
                0 => ResearchTrack::Field,
                1 => ResearchTrack::Applied,
                _ => ResearchTrack::Basic,
            };
            ui.research_ui = Some(ResearchUiState::BrowseProjects { track });
            ui.panel_selection = 0;
            None
        }
        Some(ResearchUiState::BrowseProjects { track }) => {
            if track == ResearchTrack::Field {
                // Field track: list shows active projects first, then available.
                let n_active = state.field_research.len();
                if ui.panel_selection < n_active {
                    // Selected an active project → view it
                    ui.research_ui = Some(ResearchUiState::ViewActive { track, slot_idx: ui.panel_selection });
                    ui.panel_selection = 0;
                } else {
                    // Selected an available project
                    let project_idx = ui.panel_selection - n_active;
                    let count = state.available_projects(track).len();
                    if project_idx < count && state.field_research_has_capacity() {
                        ui.research_ui = Some(ResearchUiState::ConfirmProject {
                            track,
                            project_idx,
                            double_personnel: false,
                        });
                        ui.panel_selection = 0;
                    }
                }
            } else {
                // Applied/Basic: single-slot behavior
                if state.research_slot(track).is_some() {
                    ui.research_ui = Some(ResearchUiState::ViewActive { track, slot_idx: 0 });
                    ui.panel_selection = 0;
                } else {
                    let count = state.available_projects(track).len();
                    if count > 0 {
                        ui.research_ui = Some(ResearchUiState::ConfirmProject {
                            track,
                            project_idx: ui.panel_selection,
                            double_personnel: false,
                        });
                        ui.panel_selection = 0;
                    }
                }
            }
            None
        }
        Some(ResearchUiState::ConfirmProject { track, project_idx, double_personnel }) => {
            Some(GameCommand::StartResearch { track, project_idx, double_personnel })
        }
        Some(ResearchUiState::ViewActive { track, .. }) => {
            // Confirm from ViewActive goes back to project list
            ui.research_ui = Some(ResearchUiState::BrowseProjects { track });
            ui.panel_selection = 0;
            None
        }
        None => None,
    }
}

fn handle_policy_confirm(ui: &mut UiState, _state: &GameState) -> Option<GameCommand> {
    match ui.policy_ui.clone() {
        Some(PolicyUiState::ManagePolicies { region_idx }) => {
            if ui.panel_selection == MANAGE_BARGAIN_POS {
                // Bargain with Governor (only when defiant)
                Some(GameCommand::BargainWithGovernor { region_idx })
            } else if ui.panel_selection == MANAGE_APPEASE_POS {
                // Appease Governor
                Some(GameCommand::AppeaseGovernor { region_idx })
            } else if ui.panel_selection == MANAGE_PRIORITY_POS {
                // Cycle deployment priority
                Some(GameCommand::CycleDeployPriority { region_idx })
            } else {
                // Map display position to policy_idx via sorted display order
                let policy_idx = policy_display_order()[ui.panel_selection];
                Some(GameCommand::TogglePolicy {
                    region_idx,
                    policy_idx,
                })
            }
        }
        None => None,
    }
}

fn handle_operations_confirm(ui: &mut UiState, state: &GameState) -> Option<GameCommand> {
    match ui.operations_ui.clone() {
        Some(OpsUiState::BrowseOps) => {
            let n_active = state.field_operations.len();
            let op_type_base = n_active;
            let decree_base = op_type_base + FIELD_OP_TYPE_COUNT;
            let so_base = decree_base + DECREE_COUNT;
            let loan_base = so_base + STANDING_ORDER_COUNT;

            if ui.panel_selection < n_active {
                // Selected an active op — no action (view only)
                None
            } else if ui.panel_selection < decree_base {
                // Selected an operation type
                match ui.panel_selection - op_type_base {
                    0 => {
                        // Recon — need to pick a disease
                        let targets: Vec<usize> = state.diseases.iter().enumerate()
                            .filter(|(_, d)| d.detected && d.knowledge < KNOWLEDGE_NAME)
                            .map(|(i, _)| i)
                            .collect();
                        if targets.is_empty() {
                            ui.status_message = Some("No unidentified pathogens".into());
                            None
                        } else if targets.len() == 1 {
                            // Only one target — skip selection
                            Some(GameCommand::StartFieldOp {
                                kind: FieldOpKind::Recon { disease_idx: targets[0] },
                            })
                        } else {
                            ui.operations_ui = Some(OpsUiState::SelectReconTarget);
                            ui.panel_selection = 0;
                            None
                        }
                    }
                    1 => {
                        // Emergency Response — pick a region
                        ui.operations_ui = Some(OpsUiState::SelectEmergencyTarget);
                        ui.panel_selection = 0;
                        None
                    }
                    2 => {
                        // Infra Survey — pick a region
                        ui.operations_ui = Some(OpsUiState::SelectSurveyTarget);
                        ui.panel_selection = 0;
                        None
                    }
                    3 => {
                        // Supply Chain Reinforcement — pick a region
                        ui.operations_ui = Some(OpsUiState::SelectSupplyTarget);
                        ui.panel_selection = 0;
                        None
                    }
                    4 => {
                        // Civil Order Stabilization — pick a region
                        ui.operations_ui = Some(OpsUiState::SelectCivilOrderTarget);
                        ui.panel_selection = 0;
                        None
                    }
                    _ => None,
                }
            } else if ui.panel_selection >= loan_base {
                // Loan selected — repay in full
                let loan_idx = ui.panel_selection - loan_base;
                Some(GameCommand::RepayLoan { loan_idx })
            } else if ui.panel_selection >= so_base {
                // Standing order selected — toggle
                let kind = ui.panel_selection - so_base;
                Some(GameCommand::ToggleStandingOrder { kind })
            } else {
                // Decree selected
                let decree_idx = ui.panel_selection - decree_base;
                if state.enacted_decrees.is_enacted(decree_idx) || !state.decree_unlocked(decree_idx) {
                    None
                } else if decree_idx == 2 {
                    // Sacrifice Region — needs region selection
                    ui.operations_ui = Some(OpsUiState::SelectSacrificeRegion);
                    ui.panel_selection = 0;
                    None
                } else if decree_idx == 4 {
                    // Fortify Region — needs region selection
                    ui.operations_ui = Some(OpsUiState::SelectFortifyRegion);
                    ui.panel_selection = 0;
                    None
                } else {
                    // All other decrees go through confirmation
                    ui.operations_ui = Some(OpsUiState::ConfirmDecree { decree_idx });
                    ui.panel_selection = 0;
                    None
                }
            }
        }
        Some(OpsUiState::ConfirmDecree { decree_idx }) => {
            Some(GameCommand::EnactDecree { decree_idx, region_idx: None })
        }
        Some(OpsUiState::SelectSacrificeRegion) => {
            let non_collapsed: Vec<usize> = state.regions.iter()
                .enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::EnactDecree { decree_idx: 2, region_idx: Some(region_idx) })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectFortifyRegion) => {
            let non_collapsed: Vec<usize> = state.regions.iter()
                .enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::EnactDecree { decree_idx: 4, region_idx: Some(region_idx) })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectReconTarget) => {
            let targets: Vec<usize> = state.diseases.iter().enumerate()
                .filter(|(_, d)| d.detected && d.knowledge < KNOWLEDGE_NAME)
                .map(|(i, _)| i)
                .collect();
            if let Some(&disease_idx) = targets.get(ui.panel_selection) {
                Some(GameCommand::StartFieldOp {
                    kind: FieldOpKind::Recon { disease_idx },
                })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectEmergencyTarget) => {
            let non_collapsed: Vec<usize> = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::StartFieldOp {
                    kind: FieldOpKind::EmergencyResponse { region_idx },
                })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectSurveyTarget) => {
            let non_collapsed: Vec<usize> = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::StartFieldOp {
                    kind: FieldOpKind::InfraSurvey { region_idx },
                })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectSupplyTarget) => {
            let non_collapsed: Vec<usize> = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::StartFieldOp {
                    kind: FieldOpKind::SupplyChainReinforcement { region_idx },
                })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectCivilOrderTarget) => {
            let non_collapsed: Vec<usize> = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::StartFieldOp {
                    kind: FieldOpKind::CivilOrderStabilization { region_idx },
                })
            } else {
                None
            }
        }
        None => None,
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
            options: vec![ CrisisOption { label: "Take funding".into(), description: "Get ¥500".into(), cost: None },
             CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            ],
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
            options: vec![ CrisisOption { label: "Take funding".into(), description: "Get ¥500".into(), cost: None },
             CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            ],
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
            options: vec![ CrisisOption { label: "Take funding".into(), description: "Get ¥500".into(), cost: None },
             CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            ],
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
            options: vec![ CrisisOption { label: "Take funding".into(), description: "Get ¥500".into(), cost: None },
             CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            ],
            tick_created: 0,
        });

        // Confirm WITHOUT [X] — should clear the existing preference
        let state = apply_action(&state, &Action::Confirm);
        assert!(!state.auto_resolve_crises.contains_key("aid"),
            "manually handling a crisis should clear saved preference");
    }

    #[test]
    fn jump_to_item_moves_panel_selection() {
        let state = GameState::new_default(42);

        // No panel open — JumpToItem should be ignored
        let state = apply_action(&state, &Action::JumpToItem { index: 2 });
        assert_eq!(state.ui.panel_selection, 0, "JumpToItem ignored when no panel open");

        // Open research panel (BrowseCategories: 0=Field, 1=Applied, 2=Basic, 3=UpgradeLab)
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.panel_selection, 0);

        // Key '3' → index 2 → should select Basic (index 2)
        let state = apply_action(&state, &Action::JumpToItem { index: 2 });
        assert_eq!(state.ui.panel_selection, 2, "key 3 should jump to index 2");

        // Key '1' → index 0 → should jump back to Field
        let state = apply_action(&state, &Action::JumpToItem { index: 0 });
        assert_eq!(state.ui.panel_selection, 0, "key 1 should jump to index 0");

        // Key '0' → index 9, clamped to max (3 in BrowseCategories)
        let state = apply_action(&state, &Action::JumpToItem { index: 9 });
        assert_eq!(state.ui.panel_selection, 3, "key 0 should clamp to max index");
    }

    #[test]
    fn panel_hotkey_resets_to_top_when_deep_in_wizard() {
        let state = GameState::new_default(42);
        // Open research → enter field category → now at BrowseProjects
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::Research);
        let state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseProjects { .. })));

        // Press R again — should reset to BrowseCategories, NOT close the panel
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::Research);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseCategories)));

        // Press R again at top level — now it closes
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn crisis_dismiss_closes_policy_panel() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        // Set up state with policy panel open in ManagePolicies.
        let mut state = GameState::new_default(42);
        state.ui.open_panel = Panel::Policy;
        state.ui.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: 0 });
        state.ui.panel_selection = 0;

        // Fire a crisis while in this state.
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid Offer".into(),
            description: "Choose wisely".into(),
            options: vec![
                CrisisOption { label: "Take funding".into(), description: "Get ¥500".into(), cost: None },
                CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            ],
            tick_created: 0,
        });

        // Dismiss the crisis.
        let state = apply_action(&state, &Action::Confirm);
        assert!(state.active_crisis.is_none(), "crisis should be dismissed");

        // Policy panel must be closed entirely — there is no safe intermediate state
        // to reset to (ManagePolicies is the top level and is an "action" state).
        // Closing prevents a stray Enter from accidentally toggling a policy.
        assert_eq!(state.ui.open_panel, Panel::None,
            "policy panel should close after crisis dismissal");
        assert!(state.ui.policy_ui.is_none(),
            "policy_ui should be None after crisis dismissal, got {:?}", state.ui.policy_ui);
        assert_eq!(state.ui.panel_selection, 0);

        // A stray Enter now does nothing (no panel open).
        let state = apply_action(&state, &Action::Confirm);
        assert!(!state.policies[0].border_controls,
            "stray enter after crisis dismissal must not toggle border_controls");
    }

    #[test]
    fn select_region_syncs_with_map_navigation() {
        use crate::state::MedicineUiState;

        let mut state = GameState::new_default(42);
        // Unlock a medicine so we can enter the deploy wizard
        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 100.0;

        // Open medicines panel, select first medicine, enter SelectRegion
        let state = apply_action(&state, &Action::OpenMedicines);
        let state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectRegion { .. })));

        // Press right — map and list cursor should both move
        let state = apply_action(&state, &Action::SelectRight);
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectRegion { .. })),
            "should stay in SelectRegion after left/right");
        // The map moved, so panel_selection should update to match
        assert_ne!(state.ui.map_selection, 0,
            "map should have moved from initial position");
        // Verify the list cursor tracks the map
        let order = crate::state::grid_reading_order(state.regions.len());
        let expected = order.iter().position(|&r| r == state.ui.map_selection).unwrap_or(0);
        assert_eq!(state.ui.panel_selection, expected,
            "list cursor should follow map selection");
    }
}
