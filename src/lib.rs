pub mod action;
pub mod engine;
pub mod events;
pub mod persistence;
pub mod snapshot;
pub mod state;
pub mod ui;

use action::Action;
use engine::execute_command;
use state::{
    DecreeId, DeployTarget, DECREE_COUNT, GameCommand, GameOutcome, GameState, KNOWLEDGE_NAME,
    LedgerUiState, MANAGE_NEGOTIATE_POS, MANAGE_BARGAIN_POS,
    MedicineMode, MedicineUiState, OpsUiState, Panel, PolicyId, PolicyUiState, POLICY_COUNT,
    ResearchFlatItem, ResearchUiState, SimState,
    STANDING_ORDER_COUNT, StandingOrderKind, UiState, grid_reading_order, policy_display_order,
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
                events::process_events(&mut new);
                new.ui.status_message = result.message;
                new.ui.crisis_selection = 0;
                new.ui.crisis_auto_resolve = false;
                // Restore the player to whatever panel/wizard state they were in
                // before the crisis fired. Clamp panel_selection in case the crisis
                // resolution changed the number of items in the active list.
                let max = ui::panel_selection_max(&new.ui, &new);
                if new.ui.panel_selection > max {
                    new.ui.panel_selection = max;
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
            let max = ui::panel_selection_max(&new.ui, &new);
            new.ui.select_next(new.regions.len(), max);
        }
        Action::SelectPrev => {
            let max = ui::panel_selection_max(&new.ui, &new);
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
                let max = ui::panel_selection_max(&new.ui, &new);
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
                        }
                    }
                    Some(LedgerUiState::ConfirmBuy { corp_idx }) => {
                        let held = new.portfolio.get(*corp_idx).copied().unwrap_or(0);
                        if held > 0 {
                            new.ui.ledger_ui = Some(LedgerUiState::ConfirmSell { corp_idx: *corp_idx });
                        } else {
                            // No shares to sell — skip to bailout
                            new.ui.ledger_ui = Some(LedgerUiState::ConfirmBailout { corp_idx: *corp_idx });
                        }
                    }
                    Some(LedgerUiState::ConfirmSell { corp_idx }) => {
                        new.ui.ledger_ui = Some(LedgerUiState::ConfirmBailout { corp_idx: *corp_idx });
                    }
                    Some(LedgerUiState::ConfirmBailout { corp_idx }) => {
                        new.ui.ledger_ui = Some(LedgerUiState::ConfirmBuy { corp_idx: *corp_idx });
                    }
                    _ => {}
                }
            }
            // Board: cancel the selected member's contract
            else if new.ui.open_panel == Panel::Board {
                let board_member_idx = new.ui.panel_selection;
                if new.contracts.iter().any(|c| c.board_member_idx == board_member_idx) {
                    let result = execute_command(&mut new, &GameCommand::CancelContract { board_member_idx });
                    if new.ui.status_message.is_none() {
                        new.ui.status_message = result.message;
                    }
                }
            }
            // Toggle "Assign 2x personnel" on research confirm screen (pure UI state)
            else if new.ui.open_panel == Panel::Research {
                if let Some(ResearchUiState::ConfirmProject { double_personnel, .. }) = &mut new.ui.research_ui {
                    *double_personnel = !*double_personnel;
                } else if matches!(new.ui.research_ui, Some(ResearchUiState::BrowseAll)) {
                    // Toggle auto-repeat for the selected repeatable project
                    let items = new.research_flat_items();
                    if let Some(item) = items.get(new.ui.panel_selection) {
                        // For available items, get the kind from the available list
                        let kind = match item {
                            ResearchFlatItem::Available(idx) => {
                                new.all_available_projects().get(*idx).cloned()
                            }
                            ResearchFlatItem::Active(idx) => {
                                new.active_research.get(*idx).map(|p| p.kind.clone())
                            }
                            ResearchFlatItem::FullStockpile(k) => Some(k.clone()),
                            _ => None,
                        };
                        if let Some(kind) = kind {
                            if kind.is_repeatable() {
                                execute_command(&mut new, &GameCommand::ToggleAutoRepeat { kind });
                            }
                        }
                    }
                }
            }
            // Toggle auto-rebuild infra when X pressed on RebuildInfra policy
            else if new.ui.open_panel == Panel::Policy {
                let region_idx = match &new.ui.policy_ui {
                    Some(PolicyUiState::ManagePolicies { region_idx }) => *region_idx,
                    None => new.ui.map_selection,
                };
                let display_pos = new.ui.panel_selection;
                if display_pos < POLICY_COUNT {
                    let policy = policy_display_order()[display_pos];
                    if policy == PolicyId::RebuildInfra {
                        execute_command(&mut new, &GameCommand::ToggleAutoRebuild { region_idx });
                    }
                }
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
        }
        Action::Confirm => {
            // If the typewriter animation is still playing, skip to fully revealed.
            if !new.ui.home_splash_done && !new.ui.home_splash_revealed {
                new.ui.home_splash_revealed = true;
                return new;
            }
            if new.outcome == GameOutcome::Playing {
                let state_snapshot = new.clone();
                if let Some(cmd) = handle_confirm(&mut new.ui, &state_snapshot) {
                    let result = execute_command(&mut new, &cmd);
                    events::process_events(&mut new);
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
                        GameCommand::UpgradeLab if result.success => {
                            new.ui.research_ui = Some(ResearchUiState::BrowseAll);
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::EnactDecree { .. } if result.success => {
                            // Return to BrowseOps after enacting (decrees are in the Orders panel)
                            new.ui.operations_ui = Some(OpsUiState::BrowseOps);
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::EmergencySampleDelivery { .. } if result.success => {
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
/// `engine::tick()` and `events::process_events()` are `pub(crate)`, so external
/// callers must go through this function and cannot split the pairing.
/// Engine unit tests may call `engine::tick()` directly to test game logic
/// in isolation without UI state updates.
pub fn tick_and_process(state: &GameState) -> GameState {
    // Capture the currently-selected research item before the tick so we can
    // stabilize panel_selection afterward. When new research options appear
    // (new disease identified, project completes, etc.) the flat list shifts
    // and the index-based selection would jump to a different item.
    let selected_research_item = if matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)) {
        let items = state.research_flat_items();
        items.get(state.ui.panel_selection).cloned()
    } else {
        None
    };

    let mut new = engine::tick(state);
    events::process_events(&mut new);

    // Stabilize research panel selection: find the same item in the new list.
    if let Some(ref old_item) = selected_research_item {
        if matches!(new.ui.research_ui, Some(ResearchUiState::BrowseAll)) {
            let new_items = new.research_flat_items();
            let old_kind = old_item.to_kind(state);
            let found = match old_item {
                ResearchFlatItem::UpgradeLab => {
                    new_items.iter().position(|item| matches!(item, ResearchFlatItem::UpgradeLab))
                }
                _ => {
                    old_kind.as_ref().and_then(|kind| {
                        new_items.iter().position(|item| item.to_kind(&new).as_ref() == Some(kind))
                    })
                }
            };
            if let Some(pos) = found {
                new.ui.panel_selection = pos;
            } else {
                // Item no longer exists (e.g., project completed); clamp to valid range.
                let max = new_items.len().saturating_sub(1);
                if new.ui.panel_selection > max {
                    new.ui.panel_selection = max;
                }
            }
        }
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

/// After selecting a disease, check trial status and VaccinePlatform tech.
/// - Untested: show confirmation screen (always therapeutic — can't vaccinate untested).
/// - Tested + VaccinePlatform: show mode selection (treatment vs vaccination).
/// - Tested + no VaccinePlatform: deploy immediately as therapeutic.
fn try_deploy_or_confirm(
    ui: &mut UiState,
    state: &GameState,
    medicine_idx: usize,
    region_idx: usize,
    disease_idx: usize,
) -> Option<GameCommand> {
    let med = &state.medicines[medicine_idx];
    let is_tested = med.tested_against.contains(&disease_idx);
    if !is_tested {
        let target = DeployTarget { disease_idx, mode: MedicineMode::Therapeutic };
        ui.medicine_ui = Some(MedicineUiState::ConfirmDeploy {
            medicine_idx,
            region_idx,
            target,
        });
        None
    } else if state.can_vaccinate() {
        // Show mode selection: treatment vs vaccination
        ui.medicine_ui = Some(MedicineUiState::SelectMode {
            medicine_idx,
            region_idx,
            disease_idx,
        });
        ui.panel_selection = 0;
        None
    } else {
        let target = DeployTarget { disease_idx, mode: MedicineMode::Therapeutic };
        Some(GameCommand::DeployMedicine {
            medicine_idx,
            region_idx,
            target,
        })
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
                    // skip disease selection and go straight to deploy.
                    let disease_idx = deployable[0];
                    return try_deploy_or_confirm(ui, state, medicine_idx, region_idx, disease_idx);
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
                return try_deploy_or_confirm(ui, state, medicine_idx, region_idx, disease_idx);
            }
            None
        }
        Some(MedicineUiState::SelectMode { medicine_idx, region_idx, disease_idx }) => {
            // 0 = Treatment, 1 = Vaccination
            let mode = if ui.panel_selection == 0 {
                MedicineMode::Therapeutic
            } else {
                MedicineMode::Vaccine
            };
            let target = DeployTarget { disease_idx, mode };
            Some(GameCommand::DeployMedicine {
                medicine_idx,
                region_idx,
                target,
            })
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
                    ui.research_ui = Some(ResearchUiState::ConfirmLabUpgrade);
                    ui.panel_selection = 0;
                    return None;
                }
                // Active projects: Enter is a no-op (info already visible)
                ResearchFlatItem::Active(_) => {}
                // Available projects: go to confirm screen
                ResearchFlatItem::Available(project_idx) => {
                    ui.research_ui = Some(ResearchUiState::ConfirmProject {
                        project_idx: *project_idx,
                        double_personnel: false,
                    });
                    ui.panel_selection = 0;
                }
                // Full stockpile: Enter is a no-op
                ResearchFlatItem::FullStockpile(_) => {}
            }
            None
        }
        Some(ResearchUiState::ConfirmProject { project_idx, double_personnel }) => {
            Some(GameCommand::StartResearch { project_idx, double_personnel })
        }
        Some(ResearchUiState::ConfirmLabUpgrade) => {
            Some(GameCommand::UpgradeLab)
        }
        None => None,
    }
}

fn handle_policy_confirm(ui: &mut UiState, _state: &GameState) -> Option<GameCommand> {
    match ui.policy_ui.clone() {
        Some(PolicyUiState::ManagePolicies { region_idx }) => {
            if ui.panel_selection == MANAGE_BARGAIN_POS {
                // Bargain with Governor (only when hostile)
                Some(GameCommand::BargainWithGovernor { region_idx })
            } else if ui.panel_selection == MANAGE_NEGOTIATE_POS {
                // Negotiate with Governor
                Some(GameCommand::NegotiateGovernor { region_idx })
            } else {
                // Map display position to PolicyId via sorted display order
                let policy = policy_display_order()[ui.panel_selection];
                Some(GameCommand::TogglePolicy {
                    region_idx,
                    policy,
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
            let field_ops_base = so_base + STANDING_ORDER_COUNT;
            let fire_personnel_pos = field_ops_base + 1; // after Emergency Sample Delivery
            let loan_base = field_ops_base + 2; // 2 field ops total

            if ui.panel_selection >= loan_base {
                // Loan selected — repay in full
                let loan_idx = ui.panel_selection - loan_base;
                Some(GameCommand::RepayLoan { loan_idx })
            } else if ui.panel_selection == fire_personnel_pos {
                // Fire Personnel — immediately fire 5 unassigned
                if state.personnel_available() > 0 {
                    Some(GameCommand::FirePersonnel { count: 5 })
                } else {
                    None
                }
            } else if ui.panel_selection >= field_ops_base {
                // Emergency Sample Delivery
                let eligible = crate::ui::operations::emergency_delivery_medicines(state);
                if eligible.is_empty() {
                    None
                } else {
                    ui.operations_ui = Some(OpsUiState::SelectEmergencyMedicine);
                    ui.panel_selection = 0;
                    None
                }
            } else if ui.panel_selection >= so_base {
                // Standing order selected — toggle
                let kind = StandingOrderKind::ALL[ui.panel_selection - so_base];
                Some(GameCommand::ToggleStandingOrder { kind })
            } else {
                // Decree selected
                let decree = DecreeId::from_index(ui.panel_selection);
                if state.enacted_decrees.is_enacted(decree) || !state.decree_unlocked(decree) {
                    None
                } else if decree == DecreeId::SacrificeRegion {
                    // Sacrifice Region — needs region selection
                    ui.operations_ui = Some(OpsUiState::SelectSacrificeRegion);
                    ui.panel_selection = 0;
                    None
                } else if decree == DecreeId::FortifyRegion {
                    // Fortify Region — needs region selection
                    ui.operations_ui = Some(OpsUiState::SelectFortifyRegion);
                    ui.panel_selection = 0;
                    None
                } else {
                    // All other decrees go through confirmation
                    ui.operations_ui = Some(OpsUiState::ConfirmDecree { decree });
                    ui.panel_selection = 0;
                    None
                }
            }
        }
        Some(OpsUiState::ConfirmDecree { decree }) => {
            Some(GameCommand::EnactDecree { decree, region_idx: None })
        }
        Some(OpsUiState::SelectSacrificeRegion) => {
            let non_collapsed: Vec<usize> = state.regions.iter()
                .enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&region_idx) = non_collapsed.get(ui.panel_selection) {
                Some(GameCommand::EnactDecree { decree: DecreeId::SacrificeRegion, region_idx: Some(region_idx) })
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
                Some(GameCommand::EnactDecree { decree: DecreeId::FortifyRegion, region_idx: Some(region_idx) })
            } else {
                None
            }
        }
        Some(OpsUiState::SelectEmergencyMedicine) => {
            let eligible = crate::ui::operations::emergency_delivery_medicines(state);
            if let Some(&medicine_idx) = eligible.get(ui.panel_selection) {
                ui.operations_ui = Some(OpsUiState::ConfirmEmergencyDelivery { medicine_idx });
                ui.panel_selection = 0;
            }
            None
        }
        Some(OpsUiState::ConfirmEmergencyDelivery { medicine_idx }) => {
            Some(GameCommand::EmergencySampleDelivery {
                medicine_idx,
                region_idx: ui.map_selection,
            })
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
        Some(LedgerUiState::ConfirmBailout { corp_idx }) => {
            ui.ledger_ui = Some(LedgerUiState::BrowseStocks);
            ui.panel_selection = corp_idx;
            Some(GameCommand::BailoutCorporation { corp_idx })
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
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
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
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
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
        assert_eq!(state.auto_resolve_crises.get("personnel"), Some(&1));
        assert!(state.active_crisis.is_none());
        assert!(!state.ui.crisis_auto_resolve); // reset after confirm
    }

    #[test]
    fn auto_resolve_no_preference_without_toggle() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
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
        state.auto_resolve_crises.insert("personnel".to_string(), 0);

        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
            title: "Aid Offer".into(),
            description: "Choose wisely".into(),
            options: vec![ CrisisOption { label: "Take funding".into(), description: "Get ¥500".into(), cost: None },
             CrisisOption { label: "Take personnel".into(), description: "Get 5 staff".into(), cost: None },
            ],
            tick_created: 0,
        });

        // Confirm WITHOUT [X] — should clear the existing preference
        let state = apply_action(&state, &Action::Confirm);
        assert!(!state.auto_resolve_crises.contains_key("personnel"),
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
    fn crisis_dismiss_preserves_policy_panel() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        // Set up state with policy panel open in ManagePolicies.
        let mut state = GameState::new_default(42);
        state.ui.open_panel = Panel::Policy;
        state.ui.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: 0 });
        state.ui.panel_selection = 2;

        // Fire a crisis while in this state.
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
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

        // Policy panel and selection should be preserved — player returns
        // to exactly where they were before the crisis.
        assert_eq!(state.ui.open_panel, Panel::Policy,
            "policy panel should stay open after crisis dismissal");
        assert_eq!(state.ui.policy_ui, Some(PolicyUiState::ManagePolicies { region_idx: 0 }),
            "policy_ui should be preserved after crisis dismissal");
        assert_eq!(state.ui.panel_selection, 2,
            "panel_selection should be preserved after crisis dismissal");
    }

    #[test]
    fn crisis_dismiss_preserves_ledger_wizard() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        // Set up state with ledger panel open in ConfirmBuy.
        let mut state = GameState::new_default(42);
        state.ui.open_panel = Panel::Ledger;
        state.ui.ledger_ui = Some(LedgerUiState::ConfirmBuy { corp_idx: 0 });
        state.ui.panel_selection = 0;

        // Fire a crisis while in this state.
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
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

        // Ledger should stay in ConfirmBuy — player returns to where they were.
        assert_eq!(state.ui.open_panel, Panel::Ledger,
            "ledger panel should stay open after crisis dismissal");
        assert_eq!(state.ui.ledger_ui, Some(LedgerUiState::ConfirmBuy { corp_idx: 0 }),
            "ledger_ui should preserve wizard state after crisis dismissal");
    }

    #[test]
    fn ledger_bailout_ui_flow() {
        let mut state = GameState::new_default(42);
        crate::engine::initialize_game(&mut state);
        // Drain corp 0's reserves so the bailout is meaningful
        state.corporations[0].reserves = state.corporations[0].max_reserves * 0.1;
        let cost = state.corporations[0].bailout_cost();
        state.resources.funding = cost + 500.0;

        // Open ledger, confirm on first corp → ConfirmBuy
        let state = apply_action(&state, &Action::OpenLedger);
        assert_eq!(state.ui.open_panel, Panel::Ledger);
        let state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.ledger_ui, Some(LedgerUiState::ConfirmBuy { corp_idx: 0 })));

        // X cycles: Buy → Bailout (skips Sell since we hold 0 shares)
        let state = apply_action(&state, &Action::ToggleExtra);
        assert!(matches!(state.ui.ledger_ui, Some(LedgerUiState::ConfirmBailout { corp_idx: 0 })));

        // Confirm the bailout
        let funding_before = state.resources.funding;
        let state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.ui.ledger_ui, Some(LedgerUiState::BrowseStocks),
            "should return to BrowseStocks after bailout");
        assert!(
            (state.resources.funding - (funding_before - cost)).abs() < 0.01,
            "funding should be deducted: expected {}, got {}",
            funding_before - cost, state.resources.funding
        );
        assert!(
            (state.corporations[0].reserves - state.corporations[0].max_reserves).abs() < 0.01,
            "reserves should be restored to max"
        );
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

    #[test]
    fn research_selection_stable_when_new_items_appear() {
        use crate::state::{ResearchProject, ResearchUiState, Panel};

        let mut state = GameState::new_default(42);
        // Open the research panel in BrowseAll mode
        state.ui.open_panel = Panel::Research;
        state.ui.research_ui = Some(ResearchUiState::BrowseAll);
        // Pause so tick doesn't advance simulation (we want controlled changes)
        state.sim_state = SimState::Paused;

        let items_before = state.research_flat_items();
        assert!(items_before.len() >= 2, "need at least 2 items to test selection stability");

        // Select the second item and record its identity
        state.ui.panel_selection = 1;
        let selected_kind = items_before[1].to_kind(&state);

        // Add a new active research project, which will push all Available items
        // down by one position in the flat list
        let first_available = items_before.iter()
            .find_map(|item| item.available_kind(&state))
            .expect("should have at least one available project");
        let (personnel, ticks, _funding) = state.effective_costs(&first_available);
        state.active_research.push(ResearchProject {
            kind: first_available,
            progress: 0.0,
            required_ticks: ticks,
            personnel_assigned: personnel,
        });

        // The flat list has changed — the old index 1 now points to a different item
        let items_shifted = state.research_flat_items();
        let _naive_kind = items_shifted.get(1).and_then(|item| item.to_kind(&state));
        // The shift should have changed what index 1 points to (unless item 1 was the
        // newly-added active project, which is unlikely but possible)
        // Either way, tick_and_process should stabilize the selection

        let new_state = tick_and_process(&state);

        if let Some(ref kind) = selected_kind {
            // The selection should now point to the same ResearchKind
            let new_items = new_state.research_flat_items();
            let new_selected = new_items.get(new_state.ui.panel_selection)
                .and_then(|item| item.to_kind(&new_state));
            assert_eq!(new_selected.as_ref(), Some(kind),
                "research selection should track the same item after list changes");
        }
        // If selected_kind was None (UpgradeLab), the UpgradeLab item should still be selected
        if matches!(items_before[1], ResearchFlatItem::UpgradeLab) {
            let new_items = new_state.research_flat_items();
            assert!(matches!(new_items.get(new_state.ui.panel_selection), Some(ResearchFlatItem::UpgradeLab)),
                "UpgradeLab selection should be preserved");
        }
    }
}
