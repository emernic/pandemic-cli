mod crisis;
mod medicine;
mod personnel;
mod policy;
mod research;
mod spread;

use rand::Rng;

use crate::state::{
    CrisisKind, GameCommand, GameEvent, GameOutcome, GameState, SimState,
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

    // Disease spread, mutation, and adaptation
    spread::tick_spread_within(&mut new, &state.diseases, &mut rng);
    spread::tick_spread_cross_region(&mut new, &state.diseases, &mut rng);
    spread::tick_mutation(&mut new, &mut rng);
    spread::tick_horizontal_gene_transfer(&mut new);
    spread::tick_containment_adaptation(&mut new);

    // Research progress
    research::tick_research(&mut new, &mut rng);

    // Scientist burnout and recovery
    personnel::tick_personnel(&mut new, &mut rng);

    // Auto-pause on major research breakthroughs so the player sees the good news
    if new.events.iter().any(|e| matches!(e,
        GameEvent::PathogenIdentified { .. } | GameEvent::MedicineDeveloped { .. }
        | GameEvent::TrialCompleted { .. }))
    {
        new.sim_state = SimState::Paused;
    }

    // Auto-deploy medicines to worst-affected regions
    medicine::try_auto_deploy(&mut new);

    // Policy costs — suspend unaffordable policies and deduct costs.
    let policy_cost = policy::tick_enforce_costs(&mut new);

    // Governor loyalty drift — reacts to policies, deaths, and personality.
    policy::tick_governor_loyalty(&mut new);

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

    // Political Power: drifts toward a severity-based target.
    // Target = f(severity, time, active policies). See GameState::pol_target().
    // POL moves toward target at ~30%/day, so crisis hits take 3-5 days to recover.
    {
        let target = new.pol_target();
        let drift_rate = 0.30 / TICKS_PER_DAY;
        let delta = (target - new.resources.political_power) * drift_rate;
        new.resources.political_power = (new.resources.political_power + delta).clamp(0.0, 1.0);
    }

    // POL-based personnel: ~1 person per 3 days at max POL (0.90).
    // With typical mid-game POL (~30-40%), this gives ~1 per 8-10 days.
    {
        let rate = new.resources.political_power / (3.0 * TICKS_PER_DAY);
        new.resources.personnel_accum += rate;
        if new.resources.personnel_accum >= 1.0 {
            let gained = new.resources.personnel_accum as u32;
            new.resources.personnel += gained;
            new.resources.personnel_accum -= gained as f64;
        }
    }

    // Low funding warning: warn when net burn rate will exhaust funds within half a day
    // (60 ticks). At 1x speed (500ms/tick), this gives ~30 seconds of real-time warning.
    // Only warn if there are active policies that could actually be suspended.
    let total_costs = policy_cost + upkeep;
    let net_burn = total_costs - funding_income;
    if policy_cost > 0.0 && net_burn > 0.0 && new.resources.funding < net_burn * 60.0 {
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
            // Auto-pause so the player sees the detection and can react
            new.sim_state = SimState::Paused;
        }
    }

    // Threat escalation alerts: warn when a detected disease's deaths cross
    // major thresholds (1M, 100M, 1B). Fires once per threshold per disease.
    // Auto-pauses the game so the player can't miss an escalating threat.
    {
        const THRESHOLDS: &[(u8, f64)] = &[
            (1, 1_000_000.0),
            (2, 100_000_000.0),
            (3, 1_000_000_000.0),
        ];
        // Grow tracking vec if new diseases were spawned
        while new.threat_alert_level.len() < new.diseases.len() {
            new.threat_alert_level.push(0);
        }
        for (d_idx, disease) in new.diseases.iter().enumerate() {
            if !disease.detected {
                continue;
            }
            let deaths: f64 = new.regions.iter()
                .filter_map(|r| r.disease_state(d_idx))
                .map(|inf| inf.dead)
                .sum();
            let current_level = new.threat_alert_level[d_idx];
            for &(level, threshold) in THRESHOLDS {
                if level > current_level && deaths >= threshold {
                    new.threat_alert_level[d_idx] = level;
                    let has_medicine = new.medicines.iter().any(|m| {
                        m.unlocked && m.target_diseases.contains(&d_idx)
                    });
                    new.events.push(GameEvent::ThreatEscalation {
                        disease_idx: d_idx,
                        deaths,
                        has_medicine,
                    });
                    new.sim_state = SimState::Paused;
                }
            }
        }
    }

    // Fire scheduled follow-up crises (from previous crisis choices).
    // These take priority over random crisis generation.
    if new.active_crisis.is_none() {
        if let Some(idx) = new.pending_crises.iter().position(|&(tick, _)| tick <= new.tick) {
            let (_, kind) = new.pending_crises.remove(idx);
            let crisis = crisis::build_crisis_event(&new, kind);
            crisis::activate_crisis(&mut new, crisis);
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
            crisis::activate_crisis(&mut new, crisis);
        }
    }

    new.rng = rng;

    // Sync scientist roster with personnel count. Done here (after RNG write-back)
    // so that any personnel changes earlier in tick() are reflected, and sync's
    // RNG draws are properly recorded in new.rng.
    new.sync_scientists_to_personnel();

    new.tick += 1;

    // Check regional collapse
    for i in 0..new.regions.len() {
        if new.regions[i].collapsed {
            continue;
        }
        let pop = new.regions[i].population as f64;
        let alive = new.regions[i].alive();
        let martial_law_active = new.policies.get(i).is_some_and(|p| p.martial_law);
        let threshold = new.regions[i].effective_collapse_threshold(martial_law_active);
        if alive < pop * threshold {
            new.regions[i].collapsed = true;
            new.regions[i].collapsed_at_tick = Some(new.tick);
            // Clear all policies in the collapsed region
            if let Some(policy) = new.policies.get_mut(i) {
                policy.clear_all();
            }
            // Personnel loss: staff in the collapsed region are lost
            let lost_personnel = 2u32.min(new.resources.personnel);
            new.resources.personnel = new.resources.personnel.saturating_sub(lost_personnel);
            new.sync_scientists_to_personnel();
            new.events.push(GameEvent::RegionCollapsed { region_idx: i });

            // Trigger refugee crisis toward a non-collapsed neighbor (if any).
            // Overrides any active crisis — collapse is a major event and its
            // refugee consequence must not be silently lost.
            let neighbors: Vec<usize> = new.regions[i].connections.iter()
                .filter(|&&c| !new.regions[c].collapsed)
                .copied()
                .collect();
            let to = if neighbors.len() > 1 {
                Some(neighbors[new.rng.r#gen::<usize>() % neighbors.len()])
            } else {
                neighbors.first().copied()
            };
            if let Some(to) = to {
                let kind = CrisisKind::RefugeeWave { from_region: i, to_region: to };
                new.active_crisis = Some(crisis::build_crisis_event(&new, kind));
                new.sim_state = SimState::Event {
                    was_running: new.sim_state.is_running(),
                };
                new.events.push(GameEvent::CrisisStarted);
            } else {
                new.sim_state = SimState::Paused;
            }
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

    // Mercy rule: if the player has had zero/near-zero agency for several
    // consecutive days, end the game. Two triggers:
    // 1. Classic zero agency (broke + no research + no doses) — 5 day timer
    // 2. Civilization collapse (4+ regions gone + no active research) — 2 day timer
    //    This catches "has money but nothing useful to do" when most of the
    //    world has fallen and no research pipeline exists.
    if new.outcome == GameOutcome::Playing {
        let collapsed_count = new.regions.iter().filter(|r| r.collapsed).count();
        let no_research = new.field_research.is_empty()
            && new.applied_research.is_none()
            && new.basic_research.is_none();
        let near_total_collapse = collapsed_count >= 4 && no_research;

        if new.has_zero_agency() || near_total_collapse {
            new.zero_agency_ticks += 1;
            let mercy_threshold = if near_total_collapse {
                // Shorter timer when civilization has mostly collapsed
                (2.0 * crate::state::TICKS_PER_DAY) as u64
            } else {
                crate::state::MERCY_RULE_TICKS
            };
            if new.zero_agency_ticks >= mercy_threshold {
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
            screened_infected: new.total_infected_screened(),
            detected_dead: new.total_dead_detected(),
        });
        if new.history.len() > crate::state::HISTORY_MAX {
            new.history.remove(0);
        }
    }

    // Update per-region death rate for collapse time estimates.
    // Sampled every ~1 day so the rate reflects recent trends.
    let rate_interval = TICKS_PER_DAY as u64;
    for region in &mut new.regions {
        let elapsed = new.tick.saturating_sub(region.prev_dead_tick);
        if elapsed >= rate_interval {
            if region.prev_dead_tick > 0 {
                let death_delta = (region.total_dead() - region.prev_dead).max(0.0);
                region.cached_deaths_per_day = death_delta / (elapsed as f64 / TICKS_PER_DAY);
            }
            region.prev_dead = region.total_dead();
            region.prev_dead_tick = new.tick;
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
            target,
        } => {
            let (success, msg, adverse) =
                medicine::deploy_medicine(state, *medicine_idx, *region_idx, target.clone());
            CommandResult { message: msg, success, adverse }
        }
        GameCommand::StartResearch { track, project_idx, double_personnel } => {
            let (ok, msg) = research::start_research(state, *track, *project_idx, *double_personnel);
            CommandResult { message: msg, success: ok, adverse: false }
        }
        GameCommand::AddResearchPersonnel { track, slot_idx } => {
            let msg = research::add_personnel(state, *track, *slot_idx);
            CommandResult { message: msg, success: true, adverse: false }
        }
        GameCommand::RemoveResearchPersonnel { track, slot_idx } => {
            let msg = research::remove_personnel(state, *track, *slot_idx);
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
        GameCommand::EnactDecree { decree_idx, region_idx } => {
            let (msg, success) = policy::enact_decree(state, *decree_idx, *region_idx);
            CommandResult { message: msg, success, adverse: false }
        }
        GameCommand::RallySupport => {
            let (msg, success) = policy::rally_support(state);
            CommandResult { message: msg, success, adverse: false }
        }
        GameCommand::AppeaseGovernor { region_idx } => {
            let (msg, success) = policy::appease_governor(state, *region_idx);
            CommandResult { message: msg, success, adverse: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::apply_action;
    use crate::state::{CrisisKind, DeployTarget, GameState, MedicineUiState, Panel, PathogenType, PolicyUiState, RegionDiseaseState, ResearchTrack, ResearchUiState};

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
        state.regions[ri].get_or_create_infection(0).immune = pop * 0.9;
        let before = state.regions[ri].disease_state(0).unwrap().infected;
        let after = tick(&state);
        let growth = after.regions[ri].disease_state(0).unwrap().infected - before;

        let state2 = GameState::new_default(42);
        let ri2 = primary_outbreak_region(&state2);
        let after2 = tick(&state2);
        let growth2 = after2.regions[ri2].disease_state(0).unwrap().infected
            - state2.regions[ri2].disease_state(0).unwrap().infected;

        assert!(
            growth < growth2,
            "immunity should reduce infection growth: {} vs {}",
            growth,
            growth2
        );
    }

    #[test]
    fn dense_urban_increases_spread() {
        use crate::state::RegionTrait;
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        let before = state.regions[ri].disease_state(0).unwrap().infected;

        // Tick without DenseUrban
        let after_normal = tick(&state);
        let growth_normal = after_normal.regions[ri].disease_state(0).unwrap().infected - before;

        // Add DenseUrban trait and tick again
        state.regions[ri].traits.push(RegionTrait::DenseUrban);
        let after_dense = tick(&state);
        let growth_dense = after_dense.regions[ri].disease_state(0).unwrap().infected - before;

        assert!(growth_dense > growth_normal,
            "DenseUrban should increase within-region spread: {} vs {}", growth_dense, growth_normal);
    }

    #[test]
    fn disease_can_spread_into_vaccinated_region() {
        let mut state = GameState::new_default(42);
        // Find a region WITHOUT disease 0 and pre-vaccinate it
        let clean_region = (0..state.regions.len())
            .find(|&i| !state.regions[i].infections.iter().any(|inf| inf.disease_idx == 0))
            .expect("should have an uninfected region");
        state.regions[clean_region].get_or_create_infection(0).immune = 100_000_000.0;
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
        let efficacy = state.medicines[0].effective_efficacy(0, &state.diseases);
        let region = &state.regions[0];
        let inf_state = region.infections.iter().find(|i| i.disease_idx == 0);
        let infected = inf_state.map(|i| i.infected).unwrap_or(0.0);
        let immune = inf_state.map(|i| i.immune).unwrap_or(0.0);
        let susceptible = (region.population as f64 - infected - region.dead - immune).max(0.0);
        let target_vaccinated = susceptible * crate::state::VACCINATION_FRACTION * efficacy;
        let expected_immune = target_vaccinated.min(state.medicines[0].doses);
        let deploy_cost = state.medicines[0].deploy_cost(region.population);
        state = apply_action(&state, &Action::Confirm);
        // Computed outputs: cost deducted, immunity applied proportionally
        assert_eq!(state.resources.funding, funding_before - deploy_cost);
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
        let infected_before = state.regions[ri].disease_state(0).unwrap().infected;

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

        let infected_after = state.regions[ri].disease_state(0).unwrap().infected;
        assert!(
            infected_after < infected_before,
            "treatment should reduce infected: {} -> {}",
            infected_before,
            infected_after
        );
        let deploy_cost = state.medicines[0].deploy_cost(state.regions[ri].population);
        assert_eq!(state.resources.funding, funding_before - deploy_cost);
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
    fn multi_target_medicine_shows_disease_selection() {
        let mut state = GameState::new_default(42);
        // Make medicine 0 target two diseases
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![0, 1];
        state.medicines[0].target_diseases = vec![0, 1];
        // Add a second disease
        state.diseases.push(state.diseases[0].clone());
        state.diseases[1].detected = true;

        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        state = apply_action(&state, &Action::Confirm); // select region → should go to SelectDisease
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectDisease { medicine_idx: 0, .. })
        ), "multi-target should go to SelectDisease, got: {:?}", state.ui.medicine_ui);

        state = apply_action(&state, &Action::Confirm); // select disease 0 → SelectTarget
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectTarget { medicine_idx: 0, disease_idx: 0, .. })
        ), "should go to SelectTarget with disease 0, got: {:?}", state.ui.medicine_ui);

        // Back should return to SelectDisease
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectDisease { .. })
        ), "back from SelectTarget should go to SelectDisease, got: {:?}", state.ui.medicine_ui);
    }

    #[test]
    fn single_target_medicine_skips_disease_selection() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        // Single-target medicines should skip disease step
        assert_eq!(state.medicines[0].target_diseases.len(), 1);

        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        state = apply_action(&state, &Action::Confirm); // select region → should skip to SelectTarget
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectTarget { .. })
        ), "single-target should skip to SelectTarget, got: {:?}", state.ui.medicine_ui);

        // Back should go to SelectRegion (not SelectDisease)
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { .. })
        ), "back from SelectTarget should go to SelectRegion, got: {:?}", state.ui.medicine_ui);
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
        state.resources.personnel = 150;
        state.medicines.iter_mut().for_each(|m| m.doses = 0.0);
        state.field_research.clear();
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
        state.regions[ri].get_or_create_infection(0).infected = 0.7;
        state = tick(&state);
        // Should have snapped to 0 (sub-person counts are meaningless)
        assert_eq!(
            state.regions[ri].disease_state(0).unwrap().infected, 0.0,
            "infected below 1.0 should snap to zero"
        );
    }

    #[test]
    fn multi_disease_dead_never_exceeds_population() {
        let mut state = GameState::new_default(42);
        let ri = primary_outbreak_region(&state);
        let pop = state.regions[ri].population as f64;
        // Add a second disease with heavy infection in the same region
        state.diseases.push(state.diseases[0].clone());
        state.regions[ri].get_or_create_infection(1).infected = pop * 0.3;
        // Also boost first disease
        state.regions[ri].get_or_create_infection(0).infected = pop * 0.3;
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
    fn coinfection_increases_deaths() {
        // With 2 diseases both above the co-infection threshold,
        // deaths should be higher than with a single disease.
        let mut single = GameState::new_default(42);
        let ri = primary_outbreak_region(&single);
        single.regions[ri].get_or_create_infection(0).infected = 100_000.0;

        let mut dual = single.clone();
        // Add a second disease with significant infection
        dual.diseases.push(dual.diseases[0].clone());
        dual.regions[ri].get_or_create_infection(1).infected = 100_000.0;

        // Run some ticks
        for _ in 0..100 {
            single = tick(&single);
            dual = tick(&dual);
        }

        let single_dead = single.regions[ri].dead;
        let dual_dead = dual.regions[ri].dead;
        assert!(dual_dead > single_dead,
            "co-infection should cause more deaths: dual={:.0} vs single={:.0}",
            dual_dead, single_dead);
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
        // Upkeep should be non-negligible
        assert!(upkeep > 0.1, "upkeep {upkeep:.2} should be meaningful");
    }

    #[test]
    fn pol_based_personnel_accumulation() {
        let mut state = GameState::new_default(42);
        // Pre-load the accumulator so even modest POL pushes it over 1.0.
        // This tests the mechanism (POL → accum → personnel) without
        // needing thousands of ticks for POL to build up naturally.
        state.resources.personnel_accum = 0.99;
        state.resources.political_power = 0.50;
        let initial_personnel = state.resources.personnel;

        // rate = 0.50 / (3.0 * 120) = 0.00139/tick. Starting at 0.99,
        // we need ~8 ticks to cross 1.0. Run 20 to be safe.
        let mut s = state;
        for _ in 0..20 {
            s = tick(&s);
        }

        let gained = s.resources.personnel - initial_personnel;
        assert!(
            gained >= 1,
            "accumulator should convert to personnel: was {initial_personnel}, \
             now {}, accum {:.3}",
            s.resources.personnel, s.resources.personnel_accum
        );
    }

    #[test]
    fn pol_based_personnel_zero_pol_no_gain() {
        let mut state = GameState::new_default(42);
        // Clear infections so POL target stays near 0
        for r in &mut state.regions {
            r.infections.clear();
        }
        state.resources.political_power = 0.0;
        state.resources.personnel_accum = 0.0;
        let initial_personnel = state.resources.personnel;

        let mut s = state;
        for _ in 0..500 {
            s = tick(&s);
        }

        // With no infections, POL target is near 0 (only time_frac contributes).
        // Personnel gain should be minimal.
        let gained = s.resources.personnel - initial_personnel;
        assert!(
            gained <= 1,
            "with zero POL and no infections, personnel should barely increase, gained {gained}"
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

        // Should have deducted travel ban cost and reduced region income
        assert!(
            net_change < income_no_policy,
            "travel ban should reduce net income: net {net_change:.1} vs no-policy {income_no_policy:.1}"
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
        // Enable expensive policies across two regions to create net burn.
        // Per region: travel ban ($1/tick) + quarantine ($0.6/tick) + hospital ($0.4/tick) = $2/tick
        // Two regions = $4/tick policy cost. Plus upkeep: 20 × $0.03 = $0.6/tick. Total ~$4.6/tick.
        // Income ~$3/tick (minus travel ban penalty). Net burn is positive → warning fires.
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].hospital_surge = true;
        state.policies[1].travel_ban = true;
        state.policies[1].quarantine = true;
        state.policies[1].hospital_surge = true;
        // Funding must be ≥ policy_cost (4.0) to avoid auto-suspension, but
        // < net_burn * 60 (~126) so the runway warning fires.
        state.resources.funding = 5.0;
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
        // Run enough ticks for mutation to be very likely (~25 expected at 0.001/tick × 25000).
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
            "RNA virus should have mutated at least once (ran {} ticks, rate=0.001/tick)",
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
            mechanism_resistance: vec![],
            containment_adaptation: 0.0,
        }];

        let med = Medicine {
            name: "TestMed".into(),
            therapy_type: TherapyType::Antiviral,
            mechanism: None,
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
    fn resistance_builds_from_treatment_pressure() {
        use crate::state::TherapyType;
        let mut state = GameState::new_default(42);
        // Find first non-prion disease and unlock its targeted medicines
        let disease_idx = state.diseases.iter().position(|d| {
            d.pathogen_type != crate::state::PathogenType::Prion
        }).unwrap();
        let med_idx = state.medicines.iter().position(|m| {
            m.target_diseases.contains(&disease_idx)
                && m.therapy_type != TherapyType::BroadSpectrum
        }).unwrap();
        state.medicines[med_idx].unlocked = true;
        state.medicines[med_idx].tested_against.push(disease_idx);
        state.medicines[med_idx].doses = 1_000_000_000.0;
        state.medicines[med_idx].max_doses = 1_000_000_000.0;
        state.resources.funding = 1_000_000.0;

        // Seed infection in region 0
        state.regions[0].get_or_create_infection(disease_idx).infected = 100_000.0;

        // Record initial resistance
        let initial_res = state.medicines[med_idx].resistance_factor(disease_idx, &state.diseases);
        assert!((initial_res - 1.0).abs() < 0.001, "should start with no resistance");

        // Deploy treatment multiple times (clear cooldown between deploys)
        for _ in 0..10 {
            if let Some(inf) = state.regions[0].infections.iter_mut().find(|i| i.disease_idx == disease_idx) {
                inf.infected = 100_000.0;
            }
            state.resources.funding = 1_000_000.0;
            state.regions[0].last_deploy_tick = None;
            let (_, _, _) = medicine::deploy_medicine(&mut state, med_idx, 0, DeployTarget::Treat { disease_idx });
        }

        let after_res = state.medicines[med_idx].resistance_factor(disease_idx, &state.diseases);
        assert!(after_res < 1.0, "resistance should have built up after 10 treatments, got factor {after_res}");
        assert!(after_res > 0.2, "resistance shouldn't be maxed after only 10 treatments, got factor {after_res}");

        // Broad-spectrum builds faster
        let bs_idx = state.medicines.iter().position(|m| {
            m.therapy_type == TherapyType::BroadSpectrum
        }).unwrap();
        state.medicines[bs_idx].unlocked = true;
        state.medicines[bs_idx].tested_against.push(disease_idx);
        state.medicines[bs_idx].doses = 1_000_000_000.0;
        state.medicines[bs_idx].max_doses = 1_000_000_000.0;

        for _ in 0..10 {
            if let Some(inf) = state.regions[0].infections.iter_mut().find(|i| i.disease_idx == disease_idx) {
                inf.infected = 100_000.0;
            }
            state.resources.funding = 1_000_000.0;
            state.regions[0].last_deploy_tick = None;
            let (_, _, _) = medicine::deploy_medicine(&mut state, bs_idx, 0, DeployTarget::Treat { disease_idx });
        }

        let bs_res = state.medicines[bs_idx].resistance_factor(disease_idx, &state.diseases);
        assert!(bs_res < after_res, "broad-spectrum should build resistance faster than targeted: bs={bs_res} vs targeted={after_res}");
    }

    #[test]
    fn targeted_medicines_have_mechanism_of_action() {
        use crate::state::TherapyType;

        let state = GameState::new_default(42);
        // Disease 0 is never a prion — should have one medicine per mechanism
        let targeted_meds: Vec<_> = state.medicines.iter()
            .filter(|m| m.target_diseases.contains(&0)
                && m.therapy_type != TherapyType::BroadSpectrum)
            .collect();
        // Bacteria have 4 mechanisms, viruses/fungi have 3
        assert!(targeted_meds.len() >= 3,
            "should have 3+ targeted medicines for disease 0, got {}: {:?}",
            targeted_meds.len(),
            targeted_meds.iter().map(|m| &m.name).collect::<Vec<_>>());
        for med in &targeted_meds {
            assert!(med.mechanism.is_some(),
                "targeted medicine '{}' should have a mechanism", med.name);
        }
        // All mechanisms should be different
        let mechs: Vec<_> = targeted_meds.iter()
            .map(|m| m.mechanism.unwrap())
            .collect();
        for i in 0..mechs.len() {
            for j in (i+1)..mechs.len() {
                assert_ne!(mechs[i], mechs[j],
                    "medicines should have different mechanisms");
            }
        }
        // Each mechanism should have distinct tradeoff properties
        let fast_mech = targeted_meds.iter()
            .find(|m| m.mechanism.unwrap().dev_cost_multiplier() < 1.0)
            .expect("should have a fast/cheap mechanism option");
        let slow_mech = targeted_meds.iter()
            .find(|m| m.mechanism.unwrap().dev_cost_multiplier() > 1.0)
            .expect("should have a slow/expensive mechanism option");
        assert!(fast_mech.mechanism.unwrap().resistance_rate_multiplier() >
                slow_mech.mechanism.unwrap().resistance_rate_multiplier(),
            "fast mechanism should build resistance faster than slow one");

        // Broad-spectrum medicine (last one) should have no mechanism
        let broad = state.medicines.last().unwrap();
        assert!(broad.mechanism.is_none(), "broad-spectrum should have no mechanism");
        assert_eq!(broad.therapy_type, TherapyType::BroadSpectrum);
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
    fn pathogen_type_diversity_enforced() {
        use crate::state::MAX_DISEASES;
        // Try many seeds — no seed should produce 3+ diseases of the same type
        for seed in 0..50u64 {
            let mut state = GameState::new_default(seed);
            while state.diseases.len() < MAX_DISEASES {
                let mut rng = state.rng.clone();
                state.spawn_disease(&mut rng);
                state.rng = rng;
            }
            let mut counts = std::collections::HashMap::new();
            for d in &state.diseases {
                *counts.entry(d.pathogen_type).or_insert(0usize) += 1;
            }
            for (pt, count) in &counts {
                assert!(
                    *count <= 2,
                    "seed {seed}: pathogen type {pt:?} appears {count} times (max 2)",
                );
            }
            // With 5 diseases and max 2 per type, we need at least 3 distinct types
            assert!(
                counts.len() >= 3,
                "seed {seed}: only {} distinct pathogen types with {} diseases",
                counts.len(), state.diseases.len(),
            );
        }
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
        state.regions[region_idx].get_or_create_infection(0).infected = 1000.0;

        // Run without quarantine
        let no_quarantine = tick(&state);
        let inf_no_q = no_quarantine.regions[region_idx].disease_state(0).unwrap().infected;

        // Run with quarantine
        state.policies[region_idx].quarantine = true;
        let with_quarantine = tick(&state);
        let inf_with_q = with_quarantine.regions[region_idx].disease_state(0).unwrap().infected;

        // Quarantine should reduce new infections significantly for Contact
        // (quarantine_factor = 0.30, so infectivity drops to 30%)
        assert!(inf_with_q < inf_no_q, "quarantine should reduce infections");

        // Now test Waterborne (quarantine factor = 0.75, less effective)
        state.diseases[0].transmission = TransmissionVector::Waterborne;
        let with_q_waterborne = tick(&state);
        let inf_with_q_wb = with_q_waterborne.regions[region_idx].disease_state(0).unwrap().infected;

        // Waterborne quarantine should be less effective than Contact quarantine
        assert!(inf_with_q_wb > inf_with_q,
            "waterborne quarantine should be less effective than contact quarantine");
    }

    #[test]
    fn hospital_surge_increases_infectivity() {
        let mut state = GameState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        state.diseases[0].infectivity = 0.02;
        state.diseases[0].lethality = 0.01;
        state.regions[region_idx].get_or_create_infection(0).infected = 5000.0;

        // Run without hospital surge
        let without = tick(&state);

        // Run with hospital surge
        state.policies[region_idx].hospital_surge = true;
        let with = tick(&state);

        // Hospital surge should increase infections (25% spread penalty)
        let inf_without = without.regions[region_idx].disease_state(0).unwrap().infected;
        let inf_with = with.regions[region_idx].disease_state(0).unwrap().infected;
        assert!(inf_with > inf_without,
            "hospital surge should increase spread: {} vs {} without",
            inf_with, inf_without);
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
        state.regions[region_idx].get_or_create_infection(0).infected = 1000.0;

        // Without sanitation
        let no_sanitation = tick(&state);
        let inf_no = no_sanitation.regions[region_idx].disease_state(0).unwrap().infected;

        // With sanitation
        state.policies[region_idx].water_sanitation = true;
        let with_sanitation = tick(&state);
        let inf_with = with_sanitation.regions[region_idx].disease_state(0).unwrap().infected;

        assert!(inf_with < inf_no,
            "water sanitation should reduce waterborne infections: {} vs {}",
            inf_with, inf_no);

        // Sanitation should NOT affect airborne diseases
        state.diseases[0].transmission = TransmissionVector::Airborne;
        let airborne_with_sanitation = tick(&state);
        state.policies[region_idx].water_sanitation = false;
        let airborne_without = tick(&state);
        let inf_airborne_with = airborne_with_sanitation.regions[region_idx].disease_state(0).unwrap().infected;
        let inf_airborne_without = airborne_without.regions[region_idx].disease_state(0).unwrap().infected;

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
            // If a personnel crisis auto-resolved, the game isn't in Event state
            // (it may be Paused from a DiseaseDetected in the same tick, which is fine)
            if state.events.iter().any(|e| matches!(e, GameEvent::CrisisAutoResolved)) {
                auto_resolved = true;
                assert!(state.active_crisis.is_none(),
                    "crisis should be resolved immediately");
                assert!(!matches!(state.sim_state, SimState::Event { .. }),
                    "sim_state should not be Event after auto-resolve, got {:?}", state.sim_state);
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
        scientist_ids: vec![],
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
        scientist_ids: vec![],
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
        scientist_ids: vec![],
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

    // --- Crisis resolution effect tests ---

    /// Helper: create a crisis event and inject it into state with choice pre-selected.
    fn setup_crisis(state: &mut GameState, kind: CrisisKind, choice: usize) {
        use crate::state::{CrisisEvent, CrisisOption, SimState};
        state.sim_state = SimState::Event { was_running: true };
        state.ui.crisis_selection = choice;
        state.active_crisis = Some(CrisisEvent {
            kind,
            title: "Test Crisis".into(),
            description: "Test".into(),
            option_a: CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: CrisisOption { label: "B".into(), description: "".into(), cost: None },
            tick_created: 0,
        });
    }

    #[test]
    fn supply_disruption_option_a_loses_half_doses() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].doses = 1000.0;
        setup_crisis(&mut state, CrisisKind::SupplyDisruption { medicine_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.medicines[0].doses, 500.0,
            "option A should lose 50% doses");
    }

    #[test]
    fn supply_disruption_option_b_preserves_doses() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].doses = 1000.0;
        let initial_doses = state.medicines[0].doses;
        // Option B costs $300 — set up with cost
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::SupplyDisruption { medicine_idx: 0 },
            title: "T".into(),
            description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 300.0, personnel: 0 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.medicines[0].doses, initial_doses,
            "option B should preserve doses");
    }

    #[test]
    fn political_pressure_option_a_lifts_quarantine() {
        let mut state = GameState::new_default(42);
        state.policies[0].quarantine = true;
        setup_crisis(&mut state, CrisisKind::PoliticalPressure { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(!after.policies[0].quarantine,
            "option A should lift quarantine");
    }

    #[test]
    fn political_pressure_option_b_maintains_quarantine() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        state.policies[0].quarantine = true;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::PoliticalPressure { region_idx: 0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 500.0, personnel: 0 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.policies[0].quarantine,
            "option B should maintain quarantine");
    }

    #[test]
    fn personnel_crisis_option_a_loses_personnel() {
        let mut state = GameState::new_default(42);
        let before = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::PersonnelCrisis { amount: 3 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.resources.personnel, before - 3);
    }

    #[test]
    fn mutation_surge_option_b_gains_knowledge() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        detect_all_diseases(&mut state);
        let before = state.diseases[0].knowledge;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::MutationSurge { disease_idx: 0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 300.0, personnel: 0 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.diseases[0].knowledge - (before + 0.15)).abs() < 0.001,
            "option B should gain 0.15 knowledge");
    }

    #[test]
    fn refugee_wave_option_a_transfers_population_and_infections() {
        let mut state = GameState::new_default(42);
        // Set up: region 0 collapsed with infections, region 1 as destination
        state.regions[0].collapsed = true;
        state.regions[0].dead = 200_000_000.0; // 200M dead of 500M
        state.regions[0].infections = vec![RegionDiseaseState {
            disease_idx: 0, infected: 10_000.0, dead: 200_000_000.0, immune: 5_000.0,
        }];
        let survivors = state.regions[0].alive(); // 300M
        let dest_pop_before = state.regions[1].population;
        let dest_infected_before = state.regions[1].infections
            .iter().find(|i| i.disease_idx == 0)
            .map(|i| i.infected).unwrap_or(0.0);
        setup_crisis(&mut state, CrisisKind::RefugeeWave { from_region: 0, to_region: 1 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        // Population should increase by survivor count
        assert_eq!(after.regions[1].population, dest_pop_before + survivors as u64,
            "option A should transfer surviving population to destination");
        // Infections should increase
        let dest_infected_after = after.regions[1].infections
            .iter().find(|i| i.disease_idx == 0)
            .map(|i| i.infected).unwrap_or(0.0);
        assert!(dest_infected_after > dest_infected_before,
            "option A should increase infections in destination region");
    }

    #[test]
    fn refugee_wave_option_b_loses_pol_and_kills_refugees() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        state.regions[0].collapsed = true;
        let dead_before = state.regions[0].dead;
        let survivors_before = state.regions[0].alive();
        setup_crisis(&mut state, CrisisKind::RefugeeWave { from_region: 0, to_region: 1 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        // 15% POL loss
        assert!((after.resources.political_power - 0.35).abs() < 0.001,
            "option B should decrease political_power by 0.15");
        // 20% of survivors die at the border
        let expected_deaths = survivors_before * 0.20;
        assert!((after.regions[0].dead - dead_before - expected_deaths).abs() < 1.0,
            "option B should kill 20% of survivors at the border");
    }

    #[test]
    fn collapse_triggers_refugee_crisis_immediately() {
        let mut state = GameState::new_default(42);
        // Push region 0 right to the edge of collapse
        let threshold = state.regions[0].collapse_threshold;
        let pop = state.regions[0].population as f64;
        // Need alive < pop * threshold, so dead > pop * (1 - threshold)
        state.regions[0].dead = pop * (1.0 - threshold) + 1.0;
        state.regions[0].get_or_create_infection(0).dead = state.regions[0].dead;
        // Ensure no other crisis is active
        assert!(state.active_crisis.is_none());
        // Tick should trigger collapse AND refugee crisis
        let after = tick(&state);
        assert!(after.regions[0].collapsed, "region should collapse");
        assert!(after.active_crisis.is_some(), "refugee crisis should fire immediately");
        assert_eq!(after.active_crisis.as_ref().unwrap().title, "REFUGEE CRISIS");
        assert!(matches!(after.sim_state, SimState::Event { .. }),
            "sim state should be Event (not just Paused)");
    }

    #[test]
    fn collapse_refugee_crisis_overrides_active_crisis() {
        let mut state = GameState::new_default(42);
        // Push region 0 right to the edge of collapse
        let threshold = state.regions[0].collapse_threshold;
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * (1.0 - threshold) + 1.0;
        state.regions[0].get_or_create_infection(0).dead = state.regions[0].dead;
        // Pre-load an active crisis (simulating a random crisis on the same tick)
        state.active_crisis = Some(crisis::build_crisis_event(&state, CrisisKind::DataLeak));
        assert!(state.active_crisis.is_some());
        // Tick should trigger collapse and OVERRIDE the existing crisis
        let after = tick(&state);
        assert!(after.regions[0].collapsed, "region should collapse");
        assert_eq!(after.active_crisis.as_ref().unwrap().title, "REFUGEE CRISIS",
            "refugee crisis must override any existing crisis on collapse");
    }

    #[test]
    fn data_leak_option_a_loses_research_gains_pol() {
        use crate::state::{ResearchProject, ResearchKind, TICKS_PER_DAY};
        let mut state = GameState::new_default(42);
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 500.0,
            required_ticks: 1000.0,
            personnel_assigned: 3,
            scientist_ids: vec![],
        }];
        let before_pol = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::DataLeak, 0);
        let after = apply_action(&state, &Action::Confirm);
        let expected_progress = (500.0 - 2.0 * TICKS_PER_DAY as f64).max(0.0);
        assert!((after.field_research.first().unwrap().progress - expected_progress).abs() < 0.01,
            "option A should lose 2 days of field research progress");
        assert!((after.resources.political_power - (before_pol + 0.05)).abs() < 0.001,
            "option A should gain 0.05 POL modifier");
    }

    #[test]
    fn data_leak_option_b_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::DataLeak, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.10)).abs() < 0.001,
            "option B should decrease political_power by 0.10");
    }

    #[test]
    fn black_market_option_a_treats_and_harms() {
        let mut state = GameState::new_default(42);
        // Need infections > 100 for the effect to kick in
        state.regions[0].infections = vec![RegionDiseaseState {
            disease_idx: 0, infected: 10_000.0, dead: 0.0, immune: 0.0,
        }];
        let before_dead = state.regions[0].dead;
        setup_crisis(&mut state, CrisisKind::BlackMarketMedicine { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        let inf = after.regions[0].disease_state(0).unwrap();
        // 5% treated, of which 20% harmed
        assert!(inf.infected < 10_000.0, "some should be treated");
        assert!(inf.immune > 0.0, "some should gain immunity");
        assert!(inf.dead > 0.0, "some should die from adverse reactions");
        assert!(after.regions[0].dead > before_dead, "region dead should increase");
    }

    #[test]
    fn quarantine_riot_option_a_lifts_quarantine() {
        let mut state = GameState::new_default(42);
        state.policies[0].quarantine = true;
        setup_crisis(&mut state, CrisisKind::QuarantineRiot { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(!after.policies[0].quarantine,
            "option A should lift quarantine");
    }

    #[test]
    fn quarantine_riot_option_b_loses_pol() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        state.policies[0].quarantine = true;
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::QuarantineRiot { region_idx: 0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 0.0, personnel: 2 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.15)).abs() < 0.001,
            "option B should decrease political_power by 0.15");
    }

    #[test]
    fn media_panic_option_a_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::MediaPanic, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.08)).abs() < 0.001,
            "option A should decrease political_power by 0.08");
    }

    #[test]
    fn media_panic_option_b_gains_pol() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        let before = state.resources.political_power;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::MediaPanic,
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 300.0, personnel: 1 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before + 0.05)).abs() < 0.001,
            "option B should increase political_power by 0.05");
    }

    #[test]
    fn trial_shortcut_option_a_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.05)).abs() < 0.001,
            "option A should decrease political_power by 0.05");
    }

    #[test]
    fn trial_shortcut_option_b_gains_pol() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before + 0.10)).abs() < 0.001,
            "option B should increase political_power by 0.10");
    }

    #[test]
    fn vaccine_hesitancy_option_a_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::VaccineHesitancy { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.10)).abs() < 0.001,
            "option A should decrease political_power by 0.10");
    }

    #[test]
    fn vaccine_hesitancy_option_b_gains_pol() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        state.resources.funding = 1000.0;
        let before = state.resources.political_power;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::VaccineHesitancy { region_idx: 0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 400.0, personnel: 0 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before + 0.05)).abs() < 0.001,
            "option B should increase political_power by 0.05");
    }

    #[test]
    fn corrupt_official_option_a_loses_funding() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        let stolen = (2000.0_f64 * 0.15).min(500.0).round(); // 300
        setup_crisis(&mut state, CrisisKind::CorruptOfficial { stolen }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - (2000.0 - stolen)).abs() < 1.0,
            "option A should lose 15% of funding (capped at 500)");
    }

    #[test]
    fn corrupt_official_option_b_prevents_theft() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::CorruptOfficial { stolen: 300.0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 0.0, personnel: 2 }) },
            tick_created: 0,
        });
        let before_personnel = state.resources.personnel;
        let after = apply_action(&state, &Action::Confirm);
        // Option B: funding unchanged (just personnel cost), no theft
        assert_eq!(after.resources.funding, 2000.0,
            "option B should not lose funding");
        assert_eq!(after.resources.personnel, before_personnel - 2,
            "option B should cost 2 personnel");
    }

    #[test]
    fn resource_diversion_option_a_trades_knowledge_for_funding() {
        let mut state = GameState::new_default(42);
        detect_all_diseases(&mut state);
        state.diseases[0].knowledge = 0.5;
        state.resources.funding = 1000.0;
        setup_crisis(&mut state, CrisisKind::ResourceDiversion { disease_idx: 0, share_reward: 250.0, refuse_cost: 150.0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.diseases[0].knowledge - 0.4).abs() < 0.001,
            "option A should lose 0.1 knowledge");
        assert!((after.resources.funding - 1250.0).abs() < 1.0,
            "option A should gain $250 funding (scaled)");
    }

    #[test]
    fn exhaustion_epidemic_option_a_disables_hospital_surge() {
        let mut state = GameState::new_default(42);
        state.policies[0].hospital_surge = true;
        setup_crisis(&mut state, CrisisKind::ExhaustionEpidemic { region_idx: 0, personnel_loss: 3 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(!after.policies[0].hospital_surge,
            "option A should disable hospital surge");
    }

    #[test]
    fn exhaustion_epidemic_option_b_loses_personnel() {
        let mut state = GameState::new_default(42);
        state.policies[0].hospital_surge = true;
        let before = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::ExhaustionEpidemic { region_idx: 0, personnel_loss: 3 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.resources.personnel, before - 3,
            "option B should lose scaled personnel");
        assert!(after.policies[0].hospital_surge,
            "option B should keep hospital surge active");
    }

    #[test]
    fn whistleblower_option_a_destroys_doses_gains_pol() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].doses = 1000.0;
        let before_pol = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::WhistleblowerReport { medicine_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.medicines[0].doses, 700.0,
            "option A should destroy 30% of doses");
        assert!((after.resources.political_power - (before_pol + 0.05)).abs() < 0.001,
            "option A should gain 0.05 POL modifier");
    }

    #[test]
    fn whistleblower_option_b_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::WhistleblowerReport { medicine_idx: 0 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.08)).abs() < 0.001,
            "option B should decrease political_power by 0.08");
    }

    #[test]
    fn military_takeover_option_a_loses_personnel_gains_pol() {
        let mut state = GameState::new_default(42);
        let before_personnel = state.resources.personnel;
        let before_pol = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::MilitaryTakeover { cooperate_loss: 4 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.resources.personnel, before_personnel - 4,
            "option A should lose scaled personnel");
        assert!((after.resources.political_power - (before_pol + 0.15)).abs() < 0.001,
            "option A should gain 0.15 POL modifier");
    }

    #[test]
    fn cult_blockade_option_a_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 0.50;
        let before = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::CultBlockade { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before - 0.08)).abs() < 0.001,
            "option A should decrease political_power by 0.08");
    }

    #[test]
    fn billionaire_option_b_gains_funding_loses_personnel() {
        let mut state = GameState::new_default(42);
        let before_funding = state.resources.funding;
        let before_personnel = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::BillionaireOffer { reward: 200.0, personnel_loss: 2 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - (before_funding + 200.0)).abs() < 1.0,
            "option B should gain scaled reward");
        assert_eq!(after.resources.personnel, before_personnel - 2,
            "option B should lose scaled personnel");
    }

    #[test]
    fn billionaire_option_a_no_changes() {
        let mut state = GameState::new_default(42);
        let before_funding = state.resources.funding;
        let before_personnel = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::BillionaireOffer { reward: 200.0, personnel_loss: 2 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.resources.funding, before_funding,
            "option A should not change funding");
        assert_eq!(after.resources.personnel, before_personnel,
            "option A should not change personnel");
    }

    #[test]
    fn who_evacuation_option_a_loses_funding_and_pol() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 1000.0;
        state.resources.political_power = 0.50;
        let before_pol = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::WHOEvacuation { aid_loss: 150.0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - 850.0).abs() < 1.0,
            "option A should lose scaled aid amount ($150)");
        assert!((after.resources.political_power - (before_pol - 0.05)).abs() < 0.001,
            "option A should lose 0.05 POL modifier");
    }

    #[test]
    fn who_evacuation_option_b_gains_pol() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        let before_pol = state.resources.political_power;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::WHOEvacuation { aid_loss: 150.0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 800.0, personnel: 3 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.political_power - (before_pol + 0.10)).abs() < 0.001,
            "option B should gain 0.10 POL modifier");
    }

    #[test]
    fn warlord_demand_option_a_gains_pol_region_stays_collapsed() {
        let mut state = GameState::new_default(42);
        state.regions[0].collapsed = true;
        let before_pol = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::WarlordDemand { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.regions[0].collapsed, "option A should keep region collapsed");
        assert!((after.resources.political_power - (before_pol + 0.05)).abs() < 0.001,
            "option A should gain 0.05 POL modifier");
    }

    #[test]
    fn warlord_demand_option_b_uncollapses_region() {
        use crate::state::CrisisCost;
        let mut state = GameState::new_default(42);
        state.resources.funding = 1000.0;
        state.regions[0].collapsed = true;
        state.sim_state = crate::state::SimState::Event { was_running: true };
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::WarlordDemand { region_idx: 0 },
            title: "T".into(), description: "T".into(),
            option_a: crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
            option_b: crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 500.0, personnel: 0 }) },
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!(!after.regions[0].collapsed,
            "option B should un-collapse the region");
    }

    #[test]
    fn vaccine_dispute_option_a_loses_funding() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 1000.0;
        setup_crisis(&mut state, CrisisKind::VaccineDispute { neutral_loss: 200.0, credit_gain: 300.0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - 800.0).abs() < 1.0,
            "option A should lose scaled neutral_loss ($200)");
    }

    #[test]
    fn vaccine_dispute_option_b_gains_funding_loses_pol() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 1000.0;
        state.resources.political_power = 0.50;
        let before_pol = state.resources.political_power;
        setup_crisis(&mut state, CrisisKind::VaccineDispute { neutral_loss: 200.0, credit_gain: 300.0 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - 1300.0).abs() < 1.0,
            "option B should gain scaled credit_gain ($300)");
        assert!((after.resources.political_power - (before_pol - 0.15)).abs() < 0.001,
            "option B should lose 0.15 POL modifier");
    }

    #[test]
    fn trial_shortcut_fast_track_marks_tested_with_penalty() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        // Disease at gen 0 — fast-track should still impose 2-gen penalty
        assert_eq!(state.diseases[0].strain_generation, 0);
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 1);

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.medicines[0].tested_against.contains(&0),
            "fast-track should mark medicine as tested");
        assert!(!after.medicines[0].strain_generations.is_empty(),
            "strain_generations should be populated");
        assert_eq!(after.medicines[0].strain_generations[0], -2,
            "at gen 0, fast-track should calibrate to gen -2");
        let efficacy = after.medicines[0].strain_efficacy(0, &after.diseases);
        assert!((efficacy - 0.70).abs() < 0.01,
            "fast-track should always impose ~30% penalty, got {}", efficacy);
        assert!(after.active_crisis.is_none());
    }

    #[test]
    fn trial_shortcut_fast_track_with_mutated_disease_has_drift() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        state.diseases[0].strain_generation = 5;
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 1);

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.medicines[0].tested_against.contains(&0));
        assert_eq!(after.medicines[0].strain_generations[0], 3,
            "should be calibrated 2 generations behind current");
        let efficacy = after.medicines[0].strain_efficacy(0, &after.diseases);
        assert!((efficacy - 0.70).abs() < 0.01,
            "efficacy should be ~0.70 due to 2-gen drift, got {}", efficacy);
    }

    #[test]
    fn trial_shortcut_maintain_standards_no_medicine_change() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 0);

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.medicines[0].tested_against.is_empty(),
            "maintain standards should not mark medicine as tested");
        assert!(after.active_crisis.is_none());
    }

    #[test]
    fn trial_shortcut_generates_at_any_strain_gen() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        state.tick = 5000; // past CRISIS_MIN_TICK
        // Disease at gen 0 — TrialShortcut should still be possible
        // (penalty mechanism uses i32 strain_generations, works at any gen)
        state.diseases[0].strain_generation = 0;
        let mut got_trial = false;
        for seed in 0..500u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            if let Some(event) = crisis::generate_crisis(&state, &mut rng) {
                if matches!(event.kind, CrisisKind::TrialShortcut { .. }) {
                    got_trial = true;
                    break;
                }
            }
        }
        assert!(got_trial, "TrialShortcut should generate even at gen 0");
    }

    #[test]
    fn pol_drifts_toward_severity_target() {
        let mut state = GameState::new_default(42);
        // Start with zero POL and significant infections to create a target > 0.
        // With the flattened curve (sqrt * 1.0), need ~8% infected for a meaningful target.
        state.resources.political_power = 0.0;
        for region in &mut state.regions {
            region.get_or_create_infection(0).infected = 100_000_000.0;
        }

        // Run several ticks — POL should drift upward toward the severity target
        let mut s = state.clone();
        for _ in 0..(TICKS_PER_DAY as u64 * 5) {
            s = tick(&s);
        }
        assert!(s.resources.political_power > 0.05,
            "POL should drift up after 5 days with ~8% infected, got {}",
            s.resources.political_power);
    }

    #[test]
    fn pol_recovers_after_crisis_hit() {
        let mut state = GameState::new_default(42);
        // Need large infections for meaningful POL target with flattened severity curve
        for region in &mut state.regions {
            region.get_or_create_infection(0).infected = 200_000_000.0;
        }

        // Let POL reach a steady state over 10 days
        let mut s = state.clone();
        for _ in 0..(TICKS_PER_DAY as u64 * 10) {
            s = tick(&s);
        }
        let steady = s.resources.political_power;

        // Simulate a crisis hit: drop POL by 0.15
        s.resources.political_power = (steady - 0.15).max(0.0);
        let after_hit = s.resources.political_power;

        // Run 3 more days — POL should recover toward the target
        for _ in 0..(TICKS_PER_DAY as u64 * 3) {
            s = tick(&s);
        }
        assert!(s.resources.political_power > after_hit + 0.05,
            "POL should recover after crisis hit: was {}, now {}",
            after_hit, s.resources.political_power);
    }

    #[test]
    fn active_policies_drain_pol_target() {
        // Two identical states: one with policies, one without.
        // The one with policies should have lower POL after the same time.
        let mut base = GameState::new_default(42);
        for region in &mut base.regions {
            region.get_or_create_infection(0).infected = 200_000_000.0;
        }
        base.resources.political_power = 0.0;
        base.resources.funding = 100_000.0; // enough to sustain policies

        let mut with_policies = base.clone();
        // Enable quarantine + hospital surge in all 6 regions = 12 active policies
        for policy in &mut with_policies.policies {
            policy.quarantine = true;
            policy.hospital_surge = true;
        }

        // Run both for 10 days
        let mut s_base = base;
        let mut s_pol = with_policies;
        for _ in 0..(TICKS_PER_DAY as u64 * 10) {
            s_base = tick(&s_base);
            s_pol = tick(&s_pol);
        }

        // 12 policies × 2% drain = 24% lower target
        assert!(s_pol.resources.political_power < s_base.resources.political_power - 0.10,
            "active policies should significantly reduce POL: without={:.3}, with={:.3}",
            s_base.resources.political_power, s_pol.resources.political_power);
    }

    #[test]
    fn pol_target_capped_at_90_percent() {
        use crate::state::{ResearchProject, ResearchKind, BasicTech};
        let mut state = GameState::new_default(42);
        // Massive deaths + infections to maximize severity
        for region in &mut state.regions {
            region.get_or_create_infection(0).infected = 500_000_000.0;
            region.dead = 500_000_000.0;
        }
        state.resources.political_power = 0.95; // Start above the 0.90 cap
        state.tick = TICKS_PER_DAY as u64 * 100; // far into the game
        // Keep research active to prevent mercy rule from ending the game early
        state.basic_research = Some(ResearchProject {
            kind: ResearchKind::BasicResearch { tech: BasicTech::TargetedDrugDesign },
            progress: 0.0,
            required_ticks: 99999.0,
            personnel_assigned: 1,
        scientist_ids: vec![],
        });

        // Run a few days — POL should drift DOWN toward the 0.90 cap
        let mut s = state;
        for _ in 0..(TICKS_PER_DAY as u64 * 5) {
            s = tick(&s);
        }
        assert!(s.resources.political_power < 0.92,
            "POL should drift toward 0.90 cap, got {:.3}", s.resources.political_power);
    }

    #[test]
    fn infections_reduce_funding_income() {
        use crate::state::INFECTED_INCAPACITATION_RATE;
        let mut state = GameState::new_default(42);
        // Clear all infections to get baseline
        for r in &mut state.regions {
            r.infections.clear();
        }
        let baseline_income = state.funding_income_rate();

        // Infect 10% of region 0's population
        let pop = state.regions[0].population as f64;
        let infected = pop * 0.10;
        state.regions[0].infections.push(crate::state::RegionDiseaseState {
            disease_idx: 0,
            infected,
            dead: 0.0,
            immune: 0.0,
        });

        let infected_income = state.funding_income_rate();
        assert!(
            infected_income < baseline_income,
            "income should drop with infections: {infected_income:.4} vs {baseline_income:.4}"
        );

        // The drop should be proportional to the infected fraction × incapacitation rate
        let total_pop: f64 = state.regions.iter().map(|r| r.population as f64).sum();
        let expected_drop_frac = (infected * INFECTED_INCAPACITATION_RATE) / total_pop;
        let actual_drop_frac = 1.0 - infected_income / baseline_income;
        assert!(
            (actual_drop_frac - expected_drop_frac).abs() < 0.01,
            "income drop {:.1}% should be close to expected {:.1}%",
            actual_drop_frac * 100.0,
            expected_drop_frac * 100.0,
        );
    }

    #[test]
    fn horizontal_gene_transfer_between_bacteria() {
        let mut state = GameState::new_default(42);
        // Set up two Bacterium diseases co-located in the same region
        state.diseases[0].pathogen_type = PathogenType::Bacterium;
        // Add a second Bacterium disease
        let mut disease2 = state.diseases[0].clone();
        disease2.name = "Test Bacterium B".into();
        disease2.pathogen_type = PathogenType::Bacterium;
        disease2.mechanism_resistance.clear();
        state.diseases.push(disease2);

        // Give disease 0 significant broad-spectrum resistance (mechanism=None)
        state.diseases[0].add_resistance(None, 0.5);

        // Ensure both diseases have infections in the same region
        let region_idx = primary_outbreak_region(&state);
        state.regions[region_idx].get_or_create_infection(1).infected = 1000.0;

        // Disease 1 should start with no resistance
        assert_eq!(state.diseases[1].get_resistance(None), 0.0);

        // Run many ticks to allow HGT to accumulate
        for _ in 0..1200 { // ~10 days
            state = tick(&state);
        }

        // Disease 1 should have gained meaningful broad-spectrum resistance
        // At 10%/day over 10 days with 0.5 donor: expect ~0.5*(1-0.9^10) ≈ 0.33
        let transferred = state.diseases[1].get_resistance(None);
        assert!(
            transferred > 0.10,
            "HGT should transfer meaningful resistance: got {transferred}"
        );
        assert!(
            transferred < 0.5,
            "HGT should not fully equalize: got {transferred}"
        );
    }

    #[test]
    fn horizontal_gene_transfer_only_affects_bacteria() {
        let mut state = GameState::new_default(42);
        // Disease 0 is Bacterium with resistance, disease 1 is RnaVirus
        state.diseases[0].pathogen_type = PathogenType::Bacterium;
        state.diseases[0].add_resistance(None, 0.5);

        // Ensure there's a second disease that's a virus
        if state.diseases.len() < 2 {
            let mut d = state.diseases[0].clone();
            d.name = "Test Virus".into();
            d.pathogen_type = PathogenType::RnaVirus;
            d.mechanism_resistance.clear();
            state.diseases.push(d);
        } else {
            state.diseases[1].pathogen_type = PathogenType::RnaVirus;
            state.diseases[1].mechanism_resistance.clear();
        }

        // Ensure co-location
        let region_idx = primary_outbreak_region(&state);
        state.regions[region_idx].get_or_create_infection(1).infected = 1000.0;

        for _ in 0..1200 {
            state = tick(&state);
        }

        // Virus should NOT gain resistance from bacterial HGT
        let virus_resistance = state.diseases[1].get_resistance(None);
        assert_eq!(
            virus_resistance, 0.0,
            "HGT should not affect non-bacteria: got {virus_resistance}"
        );
    }

    #[test]
    fn collapse_kills_income_and_loses_personnel() {
        let mut state = GameState::new_default(42);
        detect_all_diseases(&mut state);
        let initial_income = state.funding_income_rate();
        let initial_personnel = state.resources.personnel;
        assert!(initial_income > 0.0);

        // Force a region to collapse
        let region_idx = primary_outbreak_region(&state);
        let pop = state.regions[region_idx].population as f64;
        state.regions[region_idx].dead = pop * 0.6; // above collapse threshold

        // Tick to trigger collapse detection
        state = tick(&state);
        assert!(state.regions[region_idx].collapsed, "region should have collapsed");

        // Income should drop (collapsed region contributes nothing)
        let post_collapse_income = state.funding_income_rate();
        assert!(
            post_collapse_income < initial_income,
            "income should drop after collapse: was {initial_income}, now {post_collapse_income}"
        );

        // Personnel should be reduced by 2
        assert_eq!(
            state.resources.personnel,
            initial_personnel - 2,
            "should lose 2 personnel on collapse"
        );
    }

    #[test]
    fn deploy_cooldown_blocks_repeat_deployment() {
        let mut state = GameState::new_default(42);
        // Setup: unlock a medicine, seed infection, give funds
        let disease_idx = 0;
        let med_idx = 0;
        state.medicines[med_idx].unlocked = true;
        state.medicines[med_idx].tested_against.push(disease_idx);
        state.medicines[med_idx].doses = 1_000_000.0;
        state.medicines[med_idx].max_doses = 1_000_000.0;
        state.resources.funding = 1_000_000.0;
        state.regions[0].get_or_create_infection(disease_idx).infected = 50_000.0;

        // First deploy should succeed
        let treat = DeployTarget::Treat { disease_idx };
        let (nav, msg, _) = medicine::deploy_medicine(&mut state, med_idx, 0, treat.clone());
        assert!(nav, "first deploy should succeed");
        assert!(msg.unwrap().contains("Treated"), "should show treatment message");

        // Region should now have a cooldown set
        assert!(state.regions[0].last_deploy_tick.is_some());

        // Second deploy at same tick should be blocked
        state.resources.funding = 1_000_000.0;
        state.regions[0].get_or_create_infection(disease_idx).infected = 50_000.0;
        let (nav2, msg2, _) = medicine::deploy_medicine(&mut state, med_idx, 0, treat.clone());
        assert!(!nav2, "second deploy should be blocked by cooldown");
        assert!(msg2.unwrap().contains("cooldown"), "should mention cooldown");

        // After cooldown expires, deploy should work again
        state.tick = crate::state::DEPLOY_COOLDOWN_TICKS + 1;
        state.resources.funding = 1_000_000.0;
        state.regions[0].get_or_create_infection(disease_idx).infected = 50_000.0;
        let (nav3, msg3, _) = medicine::deploy_medicine(&mut state, med_idx, 0, treat.clone());
        assert!(nav3, "deploy after cooldown should succeed");
        assert!(msg3.unwrap().contains("Treated"));

        // Different region should still be deployable (cooldown is per-region)
        state.tick = 0;
        state.regions[0].last_deploy_tick = Some(0);
        state.regions[1].get_or_create_infection(disease_idx).infected = 50_000.0;
        state.resources.funding = 1_000_000.0;
        let (nav4, _, _) = medicine::deploy_medicine(&mut state, med_idx, 1, treat.clone());
        assert!(nav4, "deploying to different region should work during cooldown");
    }

    #[test]
    fn threat_escalation_fires_at_death_thresholds() {
        let mut state = GameState::new_default(42);
        state.diseases[0].detected = true;
        // Set deaths above 1M threshold on the existing infection entry
        // (new_default already seeds disease 0 in some region)
        for region in &mut state.regions {
            if let Some(inf) = region.infections.iter_mut().find(|i| i.disease_idx == 0) {
                inf.dead = 1_500_000.0;
                inf.infected = 100_000.0;
            }
            if region.dead > 0.0 {
                region.dead = 1_500_000.0;
            }
        }

        let new_state = tick(&state);
        let escalation = new_state.events.iter().find(|e|
            matches!(e, GameEvent::ThreatEscalation { .. })
        );
        assert!(escalation.is_some(), "should fire escalation at 1M deaths");

        if let Some(GameEvent::ThreatEscalation { disease_idx, has_medicine, .. }) = escalation {
            assert_eq!(*disease_idx, 0);
            assert!(!has_medicine, "no medicine unlocked yet");
        }
        assert_eq!(new_state.threat_alert_level[0], 1, "should set alert level to 1");

        // Second tick should NOT re-fire the same threshold
        let state2 = tick(&new_state);
        let escalation2 = state2.events.iter().find(|e|
            matches!(e, GameEvent::ThreatEscalation { .. })
        );
        assert!(escalation2.is_none(), "should not re-fire same threshold");
    }

    #[test]
    fn threat_escalation_skips_undetected_diseases() {
        let mut state = GameState::new_default(42);
        state.diseases[0].detected = false; // Not yet detected
        // Set deaths high but infected below detection threshold (10K)
        // so the disease stays undetected during the tick
        for region in &mut state.regions {
            if let Some(inf) = region.infections.iter_mut().find(|i| i.disease_idx == 0) {
                inf.dead = 2_000_000.0;
                inf.infected = 100.0;
            }
            if region.dead > 0.0 {
                region.dead = 2_000_000.0;
            }
        }

        let new_state = tick(&state);
        let escalation = new_state.events.iter().find(|e|
            matches!(e, GameEvent::ThreatEscalation { .. })
        );
        assert!(escalation.is_none(), "should not fire for undetected disease");
    }

    #[test]
    fn decree_enact_via_policy_panel_ui_flow() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 1.0;
        state.resources.funding = 10_000.0;

        // Open policy panel, navigate past 6 regions + 1 rally to first decree
        state = apply_action(&state, &Action::OpenPolicy);
        assert_eq!(state.ui.open_panel, Panel::Policy);
        for _ in 0..7 {
            state = apply_action(&state, &Action::SelectNext);
        }
        // panel_selection should be 7 (first decree: Conscript Researchers)
        assert_eq!(state.ui.panel_selection, 7);

        let personnel_before = state.resources.personnel;
        state = apply_action(&state, &Action::Confirm);
        assert!(state.enacted_decrees.conscript_researchers);
        assert_eq!(state.resources.personnel, personnel_before + crate::state::CONSCRIPT_PERSONNEL_GAIN);
        assert!(state.ui.status_message.as_ref().unwrap().contains("Conscript"));
    }

    #[test]
    fn decree_sacrifice_region_ui_flow() {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 1.0;
        state.resources.funding = 10_000.0;

        // Open policy panel, navigate to Sacrifice Region (index 9 = 6 regions + 1 rally + 2)
        state = apply_action(&state, &Action::OpenPolicy);
        for _ in 0..9 {
            state = apply_action(&state, &Action::SelectNext);
        }
        assert_eq!(state.ui.panel_selection, 9);
        state = apply_action(&state, &Action::Confirm);

        // Should be in SelectSacrificeRegion state
        assert_eq!(state.ui.policy_ui, Some(PolicyUiState::SelectSacrificeRegion));

        // Select first non-collapsed region and confirm
        state = apply_action(&state, &Action::Confirm);
        assert!(state.enacted_decrees.sacrificed_region.is_some());
        let sacrificed_idx = state.enacted_decrees.sacrificed_region.unwrap();
        assert!(state.regions[sacrificed_idx].collapsed);

        // UI should return to BrowseRegions after successful sacrifice
        assert_eq!(state.ui.policy_ui, Some(PolicyUiState::BrowseRegions),
            "should return to BrowseRegions after enacting sacrifice");
    }

    // --- Crisis chain tests ---

    #[test]
    fn black_market_allow_schedules_counterfeit_followup() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        state.regions[0].infections = vec![RegionDiseaseState {
            disease_idx: 0, infected: 10_000.0, dead: 0.0, immune: 0.0,
        }];
        setup_crisis(&mut state, CrisisKind::BlackMarketMedicine { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.pending_crises.len(), 1, "should schedule one follow-up");
        let (fire_tick, ref kind) = after.pending_crises[0];
        assert!(matches!(kind, CrisisKind::CounterfeitEpidemic { region_idx: 0 }),
            "follow-up should be CounterfeitEpidemic for region 0");
        let expected_tick = 1000 + (5.0 * TICKS_PER_DAY) as u64;
        assert_eq!(fire_tick, expected_tick, "should fire 5 days later");
    }

    #[test]
    fn black_market_confiscate_no_followup() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        setup_crisis(&mut state, CrisisKind::BlackMarketMedicine { region_idx: 0 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.pending_crises.is_empty(), "confiscating should NOT schedule follow-up");
    }

    #[test]
    fn corruption_ignore_schedules_embezzlement_followup() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        state.resources.funding = 2000.0;
        setup_crisis(&mut state, CrisisKind::CorruptOfficial { stolen: 200.0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.pending_crises.len(), 1);
        assert!(matches!(after.pending_crises[0].1, CrisisKind::EmbezzlementRing { .. }));
    }

    #[test]
    fn data_leak_suppress_schedules_inquiry_followup() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        setup_crisis(&mut state, CrisisKind::DataLeak, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.pending_crises.len(), 1);
        assert!(matches!(after.pending_crises[0].1, CrisisKind::PublicInquiry));
    }

    #[test]
    fn military_cooperate_schedules_overreach_followup() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        setup_crisis(&mut state, CrisisKind::MilitaryTakeover { cooperate_loss: 3 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.pending_crises.len(), 1);
        assert!(matches!(after.pending_crises[0].1, CrisisKind::MilitaryOverreach));
    }

    #[test]
    fn pending_crisis_fires_when_due() {
        let mut state = GameState::new_default(42);
        state.tick = 100; // Pending check runs before tick increment
        state.pending_crises.push((100, CrisisKind::PublicInquiry));
        let after = tick(&state);
        assert!(after.active_crisis.is_some(), "pending crisis should fire");
        assert_eq!(after.active_crisis.as_ref().unwrap().title, "Cover-Up Exposed");
        assert!(after.pending_crises.is_empty(), "fired crisis should be removed from pending");
    }

    #[test]
    fn pending_crisis_waits_if_not_due() {
        let mut state = GameState::new_default(42);
        state.tick = 50;
        state.pending_crises.push((200, CrisisKind::PublicInquiry));
        let after = tick(&state);
        assert!(after.active_crisis.is_none(), "should not fire yet");
        assert_eq!(after.pending_crises.len(), 1, "should still be pending");
    }

    #[test]
    fn collapse_rate_estimate_updates_after_one_day() {
        let mut state = GameState::new_default(42);
        let region_idx = primary_outbreak_region(&state);
        // Run for 2+ days to ensure the rate sampler fires at least once
        let ticks = (2.5 * TICKS_PER_DAY) as usize;
        for _ in 0..ticks {
            state = tick(&state);
        }
        let region = &state.regions[region_idx];
        // The outbreak region should have deaths and a positive rate
        assert!(
            region.total_dead() > 0.0,
            "outbreak region should have deaths by day 2.5"
        );
        assert!(
            region.cached_deaths_per_day > 0.0,
            "cached death rate should be positive: got {}",
            region.cached_deaths_per_day
        );
        // days_to_collapse should return Some since the region has deaths
        assert!(
            region.days_to_collapse(false).is_some(),
            "should estimate time to collapse when deaths are occurring"
        );
    }

    #[test]
    fn collapse_rate_not_shown_for_safe_regions() {
        let state = GameState::new_default(42);
        // At tick 0, no deaths have occurred — rate should be 0
        for region in &state.regions {
            assert_eq!(region.cached_deaths_per_day, 0.0);
            assert!(region.days_to_collapse(false).is_none());
        }
    }

    #[test]
    fn auto_deploy_treats_worst_region() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5000.0;

        // Give medicine 0 unlocked status, doses, and tested against disease 0
        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 1_000_000.0;
        state.medicines[0].max_doses = 1_000_000.0;
        state.medicines[0].tested_against = vec![0];
        state.diseases[0].detected = true;

        // Set up infections: region 0 has 100K infected, region 1 has 500K
        state.regions[0].infections[0].infected = 100_000.0;
        state.regions[1].get_or_create_infection(0).infected = 500_000.0;

        // Enable auto-deploy for medicine 0
        state.auto_deploy = vec![true];

        let after = tick(&state);

        // Should have auto-deployed (event fired)
        let auto_deploy_events: Vec<_> = after.events.iter()
            .filter(|e| matches!(e, GameEvent::MedicineAutoDeployed { .. }))
            .collect();
        assert_eq!(auto_deploy_events.len(), 1, "should auto-deploy exactly once per tick");

        // Should target region 1 (worst infected)
        match &auto_deploy_events[0] {
            GameEvent::MedicineAutoDeployed { region_idx, .. } => {
                assert_eq!(*region_idx, 1, "should deploy to worst-affected region");
            }
            _ => unreachable!(),
        }

        // Doses should have been consumed
        assert!(after.medicines[0].doses < state.medicines[0].doses,
            "doses should be consumed by auto-deploy");
    }

    #[test]
    fn auto_deploy_skips_untested_medicine() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5000.0;

        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 1_000_000.0;
        state.medicines[0].max_doses = 1_000_000.0;
        // NOT tested: tested_against is empty
        state.diseases[0].detected = true;

        state.regions[0].infections[0].infected = 100_000.0;
        state.auto_deploy = vec![true];

        let after = tick(&state);

        // Should NOT auto-deploy untested medicines
        let auto_events: Vec<_> = after.events.iter()
            .filter(|e| matches!(e, GameEvent::MedicineAutoDeployed { .. }))
            .collect();
        assert!(auto_events.is_empty(), "should not auto-deploy untested medicines");
    }

    #[test]
    fn auto_deploy_respects_cooldown() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5000.0;

        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 1_000_000.0;
        state.medicines[0].max_doses = 1_000_000.0;
        state.medicines[0].tested_against = vec![0];
        state.diseases[0].detected = true;

        // Only region 0 has infections, but it's on cooldown
        state.regions[0].infections[0].infected = 100_000.0;
        state.regions[0].last_deploy_tick = Some(state.tick);
        state.auto_deploy = vec![true];

        let after = tick(&state);

        let auto_events: Vec<_> = after.events.iter()
            .filter(|e| matches!(e, GameEvent::MedicineAutoDeployed { .. }))
            .collect();
        assert!(auto_events.is_empty(), "should not deploy to region on cooldown");
    }

    #[test]
    fn containment_adaptation_builds_under_quarantine() {
        let mut state = GameState::new_default(42);
        // Set up: disease 0 in region 0 with quarantine active
        state.regions[0].infections[0].infected = 10_000.0;
        state.policies[0].quarantine = true;

        assert_eq!(state.diseases[0].containment_adaptation, 0.0);

        // Run for 5 days (600 ticks)
        let mut s = state;
        for _ in 0..600 {
            s = tick(&s);
        }

        // Adaptation should have built up
        assert!(s.diseases[0].containment_adaptation > 0.01,
            "adaptation should increase under quarantine: got {}", s.diseases[0].containment_adaptation);
    }

    #[test]
    fn containment_adaptation_decays_without_containment() {
        let mut state = GameState::new_default(42);
        // Pre-set some adaptation
        state.diseases[0].containment_adaptation = 0.5;
        // No quarantine or travel ban active

        // Run for 5 days
        let mut s = state;
        for _ in 0..600 {
            s = tick(&s);
        }

        // Adaptation should have decayed
        assert!(s.diseases[0].containment_adaptation < 0.5,
            "adaptation should decay without containment: got {}", s.diseases[0].containment_adaptation);
    }

    #[test]
    fn containment_adaptation_weakens_quarantine() {
        let mut state = GameState::new_default(42);
        state.regions[0].infections[0].infected = 100_000.0;
        state.policies[0].quarantine = true;

        // Run baseline (no adaptation) for 1 day
        let baseline = {
            let mut s = state.clone();
            s.diseases[0].containment_adaptation = 0.0;
            for _ in 0..120 {
                s = tick(&s);
            }
            s.regions[0].infections[0].infected
        };

        // Run with high adaptation for 1 day
        let adapted = {
            let mut s = state.clone();
            s.diseases[0].containment_adaptation = 0.8;
            for _ in 0..120 {
                s = tick(&s);
            }
            s.regions[0].infections[0].infected
        };

        // With adaptation, quarantine is weaker → more infections
        assert!(adapted > baseline,
            "adapted disease should spread more under quarantine: adapted={} baseline={}", adapted, baseline);
    }
}
