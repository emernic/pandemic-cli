use rand::Rng;

use crate::state::{
    DeployTarget, GameEvent, GameOutcome, GameState, RegionDiseaseState,
};

/// Execute medicine deployment: deduct funds, apply doses (with adverse effect
/// roll for untested medicines). Pure game logic — does NOT modify UI state.
///
/// Returns (success, message, adverse):
/// - `success`: true if deployment was attempted (maps to CommandResult.success)
/// - `message`: status feedback to display (if any)
/// - `adverse`: true if an adverse reaction occurred
pub(super) fn deploy_medicine(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    target: DeployTarget,
) -> (bool, Option<String>, bool) {
    // Block after game over
    if state.outcome != GameOutcome::Playing {
        return (false, None, false);
    }
    // Block deployment to collapsed regions
    if state.regions.get(region_idx).is_some_and(|r| r.collapsed) {
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} has collapsed — deployment impossible")), false);
    }
    // Block deployment during cooldown
    let cooldown = state.regions[region_idx].deploy_cooldown_remaining(state.tick);
    if cooldown > 0 {
        let days = cooldown as f64 / crate::state::TICKS_PER_DAY;
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} on cooldown — {days:.1} days remaining")), false);
    }
    let med = &state.medicines[medicine_idx];
    let cost = med.deploy_cost(state.regions[region_idx].population);
    let med_name = med.name.clone();

    if state.resources.funding < cost {
        return (false, Some(insufficient_funds_message(cost, state.resources.funding)), false);
    }
    if state.medicines[medicine_idx].doses <= 0.0 {
        return (false, Some(format!("No doses remaining for {med_name} — manufacture more via Research")), false);
    }

    let disease_idx = match &target {
        DeployTarget::Vaccinate { disease_idx } => *disease_idx,
        DeployTarget::Treat { disease_idx } => *disease_idx,
    };

    let efficacy = state.medicines[medicine_idx].effective_efficacy(disease_idx, &state.diseases);
    let vax_mult = state.vaccination_multiplier();
    let region = &mut state.regions[region_idx];
    let region_name = region.name.clone();
    let pop = region.population as f64;

    // Look up existing infection state (don't create yet — avoid ghost entries)
    let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
    let infected = existing.map(|i| i.infected).unwrap_or(0.0);
    let dead = region.dead;
    let immune = existing.map(|i| i.immune).unwrap_or(0.0);

    let is_tested = state.medicines[medicine_idx]
        .tested_against
        .contains(&disease_idx);

    let (mut msg, adverse) = match target {
        DeployTarget::Vaccinate { .. } => {
            let susceptible = (pop - infected - dead - immune).max(0.0);
            let actual = state.medicines[medicine_idx].estimate_vaccination(susceptible, efficacy, vax_mult);
            if actual > 0.0 {
                let (adverse, adverse_deaths) = adverse_check(&mut state.rng, actual, is_tested, susceptible);
                let inf = region.get_or_create_infection(disease_idx);
                apply_immune_and_deaths(inf, actual, adverse_deaths);
                region.dead += adverse_deaths;
                deduct_deploy_costs(state, medicine_idx, region_idx, cost, actual);
                build_resistance(state, medicine_idx, disease_idx, false);
                (deploy_feedback(&med_name, &region_name, "Protected", actual, cost, adverse_deaths, efficacy), adverse)
            } else {
                (format!("No susceptible population in {region_name}"), false)
            }
        }
        DeployTarget::Treat { .. } => {
            let actual = state.medicines[medicine_idx].estimate_treatment(infected, efficacy);
            if actual > 0.0 {
                let (adverse, adverse_deaths) = adverse_check(&mut state.rng, actual, is_tested, infected);
                let inf = region.get_or_create_infection(disease_idx);
                inf.infected -= actual;
                apply_immune_and_deaths(inf, actual, adverse_deaths);
                region.dead += adverse_deaths;
                deduct_deploy_costs(state, medicine_idx, region_idx, cost, actual);
                build_resistance(state, medicine_idx, disease_idx, true);
                (deploy_feedback(&med_name, &region_name, "Treated", actual, cost, adverse_deaths, efficacy), adverse)
            } else {
                (format!("No infected population in {region_name}"), false)
            }
        }
    };

    // Warn if resistance is building — the player needs to know their medicine
    // is becoming less effective so they can research alternatives.
    if let Some(disease) = state.diseases.get(disease_idx) {
        let mech = state.medicines[medicine_idx].mechanism;
        let res_factor = disease.resistance_factor(mech);
        if res_factor < 0.5 {
            msg += " \u{26a0} HIGH RESISTANCE — consider alternative therapy";
        } else if res_factor < 0.7 {
            msg += " \u{26a0} Resistance building — efficacy declining";
        }
    }

    (true, Some(msg), adverse)
}

/// Roll for adverse reaction on untested medicines.
/// Returns (adverse_occurred, deaths). Deaths are capped at `max_deaths`
/// to prevent killing more people than the target population.
fn adverse_check(rng: &mut impl Rng, actual: f64, is_tested: bool, max_deaths: f64) -> (bool, f64) {
    if !is_tested {
        let roll: f64 = rng.r#gen();
        if roll < 0.25 {
            let deaths = (actual * 0.2).min(max_deaths);
            return (true, deaths);
        }
    }
    (false, 0.0)
}

/// Apply immune gains and adverse deaths to infection state.
/// Caller must separately add adverse_deaths to region.dead.
fn apply_immune_and_deaths(
    inf: &mut RegionDiseaseState,
    actual: f64,
    adverse_deaths: f64,
) {
    if adverse_deaths > 0.0 {
        inf.dead += adverse_deaths;
        inf.immune += actual - adverse_deaths;
    } else {
        inf.immune += actual;
    }
}

/// Deduct funds, doses, increment deploy count, and start region cooldown.
fn deduct_deploy_costs(state: &mut GameState, medicine_idx: usize, region_idx: usize, cost: f64, actual: f64) {
    state.resources.funding -= cost;
    state.medicines[medicine_idx].doses = (state.medicines[medicine_idx].doses - actual).max(0.0);
    state.medicines[medicine_idx].deployed_count += 1;
    state.regions[region_idx].last_deploy_tick = Some(state.tick);
}

/// Build resistance from deployment pressure. Treatment creates much more
/// selection pressure than vaccination. Broad-spectrum drugs build resistance
/// faster (2x) because broad selection pressure accelerates adaptation.
/// Mechanism-specific multipliers further modify: cheap/fast mechanisms
/// have high resistance rates, expensive/durable ones have low rates.
/// CombinationTherapy tech halves all resistance buildup.
fn build_resistance(state: &mut GameState, medicine_idx: usize, disease_idx: usize, is_treatment: bool) {
    let med = &state.medicines[medicine_idx];
    let mechanism = med.mechanism;
    let base = if is_treatment { 0.03 } else { 0.005 };
    let type_mult = match med.therapy_type {
        crate::state::TherapyType::BroadSpectrum => 2.0,
        _ => 1.0,
    };
    let mech_mult = mechanism.map(|m| m.resistance_rate_multiplier()).unwrap_or(1.0);
    let combo_mult = state.resistance_multiplier();
    let gain = base * type_mult * mech_mult * combo_mult;
    // Resistance lives on the disease, keyed by mechanism — so deploying
    // any CellWallInhibitor drug builds resistance that affects ALL
    // CellWallInhibitor drugs against this disease.
    if let Some(disease) = state.diseases.get_mut(disease_idx) {
        disease.add_resistance(mechanism, gain);
    }
}

pub(super) fn insufficient_funds_message(cost: f64, have: f64) -> String {
    format!("Insufficient funds! Need ${cost:.0}, have ${have:.0}")
}

/// Auto-deploy medicines to the worst-affected regions. Called once per tick.
/// For each medicine with auto_deploy enabled:
/// - Must be unlocked, tested, have doses, and have affordable deploy cost
/// - Finds the region with the most infected (for any target disease) where
///   cooldown is clear
/// - Deploys as treatment (treating infected is the reactive choice;
///   vaccination is strategic and left to the player)
pub(super) fn try_auto_deploy(state: &mut GameState) {
    // Grow auto_deploy vec if new medicines were created
    while state.auto_deploy.len() < state.medicines.len() {
        state.auto_deploy.push(false);
    }

    for med_idx in 0..state.medicines.len() {
        if !state.auto_deploy.get(med_idx).copied().unwrap_or(false) {
            continue;
        }
        let med = &state.medicines[med_idx];
        if !med.unlocked || med.doses <= 0.0 {
            continue;
        }

        // Only auto-deploy against tested diseases (avoid adverse reactions)
        let deployable = med.deployable_diseases(&state.diseases);
        let tested: Vec<usize> = deployable.iter()
            .copied()
            .filter(|d_idx| med.tested_against.contains(d_idx))
            .collect();
        if tested.is_empty() {
            continue;
        }

        // Find the region with the highest infected count for any tested target disease,
        // where cooldown is clear and region isn't collapsed
        let mut best_region: Option<usize> = None;
        let mut best_infected: f64 = 0.0;
        let mut best_disease_idx: usize = 0;

        for (r_idx, region) in state.regions.iter().enumerate() {
            if region.collapsed {
                continue;
            }
            if region.deploy_cooldown_remaining(state.tick) > 0 {
                continue;
            }
            for &d_idx in &tested {
                let infected = region.disease_state(d_idx)
                    .map(|inf| inf.infected)
                    .unwrap_or(0.0);
                if infected > best_infected {
                    best_infected = infected;
                    best_region = Some(r_idx);
                    best_disease_idx = d_idx;
                }
            }
        }

        if let Some(region_idx) = best_region {
            if best_infected <= 0.0 {
                continue;
            }

            // Check funding
            let cost = state.medicines[med_idx].deploy_cost(state.regions[region_idx].population);
            if state.resources.funding < cost {
                continue;
            }

            let target = DeployTarget::Treat { disease_idx: best_disease_idx };

            let (success, _msg, _adverse) = deploy_medicine(
                state, med_idx, region_idx, target,
            );
            if success {
                state.events.push(GameEvent::MedicineAutoDeployed {
                    medicine_idx: med_idx,
                    region_idx,
                });
            }
        }
    }
}

fn deploy_feedback(med: &str, region: &str, action: &str, doses: f64, cost: f64, adverse_deaths: f64, efficacy: f64) -> String {
    let doses_str = crate::format_number(doses);
    let eff_note = if efficacy < 1.0 {
        format!(" ({:.0}% efficacy)", efficacy * 100.0)
    } else {
        String::new()
    };
    if adverse_deaths > 0.0 {
        let killed = crate::format_number(adverse_deaths);
        format!("{action} {doses_str} in {region} with {med}{eff_note} (-${cost:.0}) -- ADVERSE REACTION: {killed} died")
    } else {
        format!("{action} {doses_str} in {region} with {med}{eff_note} (-${cost:.0})")
    }
}
