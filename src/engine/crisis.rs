use rand::Rng;

use crate::state::{
    CrisisCost, CrisisEvent, CrisisKind, CrisisOption, GameEvent, GameState, ScreeningLevel,
    SimState, CRISIS_TYPE_COOLDOWN, SEVERITY_CRIT_THRESHOLD, TICKS_PER_DAY,
};

/// Scale a dollar amount relative to current funding.
/// `fraction` is the target fraction of current funding (e.g., 0.15 = 15%).
/// Result is clamped to [min, max] and rounded to nearest ¥10.
fn scaled_cost(state: &GameState, fraction: f64, min: f64, max: f64) -> f64 {
    let raw = (state.resources.funding * fraction).clamp(min, max);
    (raw / 10.0).round() * 10.0
}

/// POL cost for closing borders on a refugee wave (escalates with each collapse).
fn refugee_pol_cost(wave: u8) -> f64 {
    match wave {
        1 => 0.15,
        2 => 0.20,
        3 => 0.25,
        _ => 0.30,
    }
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
/// INVARIANT: at least one option must be free (cost: None) so the player
/// is never softlocked.
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
                    "Cold chain failure en route. {} doses of {} at risk of spoilage.",
                    crate::format_number(loss), med_name,
                ),
                options: vec![ CrisisOption {
                    label: "Accept losses".into(),
                    description: format!("Lose {} doses", crate::format_number(loss)),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Emergency reroute (¥{:.0})", cost),
                        description: "Rerouted. Full shipment preserved.".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                ],
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
                options: vec![ CrisisOption {
                    label: "Evacuate lab".into(),
                    description: format!("Lose current {} research progress", track),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.10, 80.0, 400.0);
                    CrisisOption {
                        label: format!("Emergency containment (¥{:.0}, 3 personnel)", cost),
                        description: "Breach contained. Research continues.".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 3 }),
                    }
                },
                CrisisOption {
                    label: "Leave it".into(),
                    description: "Risk total loss if breach worsens. 30% chance it self-contains.".into(),
                    cost: None,
                },
                ],
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
                    "The regional administration in {} is threatening to revoke \
                     your quarantine operating authority.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Lift quarantine".into(),
                    description: format!("Remove quarantine in {}", region_name),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.25, 150.0, 800.0);
                    CrisisOption {
                        label: format!("Resist (¥{:.0})", cost),
                        description: "Quarantine holds.".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PersonnelCrisis { amount } => {
            let retention_cost = scaled_cost(state, 0.20, 100.0, 600.0);
            CrisisEvent {
                title: "Personnel Attrition".into(),
                description: format!(
                    "Sustained operational tempo is unsustainable. {} staff have submitted \
                     resignation notices.",
                    amount,
                ),
                options: vec![ CrisisOption {
                    label: format!("Accept resignations (−{} personnel)", amount),
                    description: format!("Lose {} personnel permanently", amount),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Retention bonus (¥{:.0})", retention_cost),
                    description: "Workers stay.".into(),
                    cost: Some(CrisisCost { funding: retention_cost, personnel: 0 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::InternationalAid { funding, personnel } => {
            CrisisEvent {
                title: "Emergency Aid Package".into(),
                description: "The N.W.H.O. emergency reserve has authorized a disbursement.".into(),
                options: vec![ CrisisOption {
                    label: format!("Emergency funding (+¥{:.0})", funding),
                    description: "Direct financial support".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Personnel support (+{} staff)", personnel),
                    description: "Trained researchers and field staff".into(),
                    cost: None,
                },
                ],
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
                options: vec![ CrisisOption {
                    label: "Ignore".into(),
                    description: "No cost, but mutation continues unchecked".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Emergency analysis (¥{:.0})", cost),
                        description: "Current strain sequenced. Pathogen knowledge updated.".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                ],
                kind,
                tick_created: tick,
            }
        }

        // --- New crisis types ---

        CrisisKind::RefugeeWave { from_region, to_region, wave } => {
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let to_name = state.regions.get(*to_region)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let survivors = state.regions.get(*from_region)
                .map(|r| r.alive()).unwrap_or(0.0);
            let survivors_m = survivors / 1_000_000.0;

            let description = match wave {
                1 => format!(
                    "{} has collapsed. {:.0}M survivors are moving toward {}. \
                     Disease carriers among them will spread the outbreak.",
                    from_name, survivors_m, to_name,
                ),
                2 => format!(
                    "A second collapse. {}: {:.0}M survivors en route to {}. \
                     Intake capacity is already strained from the first wave.",
                    from_name, survivors_m, to_name,
                ),
                3 => format!(
                    "{} has collapsed. {:.0}M additional survivors moving toward {}. \
                     Receiving systems are overwhelmed. Three regions down.",
                    from_name, survivors_m, to_name,
                ),
                _ => format!(
                    "{} has fallen. Collapse number {}. \
                     {:.0}M survivors have nowhere to go.",
                    from_name, wave, survivors_m,
                ),
            };

            let pol_cost = refugee_pol_cost(*wave);
            let pol_pct = (pol_cost * 100.0).round() as u32;

            CrisisEvent {
                title: "REFUGEE CRISIS".into(),
                description,
                options: vec![ CrisisOption {
                    label: "Open borders".into(),
                    description: format!(
                        "Accept {:.0}M refugees into {}. Population rises, infections spread.",
                        survivors_m, to_name,
                    ),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Close borders (−{}% POL)", pol_pct),
                    description: if *wave >= 3 {
                        format!(
                            "Seal the borders. {:.0}M die in the open.",
                            survivors_m * 0.20,
                        )
                    } else {
                        format!(
                            "Seal the borders. Millions die at the gates. {} stays clean.",
                            to_name,
                        )
                    },
                    cost: None, // POL cost applied in resolve
                },
                CrisisOption {
                    label: "Limited intake".into(),
                    description: format!(
                        "Accept a fraction of refugees into {}. Some spread, some die at the border.",
                        to_name,
                    ),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::DataLeak => {
            CrisisEvent {
                title: "Research Data Leaked".into(),
                description: "Classified pathogen data has appeared on open networks.".into(),
                options: vec![ CrisisOption {
                    label: "Go transparent".into(),
                    description: "Lose 2 days of research progress, gain +5% POL".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Suppress the leak".into(),
                    description: "Keep research progress, −10% POL".into(),
                    cost: None,
                },
                CrisisOption {
                    label: "No comment".into(),
                    description: "Leak circulates. −7% POL. May trigger misinformation.".into(),
                    cost: None,
                },
                ],
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
                    "Desperate people in {} are buying untested drugs on the black market.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Allow it".into(),
                    description: "Some are treated, but 20% suffer adverse reactions".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.10, 80.0, 400.0);
                    CrisisOption {
                        label: format!("Confiscate (¥{:.0})", cost),
                        description: "Seize the drugs. No treatment available for them.".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                ],
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
                    "Containment perimeter in {} has been breached. \
                     Civil unrest is escalating.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Negotiate".into(),
                    description: format!("Lift quarantine in {}, avoid violence", region_name),
                    cost: None,
                },
                 CrisisOption {
                    label: "Deploy military (−15% POL, 2 personnel)".into(),
                    description: "Maintain quarantine by force".into(),
                    cost: Some(CrisisCost { funding: 0.0, personnel: 2 }),
                },
                CrisisOption {
                    label: "Wait it out".into(),
                    description: "Containment breached temporarily. Riot subsides on its own.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MediaPanic => {
            CrisisEvent {
                title: "Communications Failure".into(),
                description: "Regional reporting systems are returning inconsistent data. \
                    Population-level noncompliance is rising.".into(),
                options: vec![ CrisisOption {
                    label: "Deprioritize".into(),
                    description: "−8% POL as institutional trust degrades".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Restore comms infrastructure (¥{:.0}, 1 personnel)", cost),
                        description: "Stabilize reporting systems, gain +5% POL".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 1 }),
                    }
                },
                ],
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
                    "The board is demanding accelerated deployment of {} ({} treatment). \
                     Trial protocols can be bypassed.",
                    disease_name, med_name,
                ),
                options: vec![ CrisisOption {
                    label: "Maintain standards".into(),
                    description: "−5% POL".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Fast-track (+10% POL)".into(),
                    description: "Clear for use at reduced efficacy".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::VaccineHesitancy { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Treatment Noncompliance".into(),
                description: format!(
                    "Compliance rates in {} have dropped below operational thresholds. \
                     Population is refusing medical directives.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Enforce compliance".into(),
                    description: "Effective but −10% POL".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.20, 120.0, 700.0);
                    CrisisOption {
                        label: format!("Incentive program (¥{:.0})", cost),
                        description: format!("Buy cooperation in {}, gain +5% POL", region_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                CrisisOption {
                    label: "Accept noncompliance".into(),
                    description: format!("Let {} refuse. Reduced treatment coverage.", region_name),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::CorruptOfficial { stolen } => {
            let stolen = *stolen;
            CrisisEvent {
                title: "Corruption Scandal".into(),
                description: format!(
                    "Internal audit flagged ¥{:.0} in unauthorized disbursements. \
                     Investigation will recover the funds but requires diverting staff.",
                    stolen,
                ),
                options: vec![ CrisisOption {
                    label: format!("Ignore it (lose ¥{:.0})", stolen),
                    description: "Write off the loss".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Investigate (2 personnel)".into(),
                    description: format!("Recover ¥{:.0}, divert 2 staff to audit", stolen),
                    cost: Some(CrisisCost { funding: 0.0, personnel: 2 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ResourceDiversion { disease_idx, share_reward, refuse_cost } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            CrisisEvent {
                title: "Research Data Request".into(),
                description: format!(
                    "A member state is demanding access to your sequencing data on {}.",
                    disease_name,
                ),
                options: vec![ CrisisOption {
                    label: format!("Share data (+¥{:.0})", share_reward),
                    description: format!("−0.1 knowledge, receive ¥{:.0}", share_reward),
                    cost: None,
                },
                 CrisisOption {
                    label: "Refuse".into(),
                    description: format!("Keep your data, lose ¥{:.0} in foreign aid", refuse_cost),
                    cost: Some(CrisisCost { funding: *refuse_cost, personnel: 0 }),
                },
                ],
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
                options: vec![ CrisisOption {
                    label: "Reduce shifts".into(),
                    description: format!("Disable hospital surge in {}. Staff recover.", region_name),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Push through (−{} personnel)", personnel_loss),
                    description: "Maintain surge. Some workers quit permanently.".into(),
                    cost: None, // Personnel cost applied in resolve
                },
                CrisisOption {
                    label: "Ignore the warnings".into(),
                    description: "Some staff leave on their own. Surge continues.".into(),
                    cost: None,
                },
                ],
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
                    "A researcher has flagged undisclosed side effects of {}.",
                    med_name,
                ),
                options: vec![ CrisisOption {
                    label: "Halt deployment".into(),
                    description: format!("Destroy 30% of {} doses, gain +5% POL", med_name),
                    cost: None,
                },
                 CrisisOption {
                    label: "Continue deployment".into(),
                    description: "Keep treating patients, −8% POL".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MilitaryTakeover { cooperate_loss } => {
            CrisisEvent {
                title: "Military Threatens Takeover".into(),
                description: "Joint command has issued an ultimatum. They want operational \
                    authority over your agency, citing security concerns.".into(),
                options: vec![ CrisisOption {
                    label: "Cooperate".into(),
                    description: format!("Cede {} personnel to military, gain +15% POL", cooperate_loss),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.30, 200.0, 1000.0);
                    CrisisOption {
                        label: format!("Resist (¥{:.0})", cost),
                        description: "Pay to fight the takeover, keep your team".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                CrisisOption {
                    label: "Stall".into(),
                    description: "Buy time. They may come back.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }

        // --- Late-game crisis types ---

        CrisisKind::CultBlockade { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Supply Route Blockade".into(),
                description: format!(
                    "An organized resistance group in {} has blockaded supply routes. \
                     They're demanding concessions before allowing deliveries to resume.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Grant concessions".into(),
                    description: "Deliveries resume, −8% POL".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.20, 120.0, 700.0);
                    CrisisOption {
                        label: format!("Clear by force (¥{:.0}, 2 personnel)", cost),
                        description: "Enforce access to supply routes".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 2 }),
                    }
                },
                CrisisOption {
                    label: "Wait them out".into(),
                    description: "Supply lines and healthcare degrade while you wait.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::BillionaireOffer { reward, personnel_loss } => {
            CrisisEvent {
                title: "Private Funding Offer".into(),
                description: format!(
                    "A private donor offers ¥{:.0} in exchange for institutional concessions. \
                    Your research staff will not be happy about the terms.", reward),
                options: vec![ CrisisOption {
                    label: "Decline politely".into(),
                    description: "Keep team morale, no funding".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Accept the deal".into(),
                    description: format!("+¥{:.0} funding, −{} personnel quit in protest", reward, personnel_loss),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::WHOEvacuation { aid_loss } => {
            CrisisEvent {
                title: "N.W.H.O. Headquarters Evacuated".into(),
                description: "A containment breach has forced N.W.H.O. headquarters to evacuate. \
                    Global coordination is degrading.".into(),
                options: vec![ CrisisOption {
                    label: "Let regions go independent".into(),
                    description: format!("Lose ¥{:.0} in aid income, −5% POL", aid_loss),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.40, 250.0, 1500.0);
                    CrisisOption {
                        label: format!("Take over coordination (¥{:.0}, 3 personnel)", cost),
                        description: "Expensive, but gain +10% POL and maintain global response".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 3 }),
                    }
                },
                CrisisOption {
                    label: "Do nothing".into(),
                    description: "Wait for N.W.H.O. to regroup. Coordination degrades.".into(),
                    cost: None,
                },
                ],
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
                    "A local commander has seized control of collapsed {}. \
                     He's demanding tribute in exchange for allowing medical access.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Refuse".into(),
                    description: format!("{} remains sealed, +5% POL", region_name),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.25, 150.0, 800.0);
                    CrisisOption {
                        label: format!("Pay tribute (¥{:.0})", cost),
                        description: format!("Medical access restored in {}", region_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                    }
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::VaccineDispute { neutral_loss, credit_gain } => {
            CrisisEvent {
                title: "Attribution Dispute".into(),
                description: "Two member states both claim credit for your treatment breakthrough. \
                    Both are threatening to cut funding if you don't back their claim.".into(),
                options: vec![ CrisisOption {
                    label: "Stay neutral".into(),
                    description: format!("Both cut funding. −¥{:.0} total.", neutral_loss),
                    cost: None,
                },
                 CrisisOption {
                    label: "Credit one side".into(),
                    description: format!("+¥{:.0} from the winner, −15% POL from the loser's allies", credit_gain),
                    cost: None,
                },
                ],
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
                options: vec![ CrisisOption {
                    label: "Attend the review".into(),
                    description: "Lose 1 day of research progress. +5% POL.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "\"I'm busy.\"".into(),
                    description: "Research continues. −5% POL.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::NamingRights { disease_idx, payout } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "the pathogen".into());
            CrisisEvent {
                title: "Naming Rights".into(),
                description: format!(
                    "A pharmaceutical consortium offers ¥{:.0} for the naming rights to {}. \
                     Their legal team assures you this is \"standard brand integration.\"",
                    payout, disease_name,
                ),
                options: vec![ CrisisOption {
                    label: "Decline".into(),
                    description: "+3% POL.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Accept (¥{:.0})", payout),
                    description: "Disease renamed. −5% POL.".into(),
                    cost: None,
                },
                ],
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
                     Verification would cost ¥{:.0}.",
                    cost,
                ),
                options: vec![ CrisisOption {
                    label: "File it".into(),
                    description: "No effect.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Investigate (¥{:.0})", cost),
                    description: "50% chance of a 2-day research breakthrough.".into(),
                    cost: Some(CrisisCost { funding: *cost, personnel: 0 }),
                },
                ],
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
                options: vec![ CrisisOption {
                    label: "Testify in person".into(),
                    description: "Lose 2 days of all research. +10% POL.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Send a deputy".into(),
                    description: "+2% POL. 40% chance of contempt charges.".into(),
                    cost: None,
                },
                CrisisOption {
                    label: "Ignore the subpoena".into(),
                    description: "Guaranteed contempt charges. −15% POL. Research uninterrupted.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ContemptOfCongress { fine } => {
            CrisisEvent {
                title: "Contempt of Congress".into(),
                description: format!(
                    "The Senate committee was not satisfied with your deputy's testimony. \
                     You have been held in contempt. Fine: ¥{:.0}.",
                    fine,
                ),
                options: vec![ CrisisOption {
                    label: format!("Pay the fine (¥{:.0})", fine),
                    description: "−8% POL.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Appeal".into(),
                    description: "Same cost, less political damage. −3% POL.".into(),
                    cost: Some(CrisisCost { funding: *fine, personnel: 0 }),
                },
                ],
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
                options: vec![ CrisisOption {
                    label: "Accept the casualties".into(),
                    description: format!("More deaths in {}, but save resources for the real fight", region_name),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Crackdown (¥{:.0}, 2 personnel)", crackdown_cost),
                    description: "Raid supply chains and shut down counterfeiters".into(),
                    cost: Some(CrisisCost { funding: crackdown_cost, personnel: 2 }),
                },
                ],
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
                     has been draining ¥{:.0}/day from the pandemic fund. Total losses: ¥{:.0}.",
                    stolen_per_day, total_stolen,
                ),
                options: vec![ CrisisOption {
                    label: format!("Purge the department (−{} personnel)", purge_cost),
                    description: "Fire everyone involved. Stops the bleeding.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Buy them off (¥{:.0})", buyoff),
                    description: "They keep what they stole".into(),
                    cost: Some(CrisisCost { funding: buyoff, personnel: 0 }),
                },
                CrisisOption {
                    label: "Tolerate the drain".into(),
                    description: "Focus on the pandemic. Embezzlement continues.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::MilitaryOverreach => {
            let resist_cost = scaled_cost(state, 0.25, 200.0, 800.0);
            CrisisEvent {
                title: "Research Data Classified".into(),
                description:
                    "The military you cooperated with has classified your pathogen data. \
                     Civilian researchers are locked out of their own findings.".into(),
                options: vec![ CrisisOption {
                    label: "Go public (−10% POL)".into(),
                    description: "Force declassification, but damages institutional credibility".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Legal challenge (¥{:.0})", resist_cost),
                    description: "Expensive but preserves civilian control".into(),
                    cost: Some(CrisisCost { funding: resist_cost, personnel: 0 }),
                },
                CrisisOption {
                    label: "Accept the classification".into(),
                    description: "Lose access to your research data. No cost.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorBuffoon { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.10, 80.0, 400.0);
            CrisisEvent {
                title: format!("{}: False All-Clear", gov_name),
                description: format!(
                    "{gov_name} officially declared the pandemic over in {region_name}. \
                     The announcement is incorrect. Civilians are abandoning health protocols."),
                options: vec![ CrisisOption {
                    label: "Damage control".into(),
                    description: "Lose 1 day research progress correcting the record".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Emergency correction (¥{:.0})", cost),
                    description: "Broadcast a formal correction. Limits behavioral relapse.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorBlowhard { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.10, 80.0, 400.0);
            CrisisEvent {
                title: format!("{}: Baseless Accusations", gov_name),
                description: format!(
                    "{gov_name} publicly accused your agency of incompetence in {region_name}. \
                     The charges are unsubstantiated."),
                options: vec![ CrisisOption {
                    label: "Ignore it".into(),
                    description: "Small POL loss. The noise will die down.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Counter-broadcast (¥{cost:.0})"),
                    description: "Respond publicly. Costs money but shuts them up.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorRecluse { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: format!("{}: Non-Cooperation", gov_name),
                description: format!(
                    "{gov_name} has gone dark. Policy enforcement in {region_name} has stalled. \
                     Field teams are operating without local authorization."),
                options: vec![ CrisisOption {
                    label: "Work around them".into(),
                    description: format!("Policy effectiveness reduced in {} until loyalty recovers", region_name),
                    cost: None,
                },
                 CrisisOption {
                    label: "Send a delegation".into(),
                    description: "Send staff to manage directly. Costs personnel.".into(),
                    cost: Some(CrisisCost { funding: 0.0, personnel: 2 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ContractOffer { .. } => {
            let offer = state.contract_offer.as_ref();
            let patron_name = offer.map(|c| c.patron.as_str()).unwrap_or("Unknown Patron");
            let short_name = patron_name.split(',').next().unwrap_or(patron_name);
            let income_day = offer.map(|c| c.income * TICKS_PER_DAY).unwrap_or(0.0);
            let condition_desc = offer.map(|c| c.condition.description()).unwrap_or_default();
            let source = offer.map(|c| c.source.as_str()).unwrap_or("");
            let contract_name = offer.map(|c| c.name.as_str()).unwrap_or("Contract");

            CrisisEvent {
                title: format!("{}: Proposal", short_name),
                description: format!(
                    "{} is offering a funding contract: {}. \
                     Income: +¥{:.0}/day. \"{}\"\n\
                     Condition: {}",
                    patron_name, contract_name, income_day, source, condition_desc,
                ),
                options: vec![
                    CrisisOption {
                        label: format!("Accept (+¥{:.0}/day)", income_day),
                        description: format!("Sign the {}. Income starts immediately.", contract_name),
                        cost: None,
                    },
                    CrisisOption {
                        label: "Decline".into(),
                        description: format!("Turn down {}. No penalty.", short_name),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PatronDemand { template_id } => {
            let contract = state.contracts.iter()
                .find(|c| c.template_id == *template_id);
            let patron_name = contract.map(|c| c.patron.as_str()).unwrap_or("Unknown Patron");
            let short_name = patron_name.split(',').next().unwrap_or(patron_name);
            let placate_cost = scaled_cost(state, 0.15, 100.0, 600.0);

            let (description, placate_desc) = match template_id {
                // Liang Wei — Shipping Lane Guarantee (forbid travel ban)
                0 => (
                    format!(
                        "{} says three of his container ships are sitting idle. \
                         He wants travel restrictions lifted or he pulls funding.",
                        short_name,
                    ),
                    "Pay his port fees and rerouting costs. Buys time.".to_string(),
                ),
                // Viktor Saldanha — Saldanha Hospitality Fund (forbid quarantine)
                1 => (
                    format!(
                        "{} is on the phone. His lawyers are drafting a withdrawal \
                         over the quarantine measures.",
                        short_name,
                    ),
                    "Cover his property insurance premiums. Buys time.".to_string(),
                ),
                // Ines Caron — Helion Research Partnership (active research)
                2 => (
                    format!(
                        "{} wants to see lab activity. Helion does not pay for empty workbenches.",
                        short_name,
                    ),
                    "Send her a progress report with billable hours. Buys time.".to_string(),
                ),
                // Marcus Holt — Holt Stability Fund (no collapse)
                3 => (
                    format!(
                        "{} is watching the markets. His fund lost 8% this week \
                         and he wants assurances.",
                        short_name,
                    ),
                    "Provide his analysts with regional stability projections. Buys time.".to_string(),
                ),
                // David Okafor — Pinnacle Confidence Fund (max threat 3)
                4 => (
                    format!(
                        "{} says bookings are down 60%. The threat level needs to \
                         come down or he walks.",
                        short_name,
                    ),
                    "Buy ad space through his media channels. Buys time.".to_string(),
                ),
                // Riko Tanaka — Pacific Mutual Actuarial Pact (max deaths)
                5 => (
                    format!(
                        "{} says the actuarial tables are breaking. Pacific Mutual \
                         cannot sustain this payout rate.",
                        short_name,
                    ),
                    "Co-sign a reinsurance arrangement. Buys time.".to_string(),
                ),
                // Margaret Aldridge — Aldridge Equipment Lease (require hospital surge)
                6 => (
                    format!(
                        "{} is not seeing the surge orders she was promised. \
                         Her warehouses are full of equipment nobody is buying.",
                        short_name,
                    ),
                    "Place a partial equipment order from her inventory. Buys time.".to_string(),
                ),
                // Col. Raymond Cross — Aegis Border Contract (require border controls)
                7 => (
                    format!(
                        "{} wants border controls enforced. His personnel are \
                         deployed and billing.",
                        short_name,
                    ),
                    "Pay his standby deployment fees. Buys time.".to_string(),
                ),
                _ => (
                    format!("{} is unhappy with your performance.", short_name),
                    "Make concessions.".to_string(),
                ),
            };

            CrisisEvent {
                title: format!("{}: Demands", short_name),
                description,
                options: vec![
                    CrisisOption {
                        label: format!("Placate (¥{:.0})", placate_cost),
                        description: placate_desc,
                        cost: Some(CrisisCost { funding: placate_cost, personnel: 0 }),
                    },
                    CrisisOption {
                        label: "Refuse".into(),
                        description: format!("{} moves closer to pulling out.", short_name),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorHardliner { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.20, 150.0, 800.0);
            CrisisEvent {
                title: format!("{}: Sovereignty Dispute", gov_name),
                description: format!(
                    "{gov_name} has declared your health mandate unconstitutional in {region_name}. \
                     Local authorities are blocking your field teams."),
                options: vec![ CrisisOption {
                    label: "Withdraw teams".into(),
                    description: format!("All restrictive policies disabled in {region_name}"),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Federal override (¥{cost:.0})"),
                    description: "Maintain operations. Governor will resent it.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                CrisisOption {
                    label: "Ignore the dispute".into(),
                    description: "Patchy policy enforcement. No cost.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorOperative { region_idx } => {
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.15, 100.0, 600.0);
            CrisisEvent {
                title: format!("{}: Financial Misconduct", gov_name),
                description: format!(
                    "{gov_name} is embezzling operational funds in the region. \
                     Your field staff are aware. Inaction signals complicity."),
                options: vec![ CrisisOption {
                    label: "Look the other way".into(),
                    description: "Lose 15% POL. Your staff lose respect.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Audit (¥{:.0})", cost),
                    description: "Trigger a financial audit. Reduces skim rate.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::GovernorMobster { region_idx } => {
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let count = state.regions.get(*region_idx)
                .map(|r| r.governor.bargain_count).unwrap_or(0);
            let demand = 200.0 * 2.0_f64.powi(count as i32);
            CrisisEvent {
                title: format!("{}: Escalating Demands", gov_name),
                description: format!(
                    "{gov_name} wants ¥{demand:.0}. Last time it was less. Next time it will be more."),
                options: vec![ CrisisOption {
                    label: "Refuse".into(),
                    description: "Lose 20% POL. They'll make your life difficult.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Pay ¥{demand:.0}"),
                    description: "They'll leave you alone. For now.".into(),
                    cost: Some(CrisisCost { funding: demand, personnel: 0 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ArkProtocol { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let collapsed_count = state.regions.iter().filter(|r| r.collapsed).count();
            CrisisEvent {
                title: "Emergency Consolidation".into(),
                description: format!(
                    "{} regions lost. Remaining personnel are overextended across {} active sites. \
                     Recommend pulling all operations back to {}.",
                    collapsed_count, 6 - collapsed_count, region_name,
                ),
                options: vec![ CrisisOption {
                    label: format!("Consolidate in {}", region_name),
                    description: "Pull out of all other regions.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Continue as-is".into(),
                    description: "Stay spread thin. Lose personnel and funding to overextension.".into(),
                    cost: Some(CrisisCost { funding: 150.0, personnel: 3 }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PublicInquiry => {
            CrisisEvent {
                title: "Data Suppression Exposed".into(),
                description:
                    "The data leak you suppressed has resurfaced. Your concealment is now \
                     a matter of public record. An independent inquiry has been demanded.".into(),
                options: vec![ CrisisOption {
                    label: "Full transparency now".into(),
                    description: "Lose 3 days research progress, gain +10% POL for honesty".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Stonewall".into(),
                    description: "−20% POL. Research intact.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::Infodemic { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            CrisisEvent {
                title: "Information Cascade".into(),
                description: format!(
                    "The earlier communications failure has cascaded in {region_name}. \
                     Field teams report population is hiding symptoms and refusing screening."),
                options: vec![ CrisisOption {
                    label: "Accept reduced visibility".into(),
                    description: format!("Screening downgraded in {region_name}"),
                    cost: None,
                },
                 CrisisOption {
                    label: "Restore screening infrastructure".into(),
                    description: "Maintain screening, but it takes resources".into(),
                    cost: Some(CrisisCost {
                        funding: scaled_cost(state, 0.15, 100.0, 500.0),
                        personnel: 1,
                    }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::SanctionsThreat { funding_loss } => {
            CrisisEvent {
                title: "Sanctions Threat".into(),
                description:
                    "The member state you sided against in the attribution dispute is retaliating. \
                     They're threatening to freeze your accounts and block supply chains.".into(),
                options: vec![ CrisisOption {
                    label: "Accept sanctions".into(),
                    description: format!("Lose ¥{funding_loss:.0} and −10% political power"),
                    cost: None,
                },
                 CrisisOption {
                    label: "Diplomatic back-channel".into(),
                    description: "Costs resources but preserves trade".into(),
                    cost: Some(CrisisCost {
                        funding: scaled_cost(state, 0.20, 150.0, 600.0),
                        personnel: 2,
                    }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
    };
    // INVARIANT: at least one option must be free so the player is never softlocked.
    debug_assert!(event.options.iter().any(|o| o.cost.is_none()),
        "Crisis '{}' has no free option: every crisis must have at least one",
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
            crisis.options.get(choice)
                .map_or(false, |opt| opt.cost.as_ref().map_or(true, |c| c.affordable(state)))
        }
        None => false,
    };
    state.active_crisis = Some(crisis);
    if can_auto {
        let message = resolve_crisis(state, auto_choice.unwrap());
        state.events.push(GameEvent::CrisisAutoResolved { message });
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
    // Record resolution tick for minimum gap enforcement
    state.last_crisis_resolved_tick = state.tick;

    // Deduct costs generically from the chosen option (affordability was
    // already checked in apply_action before we get here).
    let option = &crisis.options[choice];
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
            "Emergency reroute successful. Supply chain restored.".into()
        }
        (CrisisKind::LabAccident { targets_basic }, 0) => {
            if *targets_basic {
                state.basic_research = None;
                "Lab evacuated. Basic research project lost.".into()
            } else {
                state.applied_research = None;
                "Lab evacuated. Applied research project lost.".into()
            }
        }
        (CrisisKind::LabAccident { .. }, 1) => {
            "Containment successful. Research project saved.".into()
        }
        (CrisisKind::LabAccident { targets_basic }, _) => {
            // Leave it — 70% chance of loss, 30% chance it self-contains
            if state.rng.r#gen::<f64>() < 0.70 {
                // Breach worsens — lose the research anyway
                if *targets_basic {
                    state.basic_research = None;
                    "Breach worsened. Basic research lost.".into()
                } else {
                    state.applied_research = None;
                    "Breach worsened. Applied research lost.".into()
                }
            } else {
                "Breach self-contained. Research intact. Lucky.".into()
            }
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
            "Quarantine authority maintained".into()
        }
        (CrisisKind::PersonnelCrisis { amount }, 0) => {
            state.resources.personnel = state.resources.personnel.saturating_sub(*amount);
            // If personnel drops below what active research requires, cancel the
            // most recent field research — not enough staff to sustain it.
            let research_demand: u32 =
                state.field_research.iter().map(|p| p.personnel_assigned).sum::<u32>()
                + state.applied_research.as_ref().map_or(0, |p| p.personnel_assigned)
                + state.basic_research.as_ref().map_or(0, |p| p.personnel_assigned);
            if research_demand > state.resources.personnel
                && state.field_research.pop().is_some()
            {
                format!("Lost {} personnel. Field research cancelled, insufficient staff.",
                    amount)
            } else {
                format!("Lost {} personnel to attrition", amount)
            }
        }
        (CrisisKind::PersonnelCrisis { .. }, _) => {
            "Retention bonuses paid. Attrition stabilized.".into()
        }
        (CrisisKind::InternationalAid { funding, .. }, 0) => {
            state.resources.funding += funding;
            format!("Received ¥{:.0} in emergency funding", funding)
        }
        (CrisisKind::InternationalAid { personnel, .. }, _) => {
            state.resources.personnel += personnel;
            format!("Received {} personnel from N.W.H.O. reserve", personnel)
        }
        (CrisisKind::MutationSurge { .. }, 0) => {
            "Mutation surge ignored".into()
        }
        (CrisisKind::MutationSurge { disease_idx }, _) => {
            if let Some(disease) = state.diseases.get_mut(*disease_idx) {
                disease.knowledge = (disease.knowledge + 0.15).min(1.0);
                let name = disease.display_name(*disease_idx);
                format!("Emergency analysis complete. Gained knowledge of {}.", name)
            } else {
                "Emergency analysis complete".into()
            }
        }

        // --- New crisis resolutions ---

        (CrisisKind::RefugeeWave { from_region, to_region, .. }, 0) => {
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
            format!("{:.0}M refugees from {} accepted into {}. Population surging, infections spreading.",
                survivors_m, from_name, to_name)
        }
        (CrisisKind::RefugeeWave { from_region, wave, .. }, 1) => {
            // Close borders — refugees die at the gates, POL tanks (scaled by wave).
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            let survivors = state.regions.get(*from_region)
                .map(|r| r.alive()).unwrap_or(0.0);
            // 20% of refugees perish (added to the collapsed region's death toll)
            let border_deaths = survivors * 0.20;
            state.regions[*from_region].dead += border_deaths;
            let pol_cost = refugee_pol_cost(*wave);
            state.resources.political_power = (state.resources.political_power - pol_cost).max(0.0);
            let deaths_m = border_deaths / 1_000_000.0;
            format!("Borders closed. {:.0}M dead at the gates of {}. The world is horrified.",
                deaths_m, from_name)
        }
        (CrisisKind::RefugeeWave { from_region, to_region, wave, .. }, _) => {
            // Limited intake — accept half the refugees, half die at border
            let to_name = state.regions.get(*to_region)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            let survivors = state.regions.get(*from_region)
                .map(|r| r.alive()).unwrap_or(0.0);
            let accepted = survivors * 0.5;
            let rejected = survivors * 0.5;
            let border_deaths = rejected * 0.20;
            // Transfer half the population
            state.regions[*to_region].population += accepted as u64;
            // Transfer half the infections
            let disease_states: Vec<(usize, f64, f64)> = state.regions.get(*from_region)
                .map(|r| r.infections.iter()
                    .filter(|i| i.infected > 0.0 || i.immune > 0.0)
                    .map(|i| (i.disease_idx, i.infected * 0.5, i.immune * 0.5))
                    .collect())
                .unwrap_or_default();
            for (d_idx, infected, immune) in &disease_states {
                let inf = state.regions[*to_region].get_or_create_infection(*d_idx);
                inf.infected += infected;
                inf.immune += immune;
            }
            // Border deaths
            state.regions[*from_region].dead += border_deaths;
            // Smaller POL cost than full closure
            let pol_cost = refugee_pol_cost(*wave) * 0.5;
            state.resources.political_power = (state.resources.political_power - pol_cost).max(0.0);
            let accepted_m = accepted / 1_000_000.0;
            format!("Partial intake: {:.0}M accepted into {}. Rest turned away.", accepted_m, to_name)
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
            "Went transparent. Lost research time, gained public trust.".into()
        }
        (CrisisKind::DataLeak, 1) => {
            // Suppress — lose POL
            state.resources.political_power -= 0.10;
            // Schedule follow-up: public inquiry in 5 days
            let followup_tick = state.tick + (5.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::PublicInquiry));
            "Leak suppressed. Research intact, public confidence shaken.".into()
        }
        (CrisisKind::DataLeak, _) => {
            // No comment — moderate POL loss, 50% chance of follow-up
            state.resources.political_power -= 0.07;
            if state.rng.r#gen::<bool>() {
                let target = state.regions.iter().enumerate()
                    .filter(|(_, r)| !r.collapsed)
                    .max_by(|(_, a), (_, b)| {
                        let a_inf: f64 = a.infections.iter().map(|i| i.infected).sum();
                        let b_inf: f64 = b.infections.iter().map(|i| i.infected).sum();
                        a_inf.partial_cmp(&b_inf).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let followup_tick = state.tick + (6.0 * TICKS_PER_DAY) as u64;
                state.pending_crises.push((followup_tick, CrisisKind::Infodemic { region_idx: target }));
                "No comment. Leak spread. Misinformation taking hold.".into()
            } else {
                "No comment. Leak faded from public attention.".into()
            }
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
            format!("Black market drugs allowed in {}. Some treated, some suffered adverse reactions.", region_name)
        }
        (CrisisKind::BlackMarketMedicine { region_idx }, _) => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("Black market drugs confiscated in {}. No alternative treatment available.", region_name)
        }

        (CrisisKind::QuarantineRiot { region_idx }, 0) => {
            // Negotiate — lift quarantine
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.quarantine = false;
            }
            format!("Quarantine lifted in {} after unrest", region_name)
        }
        (CrisisKind::QuarantineRiot { .. }, 1) => {
            // Deploy military — lose POL (personnel already deducted)
            state.resources.political_power -= 0.15;
            "Military deployed. Quarantine maintained by force.".into()
        }
        (CrisisKind::QuarantineRiot { region_idx }, _) => {
            // Wait it out — quarantine temporarily breached, small POL loss
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power -= 0.03;
            // Temporary quarantine breach — some infection increase
            if let Some(region) = state.regions.get_mut(*region_idx) {
                for inf in &mut region.infections {
                    if inf.infected > 100.0 {
                        inf.infected *= 1.05;
                    }
                }
            }
            format!("Riot in {} subsided. Quarantine temporarily breached.", region_name)
        }

        (CrisisKind::MediaPanic, 0) => {
            // Ignore media — lose POL + schedule infodemic follow-up
            state.resources.political_power -= 0.08;
            // Pick the most-infected non-collapsed region for the infodemic
            let target = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .max_by(|(_, a), (_, b)| {
                    let a_inf: f64 = a.infections.iter().map(|i| i.infected).sum();
                    let b_inf: f64 = b.infections.iter().map(|i| i.infected).sum();
                    a_inf.partial_cmp(&b_inf).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            let followup_tick = state.tick + (4.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::Infodemic { region_idx: target }));
            "Communications degradation spreading. Reporting systems unreliable.".into()
        }
        (CrisisKind::MediaPanic, _) => {
            // Press conference — gain POL (costs already deducted)
            state.resources.political_power += 0.05;
            "Communications infrastructure restored. Reporting stabilized.".into()
        }

        (CrisisKind::TrialShortcut { .. }, 0) => {
            // Maintain standards — lose POL
            state.resources.political_power -= 0.05;
            "Maintained trial standards. Board noted the delay.".into()
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
            format!("Fast-tracked {} treatment. Deployed at reduced efficacy.", name)
        }

        (CrisisKind::VaccineHesitancy { region_idx }, 0) => {
            // Mandate — lose POL + governor loyalty drops + possible nationalist rebellion
            state.resources.political_power -= 0.10;
            let mut governor_rebels = false;
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.loyalty = (region.governor.loyalty - 15.0).max(0.0);
                // If loyalty drops below 30, governor may rebel against federal overreach
                if region.governor.loyalty < 30.0 {
                    governor_rebels = true;
                }
            }
            if governor_rebels {
                let followup_tick = state.tick + (3.0 * TICKS_PER_DAY) as u64;
                state.pending_crises.push((followup_tick, CrisisKind::GovernorHardliner { region_idx: *region_idx }));
                "Compliance enforced. Governor threatening to defy authority.".into()
            } else {
                "Compliance enforced. Effective but deeply resented.".into()
            }
        }
        (CrisisKind::VaccineHesitancy { region_idx }, 1) => {
            // Education campaign — costs already deducted, gain POL
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power += 0.05;
            format!("Incentive program deployed in {}. Compliance rates improving.", region_name)
        }
        (CrisisKind::VaccineHesitancy { region_idx }, _) => {
            // Accept noncompliance — infections spike from untreated spread
            state.resources.political_power -= 0.05;
            if let Some(region) = state.regions.get_mut(*region_idx) {
                for inf in &mut region.infections {
                    if inf.infected > 100.0 {
                        inf.infected *= 1.10;
                    }
                }
            }
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("Noncompliance accepted in {}. Infections spreading unchecked.", region_name)
        }

        (CrisisKind::CorruptOfficial { stolen }, 0) => {
            // Ignore — lose the stolen money (amount locked at generation time)
            state.resources.funding = (state.resources.funding - stolen).max(0.0);
            // Schedule follow-up: embezzlement ring in 4 days
            let daily_drain = (state.resources.funding * 0.05).clamp(20.0, 200.0);
            let followup_tick = state.tick + (4.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::EmbezzlementRing { stolen_per_day: daily_drain }));
            format!("Corruption ignored. ¥{:.0} lost.", stolen)
        }
        (CrisisKind::CorruptOfficial { .. }, _) => {
            // Investigate — recover money (personnel cost already deducted)
            "Investigation successful. Funds recovered, official removed.".into()
        }

        (CrisisKind::ResourceDiversion { disease_idx, share_reward, .. }, 0) => {
            // Share data — lose knowledge, gain funding
            if let Some(disease) = state.diseases.get_mut(*disease_idx) {
                disease.knowledge = (disease.knowledge - 0.1).max(0.0);
            }
            state.resources.funding += share_reward;
            format!("Research data shared. Received ¥{:.0}.", share_reward)
        }
        (CrisisKind::ResourceDiversion { .. }, _) => {
            // Refuse — costs already deducted
            "Refused to share research. Foreign aid reduced.".into()
        }

        (CrisisKind::ExhaustionEpidemic { region_idx, .. }, 0) => {
            // Reduce shifts — disable hospital surge
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.hospital_surge = false;
            }
            format!("Hospital surge suspended in {}. Staff recovering.", region_name)
        }
        (CrisisKind::ExhaustionEpidemic { personnel_loss, .. }, 1) => {
            // Push through — lose personnel
            state.resources.personnel = state.resources.personnel.saturating_sub(*personnel_loss);
            format!("{} workers quit permanently", personnel_loss)
        }
        (CrisisKind::ExhaustionEpidemic { personnel_loss, .. }, _) => {
            // Ignore the warnings — some staff leave on their own (half the loss)
            let partial_loss = (*personnel_loss + 1) / 2; // round up
            state.resources.personnel = state.resources.personnel.saturating_sub(partial_loss);
            format!("{} workers left on their own. Surge continues.", partial_loss)
        }

        (CrisisKind::WhistleblowerReport { medicine_idx }, 0) => {
            // Halt deployment — destroy doses, gain POL
            if let Some(med) = state.medicines.get_mut(*medicine_idx) {
                let destroyed = (med.doses * 0.3).round();
                med.doses = (med.doses - destroyed).max(0.0);
                state.resources.political_power += 0.05;
                format!("Halted deployment of {}. {} doses destroyed.",
                    med.name, crate::format_number(destroyed))
            } else {
                "Deployment halted".into()
            }
        }
        (CrisisKind::WhistleblowerReport { .. }, _) => {
            // Continue deployment — lose POL
            state.resources.political_power -= 0.08;
            "Continuing deployment despite concerns. Public confidence shaken.".into()
        }

        (CrisisKind::MilitaryTakeover { cooperate_loss }, 0) => {
            // Cooperate — lose personnel, gain POL
            state.resources.personnel = state.resources.personnel.saturating_sub(*cooperate_loss);
            state.resources.political_power += 0.15;
            // Schedule follow-up: military overreach in 4 days
            let followup_tick = state.tick + (4.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::MilitaryOverreach));
            format!("Ceded {} staff to military. Agency retains civilian control.", cooperate_loss)
        }
        (CrisisKind::MilitaryTakeover { .. }, 1) => {
            // Resist — costs already deducted
            "Military takeover averted. Independence maintained.".into()
        }
        (CrisisKind::MilitaryTakeover { .. }, _) => {
            // Stall — buy time, they may return
            state.resources.political_power -= 0.05;
            if state.rng.r#gen::<bool>() {
                let followup_tick = state.tick + (3.0 * TICKS_PER_DAY) as u64;
                let cooperate_loss = ((state.resources.personnel as f64 * 0.25).round() as u32).clamp(2, 8);
                state.pending_crises.push((followup_tick, CrisisKind::MilitaryTakeover { cooperate_loss }));
                "Negotiations stalled. Military will return.".into()
            } else {
                "Stalling worked. Military backed down.".into()
            }
        }

        // --- Late-game crisis resolutions ---

        (CrisisKind::CultBlockade { .. }, 0) => {
            // Negotiate — give them airtime, lose POL
            state.resources.political_power -= 0.08;
            "Concessions granted. Deliveries resume.".into()
        }
        (CrisisKind::CultBlockade { .. }, 1) => {
            // Police raid — costs already deducted
            "Blockade cleared. Supply routes restored.".into()
        }
        (CrisisKind::CultBlockade { region_idx }, _) => {
            // Wait them out — supply lines and healthcare degrade significantly
            state.resources.political_power -= 0.05;
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.healthcare_capacity = (region.healthcare_capacity - 0.10).max(0.0);
                region.supply_lines = (region.supply_lines - 0.15).max(0.0);
            }
            "Blockade dissolved after days of delays. Supply lines degraded.".into()
        }

        (CrisisKind::BillionaireOffer { .. }, 0) => {
            // Decline
            "Offer declined. Team morale intact.".into()
        }
        (CrisisKind::BillionaireOffer { reward, personnel_loss }, _) => {
            // Accept — gain funding, lose personnel, basic research disrupted
            state.resources.funding += reward;
            state.resources.personnel = state.resources.personnel.saturating_sub(*personnel_loss);
            // Billionaire's team redirected basic research priorities — 2 days of progress lost
            if let Some(proj) = &mut state.basic_research {
                let loss = 2.0 * TICKS_PER_DAY as f64;
                proj.progress = (proj.progress - loss).max(0.0);
            }
            format!("Deal accepted. ¥{:.0} received, {} researchers quit.",
                reward, personnel_loss)
        }

        (CrisisKind::WHOEvacuation { aid_loss }, 0) => {
            // Let regions go independent — lose funding and POL
            state.resources.funding = (state.resources.funding - aid_loss).max(0.0);
            state.resources.political_power -= 0.05;
            format!("N.W.H.O. coordination collapsed. Lost ¥{:.0} in aid.", aid_loss)
        }
        (CrisisKind::WHOEvacuation { .. }, 1) => {
            // Take over — costs already deducted, gain POL
            state.resources.political_power += 0.10;
            "Your agency is now coordinating the global response. Heavy responsibility.".into()
        }
        (CrisisKind::WHOEvacuation { aid_loss, .. }, _) => {
            // Do nothing — coordination degrades, lose funding AND 1 personnel wanders off
            state.resources.funding = (state.resources.funding - aid_loss * 0.75).max(0.0);
            state.resources.personnel = state.resources.personnel.saturating_sub(1);
            state.resources.political_power -= 0.03;
            format!("Coordination collapsed during inaction. Lost ¥{:.0}.", aid_loss * 0.75)
        }

        (CrisisKind::WarlordDemand { region_idx }, 0) => {
            // Refuse — gain POL, region stays collapsed
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power += 0.05;
            format!("Refused the warlord. {} remains sealed.", region_name)
        }
        (CrisisKind::WarlordDemand { region_idx }, _) => {
            // Pay tribute — costs already deducted, un-collapse the region
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.collapsed = false;
            }
            format!("Paid the warlord. Medical teams re-enter {}.", region_name)
        }

        (CrisisKind::VaccineDispute { neutral_loss, .. }, 0) => {
            // Stay neutral — lose funding from both
            state.resources.funding = (state.resources.funding - neutral_loss).max(0.0);
            format!("Stayed neutral. Both sides cut ¥{:.0} in aid.", neutral_loss)
        }
        (CrisisKind::VaccineDispute { credit_gain, .. }, _) => {
            // Credit one side — gain funding, lose POL, schedule retaliation
            state.resources.funding += credit_gain;
            state.resources.political_power -= 0.15;
            let sanctions_loss = scaled_cost(state, 0.20, 200.0, 800.0);
            let followup_tick = state.tick + (5.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::SanctionsThreat { funding_loss: sanctions_loss }));
            format!("Picked a side. ¥{:.0} from the winner.", credit_gain)
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
            format!("{} has been officially redesignated as \"{}\". ¥{:.0} deposited.",
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
        (CrisisKind::CongressionalHearing, 1) => {
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
        (CrisisKind::CongressionalHearing, _) => {
            // Ignore the subpoena — guaranteed contempt, big POL loss, research continues
            state.resources.political_power -= 0.15;
            let followup_tick = state.tick + (2.0 * TICKS_PER_DAY) as u64;
            let fine = scaled_cost(state, 0.20, 300.0, 800.0);
            state.pending_crises.push((followup_tick, CrisisKind::ContemptOfCongress { fine }));
            "Subpoena ignored. Contempt charges filed.".into()
        }

        (CrisisKind::ContemptOfCongress { fine }, 0) => {
            // Pay fine — lose money and POL
            state.resources.funding = (state.resources.funding - fine).max(0.0);
            state.resources.political_power -= 0.08;
            format!("Fine paid. ¥{:.0} deducted.", fine)
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
            format!("Counterfeit medicines killing patients in {}", region_name)
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
            format!("Department purged. {} staff fired, embezzlement ring broken.", purge)
        }
        (CrisisKind::EmbezzlementRing { .. }, 1) => {
            // Buy them off — costs already deducted
            "Paid off the embezzlers. They'll stop. For now.".into()
        }
        (CrisisKind::EmbezzlementRing { stolen_per_day, .. }, _) => {
            // Tolerate the drain — ongoing funding loss
            let loss = stolen_per_day * 3.0;
            state.resources.funding = (state.resources.funding - loss).max(0.0);
            format!("Embezzlement tolerated. ¥{:.0} lost. They're still at it.", loss)
        }

        (CrisisKind::MilitaryOverreach, 0) => {
            // Go to the press — lose POL, but research continues
            state.resources.political_power -= 0.10;
            "Went public. Military forced to release research data.".into()
        }
        (CrisisKind::MilitaryOverreach, 1) => {
            // Legal challenge — costs already deducted
            "Legal challenge successful. Civilian control of research restored.".into()
        }
        (CrisisKind::MilitaryOverreach, _) => {
            // Accept classification — lose research progress
            let loss = TICKS_PER_DAY as f64;
            if let Some(proj) = state.field_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.applied_research {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            "Classification accepted. Research data access restricted.".into()
        }

        // --- Governor archetype crisis resolutions ---

        (CrisisKind::GovernorBuffoon { .. }, 0) => {
            // Damage control — lose 1 day research progress cleaning up
            let loss = TICKS_PER_DAY as f64;
            if let Some(proj) = state.field_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            } else if let Some(proj) = &mut state.applied_research {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            "Spent a day undoing the damage. Could have been worse.".into()
        }
        (CrisisKind::GovernorBuffoon { .. }, _) => {
            // Emergency correction — costs already deducted
            "Public correction issued. Damage contained.".into()
        }

        (CrisisKind::GovernorBlowhard { .. }, 0) => {
            // Ignore it — small POL loss, noise dies down
            state.resources.political_power -= 0.05;
            "Ignored the broadcast. The accusations faded.".into()
        }
        (CrisisKind::GovernorBlowhard { .. }, _) => {
            // Counter-broadcast — costs already deducted, gain POL
            state.resources.political_power += 0.03;
            "Counter-broadcast aired. Public sees through the bluster.".into()
        }

        (CrisisKind::GovernorRecluse { .. }, 0) => {
            // Work around them — accept reduced policy effectiveness
            "Operating without local support. Policies less effective until loyalty recovers.".into()
        }
        (CrisisKind::GovernorRecluse { region_idx }, _) => {
            // Send delegation — personnel cost already deducted, +10 loyalty
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.loyalty = (region.governor.loyalty + 10.0).min(100.0);
            }
            "Delegation sent. Governor engaged. Policy enforcement resuming.".into()
        }

        // --- Contract offer resolutions ---

        (CrisisKind::ContractOffer { .. }, 0) => {
            let (_, msg) = super::contracts::accept_contract(state);
            msg.unwrap_or_else(|| "Contract offer expired.".into())
        }
        (CrisisKind::ContractOffer { .. }, _) => {
            let (_, msg) = super::contracts::reject_contract(state);
            msg.unwrap_or_else(|| "Contract offer declined.".into())
        }

        // --- Patron demand resolutions ---

        (CrisisKind::PatronDemand { template_id }, 0) => {
            // Placate — boost satisfaction back toward safe zone (cost already deducted)
            let short_name = if let Some(c) = state.contracts.iter_mut()
                .find(|c| c.template_id == *template_id)
            {
                c.satisfaction = (c.satisfaction + 0.25).min(1.0);
                c.warned = false;
                c.patron.split(',').next().unwrap_or(&c.patron).to_string()
            } else {
                "Patron".to_string()
            };
            format!("{} placated. Contract stable.", short_name)
        }
        (CrisisKind::PatronDemand { template_id }, _) => {
            // Refuse — satisfaction drops sharply
            let short_name = if let Some(c) = state.contracts.iter_mut()
                .find(|c| c.template_id == *template_id)
            {
                c.satisfaction = (c.satisfaction - 0.15).max(0.0);
                c.patron.split(',').next().unwrap_or(&c.patron).to_string()
            } else {
                "Patron".to_string()
            };
            format!("{} rebuffed. Satisfaction dropped.", short_name)
        }

        (CrisisKind::GovernorHardliner { region_idx }, 0) => {
            // Withdraw teams — disable all restrictive policies
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.travel_ban = false;
                policy.quarantine = false;
                policy.martial_law = false;
                policy.border_controls = false;
            }
            format!("Restrictive policies withdrawn in {}", region_name)
        }
        (CrisisKind::GovernorHardliner { .. }, 1) => {
            // Federal override — costs already deducted
            "Federal authority imposed. Governor forced to comply.".into()
        }
        (CrisisKind::GovernorHardliner { region_idx }, _) => {
            // Ignore the dispute — patchy enforcement, small POL loss
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.political_power -= 0.05;
            // Disable only the most aggressive policies
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.quarantine = false;
                policy.martial_law = false;
            }
            format!("Dispute unresolved. Quarantine and martial law dropped in {}", region_name)
        }

        (CrisisKind::GovernorOperative { .. }, 0) => {
            // Look the other way, lose POL
            state.resources.political_power -= 0.15;
            "Turned a blind eye. Your team noticed.".into()
        }
        (CrisisKind::GovernorOperative { region_idx }, _) => {
            // Formal audit — costs already deducted, reduce skim
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.income_skim = (region.governor.income_skim - 0.05).max(0.0);
            }
            "Audit complete. Skim rate reduced. They'll be more careful now.".into()
        }

        (CrisisKind::GovernorMobster { .. }, 0) => {
            // Refuse — lose POL
            state.resources.political_power -= 0.20;
            "Refused to pay. They're making your life difficult.".into()
        }
        (CrisisKind::GovernorMobster { region_idx }, _) => {
            // Pay: costs already deducted, increment bargain count
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.bargain_count += 1;
                region.governor.loyalty = (region.governor.loyalty + 15.0).min(100.0);
            }
            "Paid. They'll be back for more.".into()
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
            format!("Consolidation complete. All operations moved to {}.", region_name)
        }
        (CrisisKind::ArkProtocol { .. }, _) => {
            // Declined — standard cooldown prevents re-fire
            "Consolidation declined. Maintaining all active sites.".into()
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
            "Full transparency. Lost research time, rebuilt public trust.".into()
        }
        (CrisisKind::PublicInquiry, _) => {
            // Stonewall — massive POL loss
            state.resources.political_power -= 0.20;
            "Stonewalled the inquiry. Public outrage intensifies.".into()
        }

        (CrisisKind::Infodemic { region_idx }, 0) => {
            // Accept reduced visibility — downgrade screening by one level
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.screening = match policy.screening {
                    ScreeningLevel::MassRapid => ScreeningLevel::Antigen,
                    ScreeningLevel::Antigen => ScreeningLevel::Basic,
                    ScreeningLevel::Basic | ScreeningLevel::None => ScreeningLevel::None,
                };
            }
            format!("Screening downgraded in {}", region_name)
        }
        (CrisisKind::Infodemic { region_idx }, _) => {
            // Counter-information campaign — costs already deducted, screening maintained
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("Screening infrastructure restored in {}", region_name)
        }

        (CrisisKind::SanctionsThreat { funding_loss }, 0) => {
            // Accept sanctions — lose funding + POL hit
            state.resources.funding = (state.resources.funding - funding_loss).max(0.0);
            state.resources.political_power -= 0.10;
            format!("Sanctions imposed. ¥{:.0} frozen.", funding_loss)
        }
        (CrisisKind::SanctionsThreat { .. }, _) => {
            // Diplomatic back-channel — costs already deducted, trade preserved
            "Back-channel negotiations successful. Sanctions averted.".into()
        }
    };
    // Clamp POL after crisis modifications
    state.resources.political_power = state.resources.political_power.clamp(0.0, 1.0);
    // Restore sim state from Event mode. When the player manually resolves a crisis,
    // sim_state is Event { was_running }. We restore Running or Paused here so callers
    // don't need to know about this hidden rule. In the auto-resolve path (called from
    // activate_crisis() before sim_state is set to Event), this is a no-op.
    if let SimState::Event { was_running } = state.sim_state {
        state.sim_state = if was_running { SimState::Running } else { SimState::Paused };
    }
    msg
}
