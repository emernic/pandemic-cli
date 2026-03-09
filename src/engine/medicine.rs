use rand::Rng;

use crate::state::{
    DeployTarget, GameOutcome, GameState, RegionDiseaseState,
    TREATMENT_FRACTION,
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
/// Returns (navigate_back, message):
/// - `navigate_back`: true if the caller should return to SelectRegion
/// - `message`: status feedback to display (if any)
pub(super) fn deploy_medicine(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    target_selection: usize,
) -> (bool, Option<String>) {
    // Block after game over
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    // Block deployment to collapsed regions
    if state.regions.get(region_idx).is_some_and(|r| r.collapsed) {
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} has collapsed — deployment impossible")));
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
                    deploy_feedback(&med_name, &region_name, "Protected", actual, cost, adverse, efficacy)
                } else {
                    format!("No susceptible population in {region_name}")
                }
            }
            DeployTarget::Treat { .. } => {
                // Treatment is proportional: treats a fraction of infected,
                // capped by available doses. This scales naturally with
                // outbreak size — always impactful regardless of infection count.
                let target_treated = infected * TREATMENT_FRACTION * efficacy;
                let actual = target_treated.min(state.medicines[medicine_idx].doses);
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

pub(super) fn insufficient_funds_message(cost: f64, have: f64) -> String {
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
