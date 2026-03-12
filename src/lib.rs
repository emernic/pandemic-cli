pub mod action;
pub mod engine;
pub mod snapshot;
pub mod state;
pub mod ui;

use action::Action;
use engine::execute_command;
use state::{
    DeployTarget, DECREE_COUNT, GameCommand, GameOutcome, GameState, KNOWLEDGE_NAME,
    LedgerUiState, MANAGE_APPEASE_POS, MANAGE_BARGAIN_POS, MANAGE_PRIORITY_POS,
    MedicineUiState, OpsUiState, Panel, PolicyUiState, ResearchFlatItem, ResearchTrack, ResearchUiState, SimState,
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
                    new.ui.research_ui = Some(ResearchUiState::BrowseAll);
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
        Action::OpenBoard => new.ui.toggle_panel(Panel::Board, new.regions.len()),
        Action::OpenLedger => new.ui.toggle_panel(Panel::Ledger, new.regions.len()),
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
            // Ledger: switch between Buy and Sell confirmation
            if new.ui.open_panel == Panel::Ledger {
                match &new.ui.ledger_ui {
                    Some(LedgerUiState::BrowseStocks) => {
                        // X on browse = sell selected stock
                        let corp_idx = new.ui.panel_selection;
                        let held = new.portfolio.get(corp_idx).copied().unwrap_or(0);
                        if corp_idx < new.corporations.len() && held > 0 {
                            new.ui.ledger_ui = Some(LedgerUiState::ConfirmSell { corp_idx });
                            new.ui.panel_selection = 0;
                        }
                    }
                    Some(LedgerUiState::ConfirmBuy { corp_idx }) => {
                        let held = new.portfolio.get(*corp_idx).copied().unwrap_or(0);
                        if held > 0 {
                            new.ui.ledger_ui = Some(LedgerUiState::ConfirmSell { corp_idx: *corp_idx });
                        }
                    }
                    Some(LedgerUiState::ConfirmSell { corp_idx }) => {
                        new.ui.ledger_ui = Some(LedgerUiState::ConfirmBuy { corp_idx: *corp_idx });
                    }
                    _ => {}
                }
            }
            // Toggle "Assign 2x personnel" on research confirm screen (pure UI state)
            else if let Some(ResearchUiState::ConfirmProject { double_personnel, .. }) = &mut new.ui.research_ui {
                *double_personnel = !*double_personnel;
            }
            // Toggle auto-deploy when browsing medicines
            else if new.ui.open_panel == Panel::Medicines
                && matches!(new.ui.medicine_ui, None | Some(MedicineUiState::BrowseMedicines))
            {
                let unlocked = new.unlocked_medicine_indices();
                if let Some(&med_idx) = unlocked.get(new.ui.panel_selection) {
                    execute_command(&mut new, &GameCommand::ToggleAutoDeploy { med_idx });
                }
            }
            // Toggle auto-research based on which track the cursor is on
            else if matches!(new.ui.research_ui, Some(ResearchUiState::BrowseAll)) {
                let items = new.research_flat_items();
                let track = items.get(new.ui.panel_selection).and_then(|item| item.track());
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
                        GameCommand::StartResearch { .. } if result.success => {
                            new.ui.research_ui = Some(ResearchUiState::BrowseAll);
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
    // Auto-dismiss the home splash once the typewriter animation has played out.
    // The splash reveals one line per tick and has ~27 lines; after 30 ticks the
    // animation is complete and subsequent home-view renders should show the
    // dashboard, not the splash.  Without this, a no-intervention run (where no
    // panel is ever opened) would show the splash again whenever open_panel
    // resets to None (e.g. after crisis resolution).
    if !new.ui.home_splash_done && new.tick >= 30 {
        new.ui.home_splash_done = true;
    }
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
        Panel::Ledger => handle_ledger_confirm(ui, state),
        _ => None,
    }
}

fn handle_medicine_confirm(ui: &mut UiState, state: &GameState) -> Option<GameCommand> {
    match ui.medicine_ui.clone() {
        Some(MedicineUiState::BrowseMedicines) => {
            let unlocked = state.unlocked_medicine_indices();
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
        Some(ResearchUiState::BrowseAll) => {
            let items = state.research_flat_items();
            let Some(item) = items.get(ui.panel_selection) else {
                return None;
            };
            match item {
                ResearchFlatItem::UpgradeLab => {
                    return Some(GameCommand::UpgradeLab);
                }
                // Active projects: Enter is a no-op (info already visible)
                ResearchFlatItem::FieldActive(_)
                | ResearchFlatItem::AppliedActive
                | ResearchFlatItem::BasicActive => {}
                // Available projects: go to confirm screen
                ResearchFlatItem::FieldAvailable(project_idx) => {
                    ui.research_ui = Some(ResearchUiState::ConfirmProject {
                        track: ResearchTrack::Field,
                        project_idx: *project_idx,
                        double_personnel: false,
                    });
                    ui.panel_selection = 0;
                }
                ResearchFlatItem::AppliedAvailable(project_idx) => {
                    ui.research_ui = Some(ResearchUiState::ConfirmProject {
                        track: ResearchTrack::Applied,
                        project_idx: *project_idx,
                        double_personnel: false,
                    });
                    ui.panel_selection = 0;
                }
                ResearchFlatItem::BasicAvailable(project_idx) => {
                    ui.research_ui = Some(ResearchUiState::ConfirmProject {
                        track: ResearchTrack::Basic,
                        project_idx: *project_idx,
                        double_personnel: false,
                    });
                    ui.panel_selection = 0;
                }
            }
            None
        }
        Some(ResearchUiState::ConfirmProject { track, project_idx, double_personnel }) => {
            Some(GameCommand::StartResearch { track, project_idx, double_personnel })
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
            let so_base = DECREE_COUNT;
            let loan_base = so_base + STANDING_ORDER_COUNT;

            if ui.panel_selection >= loan_base {
                // Loan selected — repay in full
                let loan_idx = ui.panel_selection - loan_base;
                Some(GameCommand::RepayLoan { loan_idx })
            } else if ui.panel_selection >= so_base {
                // Standing order selected — toggle
                let kind = ui.panel_selection - so_base;
                Some(GameCommand::ToggleStandingOrder { kind })
            } else {
                // Decree selected
                let decree_idx = ui.panel_selection;
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
        None => None,
    }
}

/// Buy quantity: 10 shares per confirm press. Keeps the interaction snappy.
const LEDGER_TRADE_QUANTITY: u32 = 10;

fn handle_ledger_confirm(ui: &mut UiState, state: &GameState) -> Option<GameCommand> {
    match ui.ledger_ui.clone() {
        Some(LedgerUiState::BrowseStocks) => {
            let corp_idx = ui.panel_selection;
            if corp_idx < state.corporations.len() && !state.corporations[corp_idx].bankrupt {
                ui.ledger_ui = Some(LedgerUiState::ConfirmBuy { corp_idx });
                ui.panel_selection = 0;
            }
            None
        }
        Some(LedgerUiState::ConfirmBuy { corp_idx }) => {
            ui.ledger_ui = Some(LedgerUiState::BrowseStocks);
            ui.panel_selection = corp_idx;
            Some(GameCommand::BuyShares { corp_idx, quantity: LEDGER_TRADE_QUANTITY })
        }
        Some(LedgerUiState::ConfirmSell { corp_idx }) => {
            let held = state.portfolio.get(corp_idx).copied().unwrap_or(0);
            let quantity = held.min(LEDGER_TRADE_QUANTITY);
            ui.ledger_ui = Some(LedgerUiState::BrowseStocks);
            ui.panel_selection = corp_idx;
            if quantity > 0 {
                Some(GameCommand::SellShares { corp_idx, quantity })
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

        // Open research panel (flat list: items depend on game state)
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.panel_selection, 0);
        let max = state.research_flat_items().len().saturating_sub(1);

        // Key '3' → index 2 → should select item at index 2 (or clamp)
        let state = apply_action(&state, &Action::JumpToItem { index: 2 });
        assert_eq!(state.ui.panel_selection, 2.min(max), "key 3 should jump to index 2");

        // Key '1' → index 0 → should jump back to first item
        let state = apply_action(&state, &Action::JumpToItem { index: 0 });
        assert_eq!(state.ui.panel_selection, 0, "key 1 should jump to index 0");

        // Key '0' → index 9, clamped to max
        let state = apply_action(&state, &Action::JumpToItem { index: 9 });
        assert_eq!(state.ui.panel_selection, max, "key 0 should clamp to max index");
    }

    #[test]
    fn panel_hotkey_resets_to_top_when_deep_in_wizard() {
        let state = GameState::new_default(42);
        // Open research → confirm first item → now at ConfirmProject
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::Research);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));
        let state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ConfirmProject { .. })));

        // Press R again — should reset to BrowseAll, NOT close the panel
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::Research);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));

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
