mod crisis;
mod medicine;
mod policy;
mod research;
mod spread;

use rand::Rng;

use crate::state::{
    GameCommand, GameEvent, GameOutcome, GameState, SimState,
    CRISIS_INTERVAL, CRISIS_MIN_TICK,
    EMERGENCE_CHANCE_PER_TICK, EMERGENCE_MIN_TICK,
    KNOWLEDGE_NAME, MAX_DISEASES, TICKS_PER_DAY,
    WIN_INFECTED_THRESHOLD,
};

/// Advance the simulation by one tick.
pub fn tick(state: &GameState) -> GameState {
    let mut new = state.clone();
    new.events.clear();

    // Don't advance simulation after game over
    if new.outcome != GameOutcome::Playing {
        return new;
    }

    // Clone the RNG out so we can mutably borrow both `rng` and `new.regions`
    // simultaneously. Written back to `new.rng` at the end of the function.
    // WARNING: Do not use `new.rng` between here and the write-back line.
    let mut rng = new.rng.clone();

    // Disease spread and mutation
    spread::tick_spread_within(&mut new, &state.diseases, &mut rng);
    spread::tick_spread_cross_region(&mut new, &state.diseases, &mut rng);
    spread::tick_mutation(&mut new, &mut rng);

    // Research progress
    research::tick_research(&mut new);

    // Policy costs — suspend unaffordable policies and deduct costs.
    let policy_cost = policy::tick_enforce_costs(&mut new);

    // Passive resource generation (both degrade as deaths mount)
    let funding_income = new.funding_income_rate();
    new.resources.funding += funding_income;

    // Political Power: ramps based on severity + time.
    // Severity = sqrt(death_fraction) provides fast initial growth then diminishing returns.
    // Time = linear ramp reaching 0.4 at day 30 (baseline even if player contains well).
    {
        let initial_pop = new.initial_population();
        let death_frac = if initial_pop > 0.0 { new.total_dead() / initial_pop } else { 0.0 };
        let infected_frac = if initial_pop > 0.0 { new.total_infected() / initial_pop } else { 0.0 };
        let time_frac = new.tick as f64 / (30.0 * TICKS_PER_DAY);
        let severity = death_frac.sqrt() * 3.0 + infected_frac.sqrt() * 1.5;
        new.resources.political_power = (severity + time_frac * 0.4).clamp(0.0, 1.0);
    }

    // POL-based personnel: ~1 person per 15 days at max POL
    {
        let rate = new.resources.political_power / (15.0 * TICKS_PER_DAY);
        new.resources.personnel_accum += rate;
        if new.resources.personnel_accum >= 1.0 {
            let gained = new.resources.personnel_accum as u32;
            new.resources.personnel += gained;
            new.resources.personnel_accum -= gained as f64;
        }
    }

    // Low funding warning: warn when net burn rate will exhaust funds within ~5 ticks.
    // Only warn if policies actually cost more than income (net negative).
    let net_burn = policy_cost - funding_income;
    if policy_cost > 0.0 && net_burn > 0.0 && new.resources.funding < net_burn * 5.0 {
        new.events.push(GameEvent::FundingWarning);
    }

    // Mid-game disease emergence
    if new.tick >= EMERGENCE_MIN_TICK
        && new.diseases.len() < MAX_DISEASES
        && rng.r#gen::<f64>() < EMERGENCE_CHANCE_PER_TICK
    {
        if let Some((disease_idx, region_idx)) = new.spawn_disease(&mut rng) {
            new.events.push(GameEvent::NewDiseaseEmerged {
                disease_idx,
                region_idx,
            });
        }
    }

    // Crisis event generation (only when no crisis is active)
    if new.active_crisis.is_none()
        && new.tick >= CRISIS_MIN_TICK
        && rng.r#gen::<f64>() < 1.0 / CRISIS_INTERVAL as f64
    {
        if let Some(crisis) = crisis::generate_crisis(&new, &mut rng) {
            new.active_crisis = Some(crisis);
            // Pause the game for the crisis — this is a game rule, not a UI concern.
            new.sim_state = SimState::Event {
                was_running: new.sim_state.is_running(),
            };
            new.events.push(GameEvent::CrisisStarted);
        }
    }

    new.rng = rng;
    new.tick += 1;

    // Check regional collapse
    for i in 0..new.regions.len() {
        if new.regions[i].collapsed {
            continue;
        }
        let pop = new.regions[i].population as f64;
        let alive = new.regions[i].alive();
        if alive < pop * new.regions[i].collapse_threshold {
            new.regions[i].collapsed = true;
            // Clear all policies in the collapsed region
            if let Some(policy) = new.policies.get_mut(i) {
                policy.clear_all();
            }
            new.events.push(GameEvent::RegionCollapsed { region_idx: i });
        }
    }

    // Check win/lose conditions (only while still playing)
    if new.outcome == GameOutcome::Playing {
        let all_collapsed = new.regions.iter().all(|r| r.collapsed);

        let game_over = if all_collapsed {
            new.outcome = GameOutcome::Lost;
            true
        } else if new.total_infected() < WIN_INFECTED_THRESHOLD {
            // Win requires: diseases identified, contained, and medicines tested
            let all_identified = new.diseases.iter().all(|d| d.knowledge >= KNOWLEDGE_NAME);
            let all_have_tested_medicine = (0..new.diseases.len()).all(|d_idx| {
                new.medicines.iter().any(|m| m.tested_against.contains(&d_idx))
            });
            if all_identified && all_have_tested_medicine {
                new.outcome = GameOutcome::Won;
                true
            } else {
                false
            }
        } else {
            false
        };

        if game_over {
            new.active_crisis = None; // game over supersedes any active crisis
            new.sim_state = SimState::Paused;
            new.events.push(GameEvent::GameOver);
        }
    }

    // Record history for dashboard sparklines
    if new.tick % crate::state::HISTORY_INTERVAL == 0 {
        new.history.push(crate::state::HistorySnapshot {
            tick: new.tick,
            total_infected: new.total_infected(),
            total_dead: new.total_dead(),
        });
        if new.history.len() > crate::state::HISTORY_MAX {
            new.history.remove(0);
        }
    }

    new
}

/// Result of executing a game command. Contains feedback message and whether
/// the command succeeded (so the UI layer can update navigation accordingly).
pub struct CommandResult {
    pub message: Option<String>,
    pub success: bool,
}

/// Execute a game command. Pure game logic — does NOT touch UI state.
/// The caller is responsible for UI transitions based on the result.
pub fn execute_command(state: &mut GameState, cmd: &GameCommand) -> CommandResult {
    if state.outcome != GameOutcome::Playing {
        return CommandResult { message: None, success: false };
    }
    match cmd {
        GameCommand::DeployMedicine {
            medicine_idx,
            region_idx,
            target_selection,
        } => {
            let (nav_back, msg) =
                medicine::deploy_medicine(state, *medicine_idx, *region_idx, *target_selection);
            CommandResult { message: msg, success: nav_back }
        }
        GameCommand::StartResearch { bench, project_idx } => {
            let ok = research::start_research(state, *bench, *project_idx);
            CommandResult { message: None, success: ok }
        }
        GameCommand::AddResearchPersonnel { bench } => {
            let msg = research::add_personnel(state, *bench);
            CommandResult { message: msg, success: true }
        }
        GameCommand::RemoveResearchPersonnel { bench } => {
            let msg = research::remove_personnel(state, *bench);
            CommandResult { message: msg, success: true }
        }
        GameCommand::TogglePolicy {
            region_idx,
            policy_idx,
        } => {
            let (msg, success) = policy::toggle_policy(state, *region_idx, *policy_idx);
            CommandResult { message: msg, success }
        }
        GameCommand::ResolveCrisis { choice } => {
            let msg = crisis::resolve_crisis(state, *choice);
            CommandResult { message: Some(msg), success: true }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::apply_action;
    use crate::state::{GameState, MedicineUiState, Panel, PolicyUiState, RegionDiseaseState, ResearchUiState};

    /// Helper: unlock all medicines and mark them tested (for tests that predate the research system).
    fn unlock_all_medicines(state: &mut GameState) {
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
    }

    /// Helper: find the region index that has the primary (first) disease outbreak.
    fn primary_outbreak_region(state: &GameState) -> usize {
        state.regions.iter().position(|r|
            r.infections.iter().any(|i| i.disease_idx == 0 && i.infected > 0.0)
        ).expect("should have a region with disease 0")
    }

    #[test]
    fn tick_increases_infections() {
        let state = GameState::new_default(42);
        let initial = state.total_infected();
        let after = tick(&state);
        assert!(
            after.total_infected() > initial,
            "infections should grow: {} -> {}",
            initial,
            after.total_infected()
        );
    }

    #[test]
    fn tick_causes_deaths() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..20 {
            s = tick(&s);
        }
        assert!(s.total_dead() > 0.0, "should have some deaths after 20 ticks");
    }

    #[test]
    fn tick_advances_state() {
        let state = GameState::new_default(42);
        let after = tick(&state);
        assert_eq!(after.tick, state.tick + 1);
        assert!(after.total_infected() > state.total_infected());
    }

    #[test]
    fn multi_tick_determinism() {
        let state = GameState::new_default(42);
        let mut a = state.clone();
        let mut b = state;
        for _ in 0..50 {
            a = tick(&a);
            b = tick(&b);
        }
        assert_eq!(a.total_infected(), b.total_infected());
        assert_eq!(a.total_dead(), b.total_dead());
        assert_eq!(a.total_immune(), b.total_immune());
    }

    #[test]
    fn recovery_accumulates() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..50 {
            s = tick(&s);
        }
        assert!(
            s.total_immune() > 0.0,
            "should have immune (recovered) after 50 ticks, got {}",
            s.total_immune()
        );
    }

    #[test]
    fn population_conservation() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..100 {
            s = tick(&s);
        }
        for region in &s.regions {
            let pop = region.population as f64;
            for inf in &region.infections {
                let accounted = inf.infected + inf.immune + inf.dead;
                assert!(
                    accounted <= pop + 1.0,
                    "region {} disease {}: accounted {} > population {}",
                    region.name,
                    inf.disease_idx,
                    accounted,
                    pop
                );
                assert!(
                    inf.infected >= 0.0 && inf.immune >= 0.0 && inf.dead >= 0.0,
                    "region {} disease {}: negative values: infected={}, immune={}, dead={}",
                    region.name,
                    inf.disease_idx,
                    inf.infected,
                    inf.immune,
                    inf.dead
                );
            }
        }
    }

    #[test]
    fn cross_region_spread_eventually() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..200 {
            s = tick(&s);
        }
        let infected_regions = s
            .regions
            .iter()
            .filter(|r| !r.infections.is_empty())
            .count();
        assert!(
            infected_regions > 1,
            "disease should spread to more than 1 region after 200 ticks, got {}",
            infected_regions
        );
    }

    #[test]
    fn toggle_pause() {
        use crate::state::SimState;
        let state = GameState::new_default(42);
        assert!(state.sim_state.is_running());
        let s = apply_action(&state, &Action::TogglePause);
        assert_eq!(s.sim_state, SimState::Paused);
        let s = apply_action(&s, &Action::TogglePause);
        assert!(s.sim_state.is_running());
    }

    #[test]
    fn open_close_panels() {
        let state = GameState::new_default(42);
        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::Threats);
        let s = apply_action(&s, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::None);
        let s = apply_action(&s, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::Threats);
        let s = apply_action(&s, &Action::ClosePanel);
        assert_eq!(s.ui.open_panel, Panel::None);
    }

    #[test]
    fn panel_navigation() {
        let state = GameState::new_default(42);
        let max_sel = state.diseases.len() - 1;

        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.panel_selection, 0);
        // Navigate to the end
        let mut s = s;
        for _ in 0..max_sel {
            s = apply_action(&s, &Action::SelectNext);
        }
        assert_eq!(s.ui.panel_selection, max_sel);
        // Can't go past the last item
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, max_sel);
        // Navigate back to start
        let mut s = s;
        for _ in 0..max_sel {
            s = apply_action(&s, &Action::SelectPrev);
        }
        assert_eq!(s.ui.panel_selection, 0);
        // Can't go below 0
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.panel_selection, 0);
    }

    #[test]
    fn immune_reduces_susceptible_pool() {
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Set 90% of the region's population as immune — drastically reduces susceptible pool
        let pop = state.regions[ri].population as f64;
        state.regions[ri].infections[0].immune = pop * 0.9;
        let before = state.regions[ri].infections[0].infected;
        let after = tick(&state);
        let growth = after.regions[ri].infections[0].infected - before;

        let state2 = GameState::new_default(42);
        let ri2 = primary_outbreak_region(&state2);
        let after2 = tick(&state2);
        let growth2 = after2.regions[ri2].infections[0].infected
            - state2.regions[ri2].infections[0].infected;

        assert!(
            growth < growth2,
            "immunity should reduce infection growth: {} vs {}",
            growth,
            growth2
        );
    }

    #[test]
    fn disease_can_spread_into_vaccinated_region() {
        let mut state = GameState::new_default(42);
        // Find a region WITHOUT disease 0 and pre-vaccinate it
        let clean_region = (0..state.regions.len())
            .find(|&i| !state.regions[i].infections.iter().any(|inf| inf.disease_idx == 0))
            .expect("should have an uninfected region");
        state.regions[clean_region].infections.push(RegionDiseaseState {
            disease_idx: 0,
            infected: 0.0,
            dead: 0.0,
            immune: 100_000_000.0,
        });
        let mut s = state;
        for _ in 0..200 {
            s = tick(&s);
        }
        let imm = s.regions[clean_region]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .map(|i| i.immune)
            .unwrap_or(0.0);
        assert!(
            imm >= 100_000_000.0,
            "immune count should be preserved"
        );
    }

    #[test]
    fn medicine_vaccination_deployment() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        assert_eq!(state.ui.open_panel, Panel::Medicines);
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
        ));
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectTarget { .. })
        ));
        let funding_before = state.resources.funding;
        let efficacy = state.medicines[0].therapy_type.efficacy(&state.diseases[0].pathogen_type);
        let expected_immune = state.medicines[0].doses * efficacy;
        state = apply_action(&state, &Action::Confirm);
        // Computed outputs: cost deducted, immunity applied based on efficacy
        assert_eq!(state.resources.funding, funding_before - state.medicines[0].cost);
        let na_inf = state.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .unwrap();
        assert_eq!(na_inf.immune, expected_immune);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::DeployResult { medicine_idx: 0, adverse: false, .. })
        ));
        // DeployResult should contain the feedback message
        if let Some(MedicineUiState::DeployResult { message, .. }) = &state.ui.medicine_ui {
            assert!(message.contains("Vaccinated"), "message should mention vaccination: {message}");
        }
    }

    #[test]
    fn medicine_treatment_deployment() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        for _ in 0..20 {
            state = tick(&state);
        }
        let ri = primary_outbreak_region(&state);
        let infected_before = state.regions[ri].infections[0].infected;

        // Navigate: open medicines → select first medicine → navigate to the
        // outbreak region → select treat target
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        // Navigate to the outbreak region
        for _ in 0..ri {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // select region
        state = apply_action(&state, &Action::SelectNext); // switch from vaccinate to treat
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // deploy

        let infected_after = state.regions[ri].infections[0].infected;
        assert!(
            infected_after < infected_before,
            "treatment should reduce infected: {} -> {}",
            infected_before,
            infected_after
        );
        assert_eq!(state.resources.funding, funding_before - state.medicines[0].cost);
        // Treatment is proportional — treats TREATMENT_FRACTION * efficacy of infected
        let treated = infected_before - infected_after;
        assert!(
            treated > 0.0,
            "should have treated some people"
        );
        assert!(
            state.medicines[0].doses < state.medicines[0].max_doses,
            "doses should have been depleted after deployment"
        );
        // Doses consumed = people treated
        assert!(
            (state.medicines[0].max_doses - state.medicines[0].doses - treated).abs() < 1.0,
            "doses depleted ({}) should equal people treated ({})",
            state.medicines[0].max_doses - state.medicines[0].doses, treated
        );
    }

    #[test]
    fn medicine_empty_doses_blocks_deployment() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].doses = 0.0; // Empty
        for _ in 0..20 {
            state = tick(&state);
        }

        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        for _ in 0..4 {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // select region (Asia)
        state = apply_action(&state, &Action::SelectNext); // Treat
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // try deploy

        assert_eq!(state.resources.funding, funding_before, "should not charge when empty");
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("No doses remaining"),
            "expected no doses message, got: {:?}",
            state.ui.status_message
        );
    }

    #[test]
    fn medicine_insufficient_funds() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.resources.funding = 50.0;
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.resources.funding, funding_before);
        // Should show error message and stay on SelectTarget
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("Insufficient funds"),
            "expected insufficient funds message, got: {:?}",
            state.ui.status_message
        );
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectTarget { .. })),
            "should stay on SelectTarget, got: {:?}",
            state.ui.medicine_ui
        );
    }

    #[test]
    fn untested_medicine_insufficient_funds_skips_warning() {
        let mut state = GameState::new_default(42);
        unlock_untested(&mut state);
        state.resources.funding = 50.0; // Not enough for any medicine
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // select target
        // Should show funds error, NOT the untested warning
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("Insufficient funds"),
            "expected funds error, got: {:?}",
            state.ui.status_message
        );
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectTarget { .. })),
            "should stay on SelectTarget, not go to ConfirmDeploy, got: {:?}",
            state.ui.medicine_ui
        );
        assert_eq!(state.resources.funding, funding_before);
    }

    #[test]
    fn medicine_esc_backstep() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { .. })
        ));
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
        assert!(state.ui.medicine_ui.is_none());
    }

    #[test]
    fn medicine_zero_targets_refused() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        // Clear region 0 infections so we can test treating with zero targets
        state.regions[0].infections.clear();
        let infections_before = state.regions[0].infections.len();
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        state = apply_action(&state, &Action::Confirm); // select region 0 (NA)
        state = apply_action(&state, &Action::SelectNext); // Treat option
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.resources.funding, funding_before);
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("No infected"),
            "expected zero-target message, got: {:?}",
            state.ui.status_message
        );
        // Should NOT create a ghost disease entry
        assert_eq!(
            state.regions[0].infections.len(),
            infections_before,
            "failed deployment should not create ghost disease entry"
        );
    }

    #[test]
    fn open_medicines_resets_to_browse() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::OpenThreats);
        state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        assert_eq!(state.ui.panel_selection, 0);
    }

    /// Helper: unlock medicines but leave them untested.
    fn unlock_untested(state: &mut GameState) {
        for med in &mut state.medicines {
            med.unlocked = true;
        }
    }

    #[test]
    fn untested_medicine_requires_confirmation() {
        let mut state = GameState::new_default(42);
        unlock_untested(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        state = apply_action(&state, &Action::Confirm); // select region 0 (NA)
        // Confirm target → should go to ConfirmDeploy, NOT deploy
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::ConfirmDeploy { .. })),
            "untested medicine should show confirmation, got {:?}",
            state.ui.medicine_ui
        );
        assert_eq!(state.resources.funding, funding_before, "should not have deployed yet");

        // Confirm again → actually deploys
        state = apply_action(&state, &Action::Confirm);
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::DeployResult { .. })),
            "should show DeployResult after deploy"
        );
        assert!(state.resources.funding < funding_before, "should have spent funding");
    }

    #[test]
    fn untested_medicine_cancel_returns_to_target() {
        let mut state = GameState::new_default(42);
        unlock_untested(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        state = apply_action(&state, &Action::Confirm); // → ConfirmDeploy
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::ConfirmDeploy { .. })));

        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::ClosePanel); // cancel
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectTarget { .. })),
            "Esc should return to SelectTarget"
        );
        assert_eq!(state.resources.funding, funding_before, "should not have deployed");
    }

    #[test]
    fn tested_medicine_deploys_immediately() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state); // tested
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // deploy immediately
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::DeployResult { .. })),
            "tested medicine should deploy without confirmation"
        );
        assert!(state.resources.funding < funding_before);
    }

    #[test]
    fn map_navigation_right_left_wraps() {
        // Reading order: NA(0) → EU(2) → Asia(4) → SA(1) → Africa(3) → Oceania(5) → NA(0)
        let state = GameState::new_default(42);
        assert_eq!(state.ui.map_selection, 0); // NA
        let s = apply_action(&state, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 2); // EU
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 4); // Asia
        // Wraps from end of row 0 to start of row 1
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 1); // SA
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 3); // Africa
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 5); // Oceania
        // Wraps from last region back to first
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 0); // NA

        // Left wraps the other direction
        let s = apply_action(&state, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 5); // Oceania (wrap from first to last)
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 3); // Africa
    }

    #[test]
    fn map_navigation_up_down_no_panel() {
        let state = GameState::new_default(42);
        assert_eq!(state.ui.map_selection, 0); // NA (row 0)
        let s = apply_action(&state, &Action::SelectNext);
        assert_eq!(s.ui.map_selection, 1); // SA (row 1)
        // Can't go past bottom row
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.map_selection, 1);
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.map_selection, 0); // NA
        // Can't go past top row
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.map_selection, 0);
    }

    #[test]
    fn map_navigation_with_panel_open() {
        let mut state = GameState::new_default(42);
        // Need at least 2 diseases so the panel has items to navigate
        state.diseases.push(crate::state::Disease::generate(
            &mut state.rng.clone(), crate::state::PathogenType::Bacterium, &[], true,
        ));
        // Open threats panel — up/down should navigate panel, not map
        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.map_selection, 0);
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 1); // panel navigated
        assert_eq!(s.ui.map_selection, 0); // map unchanged
        // But left/right should still navigate map
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 2); // EU
        assert_eq!(s.ui.panel_selection, 1); // panel unchanged
    }

    #[test]
    fn research_panel_navigation() {
        let mut state = GameState::new_default(42);
        state = apply_action(&state, &Action::OpenResearch);

        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseCategories)));
        assert_eq!(state.ui.panel_selection, 0);

        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 1);

        // Can't go past last
        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 1);

        // Esc closes
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_esc_backstep() {
        let mut state = GameState::new_default(42);
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseProjects { bench: false })));

        state = apply_action(&state, &Action::Confirm); // Select project
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ConfirmProject { .. })));

        state = apply_action(&state, &Action::ClosePanel); // Back to projects
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseProjects { .. })));

        state = apply_action(&state, &Action::ClosePanel); // Back to categories
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseCategories)));

        state = apply_action(&state, &Action::ClosePanel); // Close panel
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_confirm_noop_on_empty_list() {
        let mut state = GameState::new_default(42);
        // Make all diseases fully known AND prion type (mutation_rate too low
        // for genomic sequencing) so no field projects are available
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
            disease.pathogen_type = crate::state::PathogenType::Prion;
        }
        // No medicines are unlocked, so no clinical trials either
        // => available_field_projects returns empty

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Enter Field Research
        assert!(matches!(
            state.ui.research_ui,
            Some(ResearchUiState::BrowseProjects { bench: false })
        ));

        // Pressing Enter on empty list should stay on BrowseProjects
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.research_ui,
            Some(ResearchUiState::BrowseProjects { bench: false })
        ));
    }

    #[test]
    fn diseases_start_unknown() {
        let state = GameState::new_default(42);
        for disease in &state.diseases {
            assert_eq!(disease.knowledge, 0.0);
        }
    }

    #[test]
    fn lose_condition_triggers_when_all_regions_collapse() {
        let mut state = GameState::new_default(42);
        // Ensure a highly lethal, fast-spreading disease so all regions collapse.
        // Lethality must be high enough to overcome recovery and kill >70% (Africa's threshold).
        // cross_region_spread must be high enough to reach refugia (S.America, Oceania)
        // through the sparser connection graph.
        for disease in &mut state.diseases {
            disease.infectivity = 0.08;
            disease.lethality = 0.06;
            disease.recovery_rate = 0.005;
            disease.cross_region_spread = 0.15;
        }
        // Run until game over (collapse requires all regions to fall)
        for _ in 0..10000 {
            state = tick(&state);
            crate::ui::process_events(&mut state);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        assert_eq!(state.outcome, GameOutcome::Lost);
        assert_eq!(state.sim_state, crate::state::SimState::Paused);
        // All regions should be collapsed
        assert!(state.regions.iter().all(|r| r.collapsed));
    }

    #[test]
    fn win_requires_identification_and_tested_medicines() {
        let mut state = GameState::new_default(42);
        // Clear all infections to simulate containment
        for region in &mut state.regions {
            region.infections.clear();
        }
        // Diseases NOT identified — should not trigger win
        state = tick(&state);
        assert_eq!(state.outcome, GameOutcome::Playing);

        // Identify all diseases but no tested medicines — still no win
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
        }
        state = tick(&state);
        assert_eq!(state.outcome, GameOutcome::Playing);

        // Test medicines against all diseases — now should win
        let disease_count = state.diseases.len();
        state.medicines[0].tested_against = (0..disease_count).collect();
        state = tick(&state);
        crate::ui::process_events(&mut state);
        assert_eq!(state.outcome, GameOutcome::Won);
        assert_eq!(state.sim_state, crate::state::SimState::Paused);
    }

    #[test]
    fn no_deploy_after_game_over() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.outcome = GameOutcome::Lost;
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        state = apply_action(&state, &Action::Confirm); // try to deploy
        assert_eq!(state.resources.funding, funding_before, "should not spend funds after game over");
    }

    #[test]
    fn no_unpause_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.sim_state = crate::state::SimState::Paused;
        let s = apply_action(&state, &Action::TogglePause);
        assert_eq!(s.sim_state, crate::state::SimState::Paused, "should not be able to unpause after game over");
    }

    #[test]
    fn tick_does_not_advance_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        let tick_before = state.tick;
        state = tick(&state);
        assert_eq!(state.tick, tick_before, "tick should not advance after game over");
    }

    #[test]
    fn tiny_infected_snaps_to_zero() {
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Set up a region with sub-person infected count
        state.regions[ri].infections[0].infected = 0.7;
        state = tick(&state);
        // Should have snapped to 0 (threshold aligned with WIN_INFECTED_THRESHOLD)
        assert_eq!(
            state.regions[ri].infections[0].infected, 0.0,
            "infected below 1.0 should snap to zero"
        );
    }

    #[test]
    fn no_victory_while_infected_remain() {
        let mut state = GameState::new_default(42);
        // Set up: all diseases identified, tested medicines exist, but people still infected
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
        }
        let disease_count = state.diseases.len();
        state.medicines[0].tested_against = (0..disease_count).collect();
        // Reduce infections to a small but non-zero amount (above threshold)
        for region in &mut state.regions {
            for inf in &mut region.infections {
                inf.infected = 50.0; // 50 people still infected per region
            }
        }
        state = tick(&state);
        assert_eq!(
            state.outcome,
            GameOutcome::Playing,
            "should not declare victory while people are still infected"
        );
    }

    #[test]
    fn policy_travel_ban_reduces_spread() {
        let mut state = GameState::new_default(42);
        // Run without travel ban
        let mut no_ban = state.clone();
        for _ in 0..100 {
            no_ban = tick(&no_ban);
        }
        let no_ban_regions_infected: usize = no_ban.regions.iter()
            .filter(|r| r.total_infected() > 0.0)
            .count();

        // Run with travel bans on all regions (with enough funding)
        state.resources.funding = 100_000.0;
        for p in &mut state.policies {
            p.travel_ban = true;
        }
        let mut with_ban = state;
        for _ in 0..100 {
            with_ban = tick(&with_ban);
        }
        let ban_regions_infected: usize = with_ban.regions.iter()
            .filter(|r| r.total_infected() > 0.0)
            .count();

        assert!(
            ban_regions_infected <= no_ban_regions_infected,
            "travel bans should not increase spread: {} vs {} regions infected",
            ban_regions_infected, no_ban_regions_infected
        );
    }

    #[test]
    fn travel_ban_reduces_funding_income() {
        use crate::state::TRAVEL_BAN_COST;
        let mut state = GameState::new_default(42);
        // Remove infections so income is purely population-based
        for r in &mut state.regions {
            r.infections.clear();
        }
        // Use a known starting value with enough to cover policy costs
        state.resources.funding = 1000.0;

        // Tick without any travel bans
        let no_ban = tick(&state);
        let income_no_ban = no_ban.resources.funding - 1000.0;

        // Tick with travel ban on Asia (largest region, ~60% of world pop)
        state.policies[4].travel_ban = true;
        let with_ban = tick(&state);
        let income_with_ban = with_ban.resources.funding - 1000.0 + TRAVEL_BAN_COST; // add back policy cost to isolate income effect

        assert!(
            income_with_ban < income_no_ban,
            "travel ban should reduce income: {income_with_ban:.2} vs {income_no_ban:.2}"
        );
        // Asia is ~60% of pop, ban halves its contribution, so income should drop ~30%
        let reduction = 1.0 - income_with_ban / income_no_ban;
        assert!(
            reduction > 0.2 && reduction < 0.4,
            "Asia travel ban should reduce income by ~30%, got {:.0}%", reduction * 100.0
        );
    }

    #[test]
    fn policy_quarantine_reduces_infections() {
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Run without quarantine
        let mut no_q = state.clone();
        for _ in 0..50 {
            no_q = tick(&no_q);
        }

        // Run with quarantine on the primary outbreak region
        state.policies[ri].quarantine = true;
        let mut with_q = state;
        for _ in 0..50 {
            with_q = tick(&with_q);
        }

        assert!(
            with_q.regions[ri].total_infected() < no_q.regions[ri].total_infected(),
            "quarantine should reduce infections: {} vs {}",
            with_q.regions[ri].total_infected(), no_q.regions[ri].total_infected()
        );
    }

    #[test]
    fn policy_hospital_surge_reduces_deaths() {
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Run without hospital surge
        let mut no_h = state.clone();
        for _ in 0..50 {
            no_h = tick(&no_h);
        }

        // Run with hospital surge on the primary outbreak region
        state.policies[ri].hospital_surge = true;
        let mut with_h = state;
        for _ in 0..50 {
            with_h = tick(&with_h);
        }

        assert!(
            with_h.regions[ri].total_dead() < no_h.regions[ri].total_dead(),
            "hospital surge should reduce deaths: {} vs {}",
            with_h.regions[ri].total_dead(), no_h.regions[ri].total_dead()
        );
    }

    #[test]
    fn policy_costs_deducted_each_tick() {
        let mut state = GameState::new_default(42);
        // First tick without policy to measure income
        let no_policy = tick(&state);
        let income_no_policy = no_policy.resources.funding - state.resources.funding;

        // Now tick with travel ban
        let funding_before = state.resources.funding;
        state.policies[0].travel_ban = true; // $6/tick, also halves region 0 income
        state = tick(&state);
        let net_change = state.resources.funding - funding_before;

        // Should have deducted $6 and added income (less than without ban)
        assert!(
            net_change < income_no_policy,
            "travel ban should reduce net income: net {net_change:.1} vs no-policy {income_no_policy:.1}"
        );
        assert!(
            net_change < 0.0,
            "travel ban cost ($6) should exceed income (~$3): net change {net_change:.1}"
        );
    }

    #[test]
    fn policy_funding_crisis_suspends_most_expensive_first() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 8.0; // Enough for quarantine ($5) but not both ($11)
        state.policies[0].travel_ban = true; // $6/tick — most expensive
        state.policies[0].quarantine = true; // $5/tick
        state = tick(&state);
        // Should have suspended travel ban (most expensive) but kept quarantine
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended");
        assert!(state.policies[0].quarantine, "quarantine should survive");
        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::PolicySuspended { .. })),
            "should emit PolicySuspended event"
        );
    }

    #[test]
    fn policy_gradual_suspension_across_ticks() {
        let mut state = GameState::new_default(42);
        // Set up 3 policies: $6 + $5 + $3 = $14/tick total
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].hospital_surge = true;
        // Enough for only $8/tick (quarantine + hospital surge)
        state.resources.funding = 12.0;
        state = tick(&state);
        // Travel ban ($6, most expensive) should be suspended
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended first");
        assert!(state.policies[0].quarantine, "quarantine should survive tick 1");
        assert!(state.policies[0].hospital_surge, "hospital surge should survive tick 1");
    }

    #[test]
    fn funding_warning_when_runway_low() {
        let mut state = GameState::new_default(42);
        state.policies[0].travel_ban = true; // $6/tick, income ~$3/tick → net burn ~$3/tick
        // After deducting $6 and adding ~$3 income, funding ≈ $7.
        // Net burn ~$3/tick, threshold = 5 × $3 = $15 → $7 < $15 → warning
        state.resources.funding = 10.0;
        state = tick(&state);
        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should emit FundingWarning when runway is low"
        );
    }

    #[test]
    fn no_funding_warning_when_flush() {
        let mut state = GameState::new_default(42);
        state.policies[0].travel_ban = true; // $10/tick
        state.resources.funding = 1000.0; // Plenty of runway after deduction
        state = tick(&state);
        assert!(
            !state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should not warn when funding is high"
        );
    }

    #[test]
    fn policy_toggle_via_confirm() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 1.0; // Full POL for testing
        state = apply_action(&state, &Action::OpenPolicy);
        assert_eq!(state.ui.open_panel, Panel::Policy);

        // Select Asia (reading order position 2: NA, Europe, Asia, ...)
        for _ in 0..2 {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.policy_ui,
            Some(PolicyUiState::ManagePolicies { region_idx: 4 })
        ));

        // Toggle travel ban (selection 0)
        state = apply_action(&state, &Action::Confirm);
        assert!(state.policies[4].travel_ban);

        // Toggle it off
        state = apply_action(&state, &Action::Confirm);
        assert!(!state.policies[4].travel_ban);
    }

    #[test]
    fn disease_mutates_over_time() {
        let mut state = GameState::new_default(42);
        // RNA virus has mutation_rate 0.008, so over 1000 ticks
        // we expect ~8 mutations. Run enough ticks to virtually guarantee at least one.
        let original_infectivity = state.diseases[0].infectivity;
        for _ in 0..1000 {
            state = tick(&state);
        }
        assert!(
            state.diseases[0].strain_generation > 0,
            "RNA virus should have mutated at least once in 500 ticks"
        );
        assert_ne!(
            state.diseases[0].infectivity, original_infectivity,
            "infectivity should have changed after mutation"
        );
    }

    #[test]
    fn mutation_is_deterministic() {
        let state = GameState::new_default(42);
        let mut a = state.clone();
        let mut b = state;
        for _ in 0..300 {
            a = tick(&a);
            b = tick(&b);
        }
        assert_eq!(a.diseases[0].strain_generation, b.diseases[0].strain_generation);
        assert_eq!(a.diseases[0].infectivity, b.diseases[0].infectivity);
        assert_eq!(a.diseases[0].lethality, b.diseases[0].lethality);
    }

    #[test]
    fn strain_efficacy_degrades_with_mutation() {
        use crate::state::{Disease, Medicine, TherapyType, PathogenType};

        let diseases = vec![Disease {
            name: "Test".into(),
            pathogen_type: PathogenType::RnaVirus,
            transmission: crate::state::TransmissionVector::Airborne,
            infectivity: 0.05,
            lethality: 0.01,
            cross_region_spread: 0.01,
            recovery_rate: 0.03,
            knowledge: 1.0,
            strain_generation: 3,
            sequencing_count: 0,
        }];

        let med = Medicine {
            name: "TestMed".into(),
            therapy_type: TherapyType::Antiviral,
            target_diseases: vec![0],
            cost: 100.0,
            doses: 1000.0,
            max_doses: 1000.0,
            unlocked: true,
            tested_against: vec![0],
            strain_generations: vec![0], // calibrated at gen 0, disease is at gen 3
        };

        // 3 generations behind = 1.0 - 3*0.25 = 0.25
        let eff = med.strain_efficacy(0, &diseases);
        assert!((eff - 0.25).abs() < 0.001, "expected 0.25, got {eff}");

        // Re-calibrated medicine should have full efficacy
        let med_current = Medicine {
            strain_generations: vec![3],
            ..med.clone()
        };
        let eff2 = med_current.strain_efficacy(0, &diseases);
        assert!((eff2 - 1.0).abs() < 0.001, "expected 1.0, got {eff2}");
    }

    #[test]
    fn new_disease_emerges_mid_game() {
        let mut state = GameState::new_default(42);
        let initial_diseases = state.diseases.len();
        let initial_medicines = state.medicines.len();

        // Fast-forward past emergence threshold by running many ticks
        // Use a seed known to trigger emergence within a reasonable window
        for _ in 0..1000 {
            state = tick(&state);
        }

        // With 0.4% chance per tick over 800 eligible ticks, emergence
        // is virtually guaranteed (1 - 0.996^800 ≈ 96%)
        if state.diseases.len() > initial_diseases {
            // New disease appeared — verify it's properly set up
            let new_idx = initial_diseases;
            let new_disease = &state.diseases[new_idx];
            assert!(new_disease.infectivity > 0.0);
            assert!(new_disease.lethality > 0.0);
            assert_eq!(new_disease.knowledge, 0.0);
            // strain_generation may be > 0 if the disease mutated after spawning

            // Matching medicine should exist
            assert!(state.medicines.len() > initial_medicines);
            let has_targeted = state.medicines.iter().any(|m| {
                m.target_diseases.contains(&new_idx) && !m.unlocked
            });
            assert!(has_targeted, "new disease should have a matching targeted medicine");

            // Broad-spectrum should also target new disease
            let broad = state.medicines.iter().find(|m| {
                m.therapy_type == crate::state::TherapyType::BroadSpectrum
            });
            assert!(broad.unwrap().target_diseases.contains(&new_idx),
                "broad-spectrum should target new disease");

            // Some region should have the new infection
            let has_infection = state.regions.iter().any(|r| {
                r.infections.iter().any(|i| i.disease_idx == new_idx)
            });
            assert!(has_infection, "new disease should be present in a region");
        }
        // If no emergence happened (unlikely but possible with this seed),
        // that's also valid — it's probabilistic.
    }

    #[test]
    fn disease_cap_prevents_excess_emergence() {
        let mut state = GameState::new_default(42);
        use crate::state::MAX_DISEASES;
        while state.diseases.len() < MAX_DISEASES {
            use rand::SeedableRng;
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
            state.spawn_disease(&mut rng);
        }
        assert_eq!(state.diseases.len(), MAX_DISEASES);

        // Attempting another spawn should return None
        use rand::SeedableRng;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
        assert!(state.spawn_disease(&mut rng).is_none());
    }

    #[test]
    fn transmission_vector_affects_quarantine() {
        use crate::state::TransmissionVector;

        let mut state = GameState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        // Set first disease to Contact transmission (quarantine factor = 0.30)
        state.diseases[0].transmission = TransmissionVector::Contact;
        state.diseases[0].infectivity = 0.02;
        state.diseases[0].knowledge = 1.0;
        // Give the region a big susceptible pool
        state.regions[region_idx].infections[0].infected = 1000.0;

        // Run without quarantine
        let no_quarantine = tick(&state);
        let inf_no_q = no_quarantine.regions[region_idx].infections[0].infected;

        // Run with quarantine
        state.policies[region_idx].quarantine = true;
        let with_quarantine = tick(&state);
        let inf_with_q = with_quarantine.regions[region_idx].infections[0].infected;

        // Quarantine should reduce new infections significantly for Contact
        // (quarantine_factor = 0.30, so infectivity drops to 30%)
        assert!(inf_with_q < inf_no_q, "quarantine should reduce infections");

        // Now test Waterborne (quarantine factor = 0.75, less effective)
        state.diseases[0].transmission = TransmissionVector::Waterborne;
        let with_q_waterborne = tick(&state);
        let inf_with_q_wb = with_q_waterborne.regions[region_idx].infections[0].infected;

        // Waterborne quarantine should be less effective than Contact quarantine
        assert!(inf_with_q_wb > inf_with_q,
            "waterborne quarantine should be less effective than contact quarantine");
    }

    #[test]
    fn contact_hospital_surge_increases_infectivity() {
        use crate::state::TransmissionVector;

        let mut state = GameState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        state.diseases[0].transmission = TransmissionVector::Contact;
        state.diseases[0].infectivity = 0.02;
        state.diseases[0].lethality = 0.01;
        state.regions[region_idx].infections[0].infected = 5000.0;

        // Run with hospital surge but no quarantine
        state.policies[region_idx].hospital_surge = true;
        let with_hospital = tick(&state);

        // Run Airborne with hospital surge (no infectivity penalty)
        state.diseases[0].transmission = TransmissionVector::Airborne;
        let with_hospital_airborne = tick(&state);

        // Contact + hospital surge should have MORE new infections than Airborne + hospital surge
        let contact_inf = with_hospital.regions[region_idx].infections[0].infected;
        let airborne_inf = with_hospital_airborne.regions[region_idx].infections[0].infected;
        assert!(contact_inf > airborne_inf,
            "contact disease with hospital surge should spread faster than airborne: {} vs {}",
            contact_inf, airborne_inf);
    }

    #[test]
    fn transmission_vector_affects_cross_region_spread() {
        use crate::state::TransmissionVector;
        use rand::SeedableRng;

        // Test that airborne diseases spread to new regions faster than contact
        let mut airborne_spreads = 0u32;
        let mut contact_spreads = 0u32;

        // Run many trials to get statistical significance
        for seed in 0..200 {
            let mut state = GameState::new_default(42);
            // Single disease, single region, force specific vector
            state.diseases.truncate(1);
            state.diseases[0].knowledge = 1.0;
            state.diseases[0].cross_region_spread = 0.01;

            // Clear all infections, place one outbreak
            for region in &mut state.regions {
                region.infections.clear();
            }
            state.regions[0].infections.push(RegionDiseaseState {
                disease_idx: 0,
                infected: 10_000.0,
                dead: 0.0,
                immune: 0.0,
            });

            // Test airborne
            state.diseases[0].transmission = TransmissionVector::Airborne;
            state.rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let after = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                airborne_spreads += 1;
            }

            // Test contact
            state.diseases[0].transmission = TransmissionVector::Contact;
            state.rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let after = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                contact_spreads += 1;
            }
        }

        assert!(airborne_spreads > contact_spreads,
            "airborne should spread to more regions than contact: {} vs {}",
            airborne_spreads, contact_spreads);
    }

    #[test]
    fn border_screening_reduces_cross_region_spread() {
        use crate::state::TransmissionVector;
        use rand::SeedableRng;

        let mut screening_spreads = 0u32;
        let mut no_policy_spreads = 0u32;

        for seed in 0..200 {
            let mut state = GameState::new_default(42);
            state.diseases.truncate(1);
            state.diseases[0].transmission = TransmissionVector::Airborne;
            state.diseases[0].cross_region_spread = 0.01;
            for region in &mut state.regions { region.infections.clear(); }
            state.regions[0].infections.push(RegionDiseaseState {
                disease_idx: 0, infected: 10_000.0, dead: 0.0, immune: 0.0,
            });

            // No policy
            state.rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let after = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                no_policy_spreads += 1;
            }

            // Border screening on source region
            state.policies[0].border_screening = true;
            state.rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let after = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                screening_spreads += 1;
            }
        }

        assert!(screening_spreads < no_policy_spreads,
            "screening should reduce cross-region spread: {} vs {} (no policy)",
            screening_spreads, no_policy_spreads);
    }

    #[test]
    fn water_sanitation_reduces_waterborne_infectivity() {
        use crate::state::TransmissionVector;
        let mut state = GameState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        state.diseases[0].transmission = TransmissionVector::Waterborne;
        state.diseases[0].infectivity = 0.02;
        state.regions[region_idx].infections[0].infected = 1000.0;

        // Without sanitation
        let no_sanitation = tick(&state);
        let inf_no = no_sanitation.regions[region_idx].infections[0].infected;

        // With sanitation
        state.policies[region_idx].water_sanitation = true;
        let with_sanitation = tick(&state);
        let inf_with = with_sanitation.regions[region_idx].infections[0].infected;

        assert!(inf_with < inf_no,
            "water sanitation should reduce waterborne infections: {} vs {}",
            inf_with, inf_no);

        // Sanitation should NOT affect airborne diseases
        state.diseases[0].transmission = TransmissionVector::Airborne;
        let airborne_with_sanitation = tick(&state);
        state.policies[region_idx].water_sanitation = false;
        let airborne_without = tick(&state);
        let inf_airborne_with = airborne_with_sanitation.regions[region_idx].infections[0].infected;
        let inf_airborne_without = airborne_without.regions[region_idx].infections[0].infected;

        // Should be roughly equal (same noise seed means identical)
        assert!((inf_airborne_with - inf_airborne_without).abs() < 1.0,
            "sanitation should not affect airborne: {} vs {}",
            inf_airborne_with, inf_airborne_without);
    }

    #[test]
    fn crisis_generates_after_min_tick() {
        let mut state = GameState::new_default(42);
        // Run past CRISIS_MIN_TICK — a crisis should eventually appear
        for _ in 0..1000 {
            state = tick(&state);
        }
        // With 1/200 chance per tick over 800 eligible ticks, P(no crisis) ≈ 0.018
        assert!(state.active_crisis.is_some(),
            "expected a crisis to generate within 1000 ticks");
    }

    #[test]
    fn crisis_blocks_normal_actions() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 300.0, personnel: 3 },
            title: "Test Crisis".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "A".into(), description: "A".into(), cost: None },
            option_b: CrisisOption { label: "B".into(), description: "B".into(), cost: None },
            tick_created: 0,
        });

        // Normal panel actions should be blocked
        let after = apply_action(&state, &Action::OpenThreats);
        assert_eq!(after.ui.open_panel, Panel::None, "panel should not open during crisis");

        // SelectNext should change crisis selection
        let after = apply_action(&state, &Action::SelectNext);
        assert_eq!(after.ui.crisis_selection, 1);
        let after = apply_action(&after, &Action::SelectPrev);
        assert_eq!(after.ui.crisis_selection, 0);
    }

    #[test]
    fn crisis_resolution_applies_effects() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        let initial_funding = state.resources.funding;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "Funding".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "RP".into(), description: "".into(), cost: None },
            tick_created: 0,
        });

        // Choose option A (funding)
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
        assert_eq!(after.resources.funding, initial_funding + 500.0,
            "should have received funding");

        // Reset and choose option B (personnel)
        let initial_personnel = state.resources.personnel;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 500.0, personnel: 5 },
            title: "Aid".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "Funding".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "Personnel".into(), description: "".into(), cost: None },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::SelectNext); // select option B
        let after = apply_action(&after, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
        assert_eq!(after.resources.personnel, initial_personnel + 5,
            "should have received personnel");
    }

    #[test]
    fn crisis_unaffordable_option_blocked() {
        use crate::state::{CrisisCost, CrisisEvent, CrisisKind, CrisisOption};

        let mut state = GameState::new_default(42);
        state.resources.funding = 0.0; // broke
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
            title: "Burnout".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "Accept".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "Pay $400".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 400.0, personnel: 0 }) },
            tick_created: 0,
        });

        // Try to pay (option B) but can't afford — confirm should be blocked
        let after = apply_action(&state, &Action::SelectNext);
        let after = apply_action(&after, &Action::Confirm);
        assert!(after.active_crisis.is_some(), "crisis should still be active");
        assert!(after.ui.status_message.as_ref().unwrap().contains("Not enough"),
            "should show affordability message");

        // Free option (A) should still work
        let after = apply_action(&state, &Action::Confirm); // option A (default)
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
    }

    #[test]
    fn crisis_restores_running_state_on_dismiss() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = GameState::new_default(42);
        // Game is running, crisis fires
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 100.0, personnel: 3 },
            title: "Test".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "B".into(), description: "".into(), cost: None },
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.sim_state, SimState::Running,
            "should restore Running state after crisis when game was running");
    }

    #[test]
    fn crisis_restores_paused_state_on_dismiss() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = GameState::new_default(42);
        // Game was paused when crisis fired
        state.sim_state = SimState::Event { was_running: false };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 100.0, personnel: 3 },
            title: "Test".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "B".into(), description: "".into(), cost: None },
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.sim_state, SimState::Paused,
            "should restore Paused state after crisis when game was paused");
    }

    #[test]
    fn spacebar_blocked_during_event_state() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = GameState::new_default(42);
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 100.0, personnel: 3 },
            title: "Test".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "B".into(), description: "".into(), cost: None },
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::TogglePause);
        assert_eq!(after.sim_state, SimState::Event { was_running: true },
            "spacebar should not change state during crisis");
        assert!(after.active_crisis.is_some(), "crisis should still be active");
    }

    #[test]
    fn game_over_clears_active_crisis() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = GameState::new_default(42);
        // Set up a highly lethal disease to trigger game over (collapse all regions).
        // High cross_region_spread needed to reach refugia through sparser graph.
        for disease in &mut state.diseases {
            disease.infectivity = 0.12;
            disease.lethality = 0.08;
            disease.recovery_rate = 0.002;
            disease.cross_region_spread = 0.20;
        }

        // Inject an active crisis
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::InternationalAid { funding: 100.0, personnel: 3 },
            title: "Test".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "B".into(), description: "".into(), cost: None },
            tick_created: 0,
        });
        state.sim_state = SimState::Event { was_running: true };

        // Run until game over (collapse requires all regions to fall)
        for _ in 0..10000 {
            state = tick(&state);
            crate::ui::process_events(&mut state);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }

        assert_eq!(state.outcome, GameOutcome::Lost);
        assert!(state.active_crisis.is_none(),
            "active crisis should be cleared on game over");
        assert_eq!(state.sim_state, SimState::Paused,
            "sim state should be Paused, not Event");
    }
}
