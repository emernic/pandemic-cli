use rand::Rng;

use crate::state::{
    CrisisCost, CrisisEvent, CrisisKind, CrisisOption, GameState,
    CRISIS_TYPE_COOLDOWN, TICKS_PER_DAY,
};

/// Generate a crisis event based on current game state. Returns None if no
/// suitable crisis can be generated (e.g., no valid targets for any crisis type).
pub(super) fn generate_crisis(state: &GameState, rng: &mut impl Rng) -> Option<CrisisEvent> {
    let mut candidates: Vec<CrisisKind> = Vec::new();
    let day = state.tick as f64 / TICKS_PER_DAY;

    // --- Original crisis types ---

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
        let amount = 3.max(state.resources.personnel / 5);
        candidates.push(CrisisKind::PersonnelCrisis { amount });
    }

    // International aid: always available
    let funding = 300.0 + (state.tick as f64 * 0.1).min(500.0);
    let personnel = 3 + ((state.tick as f64 * 0.005).min(5.0) as u32);
    candidates.push(CrisisKind::InternationalAid { funding, personnel });

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

    // --- New crisis types ---

    // Refugee wave: requires at least one collapsed region with a non-collapsed neighbor
    let collapsed_with_neighbor: Vec<(usize, usize)> = state.regions.iter().enumerate()
        .filter(|(_, r)| r.collapsed)
        .flat_map(|(i, r)| {
            r.connections.iter()
                .filter(|&&c| !state.regions[c].collapsed)
                .map(move |&c| (i, c))
        })
        .collect();
    if !collapsed_with_neighbor.is_empty() {
        let (from, to) = collapsed_with_neighbor[rng.r#gen::<usize>() % collapsed_with_neighbor.len()];
        candidates.push(CrisisKind::RefugeeWave { from_region: from, to_region: to });
    }

    // Data leak: requires any active research (field or applied)
    if state.field_research.is_some() || state.applied_research.is_some() {
        candidates.push(CrisisKind::DataLeak);
    }

    // Black market medicine: requires detected disease with active infections in non-collapsed region
    let regions_with_infections: Vec<usize> = state.regions.iter().enumerate()
        .filter(|(_, r)| !r.collapsed && r.infections.iter().any(|i| i.infected > 1000.0))
        .map(|(i, _)| i)
        .collect();
    if !regions_with_infections.is_empty() {
        let idx = regions_with_infections[rng.r#gen::<usize>() % regions_with_infections.len()];
        candidates.push(CrisisKind::BlackMarketMedicine { region_idx: idx });
    }

    // Quarantine riot: requires quarantine active somewhere (different from PoliticalPressure)
    if !quarantined.is_empty() && day > 5.0 {
        let idx = quarantined[rng.r#gen::<usize>() % quarantined.len()];
        candidates.push(CrisisKind::QuarantineRiot { region_idx: idx });
    }

    // Media panic: always available after day 3
    if day > 3.0 {
        candidates.push(CrisisKind::MediaPanic);
    }

    // Trial shortcut: requires identified disease (knowledge > 0) without tested medicine
    let identifiable: Vec<usize> = state.diseases.iter().enumerate()
        .filter(|(i, d)| {
            d.detected && d.knowledge > 0.0
                && !state.medicines.iter().any(|m| m.tested_against.contains(i))
        })
        .map(|(i, _)| i)
        .collect();
    if !identifiable.is_empty() {
        let idx = identifiable[rng.r#gen::<usize>() % identifiable.len()];
        candidates.push(CrisisKind::TrialShortcut { disease_idx: idx });
    }

    // Vaccine hesitancy: requires any unlocked medicine
    let regions_non_collapsed: Vec<usize> = state.regions.iter().enumerate()
        .filter(|(_, r)| !r.collapsed)
        .map(|(i, _)| i)
        .collect();
    if state.medicines.iter().any(|m| m.unlocked) && !regions_non_collapsed.is_empty() {
        let idx = regions_non_collapsed[rng.r#gen::<usize>() % regions_non_collapsed.len()];
        candidates.push(CrisisKind::VaccineHesitancy { region_idx: idx });
    }

    // Corrupt official: requires funding > 500
    if state.resources.funding > 500.0 {
        candidates.push(CrisisKind::CorruptOfficial);
    }

    // Resource diversion: requires identified disease
    let identified: Vec<usize> = state.diseases.iter().enumerate()
        .filter(|(_, d)| d.detected && d.knowledge > 0.3)
        .map(|(i, _)| i)
        .collect();
    if !identified.is_empty() {
        let idx = identified[rng.r#gen::<usize>() % identified.len()];
        candidates.push(CrisisKind::ResourceDiversion { disease_idx: idx });
    }

    // Exhaustion epidemic: requires hospital_surge active
    let hospitals_active: Vec<usize> = state.policies.iter().enumerate()
        .filter(|(_, p)| p.hospital_surge)
        .map(|(i, _)| i)
        .collect();
    if !hospitals_active.is_empty() {
        let idx = hospitals_active[rng.r#gen::<usize>() % hospitals_active.len()];
        candidates.push(CrisisKind::ExhaustionEpidemic { region_idx: idx });
    }

    // Whistleblower report: requires medicine that's been deployed (has less than original doses)
    let deployed_meds: Vec<usize> = state.medicines.iter().enumerate()
        .filter(|(_, m)| m.unlocked && m.doses > 0.0 && m.doses < m.max_doses)
        .map(|(i, _)| i)
        .collect();
    if !deployed_meds.is_empty() {
        let idx = deployed_meds[rng.r#gen::<usize>() % deployed_meds.len()];
        candidates.push(CrisisKind::WhistleblowerReport { medicine_idx: idx });
    }

    // Military takeover: requires POL < 40% and day > 8
    if state.resources.political_power < 40.0 && day > 8.0 {
        candidates.push(CrisisKind::MilitaryTakeover);
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
                    description: "Pay to save the supply".into(),
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
                    description: "Pay to maintain quarantine".into(),
                    cost: Some(CrisisCost { funding: 500.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PersonnelCrisis { amount } => {
            let retention_cost = *amount as f64 * 100.0;
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
                    label: format!("Retention bonus (${:.0})", retention_cost),
                    description: "Pay to retain staff".into(),
                    cost: Some(CrisisCost { funding: retention_cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::InternationalAid { funding, personnel } => {
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

        // --- New crisis types ---

        CrisisKind::RefugeeWave { from_region, to_region } => {
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let to_name = state.regions.get(*to_region)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Refugee Crisis".into(),
                description: format!(
                    "Millions are fleeing the collapse of {}. {} is the nearest safe haven, \
                     but refugees may carry infections.",
                    from_name, to_name,
                ),
                option_a: CrisisOption {
                    label: "Open borders".into(),
                    description: format!("Accept refugees — infections spike in {}", to_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Close borders (−10% POL)".into(),
                    description: "Turn refugees away — public backlash".into(),
                    cost: None, // POL cost applied in resolve
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::DataLeak => {
            CrisisEvent {
                title: "Research Data Leaked".into(),
                description: "Classified research data has been leaked to the press. \
                    Going transparent costs time but builds trust. Suppressing it saves time \
                    but erodes public confidence.".into(),
                option_a: CrisisOption {
                    label: "Go transparent".into(),
                    description: "Lose 2 days of research progress, gain +5% POL".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Suppress the leak".into(),
                    description: "Keep research progress, −10% POL".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::BlackMarketMedicine { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Black Market Drugs".into(),
                description: format!(
                    "Desperate people in {} are buying untested drugs on the black market. \
                     Confiscating them saves lives from adverse reactions but leaves them \
                     without any treatment.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Allow it".into(),
                    description: "Some are treated, but 20% suffer adverse reactions".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Confiscate ($200)".into(),
                    description: "Seize the drugs — safer, but no treatment for them".into(),
                    cost: Some(CrisisCost { funding: 200.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::QuarantineRiot { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Quarantine Riots".into(),
                description: format!(
                    "Violent riots have erupted in {} against the quarantine. \
                     The military can restore order, but it will be ugly.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Negotiate — ease quarantine".into(),
                    description: format!("Lift quarantine in {}, avoid violence", region_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Deploy military (−15% POL, 2 personnel)".into(),
                    description: "Maintain quarantine by force".into(),
                    cost: Some(CrisisCost { funding: 0.0, personnel: 2 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MediaPanic => {
            CrisisEvent {
                title: "Media Firestorm".into(),
                description: "Sensationalized news coverage is causing mass panic. \
                    People are hoarding supplies and ignoring health guidelines. \
                    You can hold a press conference to calm things, or focus on the real work.".into(),
                option_a: CrisisOption {
                    label: "Ignore it — focus on the crisis".into(),
                    description: "−8% POL as public confidence erodes".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Press conference ($300, 1 personnel)".into(),
                    description: "Calm the panic, gain +5% POL".into(),
                    cost: Some(CrisisCost { funding: 300.0, personnel: 1 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::TrialShortcut { disease_idx } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            CrisisEvent {
                title: "Pressure to Skip Trials".into(),
                description: format!(
                    "Politicians are demanding you skip clinical trials for {} treatment. \
                     Fast-tracking saves time but the medicine won't be tested. \
                     Maintaining standards keeps people safe but costs lives to the delay.",
                    disease_name,
                ),
                option_a: CrisisOption {
                    label: "Maintain standards".into(),
                    description: "No shortcuts — −5% POL for refusing".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Fast-track (+10% POL)".into(),
                    description: "Skip safety checks — public approves the speed".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::VaccineHesitancy { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Vaccine Hesitancy".into(),
                description: format!(
                    "Anti-vaccine sentiment is spreading in {}. People are refusing treatment. \
                     A mandate is free but authoritarian. An education campaign costs money but \
                     builds trust.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Mandate vaccines".into(),
                    description: "Effective but −10% POL".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Education campaign ($400)".into(),
                    description: format!("Spend money, gain +5% POL in {}", region_name),
                    cost: Some(CrisisCost { funding: 400.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::CorruptOfficial => {
            let stolen = (state.resources.funding * 0.15).min(500.0).round();
            CrisisEvent {
                title: "Corruption Scandal".into(),
                description: format!(
                    "An internal audit reveals an official has been siphoning ${:.0} from \
                     the pandemic response fund. Investigating recovers the money but pulls \
                     staff from the front lines.",
                    stolen,
                ),
                option_a: CrisisOption {
                    label: format!("Ignore it (lose ${:.0})", stolen),
                    description: "Let the theft go — can't spare the staff".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Investigate (2 personnel)".into(),
                    description: format!("Recover ${:.0} but divert staff for a week", stolen),
                    cost: Some(CrisisCost { funding: 0.0, personnel: 2 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ResourceDiversion { disease_idx } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            CrisisEvent {
                title: "Superpower Demands Research".into(),
                description: format!(
                    "A powerful nation is demanding access to your research data on {}. \
                     Sharing helps global coordination but gives away your advantage. \
                     Refusing protects your work but costs foreign aid.",
                    disease_name,
                ),
                option_a: CrisisOption {
                    label: "Share data (+$500)".into(),
                    description: "−0.1 knowledge but receive $500 funding as goodwill".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Refuse".into(),
                    description: "Keep your data, lose $300 in foreign aid".into(),
                    cost: Some(CrisisCost { funding: 300.0, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ExhaustionEpidemic { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Healthcare Worker Collapse".into(),
                description: format!(
                    "Hospital staff in {} are collapsing from overwork. The hospital surge \
                     program is unsustainable at this pace.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Reduce shifts".into(),
                    description: format!("Disable hospital surge in {} — staff recover", region_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Push through (−3 personnel)".into(),
                    description: "Maintain surge — some workers quit permanently".into(),
                    cost: None, // Personnel cost applied in resolve
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::WhistleblowerReport { medicine_idx } => {
            let med_name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Whistleblower: Drug Side Effects".into(),
                description: format!(
                    "A researcher is reporting that {} has unreported side effects. \
                     Halting deployment protects patients but wastes doses. \
                     Continuing saves more lives overall but risks adverse reactions.",
                    med_name,
                ),
                option_a: CrisisOption {
                    label: "Halt deployment".into(),
                    description: format!("Destroy 30% of {} doses, gain +5% POL", med_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Continue deployment".into(),
                    description: "Keep treating patients, −8% POL".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MilitaryTakeover => {
            CrisisEvent {
                title: "Military Threatens Takeover".into(),
                description: "Generals are threatening to seize control of the pandemic response, \
                    citing your agency's declining public support. You can cooperate and cede some \
                    authority, or resist and fight for independence.".into(),
                option_a: CrisisOption {
                    label: "Cooperate".into(),
                    description: "Cede 5 personnel to military, gain +15% POL".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Resist ($600)".into(),
                    description: "Pay to fight the takeover, keep your team".into(),
                    cost: Some(CrisisCost { funding: 600.0, personnel: 0 }),
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
        // --- Original crisis resolutions ---
        (CrisisKind::SupplyDisruption { medicine_idx }, 0) => {
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
            state.applied_research = None;
            "Lab evacuated — applied research project lost".into()
        }
        (CrisisKind::LabAccident, _) => {
            "Containment successful — research project saved".into()
        }
        (CrisisKind::PoliticalPressure { region_idx }, 0) => {
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

        // --- New crisis resolutions ---

        (CrisisKind::RefugeeWave { from_region, to_region }, 0) => {
            // Accept refugees — spread disease from collapsed region to destination
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            let to_name = state.regions.get(*to_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            // Add infections from the collapsed region's diseases
            let disease_states: Vec<(usize, f64)> = state.regions.get(*from_region)
                .map(|r| r.infections.iter()
                    .filter(|i| i.infected > 0.0 || i.dead > 0.0)
                    .map(|i| (i.disease_idx, (i.infected * 0.05).max(100.0)))
                    .collect())
                .unwrap_or_default();
            for (d_idx, infected) in &disease_states {
                let inf = crate::engine::medicine::get_or_create_infection(
                    &mut state.regions[*to_region], *d_idx);
                inf.infected += infected;
            }
            format!("Refugees from {} accepted into {} — infections spreading", from_name, to_name)
        }
        (CrisisKind::RefugeeWave { .. }, _) => {
            // Close borders — lose POL
            state.resources.political_power = (state.resources.political_power - 10.0).max(0.0);
            "Borders closed — refugees turned away. Public outrage.".into()
        }

        (CrisisKind::DataLeak, 0) => {
            // Go transparent — lose research progress, gain POL
            if let Some(proj) = &mut state.field_research {
                let loss = (2.0 * TICKS_PER_DAY) as f64;
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.applied_research {
                let loss = (2.0 * TICKS_PER_DAY) as f64;
                proj.progress = (proj.progress - loss).max(0.0);
            }
            state.resources.political_power = (state.resources.political_power + 5.0).min(100.0);
            "Went transparent — lost research time but gained public trust".into()
        }
        (CrisisKind::DataLeak, _) => {
            // Suppress — lose POL
            state.resources.political_power = (state.resources.political_power - 10.0).max(0.0);
            "Leak suppressed — research intact but public confidence shaken".into()
        }

        (CrisisKind::BlackMarketMedicine { region_idx }, 0) => {
            // Allow black market — some treated, some harmed
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(region) = state.regions.get_mut(*region_idx) {
                for inf in &mut region.infections {
                    if inf.infected > 100.0 {
                        let treated = inf.infected * 0.05;
                        let harmed = treated * 0.2;
                        inf.infected -= treated;
                        inf.immune += treated - harmed;
                        inf.dead += harmed;
                    }
                }
            }
            format!("Black market drugs allowed in {} — some treated, some suffered adverse reactions", region_name)
        }
        (CrisisKind::BlackMarketMedicine { region_idx }, _) => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("Black market drugs confiscated in {} — people left without treatment", region_name)
        }

        (CrisisKind::QuarantineRiot { region_idx }, 0) => {
            // Negotiate — lift quarantine
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.quarantine = false;
            }
            format!("Quarantine lifted in {} after riots — disease may spread", region_name)
        }
        (CrisisKind::QuarantineRiot { .. }, _) => {
            // Deploy military — lose POL (personnel already deducted)
            state.resources.political_power = (state.resources.political_power - 15.0).max(0.0);
            "Military deployed — quarantine maintained by force. International condemnation.".into()
        }

        (CrisisKind::MediaPanic, 0) => {
            // Ignore media — lose POL
            state.resources.political_power = (state.resources.political_power - 8.0).max(0.0);
            "Media panic continues unchecked — public confidence dropping".into()
        }
        (CrisisKind::MediaPanic, _) => {
            // Press conference — gain POL (costs already deducted)
            state.resources.political_power = (state.resources.political_power + 5.0).min(100.0);
            "Press conference calmed the panic — confidence restored".into()
        }

        (CrisisKind::TrialShortcut { .. }, 0) => {
            // Maintain standards — lose POL
            state.resources.political_power = (state.resources.political_power - 5.0).max(0.0);
            "Maintained trial standards — public frustrated by the delay".into()
        }
        (CrisisKind::TrialShortcut { disease_idx }, _) => {
            // Fast-track — gain POL, but medicine stays untested
            state.resources.political_power = (state.resources.political_power + 10.0).min(100.0);
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "the pathogen".into());
            format!("Fast-tracked treatment for {} — public approves, but safety unknown", name)
        }

        (CrisisKind::VaccineHesitancy { .. }, 0) => {
            // Mandate — lose POL
            state.resources.political_power = (state.resources.political_power - 10.0).max(0.0);
            "Vaccine mandate imposed — effective but deeply unpopular".into()
        }
        (CrisisKind::VaccineHesitancy { region_idx }, _) => {
            // Education campaign — costs already deducted, gain POL
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power = (state.resources.political_power + 5.0).min(100.0);
            format!("Education campaign in {} — vaccine acceptance improving", region_name)
        }

        (CrisisKind::CorruptOfficial, 0) => {
            // Ignore — lose the stolen money
            let stolen = (state.resources.funding * 0.15).min(500.0).round();
            state.resources.funding -= stolen;
            format!("Corruption ignored — ${:.0} lost from the pandemic fund", stolen)
        }
        (CrisisKind::CorruptOfficial, _) => {
            // Investigate — recover money (personnel cost already deducted)
            "Investigation successful — funds recovered, corrupt official removed".into()
        }

        (CrisisKind::ResourceDiversion { disease_idx }, 0) => {
            // Share data — lose knowledge, gain funding
            if let Some(disease) = state.diseases.get_mut(*disease_idx) {
                disease.knowledge = (disease.knowledge - 0.1).max(0.0);
            }
            state.resources.funding += 500.0;
            "Research data shared — received $500 in goodwill funding".into()
        }
        (CrisisKind::ResourceDiversion { .. }, _) => {
            // Refuse — costs already deducted
            "Refused to share research — foreign aid reduced".into()
        }

        (CrisisKind::ExhaustionEpidemic { region_idx }, 0) => {
            // Reduce shifts — disable hospital surge
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.hospital_surge = false;
            }
            format!("Hospital surge suspended in {} — staff recovering", region_name)
        }
        (CrisisKind::ExhaustionEpidemic { .. }, _) => {
            // Push through — lose personnel
            state.resources.personnel = state.resources.personnel.saturating_sub(3);
            "Pushed through — 3 workers quit permanently from exhaustion".into()
        }

        (CrisisKind::WhistleblowerReport { medicine_idx }, 0) => {
            // Halt deployment — destroy doses, gain POL
            if let Some(med) = state.medicines.get_mut(*medicine_idx) {
                let destroyed = (med.doses * 0.3).round();
                med.doses = (med.doses - destroyed).max(0.0);
                state.resources.political_power = (state.resources.political_power + 5.0).min(100.0);
                format!("Halted deployment of {} — {} doses destroyed. Public trusts your caution.",
                    med.name, crate::format_number(destroyed))
            } else {
                "Deployment halted".into()
            }
        }
        (CrisisKind::WhistleblowerReport { .. }, _) => {
            // Continue deployment — lose POL
            state.resources.political_power = (state.resources.political_power - 8.0).max(0.0);
            "Continuing deployment despite concerns — public confidence shaken".into()
        }

        (CrisisKind::MilitaryTakeover, 0) => {
            // Cooperate — lose personnel, gain POL
            state.resources.personnel = state.resources.personnel.saturating_sub(5);
            state.resources.political_power = (state.resources.political_power + 15.0).min(100.0);
            "Ceded 5 staff to military — agency retains civilian control with their backing".into()
        }
        (CrisisKind::MilitaryTakeover, _) => {
            // Resist — costs already deducted
            "Fought off military takeover — independence maintained at great cost".into()
        }
    }
}
