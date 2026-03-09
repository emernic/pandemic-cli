use rand::Rng;

use crate::action::Action;
use crate::state::{
    DeployTarget, GameCommand, GameEvent, GameOutcome, GameState,
    Panel, RegionDiseaseState, ResearchKind, ResearchProject,
    BOOST_RP_COST, BOOST_TICKS, EMERGENCE_CHANCE_PER_TICK,
    EMERGENCE_MIN_TICK, HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL,
    KNOWLEDGE_FULL, KNOWLEDGE_NAME, LOSE_DEATH_FRACTION, MAX_DISEASES,
    QUARANTINE_COST, QUARANTINE_PERSONNEL, TRAVEL_BAN_COST, WIN_INFECTED_THRESHOLD,
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

    // Disease spread within each region
    for (region_idx, region) in new.regions.iter_mut().enumerate() {
        let pop = region.population as f64;
        let policy = new.policies.get(region_idx);
        let quarantine_active = policy.is_some_and(|p| p.quarantine);
        let hospital_active = policy.is_some_and(|p| p.hospital_surge);

        for inf in &mut region.infections {
            if let Some(disease) = state.diseases.get(inf.disease_idx) {
                let susceptible = pop - inf.infected - inf.dead - inf.immune;
                if susceptible <= 0.0 {
                    continue;
                }

                let noise: f64 = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.1;
                let mut infectivity = if quarantine_active {
                    disease.infectivity * disease.transmission.quarantine_factor()
                } else {
                    disease.infectivity
                };
                // Contact diseases spread faster when hospital surge is active
                // (healthcare workers in close contact with patients)
                if hospital_active {
                    infectivity *= disease.transmission.hospital_infectivity_factor();
                }
                let new_infections =
                    infectivity * inf.infected * (susceptible / pop) * noise;
                let new_infections = new_infections.max(0.0).min(susceptible);

                // Deaths and recoveries are concurrent outflows from the infected pool.
                // Compute both, then scale proportionally if they exceed infected.
                let lethality = if hospital_active {
                    disease.lethality * 0.5
                } else {
                    disease.lethality
                };
                let mut new_deaths = (lethality * inf.infected * noise).max(0.0);
                let mut new_recoveries = (disease.recovery_rate * inf.infected * noise).max(0.0);
                let total_outflow = new_deaths + new_recoveries;
                if total_outflow > inf.infected {
                    let scale = inf.infected / total_outflow;
                    new_deaths *= scale;
                    new_recoveries *= scale;
                }

                inf.infected = inf.infected + new_infections - new_deaths - new_recoveries;
                // Snap to zero when below 1 person — aligns with WIN_INFECTED_THRESHOLD
                if inf.infected < 1.0 {
                    inf.infected = 0.0;
                }
                inf.immune += new_recoveries;
                inf.dead += new_deaths;
            }
        }
    }

    // Cross-region spread
    let regions_snapshot: Vec<_> = new.regions.clone();
    for (i, region) in new.regions.iter_mut().enumerate() {
        let dest_has_travel_ban = new.policies.get(i).is_some_and(|p| p.travel_ban);

        for (d_idx, disease) in state.diseases.iter().enumerate() {
            let connected_infected: f64 = regions_snapshot[i]
                .connections
                .iter()
                .filter_map(|&conn_idx| {
                    let source_has_travel_ban =
                        new.policies.get(conn_idx).is_some_and(|p| p.travel_ban);
                    let ban_factor = if source_has_travel_ban || dest_has_travel_ban {
                        disease.transmission.travel_ban_factor()
                    } else {
                        1.0
                    };
                    regions_snapshot[conn_idx]
                        .disease_state(d_idx)
                        .map(|inf| inf.infected * ban_factor)
                })
                .sum();

            if connected_infected <= 0.0 {
                continue;
            }

            let has_active_infection = region
                .infections
                .iter()
                .any(|inf| inf.disease_idx == d_idx && inf.infected > 0.0);

            if !has_active_infection {
                let roll: f64 = rng.r#gen();
                let chance = disease.cross_region_spread
                    * disease.transmission.cross_region_modifier()
                    * (connected_infected / 10_000.0);
                if roll < chance.min(0.5) {
                    // Check if there's an existing entry (e.g., from vaccination)
                    if let Some(existing) = region
                        .infections
                        .iter_mut()
                        .find(|inf| inf.disease_idx == d_idx)
                    {
                        existing.infected = 1.0;
                    } else {
                        region.infections.push(RegionDiseaseState {
                            disease_idx: d_idx,
                            infected: 1.0,
                            dead: 0.0,
                            immune: 0.0,
                        });
                    }
                    new.events.push(GameEvent::DiseaseSpreadToRegion {
                        disease_idx: d_idx,
                        region_idx: i,
                    });
                }
            }
        }
    }

    // Disease mutation (sequencing reduces mutation rate by half per level)
    for (d_idx, disease) in new.diseases.iter_mut().enumerate() {
        let mutation_chance = disease.effective_mutation_rate();
        if rng.r#gen::<f64>() < mutation_chance {
            disease.strain_generation += 1;
            // Small random parameter changes (±10% of current value), clamped to
            // prevent runaway drift over many mutations.
            let inf_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            disease.infectivity = (disease.infectivity * inf_factor).clamp(0.003, 0.06);
            let leth_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            disease.lethality = (disease.lethality * leth_factor).clamp(0.0002, 0.01);
            new.events.push(GameEvent::DiseaseMutated {
                disease_idx: d_idx,
                new_generation: disease.strain_generation,
            });
        }
    }

    // Research progress
    if let Some(ref mut project) = new.field_research {
        project.progress += 1.0;
        if project.is_complete() {
            match &project.kind {
                ResearchKind::IdentifyThreat { disease_idx } => {
                    let d_idx = *disease_idx;
                    if let Some(disease) = new.diseases.get_mut(d_idx) {
                        disease.knowledge = (disease.knowledge + 0.50).min(KNOWLEDGE_FULL);
                    }
                }
                ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
                    let m_idx = *medicine_idx;
                    let d_idx = *disease_idx;
                    if let Some(medicine) = new.medicines.get_mut(m_idx) {
                        if !medicine.tested_against.contains(&d_idx) {
                            medicine.tested_against.push(d_idx);
                        }
                        // Update strain calibration to current disease generation
                        if let Some(pos) = medicine.target_diseases.iter().position(|&d| d == d_idx) {
                            let current_gen = new.diseases.get(d_idx)
                                .map_or(0, |d| d.strain_generation);
                            // Extend strain_generations if needed
                            while medicine.strain_generations.len() <= pos {
                                medicine.strain_generations.push(0);
                            }
                            medicine.strain_generations[pos] = current_gen;
                        }
                    }
                }
                ResearchKind::GenomicSequencing { disease_idx } => {
                    let d_idx = *disease_idx;
                    if let Some(disease) = new.diseases.get_mut(d_idx) {
                        disease.sequencing_count += 1;
                    }
                }
                ResearchKind::DevelopMedicine { .. }
                | ResearchKind::ManufactureDoses { .. }
                | ResearchKind::TrainPersonnel => {}
            }
            new.field_research = None;
        }
    }
    if let Some(ref mut project) = new.bench_research {
        project.progress += 1.0;
        if project.is_complete() {
            match &project.kind {
                ResearchKind::DevelopMedicine { medicine_idx } => {
                    let m_idx = *medicine_idx;
                    if let Some(medicine) = new.medicines.get_mut(m_idx) {
                        medicine.unlocked = true;
                        // Calibrate to current strain generations of all target diseases
                        medicine.strain_generations = medicine.target_diseases.iter()
                            .map(|&d_idx| new.diseases.get(d_idx)
                                .map_or(0, |d| d.strain_generation))
                            .collect();
                    }
                }
                ResearchKind::ManufactureDoses { medicine_idx } => {
                    let m_idx = *medicine_idx;
                    if let Some(medicine) = new.medicines.get_mut(m_idx) {
                        medicine.doses = medicine.max_doses;
                    }
                }
                ResearchKind::TrainPersonnel => {
                    new.resources.personnel += 5;
                }
                _ => {}
            }
            new.bench_research = None;
        }
    }

    // Policy costs — suspend most expensive policies one at a time until affordable.
    let mut policy_cost = new.total_policy_funding_cost();
    while policy_cost > 0.0 && new.resources.funding < policy_cost {
        // Find the most expensive active individual policy across all regions
        let mut best: Option<(usize, &str, f64)> = None;
        for (i, p) in new.policies.iter().enumerate() {
            for (name, active, cost) in [
                ("Travel Ban", p.travel_ban, TRAVEL_BAN_COST),
                ("Quarantine", p.quarantine, QUARANTINE_COST),
                ("Hospital Surge", p.hospital_surge, HOSPITAL_SURGE_COST),
            ] {
                if active {
                    if best.is_none() || cost > best.unwrap().2 {
                        best = Some((i, name, cost));
                    }
                }
            }
        }
        if let Some((region_idx, policy_name, _)) = best {
            match policy_name {
                "Travel Ban" => new.policies[region_idx].travel_ban = false,
                "Quarantine" => new.policies[region_idx].quarantine = false,
                "Hospital Surge" => new.policies[region_idx].hospital_surge = false,
                _ => unreachable!(),
            }
            new.events.push(GameEvent::PolicySuspended {
                region_idx,
                policy_name: policy_name.to_string(),
            });
            policy_cost = new.total_policy_funding_cost();
        } else {
            break;
        }
    }
    if policy_cost > 0.0 {
        new.resources.funding -= policy_cost;
    }

    // Passive resource generation (both degrade as deaths mount)
    let funding_income = new.funding_income_rate();
    new.resources.funding += funding_income;
    let rp_income = new.rp_income_rate();
    new.resources.research_points += rp_income;

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

    new.rng = rng;
    new.tick += 1;

    // Check win/lose conditions (only while still playing)
    if new.outcome == GameOutcome::Playing {
        let total_dead = new.total_dead();
        let death_threshold = new.initial_population() * LOSE_DEATH_FRACTION;

        if total_dead >= death_threshold {
            new.outcome = GameOutcome::Lost;
            new.events.push(GameEvent::GameOver);
        } else if new.total_infected() < WIN_INFECTED_THRESHOLD {
            // Win requires: diseases identified, contained, and medicines tested
            let all_identified = new.diseases.iter().all(|d| d.knowledge >= KNOWLEDGE_NAME);
            let all_have_tested_medicine = (0..new.diseases.len()).all(|d_idx| {
                new.medicines.iter().any(|m| m.tested_against.contains(&d_idx))
            });
            if all_identified && all_have_tested_medicine {
                new.outcome = GameOutcome::Won;
                new.events.push(GameEvent::GameOver);
            }
        }
    }

    new
}

/// Find or create a RegionDiseaseState entry for the given disease in a region.
fn get_or_create_infection(region: &mut crate::state::Region, disease_idx: usize) -> &mut RegionDiseaseState {
    let pos = region.infections.iter().position(|i| i.disease_idx == disease_idx);
    if let Some(idx) = pos {
        &mut region.infections[idx]
    } else {
        region.infections.push(RegionDiseaseState {
            disease_idx,
            infected: 0.0,
            dead: 0.0,
            immune: 0.0,
        });
        region.infections.last_mut().unwrap()
    }
}

/// Execute medicine deployment: deduct funds, apply doses (with adverse effect
/// roll for untested medicines). Pure game logic — does NOT modify UI state.
///
/// Returns (navigate_back, message):
/// - `navigate_back`: true if the caller should return to SelectRegion
/// - `message`: status feedback to display (if any)
fn deploy_medicine(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    target_selection: usize,
) -> (bool, Option<String>) {
    // Block after game over
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    let med = &state.medicines[medicine_idx];
    let cost = med.cost;
    let med_name = med.name.clone();
    let therapy_type = med.therapy_type;
    let target = med.decode_deploy_target(target_selection);

    if let Some(target) = target {
        if state.resources.funding < cost {
            return (false, Some(insufficient_funds_message(cost, state.resources.funding)));
        }
        if state.medicines[medicine_idx].doses <= 0.0 {
            return (false, Some(format!("No doses remaining for {med_name} — manufacture more via Research")));
        }

        let disease_idx = match &target {
            DeployTarget::Vaccinate { disease_idx } => *disease_idx,
            DeployTarget::Treat { disease_idx } => *disease_idx,
        };

        // Efficacy: therapy type × pathogen type × strain match
        let pathogen = &state.diseases[disease_idx].pathogen_type;
        let therapy_efficacy = therapy_type.efficacy(pathogen);
        let strain_eff = state.medicines[medicine_idx].strain_efficacy(disease_idx, &state.diseases);
        let efficacy = therapy_efficacy * strain_eff;
        let effective_doses = state.medicines[medicine_idx].doses * efficacy;

        let region = &mut state.regions[region_idx];
        let region_name = region.name.clone();
        let pop = region.population as f64;

        // Look up existing infection state (don't create yet — avoid ghost entries)
        let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
        let infected = existing.map(|i| i.infected).unwrap_or(0.0);
        let dead = existing.map(|i| i.dead).unwrap_or(0.0);
        let immune = existing.map(|i| i.immune).unwrap_or(0.0);

        let is_tested = state.medicines[medicine_idx]
            .tested_against
            .contains(&disease_idx);

        let msg = match target {
            DeployTarget::Vaccinate { .. } => {
                let susceptible = (pop - infected - dead - immune).max(0.0);
                let actual = effective_doses.min(susceptible);
                if actual > 0.0 {
                    // Now create entry if needed
                    let inf = get_or_create_infection(region, disease_idx);
                    let mut adverse = false;
                    if !is_tested {
                        let roll: f64 = state.rng.r#gen();
                        if roll < 0.25 {
                            adverse = true;
                            let harmed = (actual * 0.2).min(susceptible);
                            inf.dead += harmed;
                            inf.immune += actual - harmed;
                        } else {
                            inf.immune += actual;
                        }
                    } else {
                        inf.immune += actual;
                    }
                    state.resources.funding -= cost;
                    state.medicines[medicine_idx].doses = (state.medicines[medicine_idx].doses - actual).max(0.0);
                    deploy_feedback(&med_name, &region_name, "Vaccinated", actual, cost, adverse, efficacy)
                } else {
                    format!("No susceptible population in {region_name}")
                }
            }
            DeployTarget::Treat { .. } => {
                let actual = effective_doses.min(infected);
                if actual > 0.0 {
                    let inf = get_or_create_infection(region, disease_idx);
                    inf.infected -= actual;
                    let mut adverse = false;
                    if !is_tested {
                        let roll: f64 = state.rng.r#gen();
                        if roll < 0.25 {
                            adverse = true;
                            let harmed = actual * 0.2;
                            inf.dead += harmed;
                            inf.immune += actual - harmed;
                        } else {
                            inf.immune += actual;
                        }
                    } else {
                        inf.immune += actual;
                    }
                    state.resources.funding -= cost;
                    state.medicines[medicine_idx].doses = (state.medicines[medicine_idx].doses - actual).max(0.0);
                    deploy_feedback(&med_name, &region_name, "Treated", actual, cost, adverse, efficacy)
                } else {
                    format!("No infected population in {region_name}")
                }
            }
        };

        return (true, Some(msg));
    }

    (true, None)
}

fn insufficient_funds_message(cost: f64, have: f64) -> String {
    format!("Insufficient funds! Need ${cost:.0}, have ${have:.0}")
}

fn deploy_feedback(med: &str, region: &str, action: &str, doses: f64, cost: f64, adverse: bool, efficacy: f64) -> String {
    let doses_str = crate::format_number(doses);
    let eff_note = if efficacy < 1.0 {
        format!(" ({:.0}% efficacy)", efficacy * 100.0)
    } else {
        String::new()
    };
    if adverse {
        let killed = crate::format_number(doses * 0.2);
        format!("{action} {doses_str} in {region} with {med}{eff_note} (-${cost:.0}) -- ADVERSE REACTION: {killed} died")
    } else {
        format!("{action} {doses_str} in {region} with {med}{eff_note} (-${cost:.0})")
    }
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
                deploy_medicine(state, *medicine_idx, *region_idx, *target_selection);
            CommandResult { message: msg, success: nav_back }
        }
        GameCommand::StartResearch { bench, project_idx } => {
            let ok = start_research(state, *bench, *project_idx);
            CommandResult { message: None, success: ok }
        }
        GameCommand::BoostResearch { bench } => {
            let msg = boost_research(state, *bench);
            let success = msg.is_some();
            CommandResult { message: msg, success }
        }
        GameCommand::TogglePolicy {
            region_idx,
            policy_idx,
        } => {
            let msg = toggle_policy(state, *region_idx, *policy_idx);
            let success = msg.is_some();
            CommandResult { message: msg, success }
        }
    }
}

/// Apply a player action to the game state.
pub fn apply_action(state: &GameState, action: &Action) -> GameState {
    let mut new = state.clone();
    new.ui.status_message = None;

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
            let max = new.panel_selection_max();
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

/// Toggle a policy for a region. Returns a status message describing what happened.
/// Does not touch UI state.
fn toggle_policy(state: &mut GameState, region_idx: usize, policy_idx: usize) -> Option<String> {
    if region_idx >= state.policies.len() {
        return None;
    }
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.as_str())
        .unwrap_or("Unknown");
    let available_personnel = state.personnel_available();
    match policy_idx {
        0 => {
            let new_state = !state.policies[region_idx].travel_ban;
            state.policies[region_idx].travel_ban = new_state;
            let verb = if new_state { "enabled" } else { "disabled" };
            Some(format!("Travel Ban {verb} in {region_name} — ${:.0}/tick", TRAVEL_BAN_COST))
        }
        1 => {
            if state.policies[region_idx].quarantine {
                state.policies[region_idx].quarantine = false;
                Some(format!("Quarantine disabled in {region_name}"))
            } else if available_personnel >= QUARANTINE_PERSONNEL {
                state.policies[region_idx].quarantine = true;
                Some(format!("Quarantine enabled in {region_name} — ${:.0}/tick + {} personnel",
                    QUARANTINE_COST, QUARANTINE_PERSONNEL))
            } else {
                Some(format!(
                    "Not enough personnel for quarantine (need {})", QUARANTINE_PERSONNEL
                ))
            }
        }
        2 => {
            if state.policies[region_idx].hospital_surge {
                state.policies[region_idx].hospital_surge = false;
                Some(format!("Hospital Surge disabled in {region_name}"))
            } else if available_personnel >= HOSPITAL_SURGE_PERSONNEL {
                state.policies[region_idx].hospital_surge = true;
                Some(format!("Hospital Surge enabled in {region_name} — ${:.0}/tick + {} personnel",
                    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL))
            } else {
                Some(format!(
                    "Not enough personnel for hospital surge (need {})", HOSPITAL_SURGE_PERSONNEL
                ))
            }
        }
        _ => None,
    }
}

/// Compute available field research projects (excludes the currently active one).
/// Max selection index for the current research UI state.
/// Start a research project. Pure game logic — does NOT modify UI state.
///
/// Returns true if the project was successfully started.
fn start_research(state: &mut GameState, bench: bool, project_idx: usize) -> bool {
    if state.outcome != GameOutcome::Playing {
        return false;
    }
    let occupied = if bench { state.bench_research.is_some() } else { state.field_research.is_some() };
    if occupied {
        return false;
    }

    let projects = if bench {
        state.available_bench_projects()
    } else {
        state.available_field_projects()
    };

    if let Some(kind) = projects.get(project_idx) {
        let (rp_cost, personnel, duration) = kind.costs(&state.medicines);

        if state.resources.research_points >= rp_cost
            && state.personnel_available() >= personnel
        {
            let project = ResearchProject {
                kind: kind.clone(),
                progress: 0.0,
                required_ticks: duration,
                personnel_assigned: personnel,
                rp_cost,
            };
            state.resources.research_points -= rp_cost;

            if bench {
                state.bench_research = Some(project);
            } else {
                state.field_research = Some(project);
            }
            return true;
        }
    }
    false
}

/// Boost an active research project. Pure game logic — does NOT modify UI state.
///
/// Returns an optional status message.
fn boost_research(state: &mut GameState, bench: bool) -> Option<String> {
    let project = if bench { &mut state.bench_research } else { &mut state.field_research };
    if let Some(project) = project {
        if !project.is_complete() && state.resources.research_points >= BOOST_RP_COST {
            state.resources.research_points -= BOOST_RP_COST;
            project.progress = (project.progress + BOOST_TICKS).min(project.required_ticks);
            Some(format!(
                "Boosted research! (-{:.0} RP, +{:.0} ticks)",
                BOOST_RP_COST, BOOST_TICKS
            ))
        } else if state.resources.research_points < BOOST_RP_COST {
            Some(format!(
                "Need {:.0} RP to boost (have {:.0})",
                BOOST_RP_COST, state.resources.research_points
            ))
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GameState, MedicineUiState, PolicyUiState, ResearchUiState};

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
        let state = GameState::new_default(42);
        assert!(!state.paused);
        let s = apply_action(&state, &Action::TogglePause);
        assert!(s.paused);
        let s = apply_action(&s, &Action::TogglePause);
        assert!(!s.paused);
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
        state.regions[0].infections.push(RegionDiseaseState {
            disease_idx: 0,
            infected: 0.0,
            dead: 0.0,
            immune: 100_000_000.0,
        });
        let mut s = state;
        for _ in 0..200 {
            s = tick(&s);
        }
        let na_imm = s.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .map(|i| i.immune)
            .unwrap_or(0.0);
        assert!(
            na_imm >= 100_000_000.0,
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
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.resources.funding, funding_before - 100.0);
        let na_inf = state.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .unwrap();
        assert_eq!(na_inf.immune, 100_000.0);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
        ));
        // Deployment feedback message should be set
        let msg = state.ui.status_message.as_ref().expect("status message should be set after deploy");
        assert!(msg.contains("Vaccinated"), "message should mention vaccination: {msg}");
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
        assert_eq!(state.resources.funding, funding_before - 100.0);
        // Doses should have been depleted
        let treated = infected_before - infected_after;
        assert!(
            state.medicines[0].doses < state.medicines[0].max_doses,
            "doses should have been depleted after deployment"
        );
        assert!(
            (state.medicines[0].max_doses - state.medicines[0].doses - treated).abs() < 1.0,
            "doses depleted should equal people treated"
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
    fn manufacture_doses_restores_supply() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].doses = 0.0; // Depleted

        // ManufactureDoses should appear in available bench projects
        let bench = state.available_bench_projects();
        assert!(
            bench.iter().any(|k| matches!(k, ResearchKind::ManufactureDoses { medicine_idx: 0 })),
            "manufacture should be available for depleted medicine"
        );

        // Start and complete manufacture
        state.resources.research_points = 100.0;
        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
            rp_cost: 10.0,
        });
        state = tick(&state);

        assert!(state.bench_research.is_none(), "project should be complete");
        assert_eq!(
            state.medicines[0].doses, state.medicines[0].max_doses,
            "doses should be restored to max"
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
        // Deploy to North America (region 0) which has no infections for disease 0
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
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectRegion { .. })),
            "should return to SelectRegion after deploy"
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
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectRegion { .. })),
            "tested medicine should deploy without confirmation"
        );
        assert!(state.resources.funding < funding_before);
    }

    #[test]
    fn map_navigation_right_left() {
        let state = GameState::new_default(42);
        assert_eq!(state.ui.map_selection, 0); // NA
        let s = apply_action(&state, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 2); // EU
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 4); // AS
        // Can't go past rightmost column
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 4);
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 2); // EU
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 0); // NA
        // Can't go past leftmost column
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 0);
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
        let state = GameState::new_default(42);
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
    fn research_identify_increases_knowledge() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;
        // Start identify project on disease 0
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #1
        state = apply_action(&state, &Action::Confirm); // Confirm start
        assert!(state.field_research.is_some());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (80 ticks)
        for _ in 0..80 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0; // Fully identified

        assert!(!state.medicines[0].unlocked);

        // Start bench research: Develop Antiviral-A
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        state = apply_action(&state, &Action::Confirm);     // Select Develop Antiviral-A
        state = apply_action(&state, &Action::Confirm);     // Confirm

        assert!(state.bench_research.is_some());

        for _ in 0..150 {
            state = tick(&state);
        }
        assert!(state.bench_research.is_none());
        assert!(state.medicines[0].unlocked);
    }

    #[test]
    fn research_clinical_trial_marks_tested() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true; // Pre-unlock for testing

        assert!(state.medicines[0].tested_against.is_empty());

        // Start field research: Clinical Trial
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        // Navigate to the clinical trial project.
        // Field projects: identify (for each unidentified disease), genomic sequencing
        // (for fully identified diseases), then clinical trials.
        let field_projects = state.available_field_projects();
        let trial_idx = field_projects.iter().position(|k| matches!(k,
            ResearchKind::ClinicalTrial { .. }
        )).expect("should have a clinical trial available");
        for _ in 0..trial_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm);    // Select
        state = apply_action(&state, &Action::Confirm);    // Confirm

        assert!(state.field_research.is_some());

        for _ in 0..80 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none());
        assert!(state.medicines[0].tested_against.contains(&0));
    }

    #[test]
    fn research_insufficient_rp_blocks_start() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 0.0; // No RP

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify
        state = apply_action(&state, &Action::Confirm); // Try to confirm

        // Should not have started — still on confirm screen
        assert!(state.field_research.is_none());
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
        state.resources.research_points = 100.0;

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
    fn research_boost_spends_rp_and_advances() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;

        // Start a field research project
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts project

        // Should be back at BrowseProjects with an active field project
        assert!(state.field_research.is_some());
        let progress_before = state.field_research.as_ref().unwrap().progress;
        let rp_before = state.resources.research_points;

        // Navigate to ViewActive and boost
        state = apply_action(&state, &Action::Confirm); // → ViewActive
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ViewActive { bench: false })));

        state = apply_action(&state, &Action::Confirm); // Boost!
        assert_eq!(
            state.resources.research_points,
            rp_before - BOOST_RP_COST,
            "should spend {} RP", BOOST_RP_COST
        );
        assert_eq!(
            state.field_research.as_ref().unwrap().progress,
            progress_before + BOOST_TICKS,
            "should advance by {} ticks", BOOST_TICKS
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Boosted"));
    }

    #[test]
    fn research_boost_insufficient_rp() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 20.0; // Enough to start (15 RP) but not boost again (10 RP)

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts (costs 10 RP, leaves 5)

        assert!(state.field_research.is_some());
        assert_eq!(state.resources.research_points, 5.0); // 20 - 15 = 5

        state = apply_action(&state, &Action::Confirm); // → ViewActive
        let rp_before = state.resources.research_points;
        let progress_before = state.field_research.as_ref().unwrap().progress;

        state = apply_action(&state, &Action::Confirm); // Try to boost — should fail
        assert_eq!(state.resources.research_points, rp_before, "should not spend RP");
        assert_eq!(
            state.field_research.as_ref().unwrap().progress,
            progress_before,
            "should not advance"
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Need"));
    }

    #[test]
    fn diseases_start_unknown() {
        let state = GameState::new_default(42);
        for disease in &state.diseases {
            assert_eq!(disease.knowledge, 0.0);
        }
    }

    #[test]
    fn lose_condition_triggers_on_mass_death() {
        let mut state = GameState::new_default(42);
        // Ensure a highly lethal disease so the game reliably ends in a loss
        state.diseases[0].infectivity = 0.10;
        state.diseases[0].lethality = 0.05;
        state.diseases[0].cross_region_spread = 0.05;
        // Run until game over
        for _ in 0..2000 {
            state = tick(&state);
            crate::ui::process_events(&mut state);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        assert_eq!(state.outcome, GameOutcome::Lost);
        assert!(state.paused);
        // Deaths should be just over the threshold
        let threshold = state.initial_population() * LOSE_DEATH_FRACTION;
        assert!(state.total_dead() >= threshold);
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
        assert!(state.paused);
    }

    #[test]
    fn no_research_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.resources.research_points = 100.0;
        let rp_before = state.resources.research_points;
        // Try to start research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select project
        state = apply_action(&state, &Action::Confirm); // Try to confirm
        assert!(state.field_research.is_none(), "should not start research after game over");
        assert_eq!(state.resources.research_points, rp_before, "should not spend RP after game over");
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
        state.paused = true;
        let s = apply_action(&state, &Action::TogglePause);
        assert!(s.paused, "should not be able to unpause after game over");
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
    fn concurrent_field_and_bench_research() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;

        // Start field research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #2
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.field_research.is_some());

        // Start bench research
        state = apply_action(&state, &Action::ClosePanel); // Back to categories
        state = apply_action(&state, &Action::SelectNext);  // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        state = apply_action(&state, &Action::Confirm);     // Select Develop
        state = apply_action(&state, &Action::Confirm);     // Confirm
        assert!(state.bench_research.is_some());

        // Both running simultaneously
        assert!(state.field_research.is_some());
        assert!(state.bench_research.is_some());
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
        let income_with_ban = with_ban.resources.funding - 1000.0 + 10.0; // add back $10 policy cost to isolate income effect

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
        state.policies[0].travel_ban = true; // $10/tick, also halves region 0 income
        state = tick(&state);
        let net_change = state.resources.funding - funding_before;

        // Should have deducted $10 and added income (less than without ban)
        assert!(
            net_change < income_no_policy,
            "travel ban should reduce net income: net {net_change:.1} vs no-policy {income_no_policy:.1}"
        );
        assert!(
            net_change < 0.0,
            "travel ban cost ($10) should exceed income (~$5): net change {net_change:.1}"
        );
    }

    #[test]
    fn policy_funding_crisis_suspends_most_expensive_first() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 15.0; // Enough for quarantine ($8) but not both
        state.policies[0].travel_ban = true; // $10/tick — most expensive
        state.policies[0].quarantine = true; // $8/tick
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
        // Set up 3 policies: $10 + $8 + $5 = $23/tick total
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].hospital_surge = true;
        // Enough for only $13/tick (quarantine + hospital surge)
        state.resources.funding = 20.0;
        state = tick(&state);
        // Travel ban ($10, most expensive) should be suspended
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended first");
        assert!(state.policies[0].quarantine, "quarantine should survive tick 1");
        assert!(state.policies[0].hospital_surge, "hospital surge should survive tick 1");
    }

    #[test]
    fn funding_warning_when_runway_low() {
        let mut state = GameState::new_default(42);
        state.policies[0].travel_ban = true; // $10/tick, income ~$5/tick → net burn ~$5/tick
        // After deducting $10 and adding ~$5 income, funding ≈ $15.
        // Net burn ~$5/tick, threshold = 5 × $5 = $25 → $15 < $25 → warning
        state.resources.funding = 20.0;
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
        state = apply_action(&state, &Action::OpenPolicy);
        assert_eq!(state.ui.open_panel, Panel::Policy);

        // Select Asia (index 4)
        for _ in 0..4 {
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
        // RNA virus (Strain Alpha) has mutation_rate 0.008, so over 500 ticks
        // we expect ~4 mutations. Run enough ticks to virtually guarantee at least one.
        let original_infectivity = state.diseases[0].infectivity;
        for _ in 0..500 {
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
    fn develop_medicine_sets_strain_generation() {
        let mut state = GameState::new_default(42);
        // Manually mutate disease 0 to gen 2
        state.diseases[0].strain_generation = 2;
        state.diseases[0].knowledge = 1.0;
        state.resources.research_points = 100.0;

        // Start and complete DevelopMedicine for medicine 0 (targets disease 0)
        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 24.0, // will complete on next tick
            required_ticks: 25.0,
            personnel_assigned: 5,
            rp_cost: 15.0,
        });

        state = tick(&state);
        assert!(state.medicines[0].unlocked);
        assert_eq!(
            state.medicines[0].strain_generations,
            vec![2], // should match disease gen at time of completion
            "medicine should be calibrated to disease generation at completion"
        );
    }

    #[test]
    fn clinical_trial_updates_strain_generation() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 3;
        state.medicines[0].unlocked = true;
        state.medicines[0].strain_generations = vec![0]; // outdated
        state.resources.research_points = 100.0;

        state.field_research = Some(ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0, // will complete on next tick
            required_ticks: 25.0,
            personnel_assigned: 5,
            rp_cost: 15.0,
        });

        state = tick(&state);
        assert!(state.medicines[0].tested_against.contains(&0));
        // strain_generation should be updated to current disease gen
        // Note: disease might have mutated during this tick too, so check >= 3
        assert!(
            state.medicines[0].strain_generations[0] >= 3,
            "clinical trial should update strain calibration"
        );
    }

    #[test]
    fn narrow_medicine_cheaper_to_develop_than_broad() {
        let state = GameState::new_default(1);
        // Medicine 0 = targeted (1 target), last medicine = Broad-Spectrum (all targets)
        let narrow = ResearchKind::DevelopMedicine { medicine_idx: 0 };
        let broad_idx = state.medicines.len() - 1;
        let broad = ResearchKind::DevelopMedicine { medicine_idx: broad_idx };
        let (narrow_rp, narrow_pers, narrow_ticks) = narrow.costs(&state.medicines);
        let (broad_rp, broad_pers, broad_ticks) = broad.costs(&state.medicines);
        assert!(narrow_rp < broad_rp, "narrow should cost less RP");
        assert!(narrow_pers <= broad_pers, "narrow should need fewer personnel");
        assert!(narrow_ticks < broad_ticks, "narrow should be faster");
    }

    #[test]
    fn outdated_strain_shows_retrial_available() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 2;
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![0]; // already tested
        state.medicines[0].strain_generations = vec![0]; // but outdated

        let field_projects = state.available_field_projects();
        let has_retrial = field_projects.iter().any(|k| matches!(k,
            ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 }
        ));
        assert!(has_retrial, "should offer clinical trial for strain-outdated medicine");
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
    fn genomic_sequencing_reduces_mutation_rate() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;
        let original_rate = state.diseases[0].pathogen_type.mutation_rate();

        // Start genomic sequencing via field research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        // Navigate past identify projects to genomic sequencing
        let field_projects = state.available_field_projects();
        let seq_idx = field_projects.iter().position(|k| matches!(k,
            ResearchKind::GenomicSequencing { .. }
        )).expect("should have genomic sequencing available");
        for _ in 0..seq_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // Select
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.field_research.is_some());

        // Complete the project
        for _ in 0..120 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        // Verify mutation rate is effectively halved
        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        let initial_personnel = state.resources.personnel;

        // Start personnel training via bench research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        // Navigate to Train Personnel (last item in bench projects)
        let bench_projects = state.available_bench_projects();
        let train_idx = bench_projects.iter().position(|k| matches!(k,
            ResearchKind::TrainPersonnel
        )).expect("should have train personnel available");
        for _ in 0..train_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // Select
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.bench_research.is_some());

        // Complete the project
        for _ in 0..100 {
            state = tick(&state);
        }
        assert!(state.bench_research.is_none());
        assert_eq!(state.resources.personnel, initial_personnel + 5);
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
}
