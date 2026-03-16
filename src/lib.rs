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
    DecreeId, DECREE_COUNT, GameCommand, GameOutcome, AppState,
    LedgerUiState, MANAGE_NEGOTIATE_POS, MANAGE_BARGAIN_POS,
    MedicineUiState, OpsUiState, Panel, PolicyId, PolicyUiState, POLICY_COUNT,
    ResearchFlatItem, ResearchKind, LabTab, LabUiState, SimState, ScreeningFormItem,
    ScreeningModality, ScreeningRunSize,
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
pub fn apply_action(state: &AppState, action: &Action) -> AppState {
    let mut new = state.clone();
    new.session.status_message = None;

    // When a crisis is active, only allow selecting options and confirming
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
                        new.session.status_message = Some("Not enough resources".into());
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
                events::process_events(&mut new, &result.events);
                new.session.status_message = result.message;
                new.ui.crisis_selection = 0;
                new.ui.crisis_auto_resolve = false;
                // Restore the player to whatever panel/wizard state they were in
                // before the crisis fired. Clamp panel_selection in case the crisis
                // resolution changed the number of items in the active list.
                let max = ui::panel_selection_max(&new.ui, &new);
                if new.ui.panel_selection > max {
                    new.ui.panel_selection = max;
                }
                // Pacing (Running/Paused) is unchanged by crises — blocking is
                // derived from active_crisis, which resolve_crisis() clears.
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
                        new.session.speed_multiplier = 1;
                    }
                    SimState::Paused => new.sim_state = SimState::Running,
                }
            }
        }
        Action::SpeedUp => {
            if new.is_effectively_running() {
                new.session.speed_multiplier = match new.session.speed_multiplier {
                    1 => 2,
                    2 => 4,
                    4 => 6,
                    _ => 1,
                };
            }
        }
        Action::OpenThreats => new.ui.toggle_panel(Panel::Threats),
        Action::OpenResearch => new.ui.toggle_panel(Panel::Research),
        Action::OpenLab => new.ui.toggle_panel(Panel::Lab),
        Action::OpenMedicines => new.ui.toggle_panel(Panel::Medicines),
        Action::OpenPolicy => new.ui.toggle_panel(Panel::Policy),
        Action::OpenOperations => new.ui.toggle_panel(Panel::Operations),
        Action::OpenBoard => new.ui.toggle_panel(Panel::Board),
        Action::OpenStocks => new.ui.toggle_panel(Panel::Ledger),
        Action::OpenHelp => new.ui.toggle_panel(Panel::Help),
        Action::ClosePanel => new.ui.close_panel(),
        Action::GoHome => new.ui.go_home(),
        Action::SelectNext => {
            if new.ui.open_panel == Panel::Research {
                new.ui.panel_selection = ui::tech_tree::navigate(
                    new.ui.panel_selection, ui::tech_tree::TreeDirection::Down,
                );
            } else {
                let max = ui::panel_selection_max(&new.ui, &new);
                new.ui.select_next(new.regions.len(), max);
            }
            sync_screening_form_selection(&mut new);
        }
        Action::SelectPrev => {
            if new.ui.open_panel == Panel::Research {
                new.ui.panel_selection = ui::tech_tree::navigate(
                    new.ui.panel_selection, ui::tech_tree::TreeDirection::Up,
                );
            } else {
                let max = ui::panel_selection_max(&new.ui, &new);
                new.ui.select_prev(new.regions.len(), max);
            }
            sync_screening_form_selection(&mut new);
        }
        Action::SelectLeft => {
            if new.ui.open_panel == Panel::Research {
                new.ui.panel_selection = ui::tech_tree::navigate(
                    new.ui.panel_selection, ui::tech_tree::TreeDirection::Left,
                );
            } else if new.ui.open_panel == Panel::Lab {
                if let Some(lab_ui) = &new.ui.lab_ui {
                    if lab_ui.is_browsing() {
                        let tab = lab_ui.tab().prev();
                        new.ui.lab_ui = Some(LabUiState::Browse { tab });
                        new.ui.panel_selection = 0;
                    }
                }
            } else {
                new.ui.select_left(new.regions.len());
            }
        }
        Action::SelectRight => {
            if new.ui.open_panel == Panel::Research {
                new.ui.panel_selection = ui::tech_tree::navigate(
                    new.ui.panel_selection, ui::tech_tree::TreeDirection::Right,
                );
            } else if new.ui.open_panel == Panel::Lab {
                if let Some(lab_ui) = &new.ui.lab_ui {
                    if lab_ui.is_browsing() {
                        let tab = lab_ui.tab().next();
                        new.ui.lab_ui = Some(LabUiState::Browse { tab });
                        new.ui.panel_selection = 0;
                    }
                }
            } else {
                new.ui.select_right(new.regions.len());
            }
        }
        Action::JumpToItem { index } => {
            // Jump directly to item N in the current panel list (only when a panel is open).
            if new.ui.open_panel != Panel::None {
                let max = ui::panel_selection_max(&new.ui, &new);
                new.ui.panel_selection = (*index).min(max);
            }
            sync_screening_form_selection(&mut new);
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
                    if new.session.status_message.is_none() {
                        new.session.status_message = result.message;
                    }
                }
            }
            // Toggle "Assign 2x personnel" on research confirm screen (pure UI state)
            else if new.ui.open_panel == Panel::Lab {
                if let Some(LabUiState::ConfirmProject { double_personnel, .. }) = &mut new.ui.lab_ui {
                    *double_personnel = !*double_personnel;
                } else if let Some(lab_ui) = &new.ui.lab_ui {
                    if lab_ui.is_browsing() {
                        if lab_ui.tab() == LabTab::Reactors {
                            // Reactors tab: X toggles "Repeat when low" for selected reactor
                            let sel = new.ui.panel_selection;
                            if sel < new.reactors.len() && new.reactors[sel].medicine_idx.is_some() {
                                execute_command(&mut new, &GameCommand::ToggleReactorRepeat { reactor_idx: sel });
                            }
                        } else {
                            // Toggle auto-repeat for the selected repeatable project
                            let items = new.lab_tab_items(lab_ui.tab());
                            if let Some(item) = items.get(new.ui.panel_selection) {
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
            // Open region filter when browsing medicines
            else if new.ui.open_panel == Panel::Medicines
                && matches!(new.ui.medicine_ui, None | Some(MedicineUiState::BrowseMedicines))
            {
                let unlocked = new.unlocked_medicine_indices();
                if let Some(&med_idx) = unlocked.get(new.ui.panel_selection) {
                    new.ui.medicine_ui = Some(MedicineUiState::RegionFilter { medicine_idx: med_idx });
                    new.ui.panel_selection = 0;
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
                    events::process_events(&mut new, &result.events);
                    // GUARDRAIL: This match is where command results are
                    // translated into follow-up UI/session updates.
                    //
                    // Do not copy this pattern into engine/, UI modules, or a
                    // new helper layer. If a command needs UI/session
                    // follow-up, add that case here. If this block starts
                    // feeling wrong for a new feature, stop and rethink the
                    // boundary before adding another shortcut.
                    match &cmd {
                        GameCommand::StartResearch { .. } if result.success => {
                            if new.ui.open_panel == Panel::Lab {
                                let tab = new.ui.lab_ui.as_ref().map(|s| s.tab()).unwrap_or(LabTab::Sequencing);
                                new.ui.lab_ui = Some(LabUiState::Browse { tab });
                                new.ui.panel_selection = 0;
                            }
                            // Research panel: keep selection in place so player
                            // can see the tech now shows "Researching" state.
                        }
                        GameCommand::StartScreening { .. } if result.success => {
                            new.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Screening });
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::StartTrial { .. } if result.success => {
                            new.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Trials });
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::DiscardHit { .. } if result.success => {
                            // Stay in trial wizard if there are more hits
                            if new.screening_hits.is_empty() {
                                new.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Trials });
                            } else {
                                new.ui.lab_ui = Some(LabUiState::TrialSelectHit);
                            }
                            new.ui.panel_selection = 0;
                        }
                        GameCommand::UpgradeLab if result.success => {
                            let tab = new.ui.lab_ui.as_ref().map(|s| s.tab()).unwrap_or(LabTab::Sequencing);
                            new.ui.lab_ui = Some(LabUiState::Browse { tab });
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
                        GameCommand::ToggleDeploy { med_idx } => {
                            // GUARDRAIL: DeployBlocked uses a session-scoped
                            // dedupe cache. Toggling deploy is the one place
                            // we intentionally clear it so a still-blocked
                            // medicine can notify again.
                            //
                            // Do not start resetting unrelated event caches
                            // from this command-routing block. If another
                            // event seems to need the same treatment, rethink
                            // that event's ownership first.
                            new.session.deploy_blocked_notified.remove(med_idx);
                        }
                        _ => {}
                    }
                    if new.session.status_message.is_none() {
                        new.session.status_message = result.message;
                    }
                }
            }
        }
        Action::Discard => {
            // Reactors tab: D toggles "Deploy medicine when finished" for selected reactor
            if new.ui.open_panel == Panel::Lab {
                if let Some(lab_ui) = &new.ui.lab_ui {
                    if lab_ui.is_browsing() && lab_ui.tab() == LabTab::Reactors {
                        let sel = new.ui.panel_selection;
                        if sel < new.reactors.len() && new.reactors[sel].medicine_idx.is_some() {
                            execute_command(&mut new, &GameCommand::ToggleReactorAutoDeploy { reactor_idx: sel });
                        }
                    }
                }
            }
            // Discard screening hit in the trial wizard
            if new.ui.open_panel == Panel::Lab {
                if let Some(LabUiState::TrialSelectHit) = &new.ui.lab_ui {
                    let hit_index = new.ui.panel_selection;
                    let cmd = GameCommand::DiscardHit { hit_index };
                    let result = execute_command(&mut new, &cmd);
                    events::process_events(&mut new, &result.events);
                    match &cmd {
                        GameCommand::DiscardHit { .. } if result.success => {
                            if new.screening_hits.is_empty() {
                                new.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Trials });
                            } else {
                                new.ui.lab_ui = Some(LabUiState::TrialSelectHit);
                            }
                            new.ui.panel_selection = new.ui.panel_selection.min(
                                new.screening_hits.len().saturating_sub(1)
                            );
                        }
                        _ => {}
                    }
                    if new.session.status_message.is_none() {
                        new.session.status_message = result.message;
                    }
                }
            }
        }
        Action::Configure => {
            // Reactors tab: C opens the medicine selector to reassign an idle reactor
            if new.ui.open_panel == Panel::Lab {
                if let Some(lab_ui) = &new.ui.lab_ui {
                    if lab_ui.is_browsing() && lab_ui.tab() == LabTab::Reactors {
                        let sel = new.ui.panel_selection;
                        if let Some(reactor) = new.reactors.get(sel) {
                            if reactor.active {
                                new.session.status_message = Some("Reactor is running. Wait for the batch to finish.".into());
                            } else {
                                new.ui.lab_ui = Some(LabUiState::ReactorSelectMedicine { reactor_idx: sel });
                                new.ui.panel_selection = 0;
                            }
                        }
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
pub fn tick_and_process(state: &AppState) -> AppState {
    // Capture the identity of the currently-selected panel item before the tick
    // so we can stabilize panel_selection afterward.  Index-based selections
    // jump when the underlying list grows, shrinks, or is re-sorted.

    // Research panel: capture the ResearchFlatItem identity.
    let selected_research_item = if state.ui.open_panel == Panel::Lab
        && matches!(state.ui.lab_ui, Some(LabUiState::Browse { .. }))
    {
        let tab = state.ui.lab_ui.as_ref().unwrap().tab();
        let items = state.lab_tab_items(tab);
        items.get(state.ui.panel_selection).cloned()
    } else {
        None
    };

    // Threats panel: capture the disease_idx at the current display position.
    let selected_disease_idx = if state.ui.open_panel == Panel::Threats {
        let order = state.threats_display_order();
        order.get(state.ui.panel_selection).copied()
    } else {
        None
    };

    // Medicines panel (BrowseMedicines): capture the medicine_idx.
    let selected_medicine_idx = if state.ui.open_panel == Panel::Medicines
        && matches!(state.ui.medicine_ui, Some(MedicineUiState::BrowseMedicines))
    {
        let indices = state.unlocked_medicine_indices();
        indices.get(state.ui.panel_selection).copied()
    } else {
        None
    };

    let (new_world, tick_events) = engine::tick(state);
    let mut new = AppState {
        world: new_world,
        ui: state.ui.clone(),
        session: state.session.clone(),
    };
    events::process_events(&mut new, &tick_events);

    // Stabilize research panel selection: find the same item in the new list.
    if let Some(ref old_item) = selected_research_item {
        if new.ui.open_panel == Panel::Lab
            && matches!(new.ui.lab_ui, Some(LabUiState::Browse { .. }))
        {
            let tab = new.ui.lab_ui.as_ref().unwrap().tab();
            let new_items = new.lab_tab_items(tab);
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
                let max = new_items.len().saturating_sub(1);
                if new.ui.panel_selection > max {
                    new.ui.panel_selection = max;
                }
            }
        }
    }

    // Stabilize threats panel selection: find the same disease in the new display order.
    if let Some(old_disease_idx) = selected_disease_idx {
        if new.ui.open_panel == Panel::Threats {
            let new_order = new.threats_display_order();
            if let Some(pos) = new_order.iter().position(|&idx| idx == old_disease_idx) {
                new.ui.panel_selection = pos;
            } else {
                let max = new_order.len().saturating_sub(1);
                if new.ui.panel_selection > max {
                    new.ui.panel_selection = max;
                }
            }
        }
    }

    // Stabilize medicines panel selection: find the same medicine in the new unlocked list.
    if let Some(old_med_idx) = selected_medicine_idx {
        if new.ui.open_panel == Panel::Medicines
            && matches!(new.ui.medicine_ui, Some(MedicineUiState::BrowseMedicines))
        {
            let new_indices = new.unlocked_medicine_indices();
            if let Some(pos) = new_indices.iter().position(|&idx| idx == old_med_idx) {
                new.ui.panel_selection = pos;
            } else {
                let max = new_indices.len().saturating_sub(1);
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
fn handle_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
    match ui.open_panel {
        Panel::Medicines => handle_medicine_confirm(ui, state),
        Panel::Research => handle_research_confirm(ui, state),
        Panel::Lab => handle_lab_confirm(ui, state),
        Panel::Policy => handle_policy_confirm(ui, state),
        Panel::Operations => handle_operations_confirm(ui, state),
        Panel::Ledger => handle_ledger_confirm(ui, state),
        Panel::Threats => handle_threats_confirm(ui, state),
        _ => None,
    }
}

fn handle_threats_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
    let display_order = state.threats_display_order();
    let disease_idx = display_order.get(ui.panel_selection).copied()?;
    // Only allow toggling detected diseases
    if !state.diseases.get(disease_idx).is_some_and(|d| d.detected) {
        return None;
    }
    Some(GameCommand::ToggleThreatVisibility { disease_idx })
}


fn handle_research_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
    use crate::state::BasicTech;
    // The selected index maps to the tech tree layout order
    let layout_techs: Vec<BasicTech> = ui::tech_tree::layout_techs();
    let tech = *layout_techs.get(ui.panel_selection)?;

    // Already unlocked — nothing to do
    if state.unlocked_techs.contains(&tech) {
        return None;
    }

    let already_researching = state.active_research.iter().any(|r| {
        matches!(r.kind, ResearchKind::BasicResearch { tech: t } if t == tech)
    });
    if already_researching {
        return None;
    }

    // If prerequisites met and project is available, start immediately
    if tech.prerequisites_met(&state.world) {
        let all = state.all_available_projects();
        let target_kind = ResearchKind::BasicResearch { tech };
        if let Some(project_idx) = all.iter().position(|k| *k == target_kind) {
            // Check if we can actually afford it — if not, queue instead
            let (personnel, _, funding) = state.effective_costs(&all[project_idx]);
            if state.resources.funding >= funding && state.personnel_available() >= personnel {
                return Some(GameCommand::StartResearch { project_idx, double_personnel: false });
            }
        }
    }

    // Can't start yet (locked, insufficient funding/personnel) — toggle queue
    Some(GameCommand::ToggleQueueTech { tech })
}

fn handle_medicine_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
    match ui.medicine_ui.clone() {
        Some(MedicineUiState::BrowseMedicines) => {
            // Enter toggles deployment on/off
            let unlocked = state.unlocked_medicine_indices();
            if let Some(&med_idx) = unlocked.get(ui.panel_selection) {
                return Some(GameCommand::ToggleDeploy { med_idx });
            }
            None
        }
        Some(MedicineUiState::RegionFilter { medicine_idx }) => {
            // Enter toggles individual regions on/off
            let order = grid_reading_order(state.regions.len());
            if let Some(&region_idx) = order.get(ui.panel_selection) {
                return Some(GameCommand::ToggleDeployRegion {
                    med_idx: medicine_idx,
                    region_idx,
                });
            }
            None
        }
        None => None,
    }
}

fn handle_lab_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
    match ui.lab_ui.clone() {
        Some(LabUiState::Browse { tab }) => {
            // Reactors tab: handled separately since it doesn't use ResearchFlatItem
            if tab == LabTab::Reactors {
                return handle_reactor_confirm(ui, state);
            }
            let items = state.lab_tab_items(tab);
            let Some(item) = items.get(ui.panel_selection) else {
                return None;
            };
            match item {
                ResearchFlatItem::UpgradeLab => {
                    ui.lab_ui = Some(LabUiState::ConfirmLabUpgrade { tab });
                    ui.panel_selection = 0;
                    return None;
                }
                // Active projects: Enter is a no-op (info already visible)
                ResearchFlatItem::Active(_) => {}
                // Available projects: go to confirm screen
                ResearchFlatItem::Available(project_idx) => {
                    ui.lab_ui = Some(LabUiState::ConfirmProject {
                        tab,
                        project_idx: *project_idx,
                        double_personnel: false,
                    });
                    ui.panel_selection = 0;
                }
                // Full stockpile: Enter is a no-op
                ResearchFlatItem::FullStockpile(_) => {}
                // Active screening runs: Enter is a no-op
                ResearchFlatItem::ActiveScreening(_) => {}
                // Start New Screening Run → open config form
                ResearchFlatItem::StartNewScreening => {
                    ui.lab_ui = Some(LabUiState::ScreeningConfigForm {
                        disease_sel: 0,
                        modality_sel: 0,
                        run_size_sel: 0,
                    });
                    ui.panel_selection = 0;
                    return None;
                }
                // Start New Trial → open trial wizard
                ResearchFlatItem::StartNewTrial => {
                    ui.lab_ui = Some(LabUiState::TrialSelectHit);
                    ui.panel_selection = 0;
                    return None;
                }
            }
            None
        }
        Some(LabUiState::ConfirmProject { project_idx, double_personnel, .. }) => {
            Some(GameCommand::StartResearch { project_idx, double_personnel })
        }
        Some(LabUiState::ConfirmLabUpgrade { .. }) => {
            Some(GameCommand::UpgradeLab)
        }
        // Screening config form: Enter on Confirm starts the run
        Some(LabUiState::ScreeningConfigForm { disease_sel, modality_sel, run_size_sel }) => {
            let items = state.screening_form_items();
            match items.get(ui.panel_selection) {
                Some(ScreeningFormItem::Confirm) => {
                    let eligible = state.screening_eligible_diseases();
                    let unlocked_mods: Vec<_> = ScreeningModality::ALL.iter()
                        .filter(|m| m.is_unlocked(&state.unlocked_techs))
                        .copied()
                        .collect();
                    let unlocked_sizes: Vec<_> = ScreeningRunSize::ALL.iter()
                        .filter(|s| s.is_unlocked())
                        .copied()
                        .collect();
                    if let (Some(&disease_idx), Some(&modality), Some(&run_size)) = (
                        eligible.get(disease_sel),
                        unlocked_mods.get(modality_sel),
                        unlocked_sizes.get(run_size_sel),
                    ) {
                        return Some(GameCommand::StartScreening {
                            disease_idx,
                            modality,
                            run_size,
                        });
                    }
                }
                // Enter on non-confirm items is a no-op (up/down to navigate)
                _ => {}
            }
            None
        }
        // Reactor medicine selection: assign the selected medicine or clear
        Some(LabUiState::ReactorSelectMedicine { reactor_idx }) => {
            let eligible = state.reactor_eligible_medicines();
            if let Some(&med_idx) = eligible.get(ui.panel_selection) {
                // Selected a medicine
                ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Reactors });
                ui.panel_selection = reactor_idx;
                return Some(GameCommand::ConfigureReactor {
                    reactor_idx,
                    medicine_idx: Some(med_idx),
                });
            } else if ui.panel_selection == eligible.len() {
                // "Clear assignment" option
                let has_medicine = state.reactors.get(reactor_idx)
                    .map(|r| r.medicine_idx.is_some())
                    .unwrap_or(false);
                if has_medicine {
                    ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Reactors });
                    ui.panel_selection = reactor_idx;
                    return Some(GameCommand::ConfigureReactor {
                        reactor_idx,
                        medicine_idx: None,
                    });
                }
            }
            None
        }
        // Trial wizard: select hit → advance to rigor selection
        Some(LabUiState::TrialSelectHit) => {
            if ui.panel_selection < state.screening_hits.len() {
                ui.lab_ui = Some(LabUiState::TrialSelectRigor {
                    hit_index: ui.panel_selection,
                });
                ui.panel_selection = 0;
            }
            None
        }
        // Trial wizard: select rigor → start trial
        Some(LabUiState::TrialSelectRigor { hit_index }) => {
            if let Some(&rigor) = crate::state::TrialRigor::ALL.get(ui.panel_selection) {
                return Some(GameCommand::StartTrial {
                    hit_index,
                    rigor,
                });
            }
            None
        }
        None => None,
    }
}

/// Handle Enter key in the Reactors tab.
fn handle_reactor_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
    let sel = ui.panel_selection;
    let reactor_count = state.reactors.len();

    // Buy reactor button
    if sel == reactor_count {
        return Some(GameCommand::BuyReactor);
    }

    let reactor = match state.reactors.get(sel) {
        Some(r) => r,
        None => return None,
    };

    if reactor.active {
        // Reactor is running — Enter is a no-op
        return None;
    }

    if reactor.medicine_idx.is_none() {
        // Empty reactor — open medicine selection wizard
        ui.lab_ui = Some(LabUiState::ReactorSelectMedicine { reactor_idx: sel });
        ui.panel_selection = 0;
        return None;
    }

    // Configured idle reactor — check if stockpile is full
    if let Some(med) = state.medicines.get(reactor.medicine_idx.unwrap()) {
        if med.doses >= med.max_doses {
            // Full stockpile — allow reassignment instead
            ui.lab_ui = Some(LabUiState::ReactorSelectMedicine { reactor_idx: sel });
            ui.panel_selection = 0;
            return None;
        }
    }

    // Start a batch
    Some(GameCommand::StartReactorBatch { reactor_idx: sel })
}

fn handle_policy_confirm(ui: &mut UiState, _state: &AppState) -> Option<GameCommand> {
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

fn handle_operations_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
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

fn handle_ledger_confirm(ui: &mut UiState, state: &AppState) -> Option<GameCommand> {
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

/// When panel_selection moves while in ScreeningConfigForm, update the
/// per-section selection so each section remembers the player's choice.
fn sync_screening_form_selection(state: &mut AppState) {
    // First, compute the new selections without borrowing lab_ui mutably
    let update = if let Some(LabUiState::ScreeningConfigForm { .. }) = &state.ui.lab_ui {
        let items = state.screening_form_items();
        items.get(state.ui.panel_selection).and_then(|item| {
            match item {
                ScreeningFormItem::Disease(d_idx) => {
                    let eligible = state.screening_eligible_diseases();
                    eligible.iter().position(|&d| d == *d_idx)
                        .map(|pos| (Some(pos), None, None))
                }
                ScreeningFormItem::Modality(m) => {
                    let unlocked: Vec<_> = ScreeningModality::ALL.iter()
                        .filter(|mod_| mod_.is_unlocked(&state.unlocked_techs))
                        .copied()
                        .collect();
                    unlocked.iter().position(|u| u == m)
                        .map(|pos| (None, Some(pos), None))
                }
                ScreeningFormItem::RunSize(s) => {
                    let unlocked: Vec<_> = ScreeningRunSize::ALL.iter()
                        .filter(|sz| sz.is_unlocked())
                        .copied()
                        .collect();
                    unlocked.iter().position(|u| u == s)
                        .map(|pos| (None, None, Some(pos)))
                }
                ScreeningFormItem::Confirm => None,
            }
        })
    } else {
        None
    };

    // Now apply the update
    if let (Some(upd), Some(LabUiState::ScreeningConfigForm {
        disease_sel, modality_sel, run_size_sel
    })) = (update, &mut state.ui.lab_ui) {
        if let Some(d) = upd.0 { *disease_sel = d; }
        if let Some(m) = upd.1 { *modality_sel = m; }
        if let Some(s) = upd.2 { *run_size_sel = s; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_cycles_through_multipliers() {
        let state = AppState::new_default(42);
        assert_eq!(state.session.speed_multiplier, 1);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.session.speed_multiplier, 2);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.session.speed_multiplier, 4);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.session.speed_multiplier, 6);

        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.session.speed_multiplier, 1);
    }

    #[test]
    fn pause_resets_speed() {
        let state = AppState::new_default(42);
        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.session.speed_multiplier, 2);

        // Pause should reset to 1x
        let state = apply_action(&state, &Action::TogglePause);
        assert_eq!(state.session.speed_multiplier, 1);
        assert!(!state.sim_state.is_running());
    }

    #[test]
    fn speed_up_ignored_when_paused() {
        let state = AppState::new_default(42);
        let state = apply_action(&state, &Action::TogglePause); // pause
        let state = apply_action(&state, &Action::SpeedUp);
        assert_eq!(state.session.speed_multiplier, 1); // unchanged
    }


    #[test]
    fn auto_resolve_toggle_during_crisis() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = AppState::new_default(42);
        // Crisis active — game is blocked via is_blocked()
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

        let mut state = AppState::new_default(42);
        // Crisis active — game is blocked via is_blocked()
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

        let mut state = AppState::new_default(42);
        // Crisis active — game is blocked via is_blocked()
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

        let mut state = AppState::new_default(42);
        // Pre-existing preference for aid crises
        state.auto_resolve_crises.insert("personnel".to_string(), 0);

        // Crisis active — game is blocked via is_blocked()
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
        let state = AppState::new_default(42);

        // No panel open — JumpToItem should be ignored
        let state = apply_action(&state, &Action::JumpToItem { index: 2 });
        assert_eq!(state.ui.panel_selection, 0, "JumpToItem ignored when no panel open");

        // Open research panel (flat list: items depend on game state)
        let state = apply_action(&state, &Action::OpenLab);
        assert_eq!(state.ui.panel_selection, 0);
        let tab = state.ui.lab_ui.as_ref().unwrap().tab();
        let max = state.lab_tab_items(tab).len().saturating_sub(1);

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
    fn research_panel_remembers_selection() {
        let state = AppState::new_default(42);

        // Open research, navigate down
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.panel_selection, 0);
        let state = apply_action(&state, &Action::SelectNext);
        let sel = state.ui.panel_selection;
        assert!(sel > 0, "should have moved selection down");

        // Close research panel
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::None);

        // Reopen — should restore previous selection
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.open_panel, Panel::Research);
        assert_eq!(state.ui.panel_selection, sel,
            "research panel should remember previous selection");

        // Switch directly to another panel and back
        let state = apply_action(&state, &Action::OpenThreats);
        assert_eq!(state.ui.open_panel, Panel::Threats);
        let state = apply_action(&state, &Action::OpenResearch);
        assert_eq!(state.ui.panel_selection, sel,
            "research panel should remember selection after switching panels");
    }

    #[test]
    fn panel_hotkey_resets_to_top_when_deep_in_wizard() {
        let state = AppState::new_default(42);
        // Open research → confirm first item → now at ConfirmProject
        let state = apply_action(&state, &Action::OpenLab);
        assert_eq!(state.ui.open_panel, Panel::Lab);
        assert!(matches!(state.ui.lab_ui, Some(LabUiState::Browse { .. })));
        let state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.lab_ui, Some(LabUiState::ConfirmProject { .. })));

        // Press R again — should reset to BrowseAll, NOT close the panel
        let state = apply_action(&state, &Action::OpenLab);
        assert_eq!(state.ui.open_panel, Panel::Lab);
        assert!(matches!(state.ui.lab_ui, Some(LabUiState::Browse { .. })));

        // Press R again at top level — now it closes
        let state = apply_action(&state, &Action::OpenLab);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn crisis_dismiss_preserves_policy_panel() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        // Set up state with policy panel open in ManagePolicies.
        let mut state = AppState::new_default(42);
        state.ui.open_panel = Panel::Policy;
        state.ui.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: 0 });
        state.ui.panel_selection = 2;

        // Fire a crisis while in this state.
        // Crisis active — game is blocked via is_blocked()
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
        let mut state = AppState::new_default(42);
        state.ui.open_panel = Panel::Ledger;
        state.ui.ledger_ui = Some(LedgerUiState::ConfirmBuy { corp_idx: 0 });
        state.ui.panel_selection = 0;

        // Fire a crisis while in this state.
        // Crisis active — game is blocked via is_blocked()
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
        let mut state = AppState::new_default(42);
        crate::engine::initialize_game(&mut state);
        // Drain corp 0's reserves so the bailout is meaningful
        state.corporations[0].reserves = state.corporations[0].max_reserves * 0.1;
        let cost = state.corporations[0].bailout_cost();
        state.resources.funding = cost + 500.0;

        // Open ledger, confirm on first corp → ConfirmBuy
        let state = apply_action(&state, &Action::OpenStocks);
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
    fn enter_on_browse_medicines_toggles_deploy_enabled() {
        use crate::state::MedicineUiState;

        let mut state = AppState::new_default(42);
        // Unlock a medicine so the panel has content
        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 100.0;

        // Open medicines panel — should be in BrowseMedicines
        let state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::BrowseMedicines)));

        // Enter toggles deploy_enabled for the selected medicine
        let was_enabled = state.deploy_enabled.get(0).copied().unwrap_or(false);
        let state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::BrowseMedicines)),
            "Enter should stay in BrowseMedicines");
        assert_ne!(state.deploy_enabled.get(0).copied().unwrap_or(false), was_enabled,
            "deploy_enabled should have toggled");
    }

    #[test]
    fn research_selection_stable_when_new_items_appear() {
        use crate::state::{ResearchProject, LabUiState, LabTab, Panel};

        let mut state = AppState::new_default(42);
        // Open the research panel in BrowseAll mode
        state.ui.open_panel = Panel::Lab;
        state.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Sequencing });
        // Pause so tick doesn't advance simulation (we want controlled changes)
        state.sim_state = SimState::Paused;

        let items_before = state.lab_tab_items(LabTab::Sequencing);
        assert!(items_before.len() >= 2, "need at least 2 items to test selection stability");

        // Select the second item and record its identity
        state.ui.panel_selection = 1;
        let selected_kind = items_before[1].to_kind(&state);

        // Add a new active research project, which will push all Available items
        // down by one position in the tab's list
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

        // The item list has changed — the old index 1 now points to a different item
        let items_shifted = state.lab_tab_items(LabTab::Sequencing);
        let _naive_kind = items_shifted.get(1).and_then(|item| item.to_kind(&state));
        // The shift should have changed what index 1 points to (unless item 1 was the
        // newly-added active project, which is unlikely but possible)
        // Either way, tick_and_process should stabilize the selection

        let new_state = tick_and_process(&state);

        if let Some(ref kind) = selected_kind {
            // The selection should now point to the same ResearchKind
            let new_items = new_state.lab_tab_items(LabTab::Sequencing);
            let new_selected = new_items.get(new_state.ui.panel_selection)
                .and_then(|item| item.to_kind(&new_state));
            assert_eq!(new_selected.as_ref(), Some(kind),
                "research selection should track the same item after list changes");
        }
        // If selected_kind was None (UpgradeLab), the UpgradeLab item should still be selected
        if matches!(items_before[1], ResearchFlatItem::UpgradeLab) {
            let new_items = new_state.lab_tab_items(LabTab::Sequencing);
            assert!(matches!(new_items.get(new_state.ui.panel_selection), Some(ResearchFlatItem::UpgradeLab)),
                "UpgradeLab selection should be preserved");
        }
    }

    #[test]
    fn threats_selection_stable_when_display_order_changes() {
        use crate::state::Panel;

        let mut state = AppState::new_default(42);
        state.ui.open_panel = Panel::Threats;
        state.sim_state = SimState::Paused;

        // Add a second disease so we have two to reorder between
        let mut disease2 = state.diseases[0].clone();
        disease2.name = "Test Disease Two".into();
        disease2.detected = true;
        state.diseases.push(disease2);
        let new_idx = state.diseases.len() - 1;
        // Give it massive deaths so it sorts first in the display order
        state.regions[0].get_or_create_infection(new_idx).dead = 999_999_999.0;

        // Now the display order is [new_idx, 0] (high deaths first)
        let order = state.threats_display_order();
        assert_eq!(order[0], new_idx, "high-death disease should sort first");
        assert_eq!(order[1], 0, "original disease should be second");

        // Select the second item (disease 0 — the original, lower-death disease)
        state.ui.panel_selection = 1;
        let selected_disease = order[1]; // disease index 0

        // tick_and_process should stabilize: panel_selection should still
        // point to disease 0 regardless of any order changes
        let new_state = tick_and_process(&state);
        let new_order = new_state.threats_display_order();
        let new_selected_disease = new_order.get(new_state.ui.panel_selection).copied();
        assert_eq!(new_selected_disease, Some(selected_disease),
            "threats selection should track the same disease after tick");
    }

    #[test]
    fn medicines_selection_stable_when_new_medicine_unlocked() {
        use crate::state::{MedicineUiState, Medicine, TherapyType, MechanismOfAction, Panel};

        let mut state = AppState::new_default(42);
        state.ui.open_panel = Panel::Medicines;
        state.ui.medicine_ui = Some(MedicineUiState::BrowseMedicines);
        state.sim_state = SimState::Paused;

        // Add a second unlocked medicine (only Broad-Spectrum exists at startup)
        state.medicines.push(Medicine {
            name: "Test Antibiotic".into(),
            therapy_type: TherapyType::Antibiotic,
            mechanism: Some(MechanismOfAction::CellWallInhibitor),
            target_diseases: vec![0],
            doses: 500_000.0,
            max_doses: 500_000.0,
            unlocked: true,
            tested_against: vec![0],
            deployed_count: 0,
            total_treated: 0.0,
            manufacturer_corp_idx: None,
            trial_efficacy: None,
            side_effect_rate: 0.0,
            resistance_rate: 0.0,
            trial_rigor: None,
            reported_efficacy: None,
            reported_side_effects: None,
            reported_resistance: None,
        });
        let last_med = state.medicines.len() - 1;

        // unlocked_medicine_indices() = [0, last_med]
        let indices = state.unlocked_medicine_indices();
        assert_eq!(indices.len(), 2);
        assert_eq!(indices[0], 0);
        assert_eq!(indices[1], last_med);

        // Select the second item (last_med)
        state.ui.panel_selection = 1;
        let selected_med = last_med;

        // tick_and_process should preserve: panel_selection still maps to last_med
        let new_state = tick_and_process(&state);
        let new_indices = new_state.unlocked_medicine_indices();
        let new_selected_med = new_indices.get(new_state.ui.panel_selection).copied();
        assert_eq!(new_selected_med, Some(selected_med),
            "medicines selection should track the same medicine after tick");
    }
}
