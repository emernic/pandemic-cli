use rand::Rng;

use crate::state::{
    CrisisCost, CrisisEvent, CrisisKind, CrisisOption, GameState,
    CRISIS_TYPE_COOLDOWN,
};

/// Generate a crisis event based on current game state. Returns None if no
/// suitable crisis can be generated (e.g., no valid targets for any crisis type).
pub(super) fn generate_crisis(state: &GameState, rng: &mut impl Rng) -> Option<CrisisEvent> {
    // Build a list of eligible crisis types based on current game state
    let mut candidates: Vec<CrisisKind> = Vec::new();

    // Supply disruption: requires at least one medicine with doses
    let meds_with_doses: Vec<usize> = state.medicines.iter().enumerate()
        .filter(|(_, m)| m.unlocked && m.doses > 0.0)
        .map(|(i, _)| i)
        .collect();
    if !meds_with_doses.is_empty() {
        let idx = meds_with_doses[rng.r#gen::<usize>() % meds_with_doses.len()];
        candidates.push(CrisisKind::SupplyDisruption { medicine_idx: idx });
    }

    // Lab accident: requires active applied research
    if state.applied_research.is_some() {
        candidates.push(CrisisKind::LabAccident);
    }

    // Political pressure: requires active quarantine somewhere
    let quarantined: Vec<usize> = state.policies.iter().enumerate()
        .filter(|(_, p)| p.quarantine)
        .map(|(i, _)| i)
        .collect();
    if !quarantined.is_empty() {
        let idx = quarantined[rng.r#gen::<usize>() % quarantined.len()];
        candidates.push(CrisisKind::PoliticalPressure { region_idx: idx });
    }

    // Personnel crisis: requires at least 5 personnel
    if state.resources.personnel >= 5 {
        let amount = 3.min(state.resources.personnel);
        candidates.push(CrisisKind::PersonnelCrisis { amount });
    }

    // International aid: always available
    let funding = 300.0 + (state.tick as f64 * 0.1).min(500.0);
    let personnel = 3 + ((state.tick as f64 * 0.005).min(5.0) as u32);
    candidates.push(CrisisKind::InternationalAid {
        funding,
        personnel,
    });

    // Mutation surge: requires a disease with strain_generation > 0 AND active infections
    let mutated: Vec<usize> = state.diseases.iter().enumerate()
        .filter(|(i, d)| {
            d.strain_generation > 0
                && state.regions.iter().any(|r| {
                    r.disease_state(*i).map_or(false, |ds| ds.infected > 0.0)
                })
        })
        .map(|(i, _)| i)
        .collect();
    if !mutated.is_empty() {
        let idx = mutated[rng.r#gen::<usize>() % mutated.len()];
        candidates.push(CrisisKind::MutationSurge { disease_idx: idx });
    }

    // Filter out crisis types that are still on cooldown
    candidates.retain(|k| {
        match state.crisis_cooldowns.get(k.tag()) {
            Some(&last_tick) => state.tick.saturating_sub(last_tick) >= CRISIS_TYPE_COOLDOWN,
            None => true,
        }
    });

    if candidates.is_empty() {
        return None;
    }

    let kind = candidates.remove(rng.r#gen::<usize>() % candidates.len());
    Some(build_crisis_event(state, kind))
}

/// Build a CrisisEvent with human-readable text for the given kind.
/// INVARIANT: option_a must ALWAYS be free (cost: None) so the player
/// is never softlocked. Paid options go in option_b.
fn build_crisis_event(state: &GameState, kind: CrisisKind) -> CrisisEvent {
    let tick = state.tick;
    let event = match &kind {
        CrisisKind::SupplyDisruption { medicine_idx } => {
            let med_name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str()).unwrap_or("Unknown");
            let doses = state.medicines.get(*medicine_idx)
                .map(|m| m.doses).unwrap_or(0.0);
            let loss = (doses * 0.5).round();
            CrisisEvent {
                title: "Supply Chain Disruption".into(),
                description: format!(
                    "A logistics failure has compromised the supply chain for {}. \
                     {} doses are at risk of spoilage.",
                    med_name, crate::format_number(loss),
                ),
                option_a: CrisisOption {
                    label: "Accept losses".into(),
                    description: format!("Lose {} doses", crate::format_number(loss)),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Emergency reroute ($300)".into(),
                    description: "Pay $300 to save the supply".into(),
                    cost: Some(CrisisCost { funding: 300.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::LabAccident => {
            CrisisEvent {
                title: "Laboratory Accident".into(),
                description: "A containment breach in your research lab threatens \
                    to destroy the current applied research project.".into(),
                option_a: CrisisOption {
                    label: "Evacuate lab".into(),
                    description: "Lose current applied research progress".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Emergency containment ($200, 3 personnel)".into(),
                    description: "Spend resources to save the project".into(),
                    cost: Some(CrisisCost { funding: 200.0, personnel: 3 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PoliticalPressure { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Political Pressure".into(),
                description: format!(
                    "Public unrest in {} is mounting against the quarantine. \
                     Political leaders demand it be lifted immediately.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Comply — lift quarantine".into(),
                    description: format!("Remove quarantine in {}", region_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Resist ($500)".into(),
                    description: "Pay $500 in political capital to maintain quarantine".into(),
                    cost: Some(CrisisCost { funding: 500.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PersonnelCrisis { amount } => {
            CrisisEvent {
                title: "Staff Burnout".into(),
                description: format!(
                    "Frontline workers are exhausted. {} personnel are threatening to resign \
                     unless conditions improve.",
                    amount,
                ),
                option_a: CrisisOption {
                    label: format!("Accept resignations (−{} personnel)", amount),
                    description: format!("Lose {} personnel permanently", amount),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Retention bonus ($400)".into(),
                    description: "Pay $400 to retain staff".into(),
                    cost: Some(CrisisCost { funding: 400.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::InternationalAid { funding, personnel } => {
            // Both options are free (they give resources, not cost them)
            CrisisEvent {
                title: "International Aid Package".into(),
                description: "The WHO is offering an emergency aid package. \
                    Choose how to allocate the support.".into(),
                option_a: CrisisOption {
                    label: format!("Emergency funding (+${:.0})", funding),
                    description: "Direct financial support".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Personnel support (+{} staff)", personnel),
                    description: "Trained researchers and field workers".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MutationSurge { disease_idx } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            CrisisEvent {
                title: "Mutation Surge".into(),
                description: format!(
                    "{} is undergoing rapid genetic drift. Emergency genomic analysis \
                     could help track the changes.",
                    disease_name,
                ),
                option_a: CrisisOption {
                    label: "Ignore — focus resources elsewhere".into(),
                    description: "No cost, but mutation continues unchecked".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Emergency analysis ($300)".into(),
                    description: "Gain +0.15 knowledge of this pathogen".into(),
                    cost: Some(CrisisCost { funding: 300.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
    };
    // INVARIANT: option_a must always be free so the player is never softlocked.
    debug_assert!(event.option_a.cost.is_none(),
        "Crisis '{}' has a cost on option_a — every crisis must have at least one free option",
        event.title);
    event
}

/// Apply the chosen crisis resolution. Returns a status message.
pub(super) fn resolve_crisis(state: &mut GameState, choice: usize) -> String {
    let crisis = match state.active_crisis.take() {
        Some(c) => c,
        None => return "No active crisis".into(),
    };

    // Record cooldown for this crisis type
    state.crisis_cooldowns.insert(crisis.kind.tag().to_string(), state.tick);

    // Deduct costs generically from the chosen option (affordability was
    // already checked in apply_action before we get here).
    let option = if choice == 0 { &crisis.option_a } else { &crisis.option_b };
    if let Some(cost) = &option.cost {
        state.resources.funding -= cost.funding;
        state.resources.personnel = state.resources.personnel.saturating_sub(cost.personnel);
    }

    match (&crisis.kind, choice) {
        (CrisisKind::SupplyDisruption { medicine_idx }, 0) => {
            // Accept losses: lose 50% of doses
            if let Some(med) = state.medicines.get_mut(*medicine_idx) {
                let lost = (med.doses * 0.5).round();
                med.doses = (med.doses - lost).max(0.0);
                format!("Lost {} doses of {} to supply disruption",
                    crate::format_number(lost), med.name)
            } else {
                "Supply disruption resolved".into()
            }
        }
        (CrisisKind::SupplyDisruption { .. }, _) => {
            "Emergency reroute successful — supply chain restored".into()
        }
        (CrisisKind::LabAccident, 0) => {
            // Evacuate — lose applied research
            state.applied_research = None;
            "Lab evacuated — applied research project lost".into()
        }
        (CrisisKind::LabAccident, _) => {
            "Containment successful — research project saved".into()
        }
        (CrisisKind::PoliticalPressure { region_idx }, 0) => {
            // Comply — lift quarantine
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.quarantine = false;
            }
            format!("Quarantine lifted in {} due to political pressure", region_name)
        }
        (CrisisKind::PoliticalPressure { .. }, _) => {
            "Political pressure resisted — quarantine maintained".into()
        }
        (CrisisKind::PersonnelCrisis { amount }, 0) => {
            // Accept resignations
            state.resources.personnel = state.resources.personnel.saturating_sub(*amount);
            format!("Lost {} personnel to burnout", amount)
        }
        (CrisisKind::PersonnelCrisis { .. }, _) => {
            "Retention bonuses paid — staff morale restored".into()
        }
        (CrisisKind::InternationalAid { funding, .. }, 0) => {
            state.resources.funding += funding;
            format!("Received ${:.0} in emergency funding", funding)
        }
        (CrisisKind::InternationalAid { personnel, .. }, _) => {
            state.resources.personnel += personnel;
            format!("Received {} personnel from international collaboration", personnel)
        }
        (CrisisKind::MutationSurge { .. }, 0) => {
            "Mutation surge ignored — focusing resources elsewhere".into()
        }
        (CrisisKind::MutationSurge { disease_idx }, _) => {
            if let Some(disease) = state.diseases.get_mut(*disease_idx) {
                disease.knowledge = (disease.knowledge + 0.15).min(1.0);
                let name = disease.display_name(*disease_idx);
                format!("Emergency analysis complete — gained knowledge of {}", name)
            } else {
                "Emergency analysis complete".into()
            }
        }
    }
}
