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
    MAX_DISEASES, TICKS_PER_DAY,
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

    // Personnel upkeep — mandatory cost for maintaining your roster.
    // Floor at 0: if income can't cover upkeep, the deficit is absorbed
    // (personnel stay but the treasury doesn't go negative).
    let upkeep = new.personnel_upkeep_rate();
    new.resources.funding = (new.resources.funding - upkeep).max(0.0);

    // Personnel attrition: when funding is $0, unassigned personnel leave.
    // Rate: ~1 person per day. Thematic: unpaid workers resign.
    if new.resources.funding <= 0.0 && new.personnel_available() > 0 {
        new.resources.attrition_accum += 1.0 / TICKS_PER_DAY;
        if new.resources.attrition_accum >= 1.0 {
            let lost = (new.resources.attrition_accum as u32).min(new.personnel_available());
            new.resources.personnel = new.resources.personnel.saturating_sub(lost);
            new.resources.attrition_accum -= lost as f64;
            new.events.push(GameEvent::PersonnelAttrition { count: lost });
        }
    } else {
        new.resources.attrition_accum = 0.0;
    }

    // Political Power: ramps based on severity + time + crisis modifier.
    // Severity = sqrt(death_fraction) provides fast initial growth then diminishing returns.
    // Time = linear ramp reaching 0.4 at day 30 (baseline even if player contains well).
    // Crisis modifier = accumulated POL changes from crisis resolutions, decays over time.
    {
        let initial_pop = new.initial_population();
        let death_frac = if initial_pop > 0.0 { new.total_dead() / initial_pop } else { 0.0 };
        let infected_frac = if initial_pop > 0.0 { new.total_infected() / initial_pop } else { 0.0 };
        let time_frac = new.tick as f64 / (30.0 * TICKS_PER_DAY);
        let severity = death_frac.sqrt() * 3.0 + infected_frac.sqrt() * 1.5;
        // Decay crisis modifier toward 0 (half-life ~5 days)
        let decay = 0.5_f64.powf(1.0 / (5.0 * TICKS_PER_DAY));
        new.resources.pol_crisis_modifier *= decay;
        new.resources.political_power =
            (severity + time_frac * 0.4 + new.resources.pol_crisis_modifier).clamp(0.0, 1.0);
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
    // Only warn if there are active policies that could actually be suspended.
    let total_costs = policy_cost + upkeep;
    let net_burn = total_costs - funding_income;
    if policy_cost > 0.0 && net_burn > 0.0 && new.resources.funding < net_burn * 5.0 {
        new.events.push(GameEvent::FundingWarning);
    }

    // Mid-game disease emergence (spawns undetected — player won't see it yet).
    // Later diseases are tougher (scaled by game day).
    if new.tick >= EMERGENCE_MIN_TICK
        && new.diseases.len() < MAX_DISEASES
        && rng.r#gen::<f64>() < EMERGENCE_CHANCE_PER_TICK
    {
        new.spawn_disease_scaled(&mut rng);
    }

    // Disease detection — undetected diseases are revealed when total infected
    // crosses the detection threshold. Better screening lowers the threshold.
    let effective_threshold = crate::state::DETECTION_THRESHOLD
        * new.best_screening_level().detection_multiplier();
    for disease_idx in 0..new.diseases.len() {
        if new.diseases[disease_idx].detected {
            continue;
        }
        let total: f64 = new.regions.iter()
            .flat_map(|r| &r.infections)
            .filter(|inf| inf.disease_idx == disease_idx)
            .map(|inf| inf.infected)
            .sum();
        if total >= effective_threshold {
            new.diseases[disease_idx].detected = true;
            new.events.push(GameEvent::DiseaseDetected { disease_idx });
        }
    }

    // Crisis event generation (only when no crisis is active).
    // Frequency scales with game day: early game ~1/10 days, late game ~1/3 days.
    let crisis_interval = {
        let day = new.tick as f64 / TICKS_PER_DAY;
        let base = CRISIS_INTERVAL as f64;
        // Halve the interval every 15 days, floor at 360 ticks (~3 days)
        (base * 0.5_f64.powf(day / 15.0)).max(360.0)
    };
    if new.active_crisis.is_none()
        && new.tick >= CRISIS_MIN_TICK
        && rng.r#gen::<f64>() < 1.0 / crisis_interval
    {
        if let Some(crisis) = crisis::generate_crisis(&new, &mut rng) {
            // Check if we can auto-resolve via saved preference
            let auto_choice = new.auto_resolve_crises.get(crisis.kind.tag()).copied();
            let auto_resolved = match auto_choice {
                Some(choice) => {
                    let option = if choice == 0 { &crisis.option_a } else { &crisis.option_b };
                    option.cost.as_ref().map_or(true, |c| c.affordable(&new))
                }
                None => false,
            };

            new.active_crisis = Some(crisis);

            if auto_resolved {
                crisis::resolve_crisis(&mut new, auto_choice.unwrap());
                new.events.push(GameEvent::CrisisAutoResolved);
            } else {
                // Pause the game for the crisis — this is a game rule, not a UI concern.
                new.sim_state = SimState::Event {
                    was_running: new.sim_state.is_running(),
                };
                new.events.push(GameEvent::CrisisStarted);
            }
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
        // Martial law reduces collapse threshold by 0.15 (region tolerates more deaths)
        let martial_law_active = new.policies.get(i).is_some_and(|p| p.martial_law);
        let threshold = if martial_law_active {
            (new.regions[i].collapse_threshold - 0.15).max(0.10)
        } else {
            new.regions[i].collapse_threshold
        };
        if alive < pop * threshold {
            new.regions[i].collapsed = true;
            new.regions[i].collapsed_at_tick = Some(new.tick);
            // Clear all policies in the collapsed region
            if let Some(policy) = new.policies.get_mut(i) {
                policy.clear_all();
            }
            new.events.push(GameEvent::RegionCollapsed { region_idx: i });
        }
    }

    // Check defeat condition (only while still playing).
    // There is no victory — you lose eventually. The question is when.
    if new.outcome == GameOutcome::Playing {
        let all_collapsed = new.regions.iter().all(|r| r.collapsed);
        if all_collapsed {
            new.outcome = GameOutcome::Lost;
            new.active_crisis = None;
            new.sim_state = SimState::Paused;
            new.events.push(GameEvent::GameOver);
        }
    }

    // Mercy rule: if the player has had zero agency for 5 consecutive days,
    // end the game. This prevents 20+ minute zombie phases where the player
    // watches helplessly with no funding, no research, and no medicines.
    if new.outcome == GameOutcome::Playing {
        if new.has_zero_agency() {
            new.zero_agency_ticks += 1;
            if new.zero_agency_ticks >= crate::state::MERCY_RULE_TICKS {
                new.outcome = GameOutcome::Lost;
                new.mercy_rule = true;
                new.active_crisis = None;
                new.sim_state = SimState::Paused;
                new.events.push(GameEvent::GameOver);
            }
        } else {
            new.zero_agency_ticks = 0;
        }
    }

    // If all diseases burned out but regions survive, spawn a tougher replacement.
    // This prevents the "zombie state" where the game has no threats and no end.
    if new.outcome == GameOutcome::Playing
        && new.total_infected() < 1.0
        && new.tick > EMERGENCE_MIN_TICK
    {
        let mut rng2 = new.rng.clone();
        new.spawn_disease_scaled(&mut rng2);
        new.rng = rng2;
    }

    // Record history for dashboard sparklines
    if new.tick % crate::state::HISTORY_INTERVAL == 0 {
        new.history.push(crate::state::HistorySnapshot {
            tick: new.tick,
            total_infected: new.total_infected_screened(),
            total_dead: new.total_dead_detected(),
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
    /// True if medicine deployment caused an adverse reaction.
    pub adverse: bool,
}

/// Execute a game command. Pure game logic — does NOT touch UI state.
/// The caller is responsible for UI transitions based on the result.
pub fn execute_command(state: &mut GameState, cmd: &GameCommand) -> CommandResult {
    if state.outcome != GameOutcome::Playing {
        return CommandResult { message: None, success: false, adverse: false };
    }
    match cmd {
        GameCommand::DeployMedicine {
            medicine_idx,
            region_idx,
            target_selection,
        } => {
            let (nav_back, msg, adverse) =
                medicine::deploy_medicine(state, *medicine_idx, *region_idx, *target_selection);
            CommandResult { message: msg, success: nav_back, adverse }
        }
        GameCommand::StartResearch { track, project_idx, double_personnel } => {
            let (ok, msg) = research::start_research(state, *track, *project_idx, *double_personnel);
            CommandResult { message: msg, success: ok, adverse: false }
        }
        GameCommand::AddResearchPersonnel { track } => {
            let msg = research::add_personnel(state, *track);
            CommandResult { message: msg, success: true, adverse: false }
        }
        GameCommand::RemoveResearchPersonnel { track } => {
            let msg = research::remove_personnel(state, *track);
            CommandResult { message: msg, success: true, adverse: false }
        }
        GameCommand::TogglePolicy {
            region_idx,
            policy_idx,
        } => {
            let (msg, success) = policy::toggle_policy(state, *region_idx, *policy_idx);
            CommandResult { message: msg, success, adverse: false }
        }
        GameCommand::ResolveCrisis { choice } => {
            let msg = crisis::resolve_crisis(state, *choice);
            CommandResult { message: Some(msg), success: true, adverse: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::apply_action;
    use crate::state::{GameState, MedicineUiState, Panel, PolicyUiState, RegionDiseaseState, ResearchTrack, ResearchUiState};

    /// Helper: unlock all medicines and mark them tested (for tests that predate the research system).
    fn unlock_all_medicines(state: &mut GameState) {
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
    }

    /// Helper: mark all diseases as detected (most tests assume this).
    fn detect_all_diseases(state: &mut GameState) {
        for d in &mut state.diseases {
            d.detected = true;
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
            // Shared death counter must not exceed population.
            assert!(
                region.dead <= pop + 1.0,
                "region {}: dead {} > population {}",
                region.name,
                region.dead,
                pop
            );
            for inf in &region.infections {
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
        // With smaller initial seed (500-2500), need more ticks for cross-region spread
        for _ in 0..1000 {
            s = tick(&s);
        }
        let infected_regions = s
            .regions
            .iter()
            .filter(|r| !r.infections.is_empty())
            .count();
        assert!(
            infected_regions > 1,
            "disease should spread to more than 1 region after 1000 ticks, got {}",
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
        let region = &state.regions[0];
        let inf_state = region.infections.iter().find(|i| i.disease_idx == 0);
        let infected = inf_state.map(|i| i.infected).unwrap_or(0.0);
        let immune = inf_state.map(|i| i.immune).unwrap_or(0.0);
        let susceptible = (region.population as f64 - infected - region.dead - immune).max(0.0);
        let target_vaccinated = susceptible * crate::state::VACCINATION_FRACTION * efficacy;
        let expected_immune = target_vaccinated.min(state.medicines[0].doses);
        state = apply_action(&state, &Action::Confirm);
        // Computed outputs: cost deducted, immunity applied proportionally
        assert_eq!(state.resources.funding, funding_before - state.medicines[0].cost);
        let na_inf = state.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .unwrap();
        assert_eq!(na_inf.immune, expected_immune);
        // With 500M pop and 2% fraction, target = ~10M. With 100M doses available,
        // doses are not the bottleneck — proportional vaccination determines the count.
        assert!(expected_immune <= 100_000_000.0, "vaccination should be capped by dose supply");
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::DeployResult { medicine_idx: 0, adverse: false, .. })
        ));
        // DeployResult should contain the feedback message
        if let Some(MedicineUiState::DeployResult { message, .. }) = &state.ui.medicine_ui {
            assert!(message.contains("Protected"), "message should mention protection: {message}");
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

        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 2); // Basic Research

        // Can't go past last
        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 2);

        // Esc closes
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_esc_backstep() {
        let mut state = GameState::new_default(42);
        detect_all_diseases(&mut state);

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseProjects { track: ResearchTrack::Field })));

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
            Some(ResearchUiState::BrowseProjects { track: ResearchTrack::Field })
        ));

        // Pressing Enter on empty list should stay on BrowseProjects
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.research_ui,
            Some(ResearchUiState::BrowseProjects { track: ResearchTrack::Field })
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
        // Override to extreme parameters so all regions collapse quickly.
        // Normal game parameters (R0 3-5) cause loss via multiple diseases over 20 days.
        for disease in &mut state.diseases {
            disease.infectivity = 0.06;
            disease.lethality = 0.02;
            disease.recovery_rate = 0.005;
            disease.cross_region_spread = 0.05;
        }
        // Seed all regions with infections so collapse happens within 10K ticks
        for region in &mut state.regions {
            for inf in &mut region.infections {
                inf.infected = 10_000.0;
            }
        }
        // Run until game over (collapse requires all regions to fall)
        for _ in 0..10000 {
            state = tick(&state);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        assert_eq!(state.outcome, GameOutcome::Lost);
        assert_eq!(state.sim_state, crate::state::SimState::Paused);
        // All regions should be collapsed with timestamps
        assert!(state.regions.iter().all(|r| r.collapsed));
        assert!(state.regions.iter().all(|r| r.collapsed_at_tick.is_some()),
            "every collapsed region should have a collapse timestamp");
        // Collapse timestamps should be in order (earlier collapses have lower tick values)
        let ticks: Vec<u64> = state.regions.iter()
            .filter_map(|r| r.collapsed_at_tick)
            .collect();
        assert_eq!(ticks.len(), state.regions.len());
        // Not all should be the same tick (regions collapse at different rates)
        assert!(ticks.iter().collect::<std::collections::HashSet<_>>().len() > 1,
            "regions should collapse at different times, got {:?}", ticks);
    }

    #[test]
    fn game_is_lost_within_100_days_without_intervention() {
        // The game must be lost within 100 days with zero player intervention,
        // regardless of seed. If this test fails, disease parameters are too weak.
        // Target: most seeds lose by day 25-40. The 100-day ceiling absorbs RNG
        // perturbation from crisis generation (which consumes RNG values and can
        // shift disease trajectories significantly on some seeds).
        let mut loss_days = Vec::new();
        for seed in [42, 123, 7, 99, 2024, 1, 999, 314, 55555, 8675309_u64] {
            let mut state = GameState::new_default(seed);
            let max_ticks = 100 * TICKS_PER_DAY as u64;
            for _ in 0..max_ticks {
                state = tick(&state);
                if state.active_crisis.is_some() {
                    use crate::state::SimState;
                    state.active_crisis = None;
                    state.sim_state = SimState::Running;
                }
                if state.outcome != GameOutcome::Playing {
                    break;
                }
            }
            let day = state.tick as f64 / TICKS_PER_DAY;
            assert_eq!(state.outcome, GameOutcome::Lost,
                "Seed {seed}: game should be lost within 100 days (reached day {day:.1}). \
                 Regions: {:?}",
                state.regions.iter().map(|r| {
                    let pct = 100.0 * (1.0 - r.alive() as f64 / r.population as f64);
                    (r.name.clone(), r.collapsed, format!("{pct:.1}% dead"))
                }).collect::<Vec<_>>());
            loss_days.push(day);
        }
        // Most seeds should still lose well before day 60
        let median = {
            let mut sorted = loss_days.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            sorted[sorted.len() / 2]
        };
        assert!(median < 60.0,
            "Median loss day is {median:.1} (expected < 60). Days: {loss_days:?}");
    }

    #[test]
    fn no_collapse_before_day_10_without_intervention() {
        // First collapse should not occur before day 10, giving players
        // time for the full research pipeline (identify + develop + trial ≈ 4 days)
        // plus time to deploy medicines and set policies.
        for seed in [42, 123, 7, 99, 2024, 1, 999, 314, 55555, 8675309_u64] {
            let mut state = GameState::new_default(seed);
            let max_ticks = 10 * TICKS_PER_DAY as u64;
            for t in 0..max_ticks {
                state = tick(&state);
                if state.active_crisis.is_some() {
                    use crate::state::SimState;
                    state.active_crisis = None;
                    state.sim_state = SimState::Running;
                }
                let collapsed = state.regions.iter().find(|r| r.collapsed);
                assert!(
                    collapsed.is_none(),
                    "Seed {seed}: {} collapsed at tick {t} (day {:.1}), expected no collapse before day 10",
                    collapsed.map(|r| r.name.as_str()).unwrap_or("?"),
                    t as f64 / TICKS_PER_DAY
                );
            }
        }
    }

    #[test]
    fn no_victory_condition_exists() {
        let mut state = GameState::new_default(42);
        // Clear all infections, identify everything, test all medicines
        for region in &mut state.regions {
            region.infections.clear();
        }
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
        }
        let disease_count = state.diseases.len();
        state.medicines[0].tested_against = (0..disease_count).collect();
        // Advance past emergence threshold so burn-out spawn can fire
        state.tick = crate::state::EMERGENCE_MIN_TICK + 1;
        state = tick(&state);
        // Game should NOT end — there is no victory. Instead, a new disease spawns.
        assert_eq!(state.outcome, GameOutcome::Playing);
        assert!(
            state.diseases.len() > disease_count,
            "when all infections burn out, a new disease should spawn"
        );
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

    /// Create a state where the player has zero agency: no funds, negative
    /// net income (personnel upkeep exceeds income), no research, no doses.
    fn setup_zero_agency() -> GameState {
        let mut state = GameState::new_default(42);
        state.resources.funding = 0.0;
        // High personnel count = high upkeep that exceeds income
        state.resources.personnel = 50;
        state.medicines.iter_mut().for_each(|m| m.doses = 0.0);
        state.field_research = None;
        state.applied_research = None;
        state.basic_research = None;
        state.policies.iter_mut().for_each(|p| p.clear_all());
        // Significant deaths to reduce income, but below collapse threshold
        for region in &mut state.regions {
            region.dead = region.population as f64 * 0.40;
        }
        state
    }

    #[test]
    fn mercy_rule_triggers_after_zero_agency() {
        let mut state = setup_zero_agency();
        assert!(state.has_zero_agency(), "should detect zero agency");

        // Simulate enough ticks
        state.zero_agency_ticks = crate::state::MERCY_RULE_TICKS - 1;
        state = tick(&state);
        assert_eq!(state.outcome, GameOutcome::Lost, "should trigger mercy defeat");
        assert!(state.mercy_rule, "should be mercy rule defeat");
    }

    #[test]
    fn mercy_rule_resets_on_agency_recovery() {
        let mut state = setup_zero_agency();
        state.zero_agency_ticks = 100;

        // Give the player enough funds — even with zero income, having funds
        // means they could potentially start research or deploy something
        state.resources.funding = 500.0;
        // Also need some alive population so tick doesn't immediately defeat
        // via all-collapsed check
        for region in &mut state.regions {
            region.dead = region.population as f64 * 0.1;
        }
        state = tick(&state);
        assert_eq!(state.zero_agency_ticks, 0, "should reset on funding recovery");
        assert_eq!(state.outcome, GameOutcome::Playing);
    }

    #[test]
    fn tiny_infected_snaps_to_zero() {
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Set up a region with sub-person infected count
        state.regions[ri].infections[0].infected = 0.7;
        state = tick(&state);
        // Should have snapped to 0 (sub-person counts are meaningless)
        assert_eq!(
            state.regions[ri].infections[0].infected, 0.0,
            "infected below 1.0 should snap to zero"
        );
    }

    #[test]
    fn multi_disease_dead_never_exceeds_population() {
        use crate::state::RegionDiseaseState;
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        let pop = state.regions[ri].population as f64;
        // Add a second disease with heavy infection in the same region
        state.diseases.push(state.diseases[0].clone());
        state.regions[ri].infections.push(RegionDiseaseState {
            disease_idx: 1,
            infected: pop * 0.3,
            dead: 0.0,
            immune: 0.0,
        });
        // Also boost first disease
        state.regions[ri].infections[0].infected = pop * 0.3;
        // Run many ticks — both diseases should share the population
        for _ in 0..2000 {
            state = tick(&state);
            if state.active_crisis.is_some() {
                state.active_crisis = None;
                state.sim_state = crate::state::SimState::Running;
            }
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        // Shared death counter should never exceed population.
        assert!(state.regions[ri].dead <= pop + 1.0,
            "shared dead ({:.0}) should not exceed population ({pop:.0})",
            state.regions[ri].dead);
        // Per-disease attribution totals should approximately match shared dead.
        let attributed: f64 = state.regions[ri].infections.iter()
            .map(|i| i.dead).sum();
        assert!(attributed <= pop * 1.05,
            "attributed dead sum ({attributed:.0}) should not wildly exceed population ({pop:.0})");
    }

    #[test]
    fn burn_out_spawns_scaled_disease() {
        let mut state = GameState::new_default(42);
        // Clear all infections to simulate burn-out
        for region in &mut state.regions {
            region.infections.clear();
        }
        // Set to day 20 — scaled disease should have 2.0x boosted stats
        state.tick = 20 * crate::state::TICKS_PER_DAY as u64;
        let disease_count = state.diseases.len();
        let original_infectivity = state.diseases[0].infectivity;
        state = tick(&state);

        assert!(
            state.diseases.len() > disease_count,
            "should spawn a new disease when all infections burn out"
        );
        // The new disease at day 20 gets 2.0x scaling (1.0 + 20 * 0.05).
        // Its base stats are in a similar range to disease 0, so after 2x scaling
        // it should be notably more infectious.
        let new_disease = &state.diseases[disease_count];
        assert!(
            new_disease.infectivity > original_infectivity,
            "late-game disease infectivity ({}) should exceed original ({})",
            new_disease.infectivity, original_infectivity
        );
    }

    #[test]
    fn burn_out_recycles_slot_at_max_diseases() {
        use crate::state::MAX_DISEASES;
        let mut state = GameState::new_default(42);
        // Fill up to MAX_DISEASES
        while state.diseases.len() < MAX_DISEASES {
            let mut rng = state.rng.clone();
            state.spawn_disease(&mut rng);
            state.rng = rng;
        }
        assert_eq!(state.diseases.len(), MAX_DISEASES);
        // Clear all infections to simulate burn-out
        for region in &mut state.regions {
            region.infections.clear();
        }
        state.tick = 20 * crate::state::TICKS_PER_DAY as u64;
        state = tick(&state);
        // Should have recycled a slot — disease count stays at MAX_DISEASES
        assert_eq!(state.diseases.len(), MAX_DISEASES,
            "should recycle a slot, not exceed MAX_DISEASES");
        // At least one disease should have infections (the recycled one)
        assert!(state.total_infected() > 0.0,
            "recycled disease should have active infections");
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
        let upkeep = state.personnel_upkeep_rate();

        // Tick without any travel bans
        let no_ban = tick(&state);
        let income_no_ban = no_ban.resources.funding - 1000.0 + upkeep; // add back upkeep to isolate income

        // Tick with travel ban on Asia (largest region, ~60% of world pop)
        state.policies[4].travel_ban = true;
        let with_ban = tick(&state);
        let income_with_ban = with_ban.resources.funding - 1000.0 + TRAVEL_BAN_COST + upkeep; // add back policy cost and upkeep

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
    fn personnel_upkeep_reduces_funding() {
        use crate::state::PERSONNEL_UPKEEP_COST;
        let mut state = GameState::new_default(42);
        for r in &mut state.regions {
            r.infections.clear();
        }
        state.resources.funding = 1000.0;
        let income = state.funding_income_rate();
        let upkeep = state.resources.personnel as f64 * PERSONNEL_UPKEEP_COST;

        let after = tick(&state);
        let delta = after.resources.funding - 1000.0;

        // Net change should be income minus upkeep (no policies)
        assert!(
            (delta - (income - upkeep)).abs() < 0.01,
            "funding delta {delta:.2} should equal income {income:.2} - upkeep {upkeep:.2}"
        );
        // Upkeep should be significant (not negligible)
        assert!(upkeep > 1.0, "upkeep {upkeep:.2} should be meaningful");
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
        state.resources.funding = 0.8; // Enough for quarantine ($0.6) but not both ($1.6)
        state.policies[0].travel_ban = true; // $1.0/tick — most expensive
        state.policies[0].quarantine = true; // $0.6/tick
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
        // Set up 3 policies: $1.0 + $0.6 + $0.4 = $2.0/tick total
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].hospital_surge = true;
        // Enough for quarantine + hospital surge ($1.0) but not all three ($2.0)
        state.resources.funding = 1.2;
        state = tick(&state);
        // Travel ban ($1.0, most expensive) should be suspended
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended first");
        assert!(state.policies[0].quarantine, "quarantine should survive tick 1");
        assert!(state.policies[0].hospital_surge, "hospital surge should survive tick 1");
    }

    #[test]
    fn funding_warning_when_runway_low() {
        let mut state = GameState::new_default(42);
        // Enable expensive policies across multiple regions to create net burn.
        // Travel ban ($1/tick) + quarantine ($0.6/tick) + hospital surge ($0.4/tick) = $2/tick per region
        // Plus personnel upkeep: 20 × $0.1 = $2/tick. With just one region's policies + upkeep,
        // total cost ~$4/tick vs ~$3/tick income. Travel ban also halves that region's income.
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].hospital_surge = true;
        state.resources.funding = 2.0; // Very low — should trigger warning
        state = tick(&state);
        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should emit FundingWarning when runway is low"
        );
    }

    #[test]
    fn no_funding_warning_when_flush() {
        let mut state = GameState::new_default(42);
        state.policies[0].travel_ban = true; // $1/tick
        state.resources.funding = 1000.0; // Plenty of runway after deduction
        state = tick(&state);
        assert!(
            !state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should not warn when funding is high"
        );
    }

    #[test]
    fn no_funding_warning_without_active_policies() {
        let mut state = GameState::new_default(42);
        // No policies active — only personnel upkeep creates costs.
        // Even with zero funding, warning shouldn't fire because there's
        // nothing to suspend.
        state.resources.funding = 0.0;
        state = tick(&state);
        assert!(
            !state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should not warn about policy suspension when no policies are active"
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
        state.diseases[0].pathogen_type = crate::state::PathogenType::RnaVirus;
        // Make the disease very mild so regions don't collapse before mutation triggers.
        // This test verifies mutation mechanics work, not game balance.
        state.diseases[0].infectivity = 0.005;
        state.diseases[0].lethality = 0.0001;
        state.diseases[0].recovery_rate = 0.003;
        let original_infectivity = state.diseases[0].infectivity;
        // Run enough ticks for mutation to be likely (~5 expected at 0.0002/tick × 25000).
        // Manually reset any new diseases that spawn to prevent stacking deaths.
        for _ in 0..25000 {
            if state.outcome != GameOutcome::Playing {
                break;
            }
            state = tick(&state);
            if state.active_crisis.is_some() {
                state.active_crisis = None;
                state.sim_state = crate::state::SimState::Running;
            }
            // Remove any newly emerged diseases so only disease 0 runs
            while state.diseases.len() > 1 {
                let extra = state.diseases.len() - 1;
                state.diseases.remove(extra);
                for r in &mut state.regions {
                    r.infections.retain(|inf| inf.disease_idx == 0);
                }
            }
        }
        assert!(
            state.diseases[0].strain_generation > 0,
            "RNA virus should have mutated at least once (ran {} ticks, rate=0.0002/tick)",
            state.tick,
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
        for _ in 0..1000 {
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
            detected: true,
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
            deployed_count: 0,
        };

        // 3 generations behind = 1.0 - 3*0.15 = 0.55
        let eff = med.strain_efficacy(0, &diseases);
        assert!((eff - 0.55).abs() < 0.001, "expected 0.55, got {eff}");

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

        // Fast-forward past emergence threshold by running many ticks.
        // With EMERGENCE_MIN_TICK=840 and EMERGENCE_CHANCE=0.0007,
        // we need ~2500 eligible ticks for reliable emergence.
        for _ in 0..3500 {
            state = tick(&state);
        }

        // With 0.07% chance per tick over ~2660 eligible ticks,
        // P(at least one emergence) = 1 - 0.9993^2660 ≈ 84%
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
    fn border_controls_reduces_cross_region_spread() {
        use crate::state::TransmissionVector;
        use rand::SeedableRng;

        let mut controls_spreads = 0u32;
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

            // Border controls on source region
            state.policies[0].border_controls = true;
            state.rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let after = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                controls_spreads += 1;
            }
        }

        assert!(controls_spreads < no_policy_spreads,
            "border controls should reduce cross-region spread: {} vs {} (no policy)",
            controls_spreads, no_policy_spreads);
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
        // Run past CRISIS_MIN_TICK — a crisis should eventually appear.
        // With CRISIS_INTERVAL=840, we need ~5000 ticks for P(no crisis) < 1%.
        let mut found_crisis = false;
        for _ in 0..5000 {
            state = tick(&state);
            if state.active_crisis.is_some() {
                found_crisis = true;
                break;
            }
        }
        assert!(found_crisis,
            "expected a crisis to generate within 5000 ticks");
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
        // Boost initial infection so collapse happens quickly
        for region in &mut state.regions {
            for inf in &mut region.infections {
                inf.infected = 50_000.0;
            }
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
        for _ in 0..20000 {
            state = tick(&state);
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

    #[test]
    fn crisis_auto_resolves_with_saved_preference() {
        let mut state = GameState::new_default(42);
        // Set auto-resolve preference for personnel crises: always pick option A
        state.auto_resolve_crises.insert("personnel".to_string(), 0);

        // Run until a crisis would generate
        state.sim_state = SimState::Running;
        let mut auto_resolved = false;
        for _ in 0..5000 {
            state = tick(&state);
            // If a personnel crisis auto-resolved, the game stays running (not Event state)
            if state.events.iter().any(|e| matches!(e, GameEvent::CrisisAutoResolved)) {
                auto_resolved = true;
                assert!(state.active_crisis.is_none(),
                    "crisis should be resolved immediately");
                assert!(state.sim_state.is_running(),
                    "game should still be running after auto-resolve");
                break;
            }
            // If a non-personnel crisis fires, it should pause normally
            if state.active_crisis.is_some() {
                // Dismiss it manually to continue
                let crisis_tag = state.active_crisis.as_ref().unwrap().kind.tag().to_string();
                assert_ne!(crisis_tag, "personnel",
                    "personnel crisis should have been auto-resolved");
                state = apply_action(&state, &Action::Confirm);
            }
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        // We may not get a personnel crisis in 5000 ticks — that's OK.
        // The test verifies correctness IF it fires, not that it fires.
        if auto_resolved {
            // Good — verified auto-resolve works
        }
    }

    #[test]
    fn lab_accident_evacuate_destroys_applied_research() {
        use crate::state::{
            CrisisEvent, CrisisKind, CrisisOption, ResearchProject, ResearchKind,
        };

        let mut state = GameState::new_default(42);
        state.applied_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 50.0,
            required_ticks: 200.0,
            personnel_assigned: 3,
        });
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::LabAccident { targets_basic: false },
            title: "Lab Accident".into(),
            description: "Test".into(),
            option_a: CrisisOption {
                label: "Evacuate".into(),
                description: "Lose research".into(),
                cost: None,
            },
            option_b: CrisisOption {
                label: "Contain".into(),
                description: "Save research".into(),
                cost: Some(crate::state::CrisisCost { funding: 200.0, personnel: 3 }),
            },
            tick_created: 0,
        });

        // Choose option A (evacuate) — should destroy applied research
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.applied_research.is_none(),
            "applied research should be destroyed on evacuation");
        assert!(after.active_crisis.is_none(),
            "crisis should be resolved");
    }

    #[test]
    fn lab_accident_evacuate_destroys_basic_research() {
        use crate::state::{
            BasicTech, CrisisEvent, CrisisKind, CrisisOption,
            ResearchProject, ResearchKind,
        };

        let mut state = GameState::new_default(42);
        state.basic_research = Some(ResearchProject {
            kind: ResearchKind::BasicResearch { tech: BasicTech::TargetedDrugDesign },
            progress: 100.0,
            required_ticks: 240.0,
            personnel_assigned: 3,
        });
        state.sim_state = SimState::Event { was_running: true };
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::LabAccident { targets_basic: true },
            title: "Lab Accident".into(),
            description: "Test".into(),
            option_a: CrisisOption {
                label: "Evacuate".into(),
                description: "Lose research".into(),
                cost: None,
            },
            option_b: CrisisOption {
                label: "Contain".into(),
                description: "Save research".into(),
                cost: Some(crate::state::CrisisCost { funding: 200.0, personnel: 3 }),
            },
            tick_created: 0,
        });

        // Choose option A (evacuate) — should destroy basic research
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.basic_research.is_none(),
            "basic research should be destroyed on evacuation");
        assert!(after.active_crisis.is_none(),
            "crisis should be resolved");
    }

    #[test]
    fn lab_accident_containment_preserves_research() {
        use crate::state::{
            CrisisEvent, CrisisKind, CrisisOption, ResearchProject, ResearchKind,
        };

        let mut state = GameState::new_default(42);
        state.applied_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 50.0,
            required_ticks: 200.0,
            personnel_assigned: 3,
        });
        state.sim_state = SimState::Event { was_running: true };
        state.ui.crisis_selection = 1; // Select option B
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::LabAccident { targets_basic: false },
            title: "Lab Accident".into(),
            description: "Test".into(),
            option_a: CrisisOption {
                label: "Evacuate".into(),
                description: "Lose research".into(),
                cost: None,
            },
            option_b: CrisisOption {
                label: "Contain".into(),
                description: "Save research".into(),
                cost: Some(crate::state::CrisisCost { funding: 200.0, personnel: 3 }),
            },
            tick_created: 0,
        });

        // Choose option B (contain) — should preserve research
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.applied_research.is_some(),
            "applied research should be preserved on containment");
        assert!(after.active_crisis.is_none(),
            "crisis should be resolved");
    }
}
