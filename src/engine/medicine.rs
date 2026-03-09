use rand::Rng;

use crate::state::{
    DeployTarget, GameOutcome, GameState, RegionDiseaseState,
};

/// Find or create a RegionDiseaseState entry for the given disease in a region.
pub(super) fn get_or_create_infection(region: &mut crate::state::Region, disease_idx: usize) -> &mut RegionDiseaseState {
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
/// Returns (navigate_back, message, adverse):
/// - `navigate_back`: true if the caller should return to SelectRegion
/// - `message`: status feedback to display (if any)
/// - `adverse`: true if an adverse reaction occurred
pub(super) fn deploy_medicine(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    target_selection: usize,
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
    let med = &state.medicines[medicine_idx];
    let cost = med.deploy_cost(state.regions[region_idx].population);
    let med_name = med.name.clone();
    let therapy_type = med.therapy_type;
    let target = med.decode_deploy_target(target_selection, &state.diseases);

    if let Some(target) = target {
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

        // Efficacy: therapy type × pathogen type × strain match × cross-reactivity
        let pathogen = &state.diseases[disease_idx].pathogen_type;
        let therapy_efficacy = therapy_type.efficacy(pathogen);
        let strain_eff = state.medicines[medicine_idx].strain_efficacy(disease_idx, &state.diseases);
        let cross_reactive = if state.medicines[medicine_idx].is_cross_reactive(disease_idx) {
            crate::state::CROSS_REACTIVE_PENALTY
        } else {
            1.0
        };
        let resistance = state.medicines[medicine_idx].resistance_factor(disease_idx);
        let efficacy = therapy_efficacy * strain_eff * cross_reactive * resistance;
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

        let (msg, adverse) = match target {
            DeployTarget::Vaccinate { .. } => {
                let susceptible = (pop - infected - dead - immune).max(0.0);
                let actual = state.medicines[medicine_idx].estimate_vaccination(susceptible, efficacy, vax_mult);
                if actual > 0.0 {
                    let (adverse, adverse_deaths) = adverse_check(&mut state.rng, actual, is_tested, susceptible);
                    let inf = get_or_create_infection(region, disease_idx);
                    apply_immune_and_deaths(inf, actual, adverse_deaths);
                    region.dead += adverse_deaths;
                    deduct_deploy_costs(state, medicine_idx, cost, actual);
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
                    let inf = get_or_create_infection(region, disease_idx);
                    inf.infected -= actual;
                    apply_immune_and_deaths(inf, actual, adverse_deaths);
                    region.dead += adverse_deaths;
                    deduct_deploy_costs(state, medicine_idx, cost, actual);
                    build_resistance(state, medicine_idx, disease_idx, true);
                    (deploy_feedback(&med_name, &region_name, "Treated", actual, cost, adverse_deaths, efficacy), adverse)
                } else {
                    (format!("No infected population in {region_name}"), false)
                }
            }
        };

        return (true, Some(msg), adverse);
    }

    (true, None, false)
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

/// Deduct funds, doses, and increment deploy count.
fn deduct_deploy_costs(state: &mut GameState, medicine_idx: usize, cost: f64, actual: f64) {
    state.resources.funding -= cost;
    state.medicines[medicine_idx].doses = (state.medicines[medicine_idx].doses - actual).max(0.0);
    state.medicines[medicine_idx].deployed_count += 1;
}

/// Build resistance from deployment pressure. Treatment creates much more
/// selection pressure than vaccination. Broad-spectrum drugs build resistance
/// faster (2x) because broad selection pressure accelerates adaptation.
fn build_resistance(state: &mut GameState, medicine_idx: usize, disease_idx: usize, is_treatment: bool) {
    let med = &state.medicines[medicine_idx];
    let base = if is_treatment { 0.03 } else { 0.005 };
    let type_mult = match med.therapy_type {
        crate::state::TherapyType::BroadSpectrum => 2.0,
        _ => 1.0,
    };
    let gain = base * type_mult;
    // Ensure resistance vec is populated (parallel to target_diseases)
    let med = &mut state.medicines[medicine_idx];
    while med.resistance.len() < med.target_diseases.len() {
        med.resistance.push(0.0);
    }
    if let Some(pos) = med.target_diseases.iter().position(|&d| d == disease_idx) {
        med.resistance[pos] = (med.resistance[pos] + gain).min(1.0);
    }
}

pub(super) fn insufficient_funds_message(cost: f64, have: f64) -> String {
    format!("Insufficient funds! Need ${cost:.0}, have ${have:.0}")
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
