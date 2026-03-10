use rand::Rng;

use crate::state::{
    CrisisCost, CrisisEvent, CrisisKind, CrisisOption, GameEvent, GameState, SimState,
    CRISIS_TYPE_COOLDOWN, SEVERITY_CRIT_THRESHOLD, TICKS_PER_DAY,
};

/// Scale a dollar amount relative to current funding.
/// `fraction` is the target fraction of current funding (e.g., 0.15 = 15%).
/// Result is clamped to [min, max] and rounded to nearest $10.
fn scaled_cost(state: &GameState, fraction: f64, min: f64, max: f64) -> f64 {
    let raw = (state.resources.funding * fraction).clamp(min, max);
    (raw / 10.0).round() * 10.0
}

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

    // Lab accident: requires active applied or basic research
    let has_applied = state.applied_research.is_some();
    let has_basic = state.basic_research.is_some();
    if has_applied || has_basic {
        // If both tracks are running, randomly target one
        let targets_basic = if has_applied && has_basic {
            rng.r#gen::<bool>()
        } else {
            has_basic
        };
        candidates.push(CrisisKind::LabAccident { targets_basic });
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

    // International aid: scales with current funding (15%) and headcount (15%)
    let funding = scaled_cost(state, 0.15, 100.0, 500.0);
    let personnel = ((state.resources.personnel as f64 * 0.15).round() as u32).clamp(2, 8);
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

    // RefugeeWave is triggered deterministically on collapse (see engine/mod.rs),
    // not generated randomly.

    // Data leak: requires any active research (field or applied)
    if !state.field_research.is_empty() || state.applied_research.is_some() {
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

    // Trial shortcut: requires an unlocked medicine that targets a disease it hasn't
    // been trialled against yet. Fast-tracking calibrates the medicine 2 generations
    // behind the current strain (30% efficacy penalty via i32 strain_generations).
    let trial_candidates: Vec<(usize, usize)> = state.medicines.iter().enumerate()
        .filter(|(_, m)| m.unlocked)
        .flat_map(|(m_idx, m)| {
            m.target_diseases.iter()
                .filter(|&&d_idx| !m.tested_against.contains(&d_idx))
                .map(move |&d_idx| (d_idx, m_idx))
        })
        .collect();
    if !trial_candidates.is_empty() {
        let &(d_idx, m_idx) = &trial_candidates[rng.r#gen::<usize>() % trial_candidates.len()];
        candidates.push(CrisisKind::TrialShortcut { disease_idx: d_idx, medicine_idx: m_idx });
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
        let stolen = (state.resources.funding * 0.15).min(500.0).round();
        candidates.push(CrisisKind::CorruptOfficial { stolen });
    }

    // Resource diversion: requires identified disease
    let identified: Vec<usize> = state.diseases.iter().enumerate()
        .filter(|(_, d)| d.detected && d.knowledge > 0.3)
        .map(|(i, _)| i)
        .collect();
    if !identified.is_empty() {
        let idx = identified[rng.r#gen::<usize>() % identified.len()];
        let share_reward = scaled_cost(state, 0.25, 150.0, 800.0);
        let refuse_cost = scaled_cost(state, 0.15, 100.0, 600.0);
        candidates.push(CrisisKind::ResourceDiversion { disease_idx: idx, share_reward, refuse_cost });
    }

    // Exhaustion epidemic: requires hospital_surge active
    let hospitals_active: Vec<usize> = state.policies.iter().enumerate()
        .filter(|(_, p)| p.hospital_surge)
        .map(|(i, _)| i)
        .collect();
    if !hospitals_active.is_empty() {
        let idx = hospitals_active[rng.r#gen::<usize>() % hospitals_active.len()];
        let personnel_loss = ((state.resources.personnel as f64 * 0.15).round() as u32).clamp(2, 5);
        candidates.push(CrisisKind::ExhaustionEpidemic { region_idx: idx, personnel_loss });
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
    if state.resources.political_power < 0.40 && day > 8.0 {
        let cooperate_loss = ((state.resources.personnel as f64 * 0.20).round() as u32).clamp(2, 6);
        candidates.push(CrisisKind::MilitaryTakeover { cooperate_loss });
    }

    // --- Late-game crisis types (day-gated) ---

    // Cult blockade: requires day > 12, deployed medicine exists
    if day > 12.0 && state.medicines.iter().any(|m| m.unlocked && m.doses > 0.0) {
        let non_collapsed: Vec<usize> = state.regions.iter().enumerate()
            .filter(|(_, r)| !r.collapsed)
            .map(|(i, _)| i)
            .collect();
        if !non_collapsed.is_empty() {
            let idx = non_collapsed[rng.r#gen::<usize>() % non_collapsed.len()];
            candidates.push(CrisisKind::CultBlockade { region_idx: idx });
        }
    }

    // Billionaire offer: requires day > 8
    if day > 8.0 {
        let reward = scaled_cost(state, 0.25, 150.0, 500.0);
        let personnel_loss = ((state.resources.personnel as f64 * 0.10).round() as u32).clamp(1, 5);
        candidates.push(CrisisKind::BillionaireOffer { reward, personnel_loss });
    }

    // WHO evacuation: requires day > 10, Europe not collapsed
    let europe_ok = state.regions.iter().any(|r| r.name == "Europe" && !r.collapsed);
    if day > 10.0 && europe_ok {
        let aid_loss = scaled_cost(state, 0.15, 100.0, 500.0);
        candidates.push(CrisisKind::WHOEvacuation { aid_loss });
    }

    // Warlord demand: requires collapsed region
    let collapsed: Vec<usize> = state.regions.iter().enumerate()
        .filter(|(_, r)| r.collapsed)
        .map(|(i, _)| i)
        .collect();
    if !collapsed.is_empty() {
        let idx = collapsed[rng.r#gen::<usize>() % collapsed.len()];
        candidates.push(CrisisKind::WarlordDemand { region_idx: idx });
    }

    // Vaccine dispute: requires day > 15, at least one unlocked medicine
    if day > 15.0 && state.medicines.iter().any(|m| m.unlocked) {
        let neutral_loss = scaled_cost(state, 0.20, 100.0, 700.0);
        let credit_gain = scaled_cost(state, 0.30, 150.0, 800.0);
        candidates.push(CrisisKind::VaccineDispute { neutral_loss, credit_gain });
    }

    // --- Dark comedy events ---

    // Performance review: day 12+ (the board doesn't care about your little pandemic)
    if day > 12.0 {
        candidates.push(CrisisKind::PerformanceReview);
    }

    // Naming rights: day 8+, requires identified disease
    if day > 8.0 {
        let nameable: Vec<usize> = state.diseases.iter().enumerate()
            .filter(|(_, d)| d.detected && d.knowledge > 0.5)
            .map(|(i, _)| i)
            .collect();
        if !nameable.is_empty() {
            let idx = nameable[rng.r#gen::<usize>() % nameable.len()];
            let payout = scaled_cost(state, 0.40, 300.0, 2000.0);
            candidates.push(CrisisKind::NamingRights { disease_idx: idx, payout });
        }
    }

    // Intern's discovery: day 5+
    if day > 5.0 {
        let cost = scaled_cost(state, 0.10, 100.0, 400.0);
        candidates.push(CrisisKind::InternDiscovery { cost });
    }

    // Congressional hearing: day 20+, requires 2+ regions in critical state
    if day > 20.0 {
        let crit_regions = state.regions.iter()
            .filter(|r| !r.collapsed && r.infections.iter().any(|i| i.infected > SEVERITY_CRIT_THRESHOLD))
            .count();
        if crit_regions >= 2 {
            candidates.push(CrisisKind::CongressionalHearing);
        }
    }

    // Ark Protocol: scheduled deterministically in tick() when 2+ regions collapse,
    // not generated randomly.

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
pub(super) fn build_crisis_event(state: &GameState, kind: CrisisKind) -> CrisisEvent {
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
                option_b: {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Emergency reroute (${:.0})", cost),
                        description: "Pay to save the supply".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::LabAccident { targets_basic } => {
            let track = if *targets_basic { "basic" } else { "applied" };
            CrisisEvent {
                title: "Laboratory Accident".into(),
                description: format!(
                    "A containment breach in your research lab threatens \
                    to destroy the current {} research project.", track
                ),
                option_a: CrisisOption {
                    label: "Evacuate lab".into(),
                    description: format!("Lose current {} research progress", track),
                    cost: None,
                },
                option_b: {
                    let cost = scaled_cost(state, 0.10, 80.0, 400.0);
                    CrisisOption {
                        label: format!("Emergency containment (${:.0}, 3 personnel)", cost),
                        description: "Spend resources to save the project".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 3 }),
                    }
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
                option_b: {
                    let cost = scaled_cost(state, 0.25, 150.0, 800.0);
                    CrisisOption {
                        label: format!("Resist (${:.0})", cost),
                        description: "Pay to maintain quarantine".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PersonnelCrisis { amount } => {
            let retention_cost = scaled_cost(state, 0.20, 100.0, 600.0);
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
                option_b: {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Emergency analysis (${:.0})", cost),
                        description: "Gain +0.15 knowledge of this pathogen".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
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
            let survivors = state.regions.get(*from_region)
                .map(|r| r.alive()).unwrap_or(0.0);
            let survivors_m = survivors / 1_000_000.0;
            CrisisEvent {
                title: "REFUGEE CRISIS".into(),
                description: format!(
                    "{} has fallen. {:.0}M survivors are fleeing toward {}. \
                     Disease carriers among them WILL spread infection.",
                    from_name, survivors_m, to_name,
                ),
                option_a: CrisisOption {
                    label: "Open borders".into(),
                    description: format!(
                        "Accept {:.0}M refugees into {}. Population rises, infections spread. But you save lives.",
                        survivors_m, to_name,
                    ),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Close borders (−15% POL)".into(),
                    description: format!(
                        "Seal the borders. Millions die at the gates. {} stays clean. The world watches.",
                        to_name,
                    ),
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
                option_b: {
                    let cost = scaled_cost(state, 0.10, 80.0, 400.0);
                    CrisisOption {
                        label: format!("Confiscate (${:.0})", cost),
                        description: "Seize the drugs — safer, but no treatment for them".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
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
                option_b: {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Press conference (${:.0}, 1 personnel)", cost),
                        description: "Calm the panic, gain +5% POL".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 1 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::TrialShortcut { disease_idx, medicine_idx } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            let med_name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            CrisisEvent {
                title: "Pressure to Skip Trials".into(),
                description: format!(
                    "Politicians are demanding you skip clinical trials for {} ({} treatment). \
                     Fast-tracking clears the medicine for use immediately but at reduced efficacy. \
                     Maintaining standards delays availability but ensures full potency.",
                    disease_name, med_name,
                ),
                option_a: CrisisOption {
                    label: "Maintain standards".into(),
                    description: "No shortcuts — −5% POL for refusing".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Fast-track (+10% POL)".into(),
                    description: "Clear for use at reduced efficacy — public approves the speed".into(),
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
                option_b: {
                    let cost = scaled_cost(state, 0.20, 120.0, 700.0);
                    CrisisOption {
                        label: format!("Education campaign (${:.0})", cost),
                        description: format!("Spend money, gain +5% POL in {}", region_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::CorruptOfficial { stolen } => {
            let stolen = *stolen;
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
        CrisisKind::ResourceDiversion { disease_idx, share_reward, refuse_cost } => {
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
                    label: format!("Share data (+${:.0})", share_reward),
                    description: format!("−0.1 knowledge but receive ${:.0} funding as goodwill", share_reward),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Refuse".into(),
                    description: format!("Keep your data, lose ${:.0} in foreign aid", refuse_cost),
                    cost: Some(CrisisCost { funding: *refuse_cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ExhaustionEpidemic { region_idx, personnel_loss } => {
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
                    label: format!("Push through (−{} personnel)", personnel_loss),
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
        CrisisKind::MilitaryTakeover { cooperate_loss } => {
            CrisisEvent {
                title: "Military Threatens Takeover".into(),
                description: "Generals are threatening to seize control of the pandemic response, \
                    citing your agency's declining public support. You can cooperate and cede some \
                    authority, or resist and fight for independence.".into(),
                option_a: CrisisOption {
                    label: "Cooperate".into(),
                    description: format!("Cede {} personnel to military, gain +15% POL", cooperate_loss),
                    cost: None,
                },
                option_b: {
                    let cost = scaled_cost(state, 0.30, 200.0, 1000.0);
                    CrisisOption {
                        label: format!("Resist (${:.0})", cost),
                        description: "Pay to fight the takeover, keep your team".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }

        // --- Late-game crisis types ---

        CrisisKind::CultBlockade { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Doomsday Cult Blockade".into(),
                description: format!(
                    "A doomsday cult in {} has blockaded supply routes, claiming the pandemic \
                     is divine punishment. They want a global broadcast to spread their message. \
                     You can give them airtime or send in the police.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Negotiate — give them airtime".into(),
                    description: "Deliveries resume, but −8% POL from the broadcast".into(),
                    cost: None,
                },
                option_b: {
                    let cost = scaled_cost(state, 0.20, 120.0, 700.0);
                    CrisisOption {
                        label: format!("Police raid (${:.0}, 2 personnel)", cost),
                        description: "Clear the blockade by force".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 2 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::BillionaireOffer { reward, personnel_loss } => {
            CrisisEvent {
                title: "Billionaire's Generous Offer".into(),
                description: format!(
                    "A tech billionaire offers ${:.0} in emergency funding — but wants \
                    naming rights to every medicine you develop. Your scientists are furious. \
                    The money would save lives. The morale cost might lose them.", reward),
                option_a: CrisisOption {
                    label: "Decline politely".into(),
                    description: "Keep team morale, no funding".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Accept the deal".into(),
                    description: format!("+${:.0} funding, −{} personnel quit in protest", reward, personnel_loss),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::WHOEvacuation { aid_loss } => {
            CrisisEvent {
                title: "WHO Headquarters Evacuated".into(),
                description: "A disease outbreak has forced WHO headquarters in Geneva to evacuate. \
                    Global coordination is collapsing. You can take over coordination (expensive) \
                    or let each region fend for itself.".into(),
                option_a: CrisisOption {
                    label: "Let regions go independent".into(),
                    description: format!("Lose ${:.0} in aid income, −5% POL", aid_loss),
                    cost: None,
                },
                option_b: {
                    let cost = scaled_cost(state, 0.40, 250.0, 1500.0);
                    CrisisOption {
                        label: format!("Take over coordination (${:.0}, 3 personnel)", cost),
                        description: "Expensive, but gain +10% POL and maintain global response".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 3 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::WarlordDemand { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Warlord Seizes Control".into(),
                description: format!(
                    "A general has declared himself ruler of collapsed {}. He demands official \
                     recognition and $500 in tribute. In exchange, he'll allow medical teams \
                     back in. Refusing means the region stays sealed off.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Refuse — maintain principles".into(),
                    description: format!("{} remains sealed, +5% POL", region_name),
                    cost: None,
                },
                option_b: {
                    let cost = scaled_cost(state, 0.25, 150.0, 800.0);
                    CrisisOption {
                        label: format!("Pay tribute (${:.0})", cost),
                        description: format!("Un-collapse {} — medical access restored", region_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::VaccineDispute { neutral_loss, credit_gain } => {
            CrisisEvent {
                title: "Vaccine Credit War".into(),
                description: "Two superpowers both claim credit for your vaccine breakthrough. \
                    They're threatening sanctions against each other — and your agency is caught \
                    in the middle. Credit one, or stay neutral and lose both.".into(),
                option_a: CrisisOption {
                    label: "Stay neutral".into(),
                    description: format!("Both sides angry — −${:.0} in combined aid cuts", neutral_loss),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Credit one side".into(),
                    description: format!("+${:.0} from the winner, −15% POL from the loser's allies", credit_gain),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }

        // --- Dark comedy events ---

        CrisisKind::PerformanceReview => {
            let total_dead: f64 = state.regions.iter().map(|r| r.dead).sum();
            let dead_str = crate::format_number(total_dead);
            CrisisEvent {
                title: "Quarterly Performance Review".into(),
                description: format!(
                    "The N.W.H.O. Board of Directors requires your attendance at the quarterly \
                     performance review. Current global mortality: {}. Agenda items include \
                     KPI alignment, travel reimbursement policy, and the break room coffee situation.",
                    dead_str,
                ),
                option_a: CrisisOption {
                    label: "Attend the review".into(),
                    description: "Lose 1 day of research progress. +5% POL.".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "\"I'm busy.\"".into(),
                    description: "Research continues. −5% POL.".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::NamingRights { disease_idx, payout } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "the pathogen".into());
            CrisisEvent {
                title: "Naming Rights Offer".into(),
                description: format!(
                    "PharmaCorp Global offers ${:.0} for the naming rights to {}. Their proposal: \
                     rename it after the CEO's ex-wife. Their legal team assures you this is \
                     \"standard brand integration practice.\"",
                    payout, disease_name,
                ),
                option_a: CrisisOption {
                    label: "Decline".into(),
                    description: "+3% POL.".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Accept (${:.0})", payout),
                    description: "Disease renamed. −5% POL.".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::InternDiscovery { cost } => {
            CrisisEvent {
                title: "The Intern Has a Theory".into(),
                description: format!(
                    "One of your unpaid interns has submitted a 47-page research proposal. \
                     They found it while reorganizing your filing cabinet. \
                     Your lead researcher calls it \"possibly brilliant, probably nonsense.\" \
                     Verification would cost ${:.0}.",
                    cost,
                ),
                option_a: CrisisOption {
                    label: "File it".into(),
                    description: "No effect.".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Investigate (${:.0})", cost),
                    description: "50% chance of a 2-day research breakthrough.".into(),
                    cost: Some(CrisisCost { funding: *cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::CongressionalHearing => {
            CrisisEvent {
                title: "Congressional Subpoena".into(),
                description:
                    "You have been subpoenaed to appear before the Senate Committee on Pandemic \
                     Preparedness and Catering Oversight. Your testimony is expected to take \
                     several days. Attendance is technically mandatory.".into(),
                option_a: CrisisOption {
                    label: "Testify in person".into(),
                    description: "Lose 2 days of all research. +10% POL.".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Send a deputy".into(),
                    description: "+2% POL. 40% chance of contempt charges.".into(),
                    cost: None,
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ContemptOfCongress { fine } => {
            CrisisEvent {
                title: "Contempt of Congress".into(),
                description: format!(
                    "The Senate committee was not satisfied with your deputy's testimony. \
                     You have been held in contempt. Fine: ${:.0}.",
                    fine,
                ),
                option_a: CrisisOption {
                    label: format!("Pay the fine (${:.0})", fine),
                    description: "−8% POL.".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Appeal".into(),
                    description: "Same cost, less political damage. −3% POL.".into(),
                    cost: Some(CrisisCost { funding: *fine, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }

        // --- Follow-up crisis types ---

        CrisisKind::CounterfeitEpidemic { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let crackdown_cost = scaled_cost(state, 0.20, 150.0, 800.0);
            CrisisEvent {
                title: "Counterfeit Medicine Deaths".into(),
                description: format!(
                    "The black market drugs you tolerated in {} have spawned a counterfeit industry. \
                     Fake medicines are killing patients. The knockoffs look identical to real treatments.",
                    region_name,
                ),
                option_a: CrisisOption {
                    label: "Accept the casualties".into(),
                    description: format!("More deaths in {}, but save resources for the real fight", region_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Crackdown (${:.0}, 2 personnel)", crackdown_cost),
                    description: "Raid supply chains and shut down counterfeiters".into(),
                    cost: Some(CrisisCost { funding: crackdown_cost, personnel: 2 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::EmbezzlementRing { stolen_per_day } => {
            let total_stolen = stolen_per_day * 4.0;
            let purge_cost = ((state.resources.personnel as f64 * 0.20).round() as u32).clamp(3, 6);
            let buyoff = scaled_cost(state, 0.25, 200.0, 1000.0);
            CrisisEvent {
                title: "Embezzlement Ring Uncovered".into(),
                description: format!(
                    "The corrupt official you ignored has recruited allies. A full embezzlement ring \
                     has been draining ${:.0}/day from the pandemic fund. Total losses: ${:.0}.",
                    stolen_per_day, total_stolen,
                ),
                option_a: CrisisOption {
                    label: format!("Purge the department (−{} personnel)", purge_cost),
                    description: "Fire everyone involved — stops the bleeding but guts your team".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Buy them off (${:.0})", buyoff),
                    description: "Pay to make the problem go away — they keep what they stole".into(),
                    cost: Some(CrisisCost { funding: buyoff, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MilitaryOverreach => {
            let resist_cost = scaled_cost(state, 0.25, 200.0, 800.0);
            CrisisEvent {
                title: "Military Seizes Research".into(),
                description:
                    "The military you cooperated with is now classifying your pathogen data. \
                     Generals want to weaponize your findings. Civilian researchers are locked out. \
                     You can go to the press or comply.".into(),
                option_a: CrisisOption {
                    label: "Go to the press (−10% POL)".into(),
                    description: "Public scandal forces military to back down, but damages your credibility".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Legal challenge (${:.0})", resist_cost),
                    description: "Fight it in court — expensive but preserves institutional trust".into(),
                    cost: Some(CrisisCost { funding: resist_cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorNationalist { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.20, 150.0, 800.0);
            CrisisEvent {
                title: format!("{} — Sovereignty Dispute", gov_name),
                description: format!(
                    "{} has declared your health mandate unconstitutional in {}. \
                     Local authorities are blocking your field teams.",
                    gov_name, region_name,
                ),
                option_a: CrisisOption {
                    label: "Withdraw teams".into(),
                    description: format!("All restrictive policies disabled in {}", region_name),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Federal override (${:.0})", cost),
                    description: "Maintain operations — governor will resent it".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorPopulist { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.20, 150.0, 800.0);
            CrisisEvent {
                title: format!("{} — General Strike", gov_name),
                description: format!(
                    "{} has called a general strike in {}. Hospital staff are walking out \
                     and citizens are refusing to cooperate with health directives.",
                    gov_name, region_name,
                ),
                option_a: CrisisOption {
                    label: "Let it run its course".into(),
                    description: "Hospital surge disabled, lose 2 personnel".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Address grievances (${:.0})", cost),
                    description: "Pay to end the strike, +10 loyalty".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorTechnocrat { region_idx } => {
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.15, 100.0, 600.0);
            CrisisEvent {
                title: format!("{} — Methodology Review", gov_name),
                description: format!(
                    "{} is demanding an independent review of your research protocols. \
                     They've frozen cooperation until your methodology meets their standards.",
                    gov_name,
                ),
                option_a: CrisisOption {
                    label: "Submit to review".into(),
                    description: "Applied research progress halved".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("Bypass review (${:.0})", cost),
                    description: "Pay consultants to certify your process".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorCooperative { region_idx } => {
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.15, 100.0, 600.0);
            CrisisEvent {
                title: format!("{} — Media Leak", gov_name),
                description: format!(
                    "{} has leaked internal situation reports to the press. \
                     Public confidence in your agency is dropping.",
                    gov_name,
                ),
                option_a: CrisisOption {
                    label: "Accept the fallout".into(),
                    description: "Lose 20% political power".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: format!("PR campaign (${:.0})", cost),
                    description: "Manage the narrative, limit the damage".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ArkProtocol { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let collapsed_count = state.regions.iter().filter(|r| r.collapsed).count();
            CrisisEvent {
                title: "THE ARK PROTOCOL".into(),
                description: format!(
                    "{} regions have fallen. Your remaining teams are spread too thin. \
                     Recommend consolidating all personnel and resources into {} — \
                     abandon all other regions.",
                    collapsed_count, region_name,
                ),
                option_a: CrisisOption {
                    label: format!("Activate — fall back to {}", region_name),
                    description: "Abandon all other regions. Concentrate everything here.".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Decline — fight on all fronts".into(),
                    description: "Maintain scattered operations. Personnel and funding lost to overextension.".into(),
                    cost: Some(CrisisCost { funding: 150.0, personnel: 3 }),
                },
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PublicInquiry => {
            CrisisEvent {
                title: "Cover-Up Exposed".into(),
                description:
                    "The data leak you suppressed has been uncovered by investigative journalists. \
                     \"PANDEMIC AGENCY HIDING RESEARCH FAILURES\" — the headlines are devastating. \
                     A full public inquiry is now demanded.".into(),
                option_a: CrisisOption {
                    label: "Full transparency now".into(),
                    description: "Lose 3 days research progress, gain +10% POL for honesty".into(),
                    cost: None,
                },
                option_b: CrisisOption {
                    label: "Stonewall".into(),
                    description: "Refuse to cooperate — −20% POL, but keep research intact".into(),
                    cost: None,
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

/// Activate a crisis: check for a saved auto-resolve preference and either
/// resolve immediately or pause the game for player input.
/// Called from tick() for both scheduled and randomly generated crises.
pub(super) fn activate_crisis(state: &mut GameState, crisis: CrisisEvent) {
    let auto_choice = state.auto_resolve_crises.get(crisis.kind.tag()).copied();
    let can_auto = match auto_choice {
        Some(choice) => {
            let option = if choice == 0 { &crisis.option_a } else { &crisis.option_b };
            option.cost.as_ref().map_or(true, |c| c.affordable(state))
        }
        None => false,
    };
    state.active_crisis = Some(crisis);
    if can_auto {
        resolve_crisis(state, auto_choice.unwrap());
        state.events.push(GameEvent::CrisisAutoResolved);
    } else {
        state.sim_state = SimState::Event {
            was_running: state.sim_state.is_running(),
        };
        state.events.push(GameEvent::CrisisStarted);
    }
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

    let msg = match (&crisis.kind, choice) {
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
        (CrisisKind::LabAccident { targets_basic }, 0) => {
            if *targets_basic {
                state.basic_research = None;
                "Lab evacuated — basic research project lost".into()
            } else {
                state.applied_research = None;
                "Lab evacuated — applied research project lost".into()
            }
        }
        (CrisisKind::LabAccident { .. }, _) => {
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
            // Open borders — refugees arrive with their diseases.
            // Transfer surviving population and all infections/immune to destination.
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            let to_name = state.regions.get(*to_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            let survivors = state.regions.get(*from_region)
                .map(|r| r.alive()).unwrap_or(0.0);
            // Transfer population
            state.regions[*to_region].population += survivors as u64;
            // Transfer all infected and immune from the collapsed region
            let disease_states: Vec<(usize, f64, f64)> = state.regions.get(*from_region)
                .map(|r| r.infections.iter()
                    .filter(|i| i.infected > 0.0 || i.immune > 0.0)
                    .map(|i| (i.disease_idx, i.infected, i.immune))
                    .collect())
                .unwrap_or_default();
            for (d_idx, infected, immune) in &disease_states {
                let inf = state.regions[*to_region].get_or_create_infection(*d_idx);
                inf.infected += infected;
                inf.immune += immune;
            }
            let survivors_m = survivors / 1_000_000.0;
            format!("{:.0}M refugees from {} accepted into {} — population surging, infections spreading",
                survivors_m, from_name, to_name)
        }
        (CrisisKind::RefugeeWave { from_region, .. }, _) => {
            // Close borders — refugees die at the gates, POL tanks.
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            let survivors = state.regions.get(*from_region)
                .map(|r| r.alive()).unwrap_or(0.0);
            // 20% of refugees perish (added to the collapsed region's death toll)
            let border_deaths = survivors * 0.20;
            state.regions[*from_region].dead += border_deaths;
            state.resources.political_power = (state.resources.political_power - 0.15).max(0.0);
            let deaths_m = border_deaths / 1_000_000.0;
            format!("Borders closed. {:.0}M dead at the gates of {}. The world is horrified.",
                deaths_m, from_name)
        }

        (CrisisKind::DataLeak, 0) => {
            // Go transparent — lose research progress, gain POL
            if let Some(proj) = state.field_research.first_mut() {
                let loss = (2.0 * TICKS_PER_DAY) as f64;
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.applied_research {
                let loss = (2.0 * TICKS_PER_DAY) as f64;
                proj.progress = (proj.progress - loss).max(0.0);
            }
            state.resources.political_power += 0.05;
            "Went transparent — lost research time but gained public trust".into()
        }
        (CrisisKind::DataLeak, _) => {
            // Suppress — lose POL
            state.resources.political_power -= 0.10;
            // Schedule follow-up: public inquiry in 5 days
            let followup_tick = state.tick + (5.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::PublicInquiry));
            "Leak suppressed — research intact but public confidence shaken".into()
        }

        (CrisisKind::BlackMarketMedicine { region_idx }, 0) => {
            // Allow black market — some treated, some harmed
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(region) = state.regions.get_mut(*region_idx) {
                let mut total_harmed = 0.0;
                for inf in &mut region.infections {
                    if inf.infected > 100.0 {
                        let treated = inf.infected * 0.05;
                        let harmed = treated * 0.2;
                        inf.infected -= treated;
                        inf.immune += treated - harmed;
                        inf.dead += harmed;
                        total_harmed += harmed;
                    }
                }
                region.dead += total_harmed;
            }
            // Schedule follow-up: counterfeit epidemic in 5 days
            let followup_tick = state.tick + (5.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::CounterfeitEpidemic { region_idx: *region_idx }));
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
            state.resources.political_power -= 0.15;
            "Military deployed — quarantine maintained by force. International condemnation.".into()
        }

        (CrisisKind::MediaPanic, 0) => {
            // Ignore media — lose POL
            state.resources.political_power -= 0.08;
            "Media panic continues unchecked — public confidence dropping".into()
        }
        (CrisisKind::MediaPanic, _) => {
            // Press conference — gain POL (costs already deducted)
            state.resources.political_power += 0.05;
            "Press conference calmed the panic — confidence restored".into()
        }

        (CrisisKind::TrialShortcut { .. }, 0) => {
            // Maintain standards — lose POL
            state.resources.political_power -= 0.05;
            "Maintained trial standards — public frustrated by the delay".into()
        }
        (CrisisKind::TrialShortcut { disease_idx, medicine_idx }, _) => {
            // Fast-track — gain POL, mark medicine as tested but 2 generations behind
            // current strain (30% efficacy penalty from drift).
            state.resources.political_power += 0.10;
            if let Some(medicine) = state.medicines.get_mut(*medicine_idx) {
                if !medicine.tested_against.contains(disease_idx) {
                    medicine.tested_against.push(*disease_idx);
                }
                // Set strain calibration 2 generations behind current, so the
                // medicine works but at reduced efficacy (~0.70x).
                if let Some(pos) = medicine.target_diseases.iter().position(|&d| d == *disease_idx) {
                    let current_gen = state.diseases.get(*disease_idx)
                        .map_or(0, |d| d.strain_generation) as i32;
                    while medicine.strain_generations.len() <= pos {
                        medicine.strain_generations.push(0);
                    }
                    medicine.strain_generations[pos] = current_gen - 2;
                }
            }
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "the pathogen".into());
            format!("Fast-tracked {} treatment — deployed at reduced efficacy", name)
        }

        (CrisisKind::VaccineHesitancy { .. }, 0) => {
            // Mandate — lose POL
            state.resources.political_power -= 0.10;
            "Vaccine mandate imposed — effective but deeply unpopular".into()
        }
        (CrisisKind::VaccineHesitancy { region_idx }, _) => {
            // Education campaign — costs already deducted, gain POL
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power += 0.05;
            format!("Education campaign in {} — vaccine acceptance improving", region_name)
        }

        (CrisisKind::CorruptOfficial { stolen }, 0) => {
            // Ignore — lose the stolen money (amount locked at generation time)
            state.resources.funding = (state.resources.funding - stolen).max(0.0);
            // Schedule follow-up: embezzlement ring in 4 days
            let daily_drain = (state.resources.funding * 0.05).clamp(20.0, 200.0);
            let followup_tick = state.tick + (4.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::EmbezzlementRing { stolen_per_day: daily_drain }));
            format!("Corruption ignored — ${:.0} lost from the pandemic fund", stolen)
        }
        (CrisisKind::CorruptOfficial { .. }, _) => {
            // Investigate — recover money (personnel cost already deducted)
            "Investigation successful — funds recovered, corrupt official removed".into()
        }

        (CrisisKind::ResourceDiversion { disease_idx, share_reward, .. }, 0) => {
            // Share data — lose knowledge, gain funding
            if let Some(disease) = state.diseases.get_mut(*disease_idx) {
                disease.knowledge = (disease.knowledge - 0.1).max(0.0);
            }
            state.resources.funding += share_reward;
            format!("Research data shared — received ${:.0} in goodwill funding", share_reward)
        }
        (CrisisKind::ResourceDiversion { .. }, _) => {
            // Refuse — costs already deducted
            "Refused to share research — foreign aid reduced".into()
        }

        (CrisisKind::ExhaustionEpidemic { region_idx, .. }, 0) => {
            // Reduce shifts — disable hospital surge
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.hospital_surge = false;
            }
            format!("Hospital surge suspended in {} — staff recovering", region_name)
        }
        (CrisisKind::ExhaustionEpidemic { personnel_loss, .. }, _) => {
            // Push through — lose personnel
            state.resources.personnel = state.resources.personnel.saturating_sub(*personnel_loss);
            format!("Pushed through — {} workers quit permanently from exhaustion", personnel_loss)
        }

        (CrisisKind::WhistleblowerReport { medicine_idx }, 0) => {
            // Halt deployment — destroy doses, gain POL
            if let Some(med) = state.medicines.get_mut(*medicine_idx) {
                let destroyed = (med.doses * 0.3).round();
                med.doses = (med.doses - destroyed).max(0.0);
                state.resources.political_power += 0.05;
                format!("Halted deployment of {} — {} doses destroyed. Public trusts your caution.",
                    med.name, crate::format_number(destroyed))
            } else {
                "Deployment halted".into()
            }
        }
        (CrisisKind::WhistleblowerReport { .. }, _) => {
            // Continue deployment — lose POL
            state.resources.political_power -= 0.08;
            "Continuing deployment despite concerns — public confidence shaken".into()
        }

        (CrisisKind::MilitaryTakeover { cooperate_loss }, 0) => {
            // Cooperate — lose personnel, gain POL
            state.resources.personnel = state.resources.personnel.saturating_sub(*cooperate_loss);
            state.resources.political_power += 0.15;
            // Schedule follow-up: military overreach in 4 days
            let followup_tick = state.tick + (4.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::MilitaryOverreach));
            format!("Ceded {} staff to military — agency retains civilian control with their backing", cooperate_loss)
        }
        (CrisisKind::MilitaryTakeover { .. }, _) => {
            // Resist — costs already deducted
            "Fought off military takeover — independence maintained at great cost".into()
        }

        // --- Late-game crisis resolutions ---

        (CrisisKind::CultBlockade { .. }, 0) => {
            // Negotiate — give them airtime, lose POL
            state.resources.political_power -= 0.08;
            "Cult got their broadcast — deliveries resume, but public is spooked".into()
        }
        (CrisisKind::CultBlockade { .. }, _) => {
            // Police raid — costs already deducted
            "Police cleared the blockade — supply routes restored".into()
        }

        (CrisisKind::BillionaireOffer { .. }, 0) => {
            // Decline
            "Declined the billionaire's offer — team morale intact".into()
        }
        (CrisisKind::BillionaireOffer { reward, personnel_loss }, _) => {
            // Accept — gain funding, lose personnel
            state.resources.funding += reward;
            state.resources.personnel = state.resources.personnel.saturating_sub(*personnel_loss);
            format!("Accepted the deal — ${:.0} received, but {} researchers quit in protest",
                reward, personnel_loss)
        }

        (CrisisKind::WHOEvacuation { aid_loss }, 0) => {
            // Let regions go independent — lose funding and POL
            state.resources.funding = (state.resources.funding - aid_loss).max(0.0);
            state.resources.political_power -= 0.05;
            format!("WHO collapsed — lost ${:.0} in aid. Regions fending for themselves.", aid_loss)
        }
        (CrisisKind::WHOEvacuation { .. }, _) => {
            // Take over — costs already deducted, gain POL
            state.resources.political_power += 0.10;
            "Your agency is now coordinating the global response. Heavy responsibility.".into()
        }

        (CrisisKind::WarlordDemand { region_idx }, 0) => {
            // Refuse — gain POL, region stays collapsed
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power += 0.05;
            format!("Refused the warlord — {} remains sealed off, but your principles are intact", region_name)
        }
        (CrisisKind::WarlordDemand { region_idx }, _) => {
            // Pay tribute — costs already deducted, un-collapse the region
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.collapsed = false;
            }
            format!("Paid the warlord — medical teams re-enter {}", region_name)
        }

        (CrisisKind::VaccineDispute { neutral_loss, .. }, 0) => {
            // Stay neutral — lose funding from both
            state.resources.funding = (state.resources.funding - neutral_loss).max(0.0);
            format!("Stayed neutral — both superpowers cut ${:.0} in aid", neutral_loss)
        }
        (CrisisKind::VaccineDispute { credit_gain, .. }, _) => {
            // Credit one side — gain funding, lose POL
            state.resources.funding += credit_gain;
            state.resources.political_power -= 0.15;
            format!("Picked a side — ${:.0} from the winner, furious allies of the loser", credit_gain)
        }

        // --- Dark comedy event resolutions ---

        (CrisisKind::PerformanceReview, 0) => {
            // Attend — lose 1 day research progress, gain POL
            let loss = TICKS_PER_DAY as f64;
            if let Some(proj) = state.field_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.applied_research {
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.basic_research {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            state.resources.political_power += 0.05;
            "Review complete. Rating: \"Meets Expectations.\"".into()
        }
        (CrisisKind::PerformanceReview, _) => {
            // Skip — lose POL
            state.resources.political_power -= 0.05;
            "Board notes your absence. A memo has been circulated.".into()
        }

        (CrisisKind::NamingRights { disease_idx, payout }, 0) => {
            // Decline — gain POL
            let _ = (disease_idx, payout);
            state.resources.political_power += 0.03;
            "Offer declined.".into()
        }
        (CrisisKind::NamingRights { disease_idx, payout }, _) => {
            // Accept — gain money, lose POL, rename the disease
            state.resources.funding += payout;
            state.resources.political_power -= 0.05;
            let old_name = state.diseases.get(*disease_idx)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| "Unknown".into());
            let names = ["Karen-7", "BrandSynergy-X", "Profit Margin Syndrome", "CEO's Regret"];
            let name_idx = (state.tick as usize) % names.len();
            if let Some(disease) = state.diseases.get_mut(*disease_idx) {
                disease.name = names[name_idx].to_string();
            }
            format!("{} has been officially redesignated as \"{}\". ${:.0} deposited.",
                old_name, names[name_idx], payout)
        }

        (CrisisKind::InternDiscovery { .. }, 0) => {
            // Ignore — nothing happens
            "Proposal filed.".into()
        }
        (CrisisKind::InternDiscovery { .. }, _) => {
            // Pursue — 50/50 gamble (costs already deducted)
            let lucky = state.rng.r#gen::<bool>();
            if lucky {
                let boost = 2.0 * TICKS_PER_DAY as f64;
                if let Some(proj) = &mut state.applied_research {
                    proj.progress += boost;
                } else if let Some(proj) = state.field_research.first_mut() {
                    proj.progress += boost;
                } else if let Some(proj) = &mut state.basic_research {
                    proj.progress += boost;
                }
                "The intern was right. Research accelerated by 2 days.".into()
            } else {
                "The intern was not right.".into()
            }
        }

        (CrisisKind::CongressionalHearing, 0) => {
            // Testify honestly — lose 2 days research, gain POL
            let loss = 2.0 * TICKS_PER_DAY as f64;
            if let Some(proj) = state.field_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            if let Some(proj) = &mut state.applied_research {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            if let Some(proj) = &mut state.basic_research {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            state.resources.political_power += 0.10;
            "Testimony concluded. Committee thanks you for your cooperation.".into()
        }
        (CrisisKind::CongressionalHearing, _) => {
            // Send deputy — small POL gain, 40% chance of contempt follow-up
            state.resources.political_power += 0.02;
            if state.rng.r#gen::<f64>() < 0.40 {
                let followup_tick = state.tick + (3.0 * TICKS_PER_DAY) as u64;
                let fine = scaled_cost(state, 0.15, 200.0, 600.0);
                state.pending_crises.push((followup_tick, CrisisKind::ContemptOfCongress { fine }));
                "Deputy testified. The committee has requested a follow-up session.".into()
            } else {
                "Deputy testified. Committee satisfied.".into()
            }
        }

        (CrisisKind::ContemptOfCongress { fine }, 0) => {
            // Pay fine — lose money and POL
            state.resources.funding = (state.resources.funding - fine).max(0.0);
            state.resources.political_power -= 0.08;
            format!("Fine paid. ${:.0} deducted.", fine)
        }
        (CrisisKind::ContemptOfCongress { .. }, _) => {
            // Fight charges — pay same fine but less POL loss
            state.resources.political_power -= 0.03;
            "Appeal filed. Legal fees applied.".into()
        }

        // --- Follow-up crisis resolutions ---

        (CrisisKind::CounterfeitEpidemic { region_idx }, 0) => {
            // Accept casualties — more deaths in the region
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(region) = state.regions.get_mut(*region_idx) {
                let mut total_killed = 0.0;
                for inf in &mut region.infections {
                    if inf.infected > 100.0 {
                        let killed = inf.infected * 0.10;
                        inf.infected -= killed;
                        inf.dead += killed;
                        total_killed += killed;
                    }
                }
                region.dead += total_killed;
            }
            format!("Counterfeit medicines killing patients in {} — no resources to stop it", region_name)
        }
        (CrisisKind::CounterfeitEpidemic { region_idx }, _) => {
            // Crackdown — costs already deducted
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("Counterfeit drug ring in {} dismantled", region_name)
        }

        (CrisisKind::EmbezzlementRing { .. }, 0) => {
            // Purge department — lose personnel
            let purge = ((state.resources.personnel as f64 * 0.20).round() as u32).clamp(3, 6);
            state.resources.personnel = state.resources.personnel.saturating_sub(purge);
            format!("Department purged — {} staff fired, embezzlement ring broken", purge)
        }
        (CrisisKind::EmbezzlementRing { .. }, _) => {
            // Buy them off — costs already deducted
            "Paid off the embezzlers — they'll stop... for now".into()
        }

        (CrisisKind::MilitaryOverreach, 0) => {
            // Go to the press — lose POL, but research continues
            state.resources.political_power -= 0.10;
            "Went public — military forced to release research data. Your credibility took a hit.".into()
        }
        (CrisisKind::MilitaryOverreach, _) => {
            // Legal challenge — costs already deducted
            "Legal challenge successful — civilian control of research restored".into()
        }

        // --- Governor personality crisis resolutions ---

        (CrisisKind::GovernorNationalist { region_idx }, 0) => {
            // Withdraw teams — disable all restrictive policies in the region
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.travel_ban = false;
                policy.quarantine = false;
                policy.martial_law = false;
                policy.border_controls = false;
            }
            format!("Restrictive policies withdrawn in {} — governor placated", region_name)
        }
        (CrisisKind::GovernorNationalist { .. }, _) => {
            // Federal override — costs already deducted
            "Federal authority imposed — governor forced to comply".into()
        }

        (CrisisKind::GovernorPopulist { region_idx }, 0) => {
            // Let it run — disable hospital surge, lose 2 personnel
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.hospital_surge = false;
            }
            state.resources.personnel = state.resources.personnel.saturating_sub(2);
            format!("Strike in {} — hospital surge suspended, 2 personnel lost", region_name)
        }
        (CrisisKind::GovernorPopulist { region_idx }, _) => {
            // Address grievances — costs already deducted, +10 loyalty
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.loyalty = (region.governor.loyalty + 10.0).min(100.0);
            }
            "Grievances addressed — strike ended, governor loyalty improved".into()
        }

        (CrisisKind::GovernorTechnocrat { .. }, 0) => {
            // Submit to review — halve applied research progress
            if let Some(proj) = &mut state.applied_research {
                proj.progress = (proj.progress * 0.5).max(0.0);
            }
            "Submitted to methodology review — applied research delayed".into()
        }
        (CrisisKind::GovernorTechnocrat { .. }, _) => {
            // Bypass review — costs already deducted
            "Bypassed the review — research continues unimpeded".into()
        }

        (CrisisKind::GovernorCooperative { .. }, 0) => {
            // Accept fallout — lose 20% POL
            state.resources.political_power -= 0.20;
            "Media leak fallout — public confidence dropped".into()
        }
        (CrisisKind::GovernorCooperative { .. }, _) => {
            // PR campaign — costs already deducted
            "PR campaign contained the leak — minimal damage".into()
        }

        (CrisisKind::ArkProtocol { region_idx }, 0) => {
            // Activate Ark Protocol
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.ark_protocol = Some(*region_idx);
            // Deactivate all policies in non-Ark regions
            for (i, policy) in state.policies.iter_mut().enumerate() {
                if i != *region_idx {
                    policy.clear_all();
                }
            }
            state.events.push(GameEvent::ArkProtocolActivated {
                region_idx: *region_idx,
            });
            format!("ARK PROTOCOL ACTIVATED — all resources consolidated in {}", region_name)
        }
        (CrisisKind::ArkProtocol { .. }, _) => {
            // Declined — standard cooldown prevents re-fire
            "Ark Protocol declined — continuing on all fronts".into()
        }

        (CrisisKind::PublicInquiry, 0) => {
            // Full transparency — lose research progress, gain POL
            let loss = (3.0 * TICKS_PER_DAY) as f64;
            if let Some(proj) = state.field_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.applied_research {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            state.resources.political_power += 0.10;
            "Full transparency — lost research time but rebuilt public trust".into()
        }
        (CrisisKind::PublicInquiry, _) => {
            // Stonewall — massive POL loss
            state.resources.political_power -= 0.20;
            "Stonewalled the inquiry — public outrage intensifies".into()
        }
    };
    // Clamp POL after crisis modifications
    state.resources.political_power = state.resources.political_power.clamp(0.0, 1.0);
    // Keep scientist roster in sync with personnel count changes
    state.sync_scientists_to_personnel();
    msg
}
