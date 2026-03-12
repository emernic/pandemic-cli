use rand::Rng;

use crate::state::{
    ActiveLoan, BoardRole, CorporationSector, CrisisCost, CrisisEvent, CrisisKind, CrisisOption,
    CrisisOperation, GameEvent, GameState, LoanLender, ResearchKind, ResearchTrack, ScreeningLevel,
    SimState, CRISIS_TYPE_COOLDOWN, LOAN_DUE_DAYS, SEVERITY_CRIT_THRESHOLD, TICKS_PER_DAY,
};

/// Scale a dollar amount relative to current funding.
/// `fraction` is the target fraction of current funding (e.g., 0.15 = 15%).
/// Result is clamped to [min, max] and rounded to nearest ¥10.
fn scaled_cost(state: &GameState, fraction: f64, min: f64, max: f64) -> f64 {
    let raw = (state.resources.funding * fraction).clamp(min, max);
    (raw / 10.0).round() * 10.0
}

/// Compute the board funding multiplier from satisfaction.
/// sat 1.0 => 1.2x (generous), sat 0.5 => 1.0x (neutral), sat 0.0 => 0.5x (slashed).
fn board_meeting_funding_multiplier(board_sat: f64) -> f64 {
    // Linear interpolation: 0.0 -> 0.5, 0.5 -> 1.0, 1.0 -> 1.2
    if board_sat >= 0.5 {
        // 0.5..1.0 maps to 1.0..1.2
        1.0 + (board_sat - 0.5) * 0.4
    } else {
        // 0.0..0.5 maps to 0.5..1.0
        0.5 + board_sat
    }
}

/// Compute the per-day funding rate the board will set at this satisfaction level.
/// Used for display in the communiqué text.
fn board_meeting_funding_rate(state: &GameState, board_sat: f64) -> f64 {
    let base_rate = state.funding_income_rate() * TICKS_PER_DAY;
    // Undo current multiplier to get raw base, then apply new multiplier
    let raw_base = if state.board_funding_multiplier > 0.0 {
        base_rate / state.board_funding_multiplier
    } else {
        base_rate
    };
    raw_base * board_meeting_funding_multiplier(board_sat)
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

/// Phase weight for crisis selection. Determines how likely a crisis type is
/// to be selected at a given game day. Returns a weight (higher = more likely).
///
/// Three phases with overlapping transitions:
/// - Early (day 0-30): bureaucratic, political, organizational problems (fade 30-50)
/// - Mid (day 10-70): infrastructure, resource, and escalating pressure (peak 24-50)
/// - Late (day 24+): survival, power struggles, dark comedy (ramp 24-40)
fn phase_weight(tag: &str, day: f64) -> f64 {
    // Smooth ramp: 0 at `start`, 1 at `peak`, stays 1 after peak
    let ramp_up = |start: f64, peak: f64| -> f64 {
        if day < start { 0.0 }
        else if day < peak { (day - start) / (peak - start) }
        else { 1.0 }
    };
    // Smooth fade: 1 before `start`, 0 at `gone`
    let fade_out = |start: f64, gone: f64| -> f64 {
        if day < start { 1.0 }
        else if day < gone { 1.0 - (day - start) / (gone - start) }
        else { 0.1 } // never fully zero — anachronistic crises are rare but possible
    };

    match tag {
        // --- Early-game: bureaucratic/organizational (fade after day 30-50) ---
        "political" | "personnel" | "dataleak" | "corrupt" |
        "media" | "whistleblower" | "hesitancy" | "aid" | "trial"
            => fade_out(30.0, 50.0),

        // Lab accidents are early-mid (fade later, research keeps going)
        "lab" => fade_out(40.0, 60.0),

        // --- Mid-game: escalating pressure (ramp up day 10-24, fade after 50-70) ---
        "supply" | "blackmarket" | "riot" | "mutation" |
        "diversion" | "exhaustion"
            => ramp_up(10.0, 24.0) * fade_out(50.0, 70.0),

        // --- Late-game: survival and power struggles (ramp up day 24-40) ---
        "military" | "cult" | "who_evac" | "warlord" | "vaccine_dispute"
            => ramp_up(24.0, 40.0),

        // Corporate detention: requires a collapsed region, so can't appear before ~day 10,
        // peak late as collapses accumulate. Follow-up has no phase bias (fires on demand).
        "field_team_detained" => ramp_up(15.0, 30.0),

        // --- Dark comedy: ramp in mid-to-late game ---
        // Performance review is funniest when things are falling apart
        "performance_review" => ramp_up(20.0, 36.0),
        // Congressional hearing is a late-game absurdity
        "congress" => ramp_up(36.0, 50.0),
        // Naming rights and intern are mid-game comedy
        "naming_rights" | "intern" => ramp_up(10.0, 20.0) * fade_out(50.0, 70.0),
        // Billionaire can show up mid-to-late
        "billionaire" => ramp_up(16.0, 30.0),

        // Default: no phase bias (follow-ups, governor crises, etc.)
        _ => 1.0,
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

    // International aid: meaningful funding vs modest personnel boost
    let funding = scaled_cost(state, 0.30, 800.0, 1500.0);
    let personnel = ((state.resources.personnel as f64 * 0.15).round() as u32).clamp(3, 5);
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
    if !quarantined.is_empty() && day > 10.0 {
        let idx = quarantined[rng.r#gen::<usize>() % quarantined.len()];
        candidates.push(CrisisKind::QuarantineRiot { region_idx: idx });
    }

    // Media panic: always available after day 6
    if day > 6.0 {
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

    // Exhaustion epidemic: fires when hospitals are overwhelmed by patient volume.
    // Requires: hospitals running (not discouraged) AND significant infection load.
    // Discourage Hospitalization prevents this — fewer patients means no staff burnout.
    let hospitals_active: Vec<usize> = state.policies.iter().enumerate()
        .filter(|(i, p)| {
            !p.discourage_hosp
                && !state.regions[*i].collapsed
                && state.regions[*i].total_infected() > 10_000.0
        })
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

    // Military takeover: requires POL < 40% and day > 16
    if state.resources.board_approval < 0.40 && day > 16.0 {
        let cooperate_loss = ((state.resources.personnel as f64 * 0.20).round() as u32).clamp(2, 6);
        candidates.push(CrisisKind::MilitaryTakeover { cooperate_loss });
    }

    // --- Late-game crisis types (day-gated) ---

    // Cult blockade: requires day > 24, deployed medicine exists
    if day > 24.0 && state.medicines.iter().any(|m| m.unlocked && m.doses > 0.0) {
        let non_collapsed: Vec<usize> = state.regions.iter().enumerate()
            .filter(|(_, r)| !r.collapsed)
            .map(|(i, _)| i)
            .collect();
        if !non_collapsed.is_empty() {
            let idx = non_collapsed[rng.r#gen::<usize>() % non_collapsed.len()];
            candidates.push(CrisisKind::CultBlockade { region_idx: idx });
        }
    }

    // Billionaire offer: requires day > 16
    if day > 16.0 {
        let reward = scaled_cost(state, 0.25, 150.0, 500.0);
        let personnel_loss = ((state.resources.personnel as f64 * 0.10).round() as u32).clamp(1, 5);
        candidates.push(CrisisKind::BillionaireOffer { reward, personnel_loss });
    }

    // WHO evacuation: requires day > 20, Europe not collapsed
    let europe_ok = state.regions.iter().any(|r| r.name == "Europe" && !r.collapsed);
    if day > 20.0 && europe_ok {
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

    // Field team detained: requires collapsed region, non-bankrupt corp there, 4+ personnel
    if !state.corporations.is_empty() && state.resources.personnel >= 4 {
        let viable: Vec<(usize, usize)> = state.regions.iter().enumerate()
            .filter(|(_, r)| r.collapsed)
            .flat_map(|(ridx, _)| {
                state.corporations.iter().enumerate()
                    .filter(move |(_, c)| c.region_idx == ridx && !c.bankrupt)
                    .map(move |(cidx, _)| (ridx, cidx))
            })
            .collect();
        if !viable.is_empty() {
            let (region_idx, corp_idx) = viable[rng.r#gen::<usize>() % viable.len()];
            let fee = scaled_cost(state, 0.22, 150.0, 700.0);
            let team_size = (state.resources.personnel / 8).clamp(2, 4);
            candidates.push(CrisisKind::FieldTeamDetained { region_idx, corp_idx, fee, team_size });
        }
    }

    // Vaccine dispute: requires day > 30, at least one unlocked medicine, at least 2 corps
    if day > 30.0 && state.medicines.iter().any(|m| m.unlocked) {
        let neutral_loss = scaled_cost(state, 0.20, 100.0, 700.0);
        let credit_gain = scaled_cost(state, 0.30, 150.0, 800.0);
        // Prefer biotech corps, fall back to any non-bankrupt corp
        let biotech: Vec<&str> = state.corporations.iter()
            .filter(|c| !c.bankrupt && c.sector == CorporationSector::Biotech)
            .map(|c| c.name.as_str())
            .collect();
        let any_corps: Vec<&str> = state.corporations.iter()
            .filter(|c| !c.bankrupt)
            .map(|c| c.name.as_str())
            .collect();
        let pool = if biotech.len() >= 2 { &biotech } else { &any_corps };
        if pool.len() >= 2 {
            let idx_a = rng.r#gen::<usize>() % pool.len();
            let idx_b = (idx_a + 1 + rng.r#gen::<usize>() % (pool.len() - 1)) % pool.len();
            let corp_a = pool[idx_a].to_string();
            let corp_b = pool[idx_b].to_string();
            candidates.push(CrisisKind::VaccineDispute { neutral_loss, credit_gain, corp_a, corp_b });
        }
    }

    // --- Dark comedy events ---

    // Performance review: day 24+ (the board doesn't care about your little pandemic)
    if day > 24.0 {
        candidates.push(CrisisKind::PerformanceReview);
    }

    // Naming rights: day 16+, requires identified disease
    if day > 16.0 {
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

    // Intern's discovery: day 10+
    if day > 10.0 {
        let cost = scaled_cost(state, 0.10, 100.0, 400.0);
        candidates.push(CrisisKind::InternDiscovery { cost });
    }

    // Congressional hearing: day 40+, requires 2+ regions in critical state
    if day > 40.0 {
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

    // Weighted random selection based on game phase
    let weights: Vec<f64> = candidates.iter()
        .map(|k| phase_weight(k.tag(), day).max(0.01))
        .collect();
    let total: f64 = weights.iter().sum();
    let mut roll = rng.r#gen::<f64>() * total;
    let mut chosen = candidates.len() - 1; // fallback to last
    for (i, w) in weights.iter().enumerate() {
        roll -= w;
        if roll <= 0.0 {
            chosen = i;
            break;
        }
    }
    let kind = candidates.remove(chosen);
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
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                        label: format!("Emergency containment (¥{:.0}, 3 personnel for 2d)", cost),
                        description: "Breach contained. Research continues. Team returns in 2 days.".into(),
                        cost: Some(CrisisCost {
                            funding: cost,
                            personnel: 3,
                            operation_days: Some(2.0),
                            operation_label: Some("Containment Team".to_string()),
                        }),
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
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                    }
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::PersonnelCrisis { amount } => {
            let retention_cost = 500.0;
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
                    cost: Some(CrisisCost { funding: retention_cost, personnel: 0, ..Default::default() }),
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
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                title: "Refugee Crisis".into(),
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
                    label: format!("Close borders (−{}% board approval)", pol_pct),
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
                description: "Classified pathogen sequencing data has surfaced on external networks. Rivals and foreign intelligence are already analyzing it.".into(),
                options: vec![ CrisisOption {
                    label: "Issue a statement (5 personnel for 2d)".into(),
                    description: "Dedicate a response team to manage disclosure. +5% board approval. Staff return in 2 days.".into(),
                    cost: Some(CrisisCost {
                        funding: 0.0,
                        personnel: 5,
                        operation_days: Some(2.0),
                        operation_label: Some("Response Team".to_string()),
                    }),
                },
                 CrisisOption {
                    label: "Suppress the leak".into(),
                    description: "Deny and contain. −10% board approval. Risk of formal inquiry if exposed.".into(),
                    cost: None,
                },
                CrisisOption {
                    label: "No comment".into(),
                    description: "Leak circulates. −7% board approval. 50% chance of media fallout.".into(),
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
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    "Containment perimeter in {} has been breached.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Negotiate".into(),
                    description: format!("Lift quarantine in {}, avoid violence", region_name),
                    cost: None,
                },
                 CrisisOption {
                    label: "Deploy military (−15% board approval, 2 personnel for 2d)".into(),
                    description: "Maintain quarantine by force. Troops return in 2 days.".into(),
                    cost: Some(CrisisCost {
                        funding: 0.0,
                        personnel: 2,
                        operation_days: Some(2.0),
                        operation_label: Some("Security Detail".to_string()),
                    }),
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
                    description: "−8% board approval as institutional trust degrades".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.15, 100.0, 600.0);
                    CrisisOption {
                        label: format!("Restore comms infrastructure (¥{:.0}, 1 personnel for 2d)", cost),
                        description: "Stabilize reporting systems, gain +5% board approval. Tech team returns in 2 days.".into(),
                        cost: Some(CrisisCost {
                            funding: cost,
                            personnel: 1,
                            operation_days: Some(2.0),
                            operation_label: Some("Comms Team".to_string()),
                        }),
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
                    description: "−5% board approval".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Fast-track (+10% board approval)".into(),
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
                    description: "Effective but −10% board approval".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.20, 120.0, 700.0);
                    CrisisOption {
                        label: format!("Incentive program (¥{:.0})", cost),
                        description: format!("Buy cooperation in {}, gain +5% board approval", region_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    label: "Investigate (2 personnel for 3d)".into(),
                    description: format!("Recover ¥{:.0}, divert 2 staff to audit. Auditors return in 3 days.", stolen),
                    cost: Some(CrisisCost {
                        funding: 0.0,
                        personnel: 2,
                        operation_days: Some(3.0),
                        operation_label: Some("Audit Team".to_string()),
                    }),
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
                    description: format!("Transfer sequencing data. Receive ¥{:.0}.", share_reward),
                    cost: None,
                },
                 CrisisOption {
                    label: "Refuse".into(),
                    description: format!("Keep your data, lose ¥{:.0} in foreign aid", refuse_cost),
                    cost: Some(CrisisCost { funding: *refuse_cost, personnel: 0, ..Default::default() }),
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
                    "Hospital staff in {} are collapsing from overwork. \
                     Patient volume is unsustainable at this pace.",
                    region_name,
                ),
                options: vec![ CrisisOption {
                    label: "Discourage hospitalization".into(),
                    description: format!("Tell patients in {} to stay home. Staff recover.", region_name),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Push through (−{} personnel)", personnel_loss),
                    description: "Keep hospitals open. Some workers quit permanently.".into(),
                    cost: None, // Personnel cost applied in resolve
                },
                CrisisOption {
                    label: "Ignore the warnings".into(),
                    description: "Some staff leave on their own. Hospitals stay open.".into(),
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
                    description: format!("Destroy 30% of {} doses, gain +5% board approval", med_name),
                    cost: None,
                },
                 CrisisOption {
                    label: "Continue deployment".into(),
                    description: "Keep treating patients, −8% board approval".into(),
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
                    description: format!("Cede {} personnel to military, gain +15% board approval", cooperate_loss),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.30, 200.0, 1000.0);
                    CrisisOption {
                        label: format!("Resist (¥{:.0})", cost),
                        description: "Pay to fight the takeover, keep your team".into(),
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    description: "Deliveries resume, −8% board approval".into(),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.20, 120.0, 700.0);
                    CrisisOption {
                        label: format!("Clear by force (¥{:.0}, 2 personnel for 2d)", cost),
                        description: "Enforce access to supply routes. Escort returns in 2 days.".into(),
                        cost: Some(CrisisCost {
                            funding: cost,
                            personnel: 2,
                            operation_days: Some(2.0),
                            operation_label: Some("Escort Detail".to_string()),
                        }),
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
                    description: format!("Lose ¥{:.0} in aid income, −5% board approval", aid_loss),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.40, 250.0, 1500.0);
                    CrisisOption {
                        label: format!("Take over coordination (¥{:.0}, 3 personnel for 5d)", cost),
                        description: "Expensive, but gain +10% board approval and maintain global response. Coordination team returns in 5 days.".into(),
                        cost: Some(CrisisCost {
                            funding: cost,
                            personnel: 3,
                            operation_days: Some(5.0),
                            operation_label: Some("Coordination Team".to_string()),
                        }),
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
                    description: format!("{} remains sealed, +5% board approval", region_name),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.25, 150.0, 800.0);
                    CrisisOption {
                        label: format!("Pay tribute (¥{:.0})", cost),
                        description: format!("Medical access restored in {}", region_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                    }
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::VaccineDispute { neutral_loss, credit_gain, corp_a, corp_b } => {
            CrisisEvent {
                title: "Attribution Dispute".into(),
                description: format!(
                    "{corp_a} and {corp_b} are both claiming credit for your treatment breakthrough. \
                     Each is threatening to pull funding unless you back their side."
                ),
                options: vec![ CrisisOption {
                    label: "Stay neutral".into(),
                    description: format!("Both cancel contracts. −¥{:.0} total.", neutral_loss),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Back {corp_a}"),
                    description: format!("+¥{:.0} from {corp_a}. −15% board approval. {corp_b} retaliates.", credit_gain),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Back {corp_b}"),
                    description: format!("+¥{:.0} from {corp_b}. −15% board approval. {corp_a} retaliates.", credit_gain),
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
                    description: "Lose 1 day of research progress. +5% board approval.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "\"I'm busy.\"".into(),
                    description: "Research continues. −5% board approval.".into(),
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
                    "A pharmaceutical consortium offers ¥{:.0} for the naming rights to {}.",
                    payout, disease_name,
                ),
                options: vec![ CrisisOption {
                    label: "Decline".into(),
                    description: "+3% board approval.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Accept (¥{:.0})", payout),
                    description: "Disease renamed. −5% board approval.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::InternDiscovery { cost } => {
            CrisisEvent {
                title: "Unsolicited Research Proposal".into(),
                description: format!(
                    "A junior analyst has submitted a research proposal through internal channels. \
                     Preliminary review is inconclusive. Verification would cost ¥{:.0}.",
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
                    cost: Some(CrisisCost { funding: *cost, personnel: 0, ..Default::default() }),
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
                    description: "Lose 2 days of all research. +10% board approval.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Send a deputy".into(),
                    description: "+2% board approval. 40% chance of contempt charges.".into(),
                    cost: None,
                },
                CrisisOption {
                    label: "Ignore the subpoena".into(),
                    description: "Guaranteed contempt charges. −15% board approval. Research uninterrupted.".into(),
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
                    description: "−8% board approval.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Appeal".into(),
                    description: "Same cost, less reputational damage. −3% board approval.".into(),
                    cost: Some(CrisisCost { funding: *fine, personnel: 0, ..Default::default() }),
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
                    label: format!("Crackdown (¥{:.0}, 2 personnel for 2d)", crackdown_cost),
                    description: "Raid supply chains and shut down counterfeiters. Agents return in 2 days.".into(),
                    cost: Some(CrisisCost {
                        funding: crackdown_cost,
                        personnel: 2,
                        operation_days: Some(2.0),
                        operation_label: Some("Enforcement Team".to_string()),
                    }),
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
                    cost: Some(CrisisCost { funding: buyoff, personnel: 0, ..Default::default() }),
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
                    label: "Release the data (−10% board approval)".into(),
                    description: "Override the restriction. Data restored to research teams.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Legal challenge (¥{:.0})", resist_cost),
                    description: "Expensive but preserves civilian control".into(),
                    cost: Some(CrisisCost { funding: resist_cost, personnel: 0, ..Default::default() }),
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
                    cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    description: "−5% board approval. The noise will die down.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Counter-broadcast (¥{cost:.0})"),
                    description: "Respond publicly. Costs money but shuts them up.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    label: "Send a delegation (2 personnel for 4d)".into(),
                    description: "Send staff to manage directly. Delegation returns in 4 days.".into(),
                    cost: Some(CrisisCost {
                        funding: 0.0,
                        personnel: 2,
                        operation_days: Some(4.0),
                        operation_label: Some("Diplomatic Delegation".to_string()),
                    }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ContractOffer { .. } => {
            let offer = state.contract_offer.as_ref();
            let member_idx = offer.map(|c| c.board_member_idx).unwrap_or(0);
            let member_name = state.board_members.get(member_idx)
                .map(|m| m.name.as_str()).unwrap_or("Board member");
            let income_day = offer.map(|c| c.income * TICKS_PER_DAY).unwrap_or(0.0);
            let condition_desc = offer.map(|c| c.condition.description()).unwrap_or_default();
            let contract_name = offer.map(|c| c.name.as_str()).unwrap_or("Contract");
            let other_count = state.board_members.len().saturating_sub(1);

            CrisisEvent {
                title: format!("{}: Contract Offer", member_name),
                description: format!(
                    "{} is offering a {}.\n\nCondition: {}.\nIncome: +¥{:.0}/day.\n\
                     Accepting will anger the other {} board members.",
                    member_name, contract_name, condition_desc, income_day, other_count,
                ),
                options: vec![
                    CrisisOption {
                        label: format!("Accept (+¥{:.0}/day)", income_day),
                        description: format!(
                            "Sign the {}. {} is pleased, but the rest of the board resents it.",
                            contract_name, member_name,
                        ),
                        cost: None,
                    },
                    CrisisOption {
                        label: "Decline".into(),
                        description: format!("{} will not be happy.", member_name),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ContractDemand { template_id } => {
            let contract = state.contracts.iter()
                .find(|c| c.template_id == *template_id);
            let member_idx = contract.map(|c| c.board_member_idx).unwrap_or(0);
            let member_name = state.board_members.get(member_idx)
                .map(|m| m.name.as_str()).unwrap_or("Board member");
            let condition_desc = contract.map(|c| c.condition.description())
                .unwrap_or_default();
            let placate_cost = scaled_cost(state, 0.15, 100.0, 600.0);

            let description = format!(
                "{} says the contract terms are being violated: {}. \
                 Continued noncompliance will lead to withdrawal.",
                member_name, condition_desc,
            );

            CrisisEvent {
                title: format!("{}: Demands", member_name),
                description,
                options: vec![
                    CrisisOption {
                        label: format!("Placate (¥{:.0})", placate_cost),
                        description: "Pay to buy time. Contract satisfaction recovers.".to_string(),
                        cost: Some(CrisisCost { funding: placate_cost, personnel: 0, ..Default::default() }),
                    },
                    CrisisOption {
                        label: "Refuse".into(),
                        description: format!("{} moves closer to pulling out.", member_name),
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
                    cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    description: "−15% board approval. Your staff lose respect.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Audit (¥{:.0})", cost),
                    description: "Trigger a financial audit. Reduces skim rate.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
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
                    description: "−20% board approval. They'll make your life difficult.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Pay ¥{demand:.0}"),
                    description: "They'll leave you alone. For now.".into(),
                    cost: Some(CrisisCost { funding: demand, personnel: 0, ..Default::default() }),
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
            let active_count = state.regions.iter().enumerate()
                .filter(|(i, r)| !r.collapsed && !state.is_abandoned(*i))
                .count();
            CrisisEvent {
                title: "Emergency Consolidation".into(),
                description: format!(
                    "{} regions lost. Remaining personnel are overextended across {} active sites. \
                     Recommend pulling all operations back to {}.",
                    collapsed_count, active_count, region_name,
                ),
                options: vec![ CrisisOption {
                    label: format!("Consolidate in {}", region_name),
                    description: "Pull out of all other regions.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Continue as-is".into(),
                    description: "Stay spread thin. Lose personnel and funding to overextension.".into(),
                    cost: Some(CrisisCost { funding: 150.0, personnel: 3, ..Default::default() }),
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
                    description: "Lose 3 days research progress, gain +10% board approval for honesty".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "Stonewall".into(),
                    description: "−20% board approval. Research intact.".into(),
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
                    label: "Restore screening infrastructure (1 personnel for 2d)".into(),
                    description: "Maintain screening. Field team returns in 2 days.".into(),
                    cost: Some(CrisisCost {
                        funding: scaled_cost(state, 0.15, 100.0, 500.0),
                        personnel: 1,
                        operation_days: Some(2.0),
                        operation_label: Some("Screening Team".to_string()),
                    }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::SanctionsThreat { funding_loss, corp_name } => {
            CrisisEvent {
                title: "Corporate Retaliation".into(),
                description: format!(
                    "{corp_name} is pulling its contracts. \
                     They're threatening to freeze your accounts and block supply access."
                ),
                options: vec![ CrisisOption {
                    label: "Accept the cuts".into(),
                    description: format!("Lose ¥{funding_loss:.0} and −10% board approval"),
                    cost: None,
                },
                 CrisisOption {
                    label: "Negotiate a settlement (2 personnel for 3d)".into(),
                    description: "Costs resources but preserves contracts. Envoys return in 3 days.".into(),
                    cost: Some(CrisisCost {
                        funding: scaled_cost(state, 0.20, 150.0, 600.0),
                        personnel: 2,
                        operation_days: Some(3.0),
                        operation_label: Some("Diplomatic Envoys".to_string()),
                    }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::LoanOffer { lender_name, lender, amount, daily_interest_rate } => {
            let due_day = state.tick as f64 / TICKS_PER_DAY + LOAN_DUE_DAYS;
            let daily_interest = amount * daily_interest_rate;
            let (lender_type_str, context_line) = match lender {
                LoanLender::Governor { region_idx } => {
                    let region_name = state.regions.get(*region_idx)
                        .map(|r| r.name.as_str()).unwrap_or("Unknown");
                    ("Governor", format!("Governor of {region_name}"))
                }
                LoanLender::Corporation { corp_idx } => {
                    let sector = state.corporations.get(*corp_idx)
                        .map(|c| format!("{:?}", c.sector)).unwrap_or_else(|| "Unknown".into());
                    ("Corporate", sector)
                }
            };
            CrisisEvent {
                title: format!("{lender_type_str} Emergency Loan Offer"),
                description: format!(
                    "{lender_name} ({context_line}) has noticed your funding gap and \
                     is offering an emergency loan of ¥{amount:.0}. \
                     Terms: ¥{daily_interest:.0}/day interest, due by day {due_day:.0}. \
                     If unpaid, they will enforce collection.",
                ),
                options: vec![
                    CrisisOption {
                        label: "Decline".into(),
                        description: "No debt. Policies remain suspended.".into(),
                        cost: None,
                    },
                    CrisisOption {
                        label: format!("Accept ¥{amount:.0} loan (¥{daily_interest:.0}/day interest, due day {due_day:.0})"),
                        description: format!(
                            "Funds restored. Repay ¥{amount:.0}+ by day {due_day:.0} or face retaliation.",
                        ),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::LoanCallIn { lender_name, lender, outstanding } => {
            // Use current outstanding from state.loans — more accurate than the value
            // captured when the crisis was queued (interest has continued accruing since).
            let repay_amount = state.loans.iter()
                .find(|l| l.lender == *lender)
                .map(|l| l.outstanding)
                .unwrap_or(*outstanding);
            let can_repay = state.resources.funding >= repay_amount;
            match lender {
                LoanLender::Governor { region_idx } => {
                    let region_name = state.regions.get(*region_idx)
                        .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
                    // Find the most expensive active policy in their region to cancel
                    let policy_name = state.policies.get(*region_idx)
                        .and_then(|p| {
                            let traits = state.regions.get(*region_idx).map(|r| r.traits.as_slice()).unwrap_or(&[]);
                            p.active_policy_costs(traits).into_iter()
                                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                                .map(|(idx, _)| crate::state::policy_display_name(idx).to_string())
                        })
                        .unwrap_or_else(|| "your most expensive policy".into());
                    CrisisEvent {
                        title: "Loan Called In".into(),
                        description: format!(
                            "Gov. {lender_name} of {region_name} is calling in the emergency loan. \
                             Outstanding balance: ¥{outstanding:.0}. \
                             Payment is overdue. They want their money now.",
                        ),
                        options: vec![
                            if can_repay {
                                CrisisOption {
                                    label: format!("Repay ¥{repay_amount:.0}"),
                                    description: "Settle the debt. Gov. loyalty improves slightly.".into(),
                                    cost: Some(CrisisCost { funding: repay_amount, personnel: 0, ..Default::default() }),
                                }
                            } else {
                                CrisisOption {
                                    label: format!("Repay ¥{repay_amount:.0} (INSUFFICIENT FUNDS)"),
                                    description: format!("Need ¥{repay_amount:.0}, have ¥{:.0}. Cannot pay.", state.resources.funding),
                                    cost: Some(CrisisCost { funding: repay_amount, personnel: 0, ..Default::default() }),
                                }
                            },
                            CrisisOption {
                                label: "Default".into(),
                                description: format!(
                                    "Gov. {lender_name} cancels your {policy_name} in {region_name} \
                                     and loyalty drops 20.",
                                ),
                                cost: None,
                            },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
                LoanLender::Corporation { corp_idx } => {
                    let corp_name = state.corporations.get(*corp_idx)
                        .map(|c| c.name.as_str()).unwrap_or(lender_name.as_str());
                    CrisisEvent {
                        title: "Debt Collectors Arrive".into(),
                        description: format!(
                            "{corp_name} has sent collectors. \
                             Outstanding loan balance: ¥{outstanding:.0}. \
                             They have options, and none of them are pleasant for you.",
                        ),
                        options: vec![
                            if can_repay {
                                CrisisOption {
                                    label: format!("Repay ¥{repay_amount:.0}"),
                                    description: "Settle the debt. No further action.".into(),
                                    cost: Some(CrisisCost { funding: repay_amount, personnel: 0, ..Default::default() }),
                                }
                            } else {
                                CrisisOption {
                                    label: format!("Repay ¥{repay_amount:.0} (INSUFFICIENT FUNDS)"),
                                    description: format!("Need ¥{repay_amount:.0}, have ¥{:.0}. Cannot pay.", state.resources.funding),
                                    cost: Some(CrisisCost { funding: repay_amount, personnel: 0, ..Default::default() }),
                                }
                            },
                            CrisisOption {
                                label: "Default".into(),
                                description: "2 researchers are 'unavailable' indefinitely. −10% board approval from smear campaign.".into(),
                                cost: None,
                            },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
            }
        }
        CrisisKind::BoardMeeting => {
            let day = tick as f64 / TICKS_PER_DAY;
            let board_sat = state.board_satisfaction();
            let new_funding_rate = board_meeting_funding_rate(state, board_sat);
            let current_rate = state.funding_income_rate() * TICKS_PER_DAY;

            let mut memo_lines: Vec<String> = Vec::new();

            // Funding decision
            let rate_word = if new_funding_rate > current_rate * 1.05 {
                "increased"
            } else if new_funding_rate < current_rate * 0.95 {
                "reduced"
            } else {
                "maintained"
            };
            memo_lines.push(format!(
                "Your operating budget has been {} to \u{00a5}{:.0} per day, effective immediately.",
                rate_word, new_funding_rate
            ));

            // Note unhappy members and their concerns
            let mut concerns: Vec<String> = Vec::new();
            for member in &state.board_members {
                if member.satisfaction >= 0.5 { continue; }
                match &member.role {
                    BoardRole::CorporateLeader { corp_idx } => {
                        let corp_name = state.corporations.get(*corp_idx)
                            .map(|c| c.name.as_str()).unwrap_or("a corporation");
                        concerns.push(format!(
                            "{} has noted continued pressure on {} operations.",
                            member.name, corp_name
                        ));
                    }
                    BoardRole::RegionGovernor { region_idx } => {
                        let region_name = state.regions.get(*region_idx)
                            .map(|r| r.name.as_str()).unwrap_or("a region");
                        concerns.push(format!(
                            "{} has raised the deteriorating situation in {}.",
                            member.name, region_name
                        ));
                    }
                    BoardRole::IndependentAdvisor => {
                        concerns.push(format!(
                            "{} has questioned the adequacy of current containment measures.",
                            member.name
                        ));
                    }
                }
            }

            // Note collapsed regions
            for region in &state.regions {
                if region.collapsed {
                    concerns.push(format!(
                        "The collapse of {} has been noted. Compensation for affected operations has been deducted from your budget.",
                        region.name
                    ));
                }
            }

            // Positive note if things are going well
            let content_count = state.board_members.iter()
                .filter(|m| m.satisfaction > 0.7).count();
            if content_count > 0 && concerns.is_empty() {
                memo_lines.push(
                    "The Board has expressed general confidence in your current approach.".into()
                );
            }

            for concern in concerns.iter().take(3) {
                memo_lines.push(concern.clone());
            }

            if board_sat < 0.3 {
                memo_lines.push(
                    "The Board expects measurable improvement before the next review period.".into()
                );
            }

            let description = format!(
                "Day {:.0} review. {}",
                day,
                memo_lines.join(" ")
            );

            CrisisEvent {
                title: "Board Communiqu\u{00e9}".into(),
                description,
                options: vec![
                    CrisisOption {
                        label: "Acknowledged".into(),
                        description: "File and proceed.".into(),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::FieldTeamDetained { region_idx, corp_idx, fee, team_size } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("the region");
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.as_str()).unwrap_or("an unnamed corporation");
            CrisisEvent {
                title: "Field Team Unreachable".into(),
                description: format!(
                    "Field team has been out of contact for 36 hours. \
                     Last GPS ping: a private compound registered to {corp_name} in {region_name}. \
                     The regional director called to discuss a resolution.",
                ),
                options: vec![
                    {
                        CrisisOption {
                            label: format!("Pay resolution fee (¥{fee:.0})"),
                            description: format!(
                                "Team returns within 24 hours. {corp_name} asks no further questions.",
                            ),
                            cost: Some(CrisisCost { funding: *fee, ..Default::default() }),
                        }
                    },
                    CrisisOption {
                        label: "Escalate through official channels".into(),
                        description: format!(
                            "No funding required. Ties up {team_size} staff for up to 7 days. \
                             The process has no enforcement authority and the outcome is uncertain.",
                        ),
                        cost: None,
                    },
                    CrisisOption {
                        label: "Write them off".into(),
                        description: format!("Personnel permanently lost. No further contact from {corp_name}."),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::NewPathogenDetected { disease_idx } => {
            // Find the region with the most infections for this disease
            let origin_region = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .max_by(|(_, a), (_, b)| {
                    let a_inf: f64 = a.infections.iter()
                        .filter(|inf| inf.disease_idx == *disease_idx)
                        .map(|inf| inf.infected).sum();
                    let b_inf: f64 = b.infections.iter()
                        .filter(|inf| inf.disease_idx == *disease_idx)
                        .map(|inf| inf.infected).sum();
                    a_inf.partial_cmp(&b_inf).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(_, r)| r.name.as_str())
                .unwrap_or("an unknown location");

            // Check if identification is already in progress
            let already_identifying = state.field_research.iter()
                .any(|p| matches!(p.kind, ResearchKind::IdentifyThreat { disease_idx: d } if d == *disease_idx));

            // Check if there's capacity for field research
            let has_capacity = state.field_research_has_capacity();

            let mut options = Vec::new();

            if !already_identifying && has_capacity {
                let identify_kind = ResearchKind::IdentifyThreat { disease_idx: *disease_idx };
                let (_, _, funding_cost) = state.effective_costs(&identify_kind);
                options.push(CrisisOption {
                    label: format!("Begin identification (¥{:.0})", funding_cost),
                    description: "Deploy field team to identify the pathogen immediately.".into(),
                    // Cost is None here — start_research handles the funding deduction
                    // and affordability check internally.
                    cost: None,
                });
            }

            options.push(CrisisOption {
                label: "Acknowledge".into(),
                description: "Noted. Continue current priorities.".into(),
                cost: None,
            });

            CrisisEvent {
                title: "New Pathogen Detected".into(),
                description: format!(
                    "Anomalous case clusters reported in {origin_region}. \
                     Preliminary analysis indicates a novel pathogen not matching any known profile. \
                     Field identification recommended before it spreads further.",
                ),
                options,
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::FieldTeamDetainedAgain { region_idx, corp_idx, fee, team_size: _ } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("the region");
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.as_str()).unwrap_or("an unnamed corporation");
            CrisisEvent {
                title: "Field Team Detained Again".into(),
                description: format!(
                    "{corp_name} has detained another field team in {region_name}. \
                     Their regional director remembers the last arrangement. \
                     The fee has been revised upward.",
                ),
                options: vec![
                    {
                        CrisisOption {
                            label: format!("Pay revised fee (¥{fee:.0})"),
                            description: format!("Team returns. {corp_name} pockets the difference."),
                            cost: Some(CrisisCost { funding: *fee, ..Default::default() }),
                        }
                    },
                    CrisisOption {
                        label: "Write them off".into(),
                        description: format!("Personnel permanently lost. {corp_name} gets nothing."),
                        cost: None,
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

/// Tick all active crisis operations. Complete ones that expire and return their personnel.
pub(super) fn tick_crisis_operations(state: &mut GameState) {
    let mut i = 0;
    while i < state.crisis_operations.len() {
        state.crisis_operations[i].ticks_remaining -= 1.0;
        if state.crisis_operations[i].ticks_remaining <= 0.0 {
            let op = state.crisis_operations.remove(i);
            state.events.push(GameEvent::CrisisTeamReturned {
                label: op.label,
                personnel: op.personnel,
            });
        } else {
            i += 1;
        }
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

    // Deduct costs generically from the chosen option. Affordability is always
    // checked before calling resolve_crisis: apply_action() for manual resolution,
    // activate_crisis() for auto-resolution.
    let option = &crisis.options[choice];
    if let Some(cost) = &option.cost {
        state.resources.funding -= cost.funding;
        if cost.personnel > 0 {
            if let Some(days) = cost.operation_days {
                // Create a temporary operation — personnel are tied up and returned later
                let label = cost.operation_label.clone()
                    .unwrap_or_else(|| "Crisis Response".to_string());
                let ticks = days * TICKS_PER_DAY;
                state.crisis_operations.push(CrisisOperation {
                    label,
                    personnel: cost.personnel,
                    ticks_remaining: ticks,
                });
            } else {
                // Permanent personnel loss
                state.resources.personnel = state.resources.personnel.saturating_sub(cost.personnel);
            }
        }
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
            if state.rng_crisis.r#gen::<f64>() < 0.70 {
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
            state.resources.board_approval = (state.resources.board_approval - pol_cost).max(0.0);
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
            state.resources.board_approval = (state.resources.board_approval - pol_cost).max(0.0);
            let accepted_m = accepted / 1_000_000.0;
            format!("Partial intake: {:.0}M accepted into {}. Rest turned away.", accepted_m, to_name)
        }

        (CrisisKind::DataLeak, 0) => {
            // Issue a statement — personnel diverted via CrisisCost, gain POL
            state.resources.board_approval += 0.05;
            "Statement issued. Board confidence up. Response team deployed for 2 days.".into()
        }
        (CrisisKind::DataLeak, 1) => {
            // Suppress — lose POL
            state.resources.board_approval -= 0.10;
            // Schedule follow-up: public inquiry in 5 days
            let followup_tick = state.tick + (5.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::PublicInquiry));
            "Leak suppressed. Board confidence shaken. Inquiry risk elevated.".into()
        }
        (CrisisKind::DataLeak, _) => {
            // No comment — moderate POL loss, 50% chance of follow-up
            state.resources.board_approval -= 0.07;
            if state.rng_crisis.r#gen::<bool>() {
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
                "No comment. Leak spreading.".into()
            } else {
                "No comment. Leak faded.".into()
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
            state.resources.board_approval -= 0.15;
            "Military deployed. Quarantine maintained by force.".into()
        }
        (CrisisKind::QuarantineRiot { region_idx }, _) => {
            // Wait it out — quarantine temporarily breached, small POL loss
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.board_approval -= 0.03;
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
            state.resources.board_approval -= 0.08;
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
            state.resources.board_approval += 0.05;
            "Communications infrastructure restored. Reporting stabilized.".into()
        }

        (CrisisKind::TrialShortcut { .. }, 0) => {
            // Maintain standards — lose POL
            state.resources.board_approval -= 0.05;
            "Maintained trial standards. Board noted the delay.".into()
        }
        (CrisisKind::TrialShortcut { disease_idx, medicine_idx }, _) => {
            // Fast-track — gain POL, mark medicine as tested but 2 generations behind
            // current strain (30% efficacy penalty from drift).
            state.resources.board_approval += 0.10;
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
            state.resources.board_approval -= 0.10;
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
            state.resources.board_approval += 0.05;
            format!("Incentive program deployed in {}. Compliance rates improving.", region_name)
        }
        (CrisisKind::VaccineHesitancy { region_idx }, _) => {
            // Accept noncompliance — infections spike from untreated spread
            state.resources.board_approval -= 0.05;
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

        (CrisisKind::ResourceDiversion { share_reward, .. }, 0) => {
            state.resources.funding += share_reward;
            format!("Research data shared. Received ¥{:.0}.", share_reward)
        }
        (CrisisKind::ResourceDiversion { .. }, _) => {
            // Refuse — costs already deducted
            "Refused to share research. Foreign aid reduced.".into()
        }

        (CrisisKind::ExhaustionEpidemic { region_idx, .. }, 0) => {
            // Discourage hospitalization — enable the policy to reduce hospital load
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.discourage_hosp = true;
            }
            format!("Hospitalization discouraged in {}. Staff recovering.", region_name)
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
                state.resources.board_approval += 0.05;
                format!("Halted deployment of {}. {} doses destroyed.",
                    med.name, crate::format_number(destroyed))
            } else {
                "Deployment halted".into()
            }
        }
        (CrisisKind::WhistleblowerReport { .. }, _) => {
            // Continue deployment — lose POL
            state.resources.board_approval -= 0.08;
            "Continuing deployment despite concerns. Public confidence shaken.".into()
        }

        (CrisisKind::MilitaryTakeover { cooperate_loss }, 0) => {
            // Cooperate — lose personnel, gain POL
            state.resources.personnel = state.resources.personnel.saturating_sub(*cooperate_loss);
            state.resources.board_approval += 0.15;
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
            state.resources.board_approval -= 0.05;
            if state.rng_crisis.r#gen::<bool>() {
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
            state.resources.board_approval -= 0.08;
            "Concessions granted. Deliveries resume.".into()
        }
        (CrisisKind::CultBlockade { .. }, 1) => {
            // Police raid — costs already deducted
            "Blockade cleared. Supply routes restored.".into()
        }
        (CrisisKind::CultBlockade { region_idx }, _) => {
            // Wait them out — supply lines and healthcare degrade significantly
            state.resources.board_approval -= 0.05;
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
            state.resources.board_approval -= 0.05;
            format!("N.W.H.O. coordination collapsed. Lost ¥{:.0} in aid.", aid_loss)
        }
        (CrisisKind::WHOEvacuation { .. }, 1) => {
            // Take over — costs already deducted, gain POL
            state.resources.board_approval += 0.10;
            "Your agency is now coordinating the global response. Heavy responsibility.".into()
        }
        (CrisisKind::WHOEvacuation { aid_loss, .. }, _) => {
            // Do nothing — coordination degrades, lose funding AND 1 personnel wanders off
            state.resources.funding = (state.resources.funding - aid_loss * 0.75).max(0.0);
            state.resources.personnel = state.resources.personnel.saturating_sub(1);
            state.resources.board_approval -= 0.03;
            format!("Coordination collapsed during inaction. Lost ¥{:.0}.", aid_loss * 0.75)
        }

        (CrisisKind::WarlordDemand { region_idx }, 0) => {
            // Refuse — gain POL, region stays collapsed
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            state.resources.board_approval += 0.05;
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
            format!("Stayed neutral. Both canceled ¥{:.0} in contracts.", neutral_loss)
        }
        (CrisisKind::VaccineDispute { credit_gain, corp_a, corp_b, .. }, option) => {
            // Back one corp — gain funding, lose POL, schedule retaliation from the other
            let (backed, retaliator) = if option == 1 { (corp_a, corp_b) } else { (corp_b, corp_a) };
            state.resources.funding += credit_gain;
            state.resources.board_approval -= 0.15;
            let sanctions_loss = scaled_cost(state, 0.20, 200.0, 800.0);
            let followup_tick = state.tick + (5.0 * TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::SanctionsThreat { funding_loss: sanctions_loss, corp_name: retaliator.clone() }));
            format!("Backed {}. ¥{:.0} deposited. {} is furious.", backed, credit_gain, retaliator)
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
            state.resources.board_approval += 0.05;
            "Review complete. Rating: \"Meets Expectations.\"".into()
        }
        (CrisisKind::PerformanceReview, _) => {
            // Skip — lose POL
            state.resources.board_approval -= 0.05;
            "Board notes your absence. A memo has been circulated.".into()
        }

        (CrisisKind::NamingRights { disease_idx, payout }, 0) => {
            // Decline — gain POL
            let _ = (disease_idx, payout);
            state.resources.board_approval += 0.03;
            "Offer declined.".into()
        }
        (CrisisKind::NamingRights { disease_idx, payout }, _) => {
            // Accept — gain money, lose POL, rename the disease
            state.resources.funding += payout;
            state.resources.board_approval -= 0.05;
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
            let lucky = state.rng_crisis.r#gen::<bool>();
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
            state.resources.board_approval += 0.10;
            "Testimony concluded. Committee thanks you for your cooperation.".into()
        }
        (CrisisKind::CongressionalHearing, 1) => {
            // Send deputy — small POL gain, 40% chance of contempt follow-up
            state.resources.board_approval += 0.02;
            if state.rng_crisis.r#gen::<f64>() < 0.40 {
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
            state.resources.board_approval -= 0.15;
            let followup_tick = state.tick + (2.0 * TICKS_PER_DAY) as u64;
            let fine = scaled_cost(state, 0.20, 300.0, 800.0);
            state.pending_crises.push((followup_tick, CrisisKind::ContemptOfCongress { fine }));
            "Subpoena ignored. Contempt charges filed.".into()
        }

        (CrisisKind::ContemptOfCongress { fine }, 0) => {
            // Pay fine — lose money and POL
            state.resources.funding = (state.resources.funding - fine).max(0.0);
            state.resources.board_approval -= 0.08;
            format!("Fine paid. ¥{:.0} deducted.", fine)
        }
        (CrisisKind::ContemptOfCongress { .. }, _) => {
            // Fight charges — pay same fine but less POL loss
            state.resources.board_approval -= 0.03;
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
            // Override restriction — lose POL, data restored to research teams
            state.resources.board_approval -= 0.10;
            "Restriction overridden. Data restored to research teams.".into()
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
            state.resources.board_approval -= 0.05;
            "Ignored the broadcast. The accusations faded.".into()
        }
        (CrisisKind::GovernorBlowhard { .. }, _) => {
            // Counter-broadcast — costs already deducted, gain POL
            state.resources.board_approval += 0.03;
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

        // --- Contract demand resolutions ---

        (CrisisKind::ContractDemand { template_id }, 0) => {
            // Placate: boost contract condition satisfaction (cost already deducted)
            let member_idx = state.contracts.iter()
                .find(|c| c.template_id == *template_id)
                .map(|c| c.board_member_idx);
            let member_name = member_idx
                .and_then(|idx| state.board_members.get(idx))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Board member".to_string());
            if let Some(c) = state.contracts.iter_mut()
                .find(|c| c.template_id == *template_id)
            {
                c.satisfaction = (c.satisfaction + 0.25).min(1.0);
                c.warned = false;
            }
            format!("{} placated. Contract stable.", member_name)
        }
        (CrisisKind::ContractDemand { template_id }, _) => {
            // Refuse: contract condition satisfaction drops sharply
            let member_idx = state.contracts.iter()
                .find(|c| c.template_id == *template_id)
                .map(|c| c.board_member_idx);
            let member_name = member_idx
                .and_then(|idx| state.board_members.get(idx))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Board member".to_string());
            if let Some(c) = state.contracts.iter_mut()
                .find(|c| c.template_id == *template_id)
            {
                c.satisfaction = (c.satisfaction - 0.15).max(0.0);
            }
            format!("{} rebuffed. Contract satisfaction dropped.", member_name)
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
            state.resources.board_approval -= 0.05;
            // Disable only the most aggressive policies
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.quarantine = false;
                policy.martial_law = false;
            }
            format!("Dispute unresolved. Quarantine and martial law dropped in {}", region_name)
        }

        (CrisisKind::GovernorOperative { .. }, 0) => {
            // Look the other way, lose POL
            state.resources.board_approval -= 0.15;
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
            state.resources.board_approval -= 0.20;
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
            state.resources.board_approval += 0.10;
            "Full transparency. Lost research time, rebuilt public trust.".into()
        }
        (CrisisKind::PublicInquiry, _) => {
            // Stonewall — massive POL loss
            state.resources.board_approval -= 0.20;
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

        (CrisisKind::SanctionsThreat { funding_loss, .. }, 0) => {
            // Accept cuts — lose funding + POL hit
            state.resources.funding = (state.resources.funding - funding_loss).max(0.0);
            state.resources.board_approval -= 0.10;
            format!("Contracts canceled. ¥{:.0} lost.", funding_loss)
        }
        (CrisisKind::SanctionsThreat { .. }, _) => {
            // Negotiate settlement — costs already deducted, contracts preserved
            "Settlement reached. Contracts preserved.".into()
        }

        // --- Emergency loan crises ---

        (CrisisKind::LoanOffer { .. }, 0) => {
            // Decline — no loan taken
            "Loan offer declined. Policy suspension stands.".into()
        }
        (CrisisKind::LoanOffer { lender_name, lender, amount, daily_interest_rate }, _) => {
            // Accept — add the loan, deposit the funds
            let due_day = state.tick as f64 / TICKS_PER_DAY + LOAN_DUE_DAYS;
            state.resources.funding += amount;
            state.loans.push(ActiveLoan {
                lender_name: lender_name.clone(),
                lender: lender.clone(),
                principal: *amount,
                outstanding: *amount,
                daily_interest_rate: *daily_interest_rate,
                due_day,
                hostile_queued: false,
            });
            format!(
                "¥{:.0} received from {}. Repay by day {:.0} or face consequences.",
                amount, lender_name, due_day
            )
        }

        (CrisisKind::LoanCallIn { lender_name, lender, .. }, 0) => {
            // Repay — cost already deducted by CrisisCost; remove the loan
            // Find and remove the matching loan
            let lender_ref = lender;
            if let Some(pos) = state.loans.iter().position(|l| l.lender == *lender_ref) {
                state.loans.remove(pos);
            }
            // Governor: small loyalty boost for honoring the debt
            if let LoanLender::Governor { region_idx } = lender {
                if let Some(region) = state.regions.get_mut(*region_idx) {
                    region.governor.loyalty = (region.governor.loyalty + 5.0).min(100.0);
                }
            }
            format!("Debt repaid to {}. Obligation cleared.", lender_name)
        }
        (CrisisKind::LoanCallIn { lender_name, lender, .. }, _) => {
            // Default — hostile action
            // Remove the loan regardless (debt is "forgiven" via enforcement)
            let lender_ref = lender;
            if let Some(pos) = state.loans.iter().position(|l| l.lender == *lender_ref) {
                state.loans.remove(pos);
            }
            match lender {
                LoanLender::Governor { region_idx } => {
                    // Cancel most expensive policy in their region + loyalty drop
                    let mut cancelled_name = None;
                    if let Some(region) = state.regions.get(*region_idx) {
                        let traits = region.traits.as_slice();
                        if let Some(policy) = state.policies.get_mut(*region_idx) {
                            if let Some((idx, _)) = policy.active_policy_costs(traits).into_iter()
                                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                            {
                                use crate::state::POLICY_IDX_SCREENING_BASE;
                                if idx == POLICY_IDX_SCREENING_BASE {
                                    cancelled_name = Some(match policy.screening {
                                        crate::state::ScreeningLevel::Basic => "Basic Screening",
                                        crate::state::ScreeningLevel::Antigen => "Med Screening",
                                        crate::state::ScreeningLevel::MassRapid => "Mass Screening",
                                        crate::state::ScreeningLevel::None => "Screening",
                                    }.to_string());
                                    policy.screening = crate::state::ScreeningLevel::None;
                                } else {
                                    cancelled_name = Some(crate::state::policy_display_name(idx).to_string());
                                    policy.set_bool(idx, false);
                                }
                            }
                        }
                    }
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.loyalty = (region.governor.loyalty - 20.0).max(0.0);
                    }
                    let region_name = state.regions.get(*region_idx)
                        .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
                    match cancelled_name {
                        Some(name) => format!(
                            "Gov. {lender_name} defaulted. {name} cancelled in {region_name}. Loyalty −20.",
                        ),
                        None => format!(
                            "Gov. {lender_name} defaulted. Loyalty −20 in {region_name}.",
                        ),
                    }
                }
                LoanLender::Corporation { .. } => {
                    // Personnel intimidation + POL smear campaign
                    let lost = 2u32.min(state.resources.personnel);
                    state.resources.personnel = state.resources.personnel.saturating_sub(lost);
                    state.resources.board_approval = (state.resources.board_approval - 0.10).max(0.0);
                    format!(
                        "{lender_name} collected. {lost} researchers 'unavailable'. −10% board approval from smear campaign.",
                    )
                }
            }
        }


        // --- Pathogen detection alert ---

        (CrisisKind::NewPathogenDetected { disease_idx }, 0) if crisis.options.len() > 1 => {
            // "Begin identification" — the first option (only present when identification
            // is available). Funding cost was already deducted by CrisisCost.
            // Start the identification research project.
            let projects = state.available_projects(ResearchTrack::Field);
            if let Some(idx) = projects.iter().position(|k| matches!(k, ResearchKind::IdentifyThreat { disease_idx: d } if *d == *disease_idx)) {
                let (ok, msg) = super::research::start_research(state, ResearchTrack::Field, idx, false);
                if ok {
                    let name = state.diseases.get(*disease_idx)
                        .map(|d| d.display_name(*disease_idx))
                        .unwrap_or_else(|| format!("Pathogen #{}", disease_idx + 1));
                    format!("Field identification of {} initiated.", name)
                } else {
                    msg.unwrap_or_else(|| "Could not start identification.".into())
                }
            } else {
                "Identification no longer available.".into()
            }
        }
        (CrisisKind::NewPathogenDetected { .. }, _) => {
            "Alert acknowledged.".into()
        }

        // --- Board meeting communiqué ---

        (CrisisKind::BoardMeeting, _) => {
            // Single option: Acknowledged. Apply funding multiplier.
            let board_sat = state.board_satisfaction();
            state.board_funding_multiplier = board_meeting_funding_multiplier(board_sat);
            "Board communiqué filed.".into()
        }


        // --- Corporate detention crises ---

        (CrisisKind::FieldTeamDetained { corp_idx, region_idx, fee, team_size }, 0) => {
            // Pay the fee (cost already deducted). Schedule follow-up detention.
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.clone()).unwrap_or_else(|| "the corporation".into());
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "the region".into());
            let followup_fee = fee * 1.6;
            let followup_tick = state.tick + (10.0 * crate::engine::TICKS_PER_DAY) as u64;
            state.pending_crises.push((followup_tick, CrisisKind::FieldTeamDetainedAgain {
                region_idx: *region_idx,
                corp_idx: *corp_idx,
                fee: followup_fee,
                team_size: *team_size,
            }));
            format!("¥{fee:.0} paid. Team released. {corp_name} now knows what you'll pay in {region_name}.")
        }
        (CrisisKind::FieldTeamDetained { team_size, .. }, 1) => {
            // Escalate through official channels. Cost is None, so we handle all
            // personnel effects here directly. Three outcomes, probabilistic.
            let roll = state.rng_crisis.r#gen::<f64>();
            let (days, msg): (f64, &str) = if roll < 0.15 {
                // 15%: corp escalates in response. Staff still committed, team unreachable.
                (7.0, "The corporation escalated in response. Staff are locked into the process and the team is still unreachable.")
            } else if roll < 0.45 {
                // 30%: process moves faster than expected.
                (4.0, "The process moved faster than expected. Team flagged for early release. No guarantees.")
            } else {
                // 55%: nothing moves. Staff tied up for 7 days, process stalls.
                (7.0, "Escalation logged. No enforcement authority in the region. Process runs its course.")
            };
            state.crisis_operations.push(crate::state::CrisisOperation {
                label: "Official Channels Process".to_string(),
                personnel: *team_size,
                ticks_remaining: days * crate::engine::TICKS_PER_DAY,
            });
            msg.into()
        }
        (CrisisKind::FieldTeamDetained { team_size, .. }, _) => {
            // Write them off — permanent personnel loss.
            state.resources.personnel = state.resources.personnel.saturating_sub(*team_size);
            format!("Team written off. {team_size} personnel lost permanently.")
        }

        (CrisisKind::FieldTeamDetainedAgain { fee, team_size: _, corp_idx, .. }, 0) => {
            // Pay revised fee (cost already deducted).
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.clone()).unwrap_or_else(|| "the corporation".into());
            format!("¥{fee:.0} paid. Team released. {corp_name} has established a reliable revenue stream.")
        }
        (CrisisKind::FieldTeamDetainedAgain { team_size, corp_idx, .. }, _) => {
            // Write them off.
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.clone()).unwrap_or_else(|| "the corporation".into());
            state.resources.personnel = state.resources.personnel.saturating_sub(*team_size);
            format!("Team written off. {team_size} personnel lost. {corp_name} gets nothing this time.")
        }
    };
    // Clamp POL after crisis modifications
    state.resources.board_approval = state.resources.board_approval.clamp(0.0, 1.0);
    // Restore sim state from Event mode. When the player manually resolves a crisis,
    // sim_state is Event { was_running }. We restore Running or Paused here so callers
    // don't need to know about this hidden rule. In the auto-resolve path (called from
    // activate_crisis() before sim_state is set to Event), this is a no-op.
    if let SimState::Event { was_running } = state.sim_state {
        state.sim_state = if was_running { SimState::Running } else { SimState::Paused };
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_weights_shift_with_game_day() {
        // Early-game bureaucratic crises should dominate early, fade late
        assert!(phase_weight("political", 3.0) > phase_weight("political", 60.0),
            "political pressure should be more likely early than late");
        assert!(phase_weight("corrupt", 5.0) > phase_weight("corrupt", 60.0),
            "corrupt official should be more likely early than late");

        // Late-game survival crises should be absent early, present late
        assert!(phase_weight("military", 5.0) < phase_weight("military", 50.0),
            "military takeover should be more likely late than early");
        assert!(phase_weight("warlord", 3.0) < phase_weight("warlord", 50.0),
            "warlord demands should be more likely late than early");
        assert!(phase_weight("cult", 3.0) < phase_weight("cult", 50.0),
            "cult blockade should be more likely late than early");

        // Mid-game crises should peak in the middle
        assert!(phase_weight("supply", 40.0) > phase_weight("supply", 2.0),
            "supply disruption should be more likely mid-game than very early");
        assert!(phase_weight("supply", 40.0) > phase_weight("supply", 80.0),
            "supply disruption should be more likely mid-game than very late");

        // No crisis type should ever have zero weight (anachronistic = rare but possible)
        assert!(phase_weight("political", 60.0) > 0.0,
            "even late-game, bureaucratic crises should have non-zero weight");
    }

    #[test]
    fn early_game_generates_bureaucratic_crises() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = GameState::new_default(42);
        // Day 5: early game
        state.tick = (5.0 * TICKS_PER_DAY) as u64;
        // Ensure preconditions for various crisis types
        state.policies[0].quarantine = true;
        state.medicines[0].doses = 50.0; // for supply disruption

        let mut tags: Vec<&str> = Vec::new();
        for seed in 0..50u64 {
            let mut r = ChaCha8Rng::seed_from_u64(seed);
            if let Some(crisis) = generate_crisis(&state, &mut r) {
                tags.push(crisis.kind.tag());
            }
        }

        // At day 5, late-game crises (military, cult, warlord, etc.) should
        // be absent or extremely rare since they have near-zero weight
        let late_count = tags.iter()
            .filter(|&&t| matches!(t, "military" | "cult" | "warlord" | "vaccine_dispute" | "who_evac"))
            .count();
        assert!(late_count <= 2,
            "at day 5, late-game crises should be rare, got {}/{}",
            late_count, tags.len());
    }

    #[test]
    fn field_team_detained_generates_in_collapsed_region() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = (20.0 * TICKS_PER_DAY) as u64;
        // Collapse a region and ensure it has a non-bankrupt corporation
        state.regions[0].collapsed = true;
        state.resources.personnel = 20; // ensure >= 4

        let mut found = false;
        for seed in 0..100u64 {
            let mut r = ChaCha8Rng::seed_from_u64(seed);
            if let Some(crisis) = generate_crisis(&state, &mut r) {
                if crisis.kind.tag() == "field_team_detained" {
                    found = true;
                    // Must have 3 options: pay, escalate, write off
                    assert_eq!(crisis.options.len(), 3, "FieldTeamDetained must have 3 options");
                    // Must have at least one free option (write them off)
                    assert!(crisis.options.iter().any(|o| o.cost.is_none()),
                        "FieldTeamDetained must have a free option");
                    break;
                }
            }
        }
        assert!(found, "FieldTeamDetained should appear within 100 seeds when preconditions are met");
    }

    #[test]
    fn field_team_detained_pay_schedules_followup() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = (20.0 * TICKS_PER_DAY) as u64;
        state.regions[0].collapsed = true;
        state.resources.funding = 10000.0;
        state.resources.personnel = 20;

        // Find the corp_idx for a corp in region 0
        let corp_idx = state.corporations.iter()
            .position(|c| c.region_idx == 0 && !c.bankrupt)
            .expect("should have a corp in region 0");

        let fee = 300.0;
        let kind = CrisisKind::FieldTeamDetained {
            region_idx: 0,
            corp_idx,
            fee,
            team_size: 3,
        };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        state.sim_state = crate::state::SimState::Event { was_running: false };

        let pending_before = state.pending_crises.len();
        let msg = resolve_crisis(&mut state, 0); // Pay
        assert!(state.pending_crises.len() > pending_before, "paying should schedule follow-up");
        assert!(state.pending_crises.iter().any(|(_, k)| matches!(k, CrisisKind::FieldTeamDetainedAgain { .. })),
            "follow-up should be FieldTeamDetainedAgain");
        assert!(msg.contains("paid"), "resolution message should mention payment");
    }

    #[test]
    fn field_team_detained_write_off_loses_personnel() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = (20.0 * TICKS_PER_DAY) as u64;
        state.regions[0].collapsed = true;
        state.resources.personnel = 20;

        let corp_idx = state.corporations.iter()
            .position(|c| c.region_idx == 0 && !c.bankrupt)
            .expect("should have a corp in region 0");

        let kind = CrisisKind::FieldTeamDetained {
            region_idx: 0,
            corp_idx,
            fee: 300.0,
            team_size: 3,
        };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        state.sim_state = crate::state::SimState::Event { was_running: false };

        let personnel_before = state.resources.personnel;
        resolve_crisis(&mut state, 2); // Write them off
        assert_eq!(state.resources.personnel, personnel_before - 3, "write-off should lose team_size personnel");
    }

    #[test]
    fn new_pathogen_detected_offers_identification() {
        let mut state = GameState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0; // Unknown — needs identification
        state.resources.funding = 2000.0;
        state.resources.personnel = 20;

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);

        assert_eq!(crisis.title, "New Pathogen Detected");
        // Should have 2 options: Begin identification + Acknowledge
        assert_eq!(crisis.options.len(), 2,
            "should offer identification and acknowledge");
        assert!(crisis.options[0].label.contains("identification"),
            "first option should be identification");
        assert!(crisis.options[1].label.contains("Acknowledge"),
            "second option should be acknowledge");
    }

    #[test]
    fn new_pathogen_detected_begin_identification_starts_research() {
        let mut state = GameState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        state.resources.funding = 2000.0;
        state.resources.personnel = 20;

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        state.sim_state = crate::state::SimState::Event { was_running: false };

        assert!(state.field_research.is_empty(), "no research before resolution");
        let msg = resolve_crisis(&mut state, 0); // Begin identification
        assert!(!state.field_research.is_empty(),
            "identification research should start after choosing option A");
        assert!(matches!(
            &state.field_research[0].kind,
            ResearchKind::IdentifyThreat { disease_idx: 0 }
        ), "should be identifying disease 0");
        assert!(msg.contains("initiated"), "message should confirm initiation: {}", msg);
    }

    #[test]
    fn new_pathogen_detected_dismiss_does_nothing() {
        let mut state = GameState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        state.resources.funding = 2000.0;

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        // Dismiss is option index 1 (when identification is available)
        state.active_crisis = Some(crisis);
        state.sim_state = crate::state::SimState::Event { was_running: false };

        let funding_before = state.resources.funding;
        resolve_crisis(&mut state, 1); // Acknowledge
        assert!(state.field_research.is_empty(), "no research should start on dismiss");
        assert_eq!(state.resources.funding, funding_before, "no funding should be deducted on dismiss");
    }

    #[test]
    fn new_pathogen_detected_no_identification_when_already_researching() {
        let mut state = GameState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        // Already identifying disease 0
        state.field_research.push(crate::state::ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);

        // Should only have "Acknowledge" option since identification is already running
        assert_eq!(crisis.options.len(), 1,
            "should only offer acknowledge when identification is already in progress");
    }
}
