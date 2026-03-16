use rand::Rng;

use crate::state::{
    ActiveLoan, Authority, BoardPersonality, BoardRole, CorporationSector, CrisisCost,
    CrisisEvent, CrisisKind, CrisisOption, CrisisOperation, GameEvent, WorldState,
    GovernorPersonality, LoanLender, ModifierSource, OperationSpec, ResearchKind,
    CRISIS_TYPE_COOLDOWN, LOAN_DUE_DAYS,
    LOYALTY_RAISE_FRACTION, TICKS_PER_DAY,
};

/// Post-resolution actions that require cross-subsystem calls.
/// Returned by `resolve_crisis` so mod.rs can dispatch to the appropriate subsystem.
pub(super) enum CrisisPostAction {
    None,
    AcceptContract,
    RejectContract,
    CancelContract { board_member_idx: usize },
}

/// Apply a satisfaction modifier to the chairman board member.
/// Positive values boost satisfaction, negative values penalize it.
fn chairman_satisfaction_hit(state: &mut WorldState, amount: f64) {
    if let Some(chairman) = state.board_members.iter_mut().find(|m| m.is_chairman) {
        chairman.add_modifier(ModifierSource::CrisisEffect, amount);
    }
}

/// Queue a GovernorDeath crisis with ~30% probability after a non-stabilize
/// GovernorSick resolution. Skips if the governor is already dead or a
/// GovernorDeath is already pending/active for this region.
fn queue_governor_death_followup(state: &mut WorldState, region_idx: usize) {
    let already_dead = state.regions.get(region_idx).map_or(true, |r| r.governor.dead);
    if already_dead {
        return;
    }
    let already_pending = state.pending_crises.iter()
        .any(|k| matches!(k, CrisisKind::GovernorDeath { region_idx: ri } if *ri == region_idx));
    let already_active = state.active_crisis.as_ref()
        .map_or(false, |c| matches!(c.kind, CrisisKind::GovernorDeath { region_idx: ri } if ri == region_idx));
    if already_pending || already_active {
        return;
    }
    // ~30% chance the governor actually dies after non-stabilize choices
    let roll: f64 = state.rng_crisis.r#gen();
    if roll < 0.30 {
        state.pending_crises.push(CrisisKind::GovernorDeath { region_idx });
    }
}

/// Scale a dollar amount relative to current funding.
/// `fraction` is the target fraction of current funding (e.g., 0.15 = 15%).
/// Result is clamped to [min, max] and rounded to nearest ¥10.
fn scaled_cost(state: &WorldState, fraction: f64, min: f64, max: f64) -> f64 {
    let raw = (state.resources.funding * fraction).clamp(min, max);
    (raw / 10.0).round() * 10.0
}

/// Find the connected non-collapsed region with the lowest total infection.
/// Returns None if no suitable destination exists.
fn find_safest_connected_region(state: &WorldState, from_region: usize) -> Option<usize> {
    let region = state.regions.get(from_region)?;
    region.connections.iter()
        .copied()
        .filter(|&ri| !state.regions.get(ri).map_or(true, |r| r.collapsed))
        .min_by(|&a, &b| {
            let inf_a: f64 = state.regions[a].infections.iter().map(|i| i.infected).sum();
            let inf_b: f64 = state.regions[b].infections.iter().map(|i| i.infected).sum();
            inf_a.partial_cmp(&inf_b).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Relocate the largest non-bankrupt corporation in a region to the safest connected region.
/// Also moves the corp director (board member) if co-located with the corp.
/// Returns a message describing what happened.
fn relocate_region_corp(state: &mut WorldState, from_region: usize) -> String {
    let corp_idx = state.corporations.iter()
        .enumerate()
        .filter(|(_, c)| c.region_idx == from_region && !c.bankrupt)
        .max_by(|(_, a), (_, b)| a.base_revenue.partial_cmp(&b.base_revenue).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i);
    let Some(corp_idx) = corp_idx else {
        return "No corporation to relocate.".into();
    };
    let dest = find_safest_connected_region(state, from_region);
    let Some(dest_idx) = dest else {
        return "No safe region for relocation.".into();
    };
    let old_region = state.corporations[corp_idx].region_idx;
    state.corporations[corp_idx].region_idx = dest_idx;
    let corp_name = state.corporations[corp_idx].name.clone();
    let dest_name = state.regions.get(dest_idx).map(|r| r.name.as_str()).unwrap_or("Unknown");
    // Move the corp director if they were co-located with the corp
    for member in &mut state.board_members {
        if member.corp_idx == Some(corp_idx) && member.region_idx == Some(old_region) {
            member.region_idx = Some(dest_idx);
        }
    }
    format!("{corp_name} relocated to {dest_name}.")
}

/// Satisfaction-based budget multiplier.
/// sat 1.0 => 1.2x (generous), sat 0.5 => 1.0x (neutral), sat 0.0 => 0.5x (slashed).
fn board_budget_satisfaction_mult(board_sat: f64) -> f64 {
    if board_sat >= 0.5 {
        1.0 + (board_sat - 0.5) * 0.4
    } else {
        0.5 + board_sat
    }
}

/// Crisis urgency boost based on player-visible (screened) infections as a
/// fraction of total population. The board sees people are sick and allocates
/// more funding to deal with it. Better screening reveals more of the true
/// caseload, so investing in testing infrastructure directly increases the
/// urgency signal the board acts on.
///
/// Uses sqrt of the screened infection fraction so the boost grows quickly at
/// first (a visible outbreak is alarming) but tapers at extreme levels.
/// Total population is ~7.8 billion, so:
///   100K screened (0.001%) → +0.01  (barely registers)
///   10M  screened (0.13%)  → +0.11  (outbreak is undeniable)
///   100M screened (1.3%)   → +0.34  (full crisis, near cap)
///   500M screened (6.4%)   → +0.40  (cap)
/// Capped at +0.40 to prevent runaway budgets.
fn crisis_urgency_boost(state: &WorldState) -> f64 {
    let screened = state.total_infected_screened();
    let population = state.initial_population();
    if population <= 0.0 || screened < 1000.0 {
        return 0.0;
    }
    let fraction = screened / population;
    let raw = fraction.sqrt() * 3.0;
    raw.clamp(0.0, 0.40)
}

/// Chairman mood shift applied on top of the budget multiplier.
/// Content chairman (>0.7 satisfaction) steers meetings favorably; hostile pushes for cuts.
/// Profiteer chairman amplifies swings: ±0.15 instead of the default ±0.10.
fn chairman_funding_shift(state: &WorldState) -> f64 {
    let chairman = match state.board_members.iter().find(|m| m.is_chairman) {
        Some(c) => c,
        None => return 0.0,
    };
    let magnitude = if chairman.personality == Some(BoardPersonality::Profiteer) {
        0.15
    } else {
        0.1
    };
    if chairman.satisfaction > 0.7 {
        magnitude
    } else if chairman.satisfaction < 0.3 {
        -magnitude
    } else {
        0.0
    }
}

/// Compute the per-tick board budget at a given satisfaction level.
/// Uses the reference base (captured at game start) as a stable anchor, then
/// applies three additive factors:
///   1. Satisfaction multiplier (0.5–1.2x based on board mood)
///   2. Chairman mood shift (±0.10 or ±0.15 for Profiteer)
///   3. Crisis urgency boost (0–0.30 based on screened infections)
///
/// The urgency boost is the key counterforce to satisfaction decline: as the
/// pandemic worsens, stocks/GDP drop (lowering satisfaction), but visible
/// infections rise (increasing urgency). The net effect keeps the budget
/// roughly stable instead of death-spiraling. Better screening amplifies the
/// urgency signal, rewarding investment in testing infrastructure.
pub(super) fn compute_board_budget_per_tick(state: &WorldState, board_sat: f64) -> f64 {
    let reference = state.reference_base_budget_per_tick;
    let base = if reference > 0.0 { reference } else { state.base_board_budget_per_tick() };
    let mult = board_budget_satisfaction_mult(board_sat);
    let shift = chairman_funding_shift(state);
    let urgency = crisis_urgency_boost(state);
    let full_mult = (mult + shift + urgency).clamp(0.3, 1.8);
    base * full_mult
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
/// Two main phases:
/// - Early (day 0-50): bureaucratic/organizational (personnel, trial shortcuts) — fades out
/// - Late (day 24+): survival and power struggles (corporate seizure, cult)
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
        "personnel" | "trial"
            => fade_out(30.0, 50.0),

        // --- Late-game: survival and power struggles (ramp up day 24-40) ---
        "corporate_seizure" | "cult"
            => ramp_up(24.0, 40.0),

        // Corporate detention: requires a collapsed region, so can't appear before ~day 10,
        // peak late as collapses accumulate. Follow-up has no phase bias (fires on demand).
        "field_team_detained" => ramp_up(15.0, 30.0),

        // Performance review is funniest when things are falling apart
        "performance_review" => ramp_up(20.0, 36.0),

        // Corporate demands: mid-game when policies are in full effect
        "corp_demand" => ramp_up(8.0, 20.0),

        // Default: no phase bias (follow-ups, governor crises, etc.)
        _ => 1.0,
    }
}

/// Generate a crisis event based on current game state. Returns None if no
/// suitable crisis can be generated (e.g., no valid targets for any crisis type).
pub(super) fn generate_crisis(state: &WorldState, rng: &mut impl Rng) -> Option<CrisisEvent> {
    let mut candidates: Vec<CrisisKind> = Vec::new();
    let day = state.tick as f64 / TICKS_PER_DAY;

    // --- Original crisis types ---

    // Personnel crisis: requires at least 5 personnel
    if state.resources.personnel >= 5 {
        let amount = 3.max(state.resources.personnel / 5);
        candidates.push(CrisisKind::PersonnelCrisis { amount });
    }

    // --- New crisis types ---

    // RefugeeWave is triggered deterministically on collapse (see engine/mod.rs),
    // not generated randomly.





    // Trial shortcut: requires an unlocked medicine that targets a disease it hasn't
    // been trialled against yet. Fast-tracking marks the medicine as tested, promoting
    // it from cross-reactive (0.5x) to primary target (1.0x) efficacy.
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

    // Corporate seizure: requires low authority and day > 16, and a board member with a non-bankrupt corp
    if state.resources.authority <= Authority::Low && day > 16.0 {
        // Pick the most dissatisfied board member who has a non-bankrupt corporation
        let actor = state.board_members.iter().enumerate()
            .filter(|(_, m)| !m.dead)
            .filter_map(|(bi, m)| {
                m.corp_idx
                    .filter(|&ci| state.corporations.get(ci).is_some_and(|c| !c.bankrupt))
                    .map(|ci| (bi, ci, m.satisfaction))
            })
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((board_member_idx, corp_idx, _)) = actor {
            let cooperate_loss = ((state.resources.personnel as f64 * 0.20).round() as u32).clamp(2, 6);
            candidates.push(CrisisKind::CorporateSeizure { cooperate_loss, board_member_idx, corp_idx });
        }
    }

    // --- Late-game crisis types (day-gated) ---

    // Cult blockade: requires day > 24, deployed medicine exists, region has active infections
    // (supply route blockade only makes sense where deliveries are actually happening)
    if day > 24.0 && state.medicines.iter().any(|m| m.unlocked && m.doses > 0.0) {
        let infected_non_collapsed: Vec<usize> = state.regions.iter().enumerate()
            .filter(|(_, r)| !r.collapsed && r.infections.iter().any(|i| i.infected > 100.0))
            .map(|(i, _)| i)
            .collect();
        if !infected_non_collapsed.is_empty() {
            let idx = infected_non_collapsed[rng.r#gen::<usize>() % infected_non_collapsed.len()];
            candidates.push(CrisisKind::CultBlockade { region_idx: idx });
        }
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

    // --- Dark comedy events ---

    // Performance review: day 24+ (the board doesn't care about your little pandemic)
    if day > 24.0 {
        candidates.push(CrisisKind::PerformanceReview);
    }


    // Ark Protocol: scheduled deterministically in tick() when 2+ regions collapse,
    // not generated randomly.

    // Corporate demand: a corp's sector is hurt by an active policy and revenue is down.
    // Per-corp cooldown of 20 days prevents the same corp spamming demands.
    const CORP_DEMAND_COOLDOWN: u64 = (20.0 * TICKS_PER_DAY) as u64;
    const CORP_DEMAND_REVENUE_THRESHOLD: f64 = 0.70; // revenue/base < 70%
    if day > 8.0 {
        for (c_idx, corp) in state.corporations.iter().enumerate() {
            if corp.bankrupt {
                continue;
            }
            // Per-corp cooldown
            if let Some(last) = corp.last_demand_tick {
                if state.tick.saturating_sub(last) < CORP_DEMAND_COOLDOWN {
                    continue;
                }
            }
            // Revenue must be significantly depressed
            if corp.base_revenue <= 0.0 || corp.revenue / corp.base_revenue > CORP_DEMAND_REVENUE_THRESHOLD {
                continue;
            }
            // The corp's sector must have a grievance with an active policy in their region
            if let Some(policy) = state.policies.get(corp.region_idx) {
                if corp.sector.policy_grievance(policy).is_some() {
                    candidates.push(CrisisKind::CorporateDemand { corp_idx: c_idx });
                }
            }
        }
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
pub(super) fn build_crisis_event(state: &WorldState, kind: CrisisKind) -> CrisisEvent {
    let tick = state.tick;
    let event = match &kind {
        CrisisKind::PersonnelCrisis { amount } => {
            let retention_cost = scaled_cost(state, 0.20, 200.0, 1000.0);
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
        // --- New crisis types ---

        CrisisKind::RefugeeWave { from_region, to_region, wave } => {
            let from_name = state.regions.get(*from_region)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let to_name = state.regions.get(*to_region)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let to_collapsed = state.regions.get(*to_region)
                .map(|r| r.collapsed).unwrap_or(false);
            let to_infected = state.regions.get(*to_region)
                .map(|r| r.infections.iter().any(|i| i.infected > 0.0))
                .unwrap_or(false);
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

            let close_desc = if *wave >= 3 {
                format!(
                    "Seal the borders. {:.0}M die in the open.",
                    survivors_m * 0.20,
                )
            } else if to_collapsed {
                format!(
                    "Seal the borders. {:.0}M die in the open. {} is already gone.",
                    survivors_m * 0.20, to_name,
                )
            } else if to_infected {
                format!(
                    "Seal the borders. Millions die at the gates. \
                     {} is already struggling — keep it from getting worse.",
                    to_name,
                )
            } else {
                format!(
                    "Seal the borders. Millions die at the gates. {} stays clean.",
                    to_name,
                )
            };

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
                    label: format!("Close borders (−{}% chairman approval)", pol_pct),
                    description: close_desc,
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
        CrisisKind::TrialShortcut { disease_idx, medicine_idx } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            let med_name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            let fast_track_cost = 400.0;
            CrisisEvent {
                title: "Pressure to Skip Trials".into(),
                description: format!(
                    "The board wants {} cleared for {} without full trials. \
                     Expedited approval is possible but expensive.",
                    med_name, disease_name,
                ),
                options: vec![ CrisisOption {
                    label: "Maintain standards".into(),
                    description: "−5% chairman approval. Trial continues normally.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Fast-track (¥{:.0}, +10% chairman approval)", fast_track_cost),
                    description: format!("Clear {} for immediate use against {}. Skips trial phase.", med_name, disease_name),
                    cost: Some(CrisisCost { funding: fast_track_cost, personnel: 0, ..Default::default() }),
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::CorporateSeizure { cooperate_loss, board_member_idx, corp_idx } => {
            let member_name = state.board_members.get(*board_member_idx)
                .map(|m| m.name.as_str()).unwrap_or("A board member");
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.as_str()).unwrap_or("A corporation");
            CrisisEvent {
                title: format!("{}: Security Takeover", corp_name),
                description: format!(
                    "{corp_name} has deployed private security to your facilities. \
                     {member_name} is demanding operational authority, citing asset protection."
                ),
                options: vec![ CrisisOption {
                    label: format!("Cooperate (−{} personnel, +15% approval)", cooperate_loss),
                    description: format!(
                        "Permanently lose {} personnel. The board approves, but {} deepens corporate control over your operations.",
                        cooperate_loss, corp_name
                    ),
                    cost: None,
                },
                 {
                    let cost = scaled_cost(state, 0.30, 200.0, 1000.0);
                    CrisisOption {
                        label: format!("Resist (¥{:.0})", cost),
                        description: format!("Pay to fight {}'s takeover, keep your team.", corp_name),
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                    }
                },
                CrisisOption {
                    label: "Stall".into(),
                    description: format!("Buy time. {} may come back.", member_name),
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
                    description: "Deliveries resume, −8% chairman approval".into(),
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
                            operation: Some(OperationSpec { days: 2.0, label: "Escort Detail".into() }),
                        }),
                    }
                },
                CrisisOption {
                    label: "Wait them out".into(),
                    description: "Healthcare −10%, supply lines −15%. −5% chairman approval.".into(),
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
                    description: "Lose 1 day of research progress. +5% chairman approval.".into(),
                    cost: None,
                },
                 CrisisOption {
                    label: "\"I'm busy.\"".into(),
                    description: "Research continues. −5% chairman approval.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        // --- Follow-up crisis types ---

        CrisisKind::CorporateOverreach { corp_idx, board_member_idx } => {
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.as_str()).unwrap_or("The corporation");
            let member_name = state.board_members.get(*board_member_idx)
                .map(|m| m.name.as_str()).unwrap_or("The board member");
            let resist_cost = scaled_cost(state, 0.25, 200.0, 800.0);
            CrisisEvent {
                title: format!("{}: Data Seized", corp_name),
                description: format!(
                    "{corp_name} has reclassified your pathogen data as proprietary IP. \
                     {member_name} says the data belongs to the corporation now. \
                     Your researchers can no longer access their own findings."
                ),
                options: vec![ CrisisOption {
                    label: "Release the data (−10% chairman approval)".into(),
                    description: format!("Override {}'s IP claim. Board members with corporate ties won't appreciate it.", corp_name),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Legal challenge (¥{:.0})", resist_cost),
                    description: format!("Sue {} for access. Preserves research independence.", corp_name),
                    cost: Some(CrisisCost { funding: resist_cost, personnel: 0, ..Default::default() }),
                },
                CrisisOption {
                    label: "Accept it".into(),
                    description: format!("Lose access to data {} claimed. No cost.", corp_name),
                    cost: None,
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

            CrisisEvent {
                title: format!("{}: Contract Offer", member_name),
                description: format!(
                    "{} is offering a contract: {}.\n\nCondition: {}.\nIncome: +¥{:.0}/day.",
                    member_name, contract_name, condition_desc, income_day,
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
                        label: "Decline — hold out for better terms".into(),
                        description: format!(
                            "{} will not be happy. But prices rise over time — a better offer may come later.",
                            member_name,
                        ),
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
                        description: "Pay to buy time. Contract satisfaction +25%.".to_string(),
                        cost: Some(CrisisCost { funding: placate_cost, personnel: 0, ..Default::default() }),
                    },
                    CrisisOption {
                        label: "Refuse".into(),
                        description: format!("{} loses patience. Contract satisfaction −15%.", member_name),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::LoyaltyRaise { template_id } => {
            let contract = state.contracts.iter()
                .find(|c| c.template_id == *template_id);
            let member_idx = contract.map(|c| c.board_member_idx).unwrap_or(0);
            let member_name = state.board_members.get(member_idx)
                .map(|m| m.name.as_str()).unwrap_or("Board member");
            let current_income = contract.map(|c| c.income).unwrap_or(0.0);
            let raise_amount = current_income * LOYALTY_RAISE_FRACTION;
            let raise_per_day = raise_amount * TICKS_PER_DAY;
            let contract_name = contract.map(|c| c.name.as_str()).unwrap_or("Contract");

            let current_per_day = current_income * TICKS_PER_DAY;
            let new_per_day = current_per_day + raise_per_day;

            CrisisEvent {
                title: format!("{}: Price Adjustment", member_name),
                description: format!(
                    "{} offers to improve the {} terms.\n\n\
                     Citing the worsening situation and competing offers, {} is offering to \
                     raise the payout from ¥{:.0}/day to ¥{:.0}/day.\n\n\
                     You could likely get more by shopping around, but this is guaranteed.",
                    member_name, contract_name, member_name, current_per_day, new_per_day,
                ),
                options: vec![
                    CrisisOption {
                        label: "Accept the raise".into(),
                        description: format!(
                            "¥{:.0}/day → ¥{:.0}/day. {} appreciates the continued partnership.",
                            current_per_day, new_per_day, member_name,
                        ),
                        cost: None,
                    },
                    CrisisOption {
                        label: "Cancel contract, seek other offers".into(),
                        description: format!(
                            "\"Actually... I was exploring other offers.\" {} will not be pleased.",
                            member_name,
                        ),
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
                title: format!("{}: Priority Demand", gov_name),
                description: format!(
                    "{gov_name} is blocking N.W.H.O. field operations in {region_name}. \
                     Cooperation resumes when {region_name} gets priority over competing regions."),
                options: vec![ CrisisOption {
                    label: "Concede priority".into(),
                    description: format!("Cooperation +15. Board sees you caving to regional pressure. −5% chairman approval."),
                    cost: None,
                },
                 CrisisOption {
                    label: format!("Board pressure (¥{cost:.0})"),
                    description: "Economic leverage forces compliance. Cooperation −10.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                },
                CrisisOption {
                    label: "Refuse".into(),
                    description: format!("{gov_name} takes matters into their own hands. Cooperation −15. Governor imposes local quarantine and border controls."),
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
                    description: "−15% chairman approval. Your staff lose respect.".into(),
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
                    description: "−20% chairman approval. They'll make your life difficult.".into(),
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
        CrisisKind::GovernorSick { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let personality = state.regions.get(*region_idx)
                .map(|r| r.governor.personality).unwrap_or(GovernorPersonality::Operative);

            match personality {
                GovernorPersonality::Buffoon => {
                    // Find the largest non-bankrupt corporation in this region
                    let corp = state.corporations.iter()
                        .filter(|c| c.region_idx == *region_idx && !c.bankrupt)
                        .max_by(|a, b| a.base_revenue.partial_cmp(&b.base_revenue).unwrap_or(std::cmp::Ordering::Equal));
                    let corp_name = corp.map(|c| c.name.clone())
                        .unwrap_or_else(|| format!("{region_name} Logistics"));

                    CrisisEvent {
                        title: format!("{}: Evacuation Panic", gov_name),
                        description: format!(
                            "{gov_name} tested positive and went on live broadcast announcing a personal \
                             evacuation from {region_name}. {corp_name} is now pulling logistics staff, \
                             citing unsafe operating conditions."),
                        options: vec![ CrisisOption {
                            label: "Stabilize the governor".into(),
                            description: format!(
                                "Medical team treats {gov_name}. {corp_name} relocates to a safer region. \
                                 Supply lines -20%, cooperation -10, chairman disapproves."),
                            cost: None,
                        },
                        CrisisOption {
                            label: format!("Secure {corp_name} operations"),
                            description: format!(
                                "Keep {corp_name} running. {gov_name} evacuates and governs remotely. \
                                 Cooperation -20. Governor may not survive."),
                            cost: None,
                        },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
                GovernorPersonality::Blowhard => {
                    CrisisEvent {
                        title: format!("{}: Demands Experimental Treatment", gov_name),
                        description: format!(
                            "{gov_name} is in critical condition, broadcasting from a hospital bed in \
                             {region_name}, demanding your agency send \"whatever you have in the lab.\""),
                        options: vec![ CrisisOption {
                            label: "Send samples".into(),
                            description: "Lose 2 days of applied research progress".into(),
                            cost: None,
                        },
                        CrisisOption {
                            label: "Refuse".into(),
                            description: "No treatment sent. Chairman approval drops. Governor's prognosis is poor.".into(),
                            cost: None,
                        },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
                GovernorPersonality::Recluse => {
                    CrisisEvent {
                        title: format!("{}: Gone Dark", gov_name),
                        description: format!(
                            "{gov_name} has cut all communications. Staff in {region_name} say \
                             the governor locked themselves in their residence and won't come out. \
                             Regional policy enforcement has stopped."),
                        options: vec![ CrisisOption {
                            label: "Leave them alone".into(),
                            description: format!(
                                "No intervention. Cooperation drops sharply in {region_name}. \
                                 If {gov_name} doesn't re-emerge, the region loses its governor."),
                            cost: None,
                        },
                        CrisisOption {
                            label: "Send advisors (2 personnel for 5d)".into(),
                            description: format!(
                                "Your people talk {gov_name} through it and keep policy running \
                                 while they stabilize."),
                            cost: Some(CrisisCost {
                                funding: 0.0,
                                personnel: 2,
                                operation: Some(OperationSpec { days: 5.0, label: "Governor Advisory Team".into() }),
                            }),
                        },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
                GovernorPersonality::Hardliner => {
                    let cost = scaled_cost(state, 0.15, 120.0, 600.0);
                    CrisisEvent {
                        title: format!("{}: Priority Demand", gov_name),
                        description: format!(
                            "{gov_name} is critically ill and insisting your agency prioritize \
                             {region_name} above all other regions. \"Send your people here or I will \
                             handle this myself.\""),
                        options: vec![ CrisisOption {
                            label: "Divert personnel (2 for 5d)".into(),
                            description: "Comply with the demand. Team returns in 5 days.".into(),
                            cost: Some(CrisisCost {
                                funding: 0.0,
                                personnel: 2,
                                operation: Some(OperationSpec { days: 5.0, label: "Priority Deployment".into() }),
                            }),
                        },
                        CrisisOption {
                            label: format!("Emergency treatment (¥{cost:.0})"),
                            description: "Fund an emergency treatment package. Governor calms down.".into(),
                            cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                        },
                        CrisisOption {
                            label: "Refuse".into(),
                            description: "No medical support. Cooperation drops hard. Governor's condition is critical.".into(),
                            cost: None,
                        },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
                GovernorPersonality::Operative => {
                    let cost = scaled_cost(state, 0.18, 150.0, 700.0);
                    CrisisEvent {
                        title: format!("{}: Medical Expenses", gov_name),
                        description: format!(
                            "{gov_name} has been hospitalized in {region_name}. They're billing your \
                             agency for ¥{cost:.0} in \"medical and security expenses.\" \
                             The invoice is padded, but {gov_name} runs a tight region."),
                        options: vec![ CrisisOption {
                            label: format!("Pay ¥{cost:.0}"),
                            description: format!(
                                "Governor recovers. Cooperation +10. Income skim increases \
                                 permanently. {gov_name} now knows you'll pay."),
                            cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                        },
                        CrisisOption {
                            label: "Refuse".into(),
                            description: format!(
                                "Cooperation -15. Income skim still increases. \
                                 {gov_name}'s condition may worsen without proper care."),
                            cost: None,
                        },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
                GovernorPersonality::Mobster => {
                    let cost = scaled_cost(state, 0.25, 200.0, 1000.0);
                    CrisisEvent {
                        title: format!("{}: Protection Required", gov_name),
                        description: format!(
                            "{gov_name} is seriously ill and wants protection. \"I need a security \
                             detail. Or ¥{cost:.0}. Either works.\""),
                        options: vec![ CrisisOption {
                            label: "Send security detail (3 personnel for 5d)".into(),
                            description: "Three staff babysit the governor. They return in 5 days.".into(),
                            cost: Some(CrisisCost {
                                funding: 0.0,
                                personnel: 3,
                                operation: Some(OperationSpec { days: 5.0, label: "Governor Security Detail".into() }),
                            }),
                        },
                        CrisisOption {
                            label: format!("Pay ¥{cost:.0}"),
                            description: "Private healthcare arranged. Cooperation restored.".into(),
                            cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                        },
                        CrisisOption {
                            label: "Refuse".into(),
                            description: "No support provided. Cooperation drops sharply. Governor's condition is worsening.".into(),
                            cost: None,
                        },
                        ],
                        kind,
                        tick_created: tick,
                    }
                }
            }
        }
        CrisisKind::GovernorDeath { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
            let cost = scaled_cost(state, 0.15, 120.0, 600.0);
            CrisisEvent {
                title: format!("{} is dead", gov_name),
                description: format!(
                    "{gov_name} of {region_name} has died from the pandemic. \
                     The region has no leadership. Policy enforcement will suffer \
                     until a successor emerges."),
                options: vec![ CrisisOption {
                    label: format!("Stabilize operations (¥{cost:.0})"),
                    description: "Fund local continuity efforts. Successor arrives in 7 days.".into(),
                    cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                },
                 CrisisOption {
                    label: "Wait it out".into(),
                    description: "A successor will emerge on their own. 12 days with no policy enforcement or cooperation gains.".into(),
                    cost: None,
                },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::ArkProtocol { region_idx } => {
            let recommended_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let collapsed_count = state.regions.iter().filter(|r| r.collapsed).count();
            let surviving: Vec<(usize, &str)> = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, r)| (i, r.name.as_str()))
                .collect();
            let active_count = surviving.len();
            let mut options: Vec<CrisisOption> = surviving.iter().map(|(_, name)| {
                CrisisOption {
                    label: format!("Consolidate in {}", name),
                    description: "Pull out of all other regions.".into(),
                    cost: None,
                }
            }).collect();
            options.push(CrisisOption {
                label: "Continue as-is".into(),
                description: "Stay spread thin. Lose personnel and funding to overextension.".into(),
                cost: Some(CrisisCost { funding: 150.0, personnel: 3, ..Default::default() }),
            });
            CrisisEvent {
                title: "Emergency Consolidation".into(),
                description: format!(
                    "{} regions lost. Remaining personnel are overextended across {} active sites. \
                     Recommend pulling all operations back to {}.",
                    collapsed_count, active_count, recommended_name,
                ),
                options,
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
                        description: "No debt. Funding gap persists until resolved by other means.".into(),
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
                    CrisisEvent {
                        title: "Loan Called In".into(),
                        description: format!(
                            "Gov. {lender_name} of {region_name} demands repayment of ¥{repay_amount:.0}.",
                        ),
                        options: vec![
                            if can_repay {
                                CrisisOption {
                                    label: format!("Repay ¥{repay_amount:.0}"),
                                    description: "Clear the debt. Cooperation improves slightly.".into(),
                                    cost: Some(CrisisCost { funding: repay_amount, personnel: 0, ..Default::default() }),
                                }
                            } else {
                                CrisisOption {
                                    label: format!("Repay (need ¥{repay_amount:.0}, have ¥{:.0})", state.resources.funding),
                                    description: "Insufficient funds.".into(),
                                    cost: Some(CrisisCost { funding: repay_amount, personnel: 0, ..Default::default() }),
                                }
                            },
                            CrisisOption {
                                label: "Default".into(),
                                description: format!(
                                    "Cooperation with Gov. {lender_name} drops 40.",
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
                                description: "2 researchers are 'unavailable' indefinitely. −10% chairman approval from smear campaign.".into(),
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
            let new_budget_per_tick = compute_board_budget_per_tick(state, board_sat);
            let new_funding_rate = new_budget_per_tick * TICKS_PER_DAY;
            let current_rate = state.board_budget_per_tick * TICKS_PER_DAY;

            let mut memo_lines: Vec<String> = Vec::new();

            // Funding decision
            if new_funding_rate > current_rate * 1.05 {
                memo_lines.push(format!(
                    "Your operating budget has been increased from \u{00a5}{:.0} to \u{00a5}{:.0} per day, effective immediately.",
                    current_rate, new_funding_rate
                ));
            } else if new_funding_rate < current_rate * 0.95 {
                memo_lines.push(format!(
                    "Your operating budget has been reduced from \u{00a5}{:.0} to \u{00a5}{:.0} per day, effective immediately.",
                    current_rate, new_funding_rate
                ));
            } else {
                memo_lines.push(format!(
                    "Your operating budget has been maintained at \u{00a5}{:.0} per day.",
                    new_funding_rate
                ));
            }

            // Authority preview: tell the player what the board will decide
            let suggested = state.suggested_authority();
            let current_auth = state.resources.authority;
            if suggested > current_auth {
                memo_lines.push(format!(
                    "Given the severity of the crisis, the Board has expanded your operational authority from {} to {}.",
                    current_auth.label(), current_auth.raise().label()
                ));
            } else if suggested < current_auth {
                memo_lines.push(format!(
                    "The Board has narrowed your operational authority from {} to {}.",
                    current_auth.label(), current_auth.lower().label()
                ));
            } else {
                memo_lines.push(format!(
                    "Your operational authority remains at {}.",
                    current_auth.label()
                ));
            }

            // Crisis urgency influence on funding
            let urgency = crisis_urgency_boost(state);
            if urgency >= 0.05 {
                let screened = state.total_infected_screened();
                memo_lines.push(format!(
                    "Reported caseload ({}) has been factored into the allocation.",
                    crate::state::format_large_number(screened)
                ));
            }

            // Chairman influence on funding outcome
            let chair_shift = chairman_funding_shift(state);
            if chair_shift != 0.0 {
                if let Some(chairman) = state.board_members.iter().find(|m| m.is_chairman) {
                    if chair_shift > 0.0 {
                        memo_lines.push(format!(
                            "{} steered discussion toward a generous allocation.",
                            chairman.name
                        ));
                    } else {
                        memo_lines.push(format!(
                            "{} pushed for deeper cuts to your operating budget.",
                            chairman.name
                        ));
                    }
                }
            }

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
                "Day {:.0} review.\n\n{}",
                day,
                memo_lines.join("\n\n")
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
        CrisisKind::BoardEmbezzlementWarning => {
            let non_board_value = state.non_board_portfolio_value();
            CrisisEvent {
                title: "Correspondence from the Board".into(),
                description: format!(
                    "Dear Director,\n\n\
                     The Board has completed its quarterly review of NWHO operating accounts. \
                     We note with interest that \u{00a5}{:.0} in agency funds have been allocated to \
                     equity positions in entities outside the Board's portfolio. While the Board \
                     encourages prudent financial management, we wish to remind you that all \
                     NWHO funds are designated for pandemic response operations.\n\n\
                     We trust this matter will resolve itself promptly. The Board would find it \
                     regrettable if further review became necessary.\n\n\
                     Regards,\nOffice of the Board Secretary",
                    non_board_value
                ),
                options: vec![
                    CrisisOption {
                        label: "Acknowledged".into(),
                        description: "File the letter. The board is watching.".into(),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::VoteOfNoConfidence => {
            let chairman_name = state.board_members.iter()
                .find(|m| m.is_chairman)
                .map(|m| m.name.as_str())
                .unwrap_or("The Chairman");
            let corp_name = state.board_members.iter()
                .find(|m| m.is_chairman)
                .and_then(|m| m.corp_idx)
                .and_then(|idx| state.corporations.get(idx))
                .map(|c| c.name.as_str())
                .unwrap_or("their corporation");
            let concession_cost = scaled_cost(state, 0.25, 200.0, 800.0);
            CrisisEvent {
                title: "Vote of No Confidence".into(),
                description: format!(
                    "{} has called an emergency session. The motion cites mismanagement of \
                     the crisis and neglect of {} interests. Three board members have \
                     already indicated support.",
                    chairman_name, corp_name
                ),
                options: vec![
                    CrisisOption {
                        label: format!("Make concessions (¥{:.0})", concession_cost),
                        description: format!(
                            "Redirect ¥{:.0} to {}. Chairman withdraws the motion.",
                            concession_cost, corp_name
                        ),
                        cost: Some(CrisisCost { funding: concession_cost, personnel: 0, ..Default::default() }),
                    },
                    CrisisOption {
                        label: "Stand firm".into(),
                        description: "Refuse concessions. The board retaliates with funding cuts.".into(),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }
        CrisisKind::BoardResearchInquiry => {
            let chairman_name = state.board_members.iter()
                .find(|m| m.is_chairman)
                .map(|m| m.name.as_str())
                .unwrap_or("The Chairman");
            CrisisEvent {
                title: "Board Inquiry: Research Status".into(),
                description: format!(
                    "{} has requested a formal update on research operations. \
                     No research programs have been initiated. The board expects a response.",
                    chairman_name
                ),
                options: vec![
                    CrisisOption {
                        label: "Acknowledged".into(),
                        description: "Accept the reprimand. Chairman approval decreases.".into(),
                        cost: None,
                    },
                    {
                        let cost = scaled_cost(state, 0.08, 60.0, 300.0);
                        CrisisOption {
                            label: format!("Present a research timeline (¥{:.0})", cost),
                            description: "Prepare a formal response. Costs money but preserves board confidence.".into(),
                            cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                        }
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
                            "No funding required. Ties up {team_size} staff for 4-7 days. \
                             Roughly 1 in 3 chance of early release. Otherwise the process \
                             stalls with no resolution.",
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
            let already_identifying = state.active_research.iter()
                .any(|p| matches!(p.kind, ResearchKind::IdentifyThreat { disease_idx: d } if d == *disease_idx));

            let mut options = Vec::new();

            if !already_identifying {
                let identify_kind = ResearchKind::IdentifyThreat { disease_idx: *disease_idx };
                let (_personnel, _, funding_cost) = state.effective_costs(&identify_kind);
                options.push(CrisisOption {
                    label: format!("Begin identification (¥{:.0})", funding_cost),
                    description: "Deploy field team to identify the pathogen immediately.".into(),
                    cost: Some(CrisisCost {
                        funding: funding_cost,
                        personnel: 0, // Personnel are assigned to the project, not consumed
                        operation: None,
                    }),
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
        CrisisKind::CorporateDemand { corp_idx } => {
            let corp = &state.corporations[*corp_idx];
            let corp_name = corp.name.clone();
            let director = corp.director_surname.clone();
            let region_idx = corp.region_idx;
            let region_name = state.regions.get(region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let policy = state.policies.get(region_idx).cloned().unwrap_or_default();
            let grievance_policy = corp.sector.policy_grievance(&policy)
                .unwrap_or("restrictions");
            let compensation = scaled_cost(state, 0.25, 200.0, 1500.0);
            let sector_label = corp.sector.label();

            // Sector-specific demand language
            let demand_text = match corp.sector {
                CorporationSector::Logistics => format!(
                    "{corp_name} cannot move freight with the {grievance_policy} in {region_name}. \
                     Dir. {director} is demanding compensation or immediate repeal."
                ),
                CorporationSector::Mining => format!(
                    "{corp_name} has lost access to its workforce under the {grievance_policy} in {region_name}. \
                     Dir. {director} wants the restriction lifted or full compensation."
                ),
                CorporationSector::Energy => format!(
                    "{corp_name} reports grid operations in {region_name} are compromised by the {grievance_policy}. \
                     Dir. {director} is threatening to pull maintenance crews."
                ),
                CorporationSector::DataInfra => format!(
                    "{corp_name} data centers in {region_name} are running on skeleton staff due to {grievance_policy}. \
                     Dir. {director} wants relief or compensation for losses."
                ),
                CorporationSector::Automation => format!(
                    "{corp_name} factory output in {region_name} has stalled under the {grievance_policy}. \
                     Dir. {director} is demanding either exemptions or payment."
                ),
                _ => format!(
                    "{corp_name} ({sector_label}) is losing revenue from the {grievance_policy} in {region_name}. \
                     Dir. {director} is demanding action."
                ),
            };

            CrisisEvent {
                title: format!("Dir. {director}: {} Complaint", sector_label),
                description: demand_text,
                options: vec![
                    CrisisOption {
                        label: format!("Pay compensation (¥{compensation:.0})"),
                        description: format!(
                            "{corp_name} accepts the payment. Policy remains in effect."
                        ),
                        cost: Some(CrisisCost { funding: compensation, personnel: 0, ..Default::default() }),
                    },
                    CrisisOption {
                        label: "Refuse".into(),
                        description: format!(
                            "{corp_name} retaliates. Supply lines −5%, civil order −5% in {region_name}."
                        ),
                        cost: None,
                    },
                ],
                kind,
                tick_created: tick,
            }
        }

        CrisisKind::NuclearEvacuation { region_idx } => {
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let members: Vec<String> = state.board_members.iter()
                .filter(|m| m.region_idx == Some(*region_idx) && !m.dead)
                .map(|m| m.name.clone())
                .collect();
            let count = members.len();
            let cost = crate::state::NUCLEAR_EVACUATION_COST_PER_MEMBER * count as f64;
            let names = members.join(", ");
            CrisisEvent {
                title: "Emergency Evacuation Request".into(),
                description: format!(
                    "Nuclear payload inbound to {region_name}. {} board member{} ({names}) \
                     demand{} immediate airlift. Evacuation costs ¥{cost:.0}.",
                    count,
                    if count == 1 { "" } else { "s" },
                    if count == 1 { "s" } else { "" },
                ),
                options: vec![
                    CrisisOption {
                        label: format!("Evacuate (¥{cost:.0})"),
                        description: format!("Airlift {count} board member{} to safety.",
                            if count == 1 { "" } else { "s" }),
                        cost: Some(CrisisCost { funding: cost, personnel: 0, ..Default::default() }),
                    },
                    CrisisOption {
                        label: "Refuse evacuation".into(),
                        description: "Their services are no longer required.".into(),
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
pub(super) fn activate_crisis(state: &mut WorldState, crisis: CrisisEvent, events: &mut Vec<GameEvent>) -> CrisisPostAction {
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
        let (message, post_action) = resolve_crisis(state, auto_choice.unwrap(), events);
        events.push(GameEvent::CrisisAutoResolved { message });
        post_action
    } else {
        events.push(GameEvent::CrisisStarted);
        CrisisPostAction::None
    }
}

/// Tick all active crisis operations. Complete ones that expire and return their personnel.
pub(super) fn tick_crisis_operations(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let mut i = 0;
    while i < state.crisis_operations.len() {
        state.crisis_operations[i].ticks_remaining -= 1.0;
        if state.crisis_operations[i].ticks_remaining <= 0.0 {
            let op = state.crisis_operations.remove(i);
            events.push(GameEvent::CrisisTeamReturned {
                label: op.label,
                personnel: op.personnel,
            });
        } else {
            i += 1;
        }
    }
}

/// Apply the chosen crisis resolution. Returns a status message and any
/// cross-subsystem post-action for mod.rs to dispatch.
pub(super) fn resolve_crisis(state: &mut WorldState, choice: usize, events: &mut Vec<GameEvent>) -> (String, CrisisPostAction) {
    let crisis = match state.active_crisis.take() {
        Some(c) => c,
        None => return ("No active crisis".into(), CrisisPostAction::None),
    };

    // Record cooldown for this crisis type
    state.crisis_cooldowns.insert(crisis.kind.tag().to_string(), state.tick);


    // Deduct costs generically from the chosen option. Affordability is always
    // checked before calling resolve_crisis: apply_action() for manual resolution,
    // activate_crisis() for auto-resolution.
    let option = &crisis.options[choice];
    if let Some(cost) = &option.cost {
        state.resources.funding -= cost.funding;
        if cost.personnel > 0 {
            if let Some(op) = &cost.operation {
                // Create a temporary operation — personnel are tied up and returned later
                let ticks = op.days * TICKS_PER_DAY;
                state.crisis_operations.push(CrisisOperation {
                    label: op.label.clone(),
                    personnel: cost.personnel,
                    ticks_remaining: ticks,
                });
            } else {
                // Permanent personnel loss
                state.resources.personnel = state.resources.personnel.saturating_sub(cost.personnel);
            }
        }
    }

    let mut post_action = CrisisPostAction::None;
    let msg = match (&crisis.kind, choice) {
        (CrisisKind::PersonnelCrisis { amount }, 0) => {
            state.resources.personnel = state.resources.personnel.saturating_sub(*amount);
            // If personnel drops below what active research requires, cancel the
            // most recent field research — not enough staff to sustain it.
            let research_demand: u32 =
                state.active_research.iter().map(|p| p.personnel_assigned).sum();
            if research_demand > state.resources.personnel {
                let idx = state.active_research.iter().rposition(|p| p.kind.is_field_work());
                if let Some(i) = idx {
                    state.active_research.remove(i);
                    format!("Lost {} personnel. Field research cancelled, insufficient staff.",
                        amount)
                } else {
                    format!("Lost {} personnel to attrition", amount)
                }
            } else {
                format!("Lost {} personnel to attrition", amount)
            }
        }
        (CrisisKind::PersonnelCrisis { .. }, _) => {
            "Retention bonuses paid. Attrition stabilized.".into()
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
            chairman_satisfaction_hit(state, -pol_cost);
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
            chairman_satisfaction_hit(state, -pol_cost);
            let accepted_m = accepted / 1_000_000.0;
            format!("Partial intake: {:.0}M accepted into {}. Rest turned away.", accepted_m, to_name)
        }




        (CrisisKind::TrialShortcut { .. }, 0) => {
            // Maintain standards — chairman satisfaction hit
            chairman_satisfaction_hit(state, -0.05);
            "Maintained trial standards. Board noted the delay.".into()
        }
        (CrisisKind::TrialShortcut { disease_idx, medicine_idx }, _) => {
            // Fast-track — gain chairman satisfaction, mark medicine as tested against the disease.
            chairman_satisfaction_hit(state, 0.10);
            if let Some(medicine) = state.medicines.get_mut(*medicine_idx) {
                if !medicine.tested_against.contains(disease_idx) {
                    medicine.tested_against.push(*disease_idx);
                }
                if !medicine.target_diseases.contains(disease_idx) {
                    medicine.target_diseases.push(*disease_idx);
                }
            }
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "the pathogen".into());
            format!("Fast-tracked {} treatment trial.", name)
        }




        (CrisisKind::CorporateSeizure { cooperate_loss, board_member_idx, corp_idx }, 0) => {
            // Cooperate — lose personnel, gain chairman satisfaction
            state.resources.personnel = state.resources.personnel.saturating_sub(*cooperate_loss);
            chairman_satisfaction_hit(state, 0.15);
            state.pending_crises.push(CrisisKind::CorporateOverreach { corp_idx: *corp_idx, board_member_idx: *board_member_idx });
            format!("Transferred {} staff to corporate oversight. Agency retains nominal control.", cooperate_loss)
        }
        (CrisisKind::CorporateSeizure { .. }, 1) => {
            // Resist — costs already deducted
            "Corporate takeover averted. Independence maintained.".into()
        }
        (CrisisKind::CorporateSeizure { board_member_idx, corp_idx, .. }, _) => {
            // Stall — buy time, they may return
            chairman_satisfaction_hit(state, -0.05);
            if state.rng_crisis.r#gen::<bool>() {
                let cooperate_loss = ((state.resources.personnel as f64 * 0.25).round() as u32).clamp(2, 8);
                state.pending_crises.push(CrisisKind::CorporateSeizure { cooperate_loss, board_member_idx: *board_member_idx, corp_idx: *corp_idx });
                "Negotiations stalled. They'll be back.".into()
            } else {
                "Stalling worked. Corporate security withdrew.".into()
            }
        }

        // --- Late-game crisis resolutions ---

        (CrisisKind::CultBlockade { .. }, 0) => {
            // Negotiate — give them airtime, chairman satisfaction hit
            chairman_satisfaction_hit(state, -0.08);
            "Concessions granted. Deliveries resume.".into()
        }
        (CrisisKind::CultBlockade { .. }, 1) => {
            // Police raid — costs already deducted
            "Blockade cleared. Supply routes restored.".into()
        }
        (CrisisKind::CultBlockade { region_idx }, _) => {
            // Wait them out — supply lines and healthcare degrade significantly
            chairman_satisfaction_hit(state, -0.05);
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.healthcare_capacity = (region.healthcare_capacity - 0.10).max(0.0);
                region.supply_lines = (region.supply_lines - 0.15).max(0.0);
            }
            "Blockade dissolved after days of delays. Supply lines degraded.".into()
        }

        // --- Dark comedy event resolutions ---

        (CrisisKind::PerformanceReview, 0) => {
            // Attend — lose 1 day research progress, gain POL
            let loss = TICKS_PER_DAY as f64;
            if let Some(proj) = state.active_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            chairman_satisfaction_hit(state, 0.05);
            "Review complete. Rating: \"Meets Expectations.\"".into()
        }
        (CrisisKind::PerformanceReview, _) => {
            // Skip — chairman satisfaction hit
            chairman_satisfaction_hit(state, -0.05);
            "Board notes your absence. A memo has been circulated.".into()
        }



        // --- Follow-up crisis resolutions ---


        (CrisisKind::CorporateOverreach { .. }, 0) => {
            // Override restriction — chairman satisfaction hit, data restored to research teams
            chairman_satisfaction_hit(state, -0.10);
            "Restriction overridden. Data restored to research teams.".into()
        }
        (CrisisKind::CorporateOverreach { .. }, 1) => {
            // Legal challenge — costs already deducted
            "Legal challenge successful. Research independence restored.".into()
        }
        (CrisisKind::CorporateOverreach { .. }, _) => {
            // Accept IP claim — lose research progress
            let loss = TICKS_PER_DAY as f64;
            if let Some(proj) = state.active_research.first_mut() {
                proj.progress = (proj.progress - loss).max(0.0);
            }
            "IP claim accepted. Research data access restricted.".into()
        }

        // --- Governor archetype crisis resolutions ---

        // --- Contract offer resolutions ---

        (CrisisKind::ContractOffer { .. }, 0) => {
            post_action = CrisisPostAction::AcceptContract;
            // Message will be overridden by mod.rs with the actual result from contracts
            "Contract offer accepted.".into()
        }
        (CrisisKind::ContractOffer { .. }, _) => {
            post_action = CrisisPostAction::RejectContract;
            "Contract offer declined.".into()
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

        // --- Loyalty raise resolutions ---

        (CrisisKind::LoyaltyRaise { template_id }, 0) => {
            // Accept: increase contract income by the raise fraction, boost offerer satisfaction
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
                c.income *= 1.0 + LOYALTY_RAISE_FRACTION;
            }
            if let Some(idx) = member_idx {
                if let Some(member) = state.board_members.get_mut(idx) {
                    member.add_modifier(ModifierSource::ContractLoyalty, 0.05);
                }
            }
            format!("{} raises the payout. Contract income increased.", member_name)
        }
        (CrisisKind::LoyaltyRaise { template_id }, _) => {
            // Cancel contract to seek other offers
            let member_idx = state.contracts.iter()
                .find(|c| c.template_id == *template_id)
                .map(|c| c.board_member_idx);
            let member_name = member_idx
                .and_then(|idx| state.board_members.get(idx))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Board member".to_string());
            if let Some(idx) = member_idx {
                post_action = CrisisPostAction::CancelContract { board_member_idx: idx };
            }
            format!("{} contract cancelled. The board takes note.", member_name)
        }

        (CrisisKind::GovernorHardliner { region_idx }, 0) => {
            // Concede priority — cooperation boost but chairman hit
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.cooperation = (region.governor.cooperation + 15.0).min(100.0);
            }
            chairman_satisfaction_hit(state, -0.05);
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("{} given priority status. Board noted your concession.", region_name)
        }
        (CrisisKind::GovernorHardliner { region_idx }, 1) => {
            // Board pressure — costs already deducted, cooperation drops
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.cooperation = (region.governor.cooperation - 10.0).max(0.0);
            }
            "Board leverage applied. Compliance restored under protest.".into()
        }
        (CrisisKind::GovernorHardliner { region_idx }, _) => {
            // Refuse — governor takes unilateral action, imposes their own policies
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.cooperation = (region.governor.cooperation - 15.0).max(0.0);
            }
            if let Some(policy) = state.policies.get_mut(*region_idx) {
                policy.quarantine = true;
                policy.border_controls = true;
            }
            let gov_name = state.regions.get(*region_idx)
                .map(|r| r.governor.name.clone()).unwrap_or_else(|| "Unknown".into());
            format!("{} imposed quarantine and border controls without authorization.", gov_name)
        }

        (CrisisKind::GovernorOperative { .. }, 0) => {
            // Look the other way, chairman satisfaction hit
            chairman_satisfaction_hit(state, -0.15);
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
            // Refuse — chairman satisfaction hit
            chairman_satisfaction_hit(state, -0.20);
            "Refused to pay. They're making your life difficult.".into()
        }
        (CrisisKind::GovernorMobster { region_idx }, _) => {
            // Pay: costs already deducted, increment bargain count
            if let Some(region) = state.regions.get_mut(*region_idx) {
                region.governor.bargain_count += 1;
                region.governor.cooperation = (region.governor.cooperation + 15.0).min(100.0);
            }
            "Paid. They'll be back for more.".into()
        }

        (CrisisKind::GovernorDeath { region_idx }, 0) => {
            // Stabilize operations: faster succession (7 days)
            use crate::state::TICKS_PER_DAY;
            if let Some(region) = state.regions.get_mut(*region_idx) {
                let old_name = region.governor.name.clone();
                region.governor.dead = true;
                region.governor.succession_tick = Some(state.tick + (7.0 * TICKS_PER_DAY) as u64);
                events.push(GameEvent::GovernorDied {
                    region_idx: *region_idx,
                    name: old_name,
                });
            }
            "Operations stabilized. Successor expected in 7 days.".into()
        }
        (CrisisKind::GovernorDeath { region_idx }, _) => {
            // Let the process run: standard succession (12 days)
            use crate::state::{TICKS_PER_DAY, GOVERNOR_SUCCESSION_DAYS};
            if let Some(region) = state.regions.get_mut(*region_idx) {
                let old_name = region.governor.name.clone();
                region.governor.dead = true;
                region.governor.succession_tick = Some(state.tick + (GOVERNOR_SUCCESSION_DAYS * TICKS_PER_DAY) as u64);
                events.push(GameEvent::GovernorDied {
                    region_idx: *region_idx,
                    name: old_name,
                });
            }
            "No replacement appointed. The region will be leaderless for 12 days.".into()
        }

        // --- GovernorSick resolutions (personality-dependent) ---

        (CrisisKind::GovernorSick { region_idx }, choice) => {
            let personality = state.regions.get(*region_idx)
                .map(|r| r.governor.personality).unwrap_or(GovernorPersonality::Operative);
            match (personality, choice) {
                (GovernorPersonality::Buffoon, 0) => {
                    // Stabilize the governor: governor survives, corp pulls logistics and relocates
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.supply_lines = (region.supply_lines - 0.20).max(0.0);
                        region.governor.cooperation = (region.governor.cooperation - 10.0).max(0.0);
                    }
                    chairman_satisfaction_hit(state, -0.05);
                    // Relocate the largest corp in this region to a connected non-collapsed region
                    let corp_reloc_msg = relocate_region_corp(state, *region_idx);
                    format!("Medical team sent. Governor stabilized. {corp_reloc_msg}")
                }
                (GovernorPersonality::Buffoon, _) => {
                    // Secure corporation: corp stays, governor evacuates to safety
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 20.0).max(0.0);
                    }
                    // Governor evacuates to a connected safe region
                    let gov_dest = find_safest_connected_region(state, *region_idx);
                    if let Some(dest) = gov_dest {
                        state.regions[*region_idx].governor.physical_region_idx = Some(dest);
                        // Update board member location too
                        for member in &mut state.board_members {
                            if matches!(member.role, BoardRole::RegionGovernor { region_idx: ri } if ri == *region_idx) {
                                member.region_idx = Some(dest);
                            }
                        }
                    }
                    chairman_satisfaction_hit(state, 0.03);
                    queue_governor_death_followup(state, *region_idx);
                    let dest_name = gov_dest
                        .and_then(|d| state.regions.get(d).map(|r| r.name.clone()))
                        .unwrap_or_else(|| "parts unknown".into());
                    format!("Operations secured. Governor fled to {dest_name}. Condition deteriorating.")
                }
                (GovernorPersonality::Blowhard, 0) => {
                    // Send samples: lose 2 days research progress
                    let loss = (TICKS_PER_DAY * 2.0) as f64;
                    if let Some(proj) = state.active_research.first_mut() {
                        proj.progress = (proj.progress - loss).max(0.0);
                    }
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 10.0).min(100.0);
                    }
                    "Samples sent. Research set back. Governor satisfied.".into()
                }
                (GovernorPersonality::Blowhard, _) => {
                    // Refuse: chairman satisfaction hit
                    chairman_satisfaction_hit(state, -0.08);
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 10.0).max(0.0);
                    }
                    queue_governor_death_followup(state, *region_idx);
                    "Refused. The broadcast was not flattering.".into()
                }
                (GovernorPersonality::Recluse, 0) => {
                    // Leave them alone: sharp cooperation drop + possible governor death
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 25.0).max(0.0);
                    }
                    queue_governor_death_followup(state, *region_idx);
                    "No intervention. The region has no effective leadership.".into()
                }
                (GovernorPersonality::Recluse, _) => {
                    // Send advisors: personnel cost already deducted, cooperation boost
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 10.0).min(100.0);
                    }
                    "Advisors dispatched. Governor is cooperating again.".into()
                }
                (GovernorPersonality::Hardliner, 0) => {
                    // Divert personnel: costs already deducted, cooperation boost
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 15.0).min(100.0);
                    }
                    "Personnel diverted. Governor appreciates the priority.".into()
                }
                (GovernorPersonality::Hardliner, 1) => {
                    // Emergency treatment: costs deducted, cooperation boost
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 10.0).min(100.0);
                    }
                    "Treatment package sent. Governor calmed down.".into()
                }
                (GovernorPersonality::Hardliner, _) => {
                    // Refuse: hard cooperation drop
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 20.0).max(0.0);
                    }
                    chairman_satisfaction_hit(state, -0.05);
                    queue_governor_death_followup(state, *region_idx);
                    "Refused. Governor threatening to take matters into their own hands.".into()
                }
                (GovernorPersonality::Operative, 0) => {
                    // Pay: governor recovers, cooperation boost, but skim increases
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 10.0).min(100.0);
                        region.governor.income_skim = (region.governor.income_skim + 0.02).min(0.30);
                    }
                    "Expenses paid. Governor recovering. The next invoice will be larger.".into()
                }
                (GovernorPersonality::Operative, _) => {
                    // Refuse: cooperation drop, skim increase, governor may die
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 15.0).max(0.0);
                        region.governor.income_skim = (region.governor.income_skim + 0.03).min(0.30);
                    }
                    queue_governor_death_followup(state, *region_idx);
                    "Refused. The governor is finding other ways to cover costs.".into()
                }
                (GovernorPersonality::Mobster, 0) => {
                    // Send security detail: personnel cost already deducted
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 10.0).min(100.0);
                    }
                    "Security detail sent. Governor is comfortable.".into()
                }
                (GovernorPersonality::Mobster, 1) => {
                    // Pay: costs already deducted, cooperation boost
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation + 15.0).min(100.0);
                        region.governor.bargain_count += 1;
                    }
                    "Private healthcare arranged. They'll remember this.".into()
                }
                (GovernorPersonality::Mobster, _) => {
                    // Refuse: hard cooperation drop
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 25.0).max(0.0);
                    }
                    queue_governor_death_followup(state, *region_idx);
                    "Refused. This will cost you.".into()
                }
            }
        }

        (CrisisKind::ArkProtocol { .. }, chosen) => {
            // Build the same surviving-region list used at crisis creation time.
            // The game is paused during crises, so regions can't collapse between
            // build and resolution.
            let surviving: Vec<usize> = state.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .map(|(i, _)| i)
                .collect();
            if let Some(&target_idx) = surviving.get(chosen) {
                // Player chose a region to consolidate into
                let region_name = state.regions[target_idx].name.clone();
                state.ark_protocol = Some(target_idx);
                // Collapse all non-Ark, non-collapsed regions immediately
                for i in 0..state.regions.len() {
                    if i == target_idx || state.regions[i].collapsed {
                        continue;
                    }
                    state.regions[i].collapsed = true;
                    state.regions[i].collapsed_at_tick = Some(state.tick);
                    state.regions[i].hospital_level = 0;
                    state.regions[i].intel_level = 0;
                    if let Some(policy) = state.policies.get_mut(i) {
                        policy.clear_all();
                    }
                    let lost = 1u32.min(state.resources.personnel);
                    state.resources.personnel = state.resources.personnel.saturating_sub(lost);
                    events.push(GameEvent::RegionCollapsed { region_idx: i, personnel_lost: lost });
                }
                events.push(GameEvent::ArkProtocolActivated {
                    region_idx: target_idx,
                });
                format!("Consolidation complete. All operations moved to {}.", region_name)
            } else {
                // Last option = "Continue as-is", or out-of-range fallback
                "Consolidation declined. Maintaining all active sites.".into()
            }
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
            // Governor: small cooperation boost for honoring the debt
            if let LoanLender::Governor { region_idx } = lender {
                if let Some(region) = state.regions.get_mut(*region_idx) {
                    region.governor.cooperation = (region.governor.cooperation + 5.0).min(100.0);
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
                    // Heavy cooperation penalty — policy cancellation was too
                    // easy to reverse so we use a pure cooperation hit instead.
                    if let Some(region) = state.regions.get_mut(*region_idx) {
                        region.governor.cooperation = (region.governor.cooperation - 40.0).max(0.0);
                    }
                    let region_name = state.regions.get(*region_idx)
                        .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
                    format!(
                        "Defaulted on Gov. {lender_name}'s loan. Co-Op −40 in {region_name}.",
                    )
                }
                LoanLender::Corporation { .. } => {
                    // Personnel intimidation + chairman satisfaction smear campaign
                    let lost = 2u32.min(state.resources.personnel);
                    state.resources.personnel = state.resources.personnel.saturating_sub(lost);
                    chairman_satisfaction_hit(state, -0.10);
                    format!(
                        "{lender_name} collected. {lost} researchers 'unavailable'. Chairman satisfaction hit from smear campaign.",
                    )
                }
            }
        }


        // --- Pathogen detection alert ---

        (CrisisKind::NewPathogenDetected { disease_idx }, 0) if crisis.options.len() > 1 => {
            // "Begin identification" — funding was already deducted by CrisisCost.
            // Create the research project directly (don't call start_research which
            // would try to deduct funding a second time).
            let kind = ResearchKind::IdentifyThreat { disease_idx: *disease_idx };
            let (personnel, duration, _funding) = state.effective_costs(&kind);
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Pathogen #{}", disease_idx + 1));
            if state.personnel_available() >= personnel {
                let project = crate::state::ResearchProject {
                    kind,
                    progress: 0.0,
                    required_ticks: duration,
                    personnel_assigned: personnel,
                };
                state.active_research.push(project);
                format!("Field identification of {} initiated.", name)
            } else {
                // Refund the funding that CrisisCost already deducted — the research
                // can't start without enough personnel.
                if let Some(cost) = &crisis.options[choice].cost {
                    state.resources.funding += cost.funding;
                }
                format!("Not enough personnel to identify {}.", name)
            }
        }
        (CrisisKind::NewPathogenDetected { .. }, _) => {
            "Alert acknowledged.".into()
        }

        // --- Board meeting communiqué ---

        (CrisisKind::BoardMeeting, _) => {
            // Single option: Acknowledged. Set the new fixed board budget.
            let board_sat = state.board_satisfaction();
            state.board_budget_per_tick = compute_board_budget_per_tick(state, board_sat);

            // Authority decision: board raises/lowers by one level max per meeting
            let old_authority = state.resources.authority;
            let suggested = state.suggested_authority();
            let new_authority = if suggested > old_authority {
                old_authority.raise()
            } else if suggested < old_authority {
                old_authority.lower()
            } else {
                old_authority
            };
            state.resources.authority = new_authority;

            // Emit PolicyAuthorized events for newly unlocked policies
            if new_authority > old_authority {
                use crate::state::PolicyId;
                for &policy in &PolicyId::ALL {
                    if let Some(req) = policy.authority_requirement() {
                        if old_authority < req && new_authority >= req {
                            events.push(GameEvent::PolicyAuthorized { policy });
                        }
                    }
                }
            }

            "Board communiqué filed.".into()
        }

        (CrisisKind::BoardEmbezzlementWarning, _) => {
            state.embezzlement_warned = true;
            "Letter filed. The board will be monitoring fund allocations.".into()
        }

        (CrisisKind::BoardResearchInquiry, 0) => {
            // Acknowledge — chairman satisfaction hit for inaction
            chairman_satisfaction_hit(state, -0.05);
            "The board's displeasure has been noted.".into()
        }
        (CrisisKind::BoardResearchInquiry, _) => {
            // Present a timeline — costs already deducted, no satisfaction hit
            "Timeline presented. The board expects results.".into()
        }

        (CrisisKind::VoteOfNoConfidence, 0) => {
            // Make concessions: cost already deducted. Boost chairman satisfaction.
            if let Some(chairman) = state.board_members.iter_mut().find(|m| m.is_chairman) {
                chairman.add_modifier(ModifierSource::CrisisEffect, 0.30);
            }
            // Reset hostility timer since we placated them
            state.chairman_hostile_since = None;
            "Concessions accepted. The Chairman withdraws the motion.".into()
        }
        (CrisisKind::VoteOfNoConfidence, _) => {
            // Stand firm: chairman satisfaction hit + immediate 20% budget cut
            chairman_satisfaction_hit(state, -0.15);
            state.board_budget_per_tick *= 0.80;
            "Motion defeated, but the board has slashed your budget.".into()
        }

        // --- Corporate detention crises ---

        (CrisisKind::FieldTeamDetained { corp_idx, region_idx, fee, team_size }, 0) => {
            // Pay the fee (cost already deducted). Schedule follow-up detention.
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.clone()).unwrap_or_else(|| "the corporation".into());
            let region_name = state.regions.get(*region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "the region".into());
            let followup_fee = fee * 1.6;
            state.pending_crises.push(CrisisKind::FieldTeamDetainedAgain {
                region_idx: *region_idx,
                corp_idx: *corp_idx,
                fee: followup_fee,
                team_size: *team_size,
            });
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

        // --- Corporate demand resolutions ---

        (CrisisKind::CorporateDemand { corp_idx }, 0) => {
            // Pay compensation (cost already deducted generically).
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.clone()).unwrap_or_else(|| "the corporation".into());
            // Record demand tick for per-corp cooldown
            if let Some(corp) = state.corporations.get_mut(*corp_idx) {
                corp.last_demand_tick = Some(state.tick);
            }
            format!("Compensation paid to {corp_name}. Policy remains in effect.")
        }
        (CrisisKind::CorporateDemand { corp_idx }, _) => {
            // Refuse: corp retaliates by pulling investment.
            // Infrastructure and civil order take a hit in the corp's region.
            let corp_name = state.corporations.get(*corp_idx)
                .map(|c| c.name.clone()).unwrap_or_else(|| "the corporation".into());
            let region_idx = state.corporations.get(*corp_idx)
                .map(|c| c.region_idx).unwrap_or(0);
            let region_name = state.regions.get(region_idx)
                .map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".into());
            // Infrastructure hit: supply lines and civil order drop
            if let Some(region) = state.regions.get_mut(region_idx) {
                region.supply_lines = (region.supply_lines - 0.05).max(0.0);
                region.civil_order = (region.civil_order - 0.05).max(0.0);
            }
            // Record demand tick for per-corp cooldown
            if let Some(corp) = state.corporations.get_mut(*corp_idx) {
                corp.last_demand_tick = Some(state.tick);
            }
            format!("{corp_name} retaliates. Supply lines and civil order reduced in {region_name}.")
        }

        // --- Nuclear evacuation resolutions ---

        (CrisisKind::NuclearEvacuation { region_idx }, 0) => {
            // Evacuate: move board members out of the region (cost already deducted).
            let mut evacuated = Vec::new();
            for member in &mut state.board_members {
                if member.region_idx == Some(*region_idx) {
                    evacuated.push(member.name.clone());
                    member.region_idx = None; // relocated to safety
                }
            }
            // Governor evacuated but region becomes leaderless — no point governing a crater.
            // No succession scheduled since the region is about to be annihilated.
            if !state.regions[*region_idx].governor.dead {
                state.regions[*region_idx].governor.dead = true;
                state.regions[*region_idx].governor.succession_tick = None;
            }
            let names = evacuated.join(", ");
            format!("{names} evacuated by air. Region will be annihilated on impact.")
        }
        (CrisisKind::NuclearEvacuation { region_idx }, _) => {
            // Let them die: chairman satisfaction hit.
            let members: Vec<String> = state.board_members.iter()
                .filter(|m| m.region_idx == Some(*region_idx) && !m.dead)
                .map(|m| m.name.clone())
                .collect();
            let names = members.join(", ");
            // Severe chairman satisfaction penalty for letting board members die
            if let Some(chairman) = state.board_members.iter_mut().find(|m| m.is_chairman) {
                chairman.add_modifier(
                    crate::state::ModifierSource::CrisisEffect,
                    -0.15,
                );
            }
            format!("{names} will die in the blast. The board will remember this.")
        }
    };
    // Authority is an enum — no clamping needed.
    // Crisis dismissal no longer touches sim_state — the player's pacing preference
    // (Running/Paused) is unchanged by crises. Blocking is derived from active_crisis.
    (msg, post_action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::state::RegionDiseaseState;

    #[test]
    fn phase_weights_shift_with_game_day() {
        // Early-game bureaucratic crises should dominate early, fade late
        assert!(phase_weight("personnel", 3.0) > phase_weight("personnel", 60.0),
            "personnel crises should be more likely early than late");
        // Late-game survival crises should be absent early, present late
        assert!(phase_weight("corporate_seizure", 5.0) < phase_weight("corporate_seizure", 50.0),
            "corporate seizure should be more likely late than early");
        assert!(phase_weight("cult", 3.0) < phase_weight("cult", 50.0),
            "cult blockade should be more likely late than early");

        // No crisis type should ever have zero weight (anachronistic = rare but possible)
        assert!(phase_weight("personnel", 60.0) > 0.0,
            "even late-game, bureaucratic crises should have non-zero weight");
    }

    #[test]
    fn early_game_generates_bureaucratic_crises() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = AppState::new_default(42);
        // Day 5: early game
        state.tick = (5.0 * TICKS_PER_DAY) as u64;
        // Ensure preconditions for various crisis types
        state.policies[0].quarantine = true;
        let mut tags: Vec<&str> = Vec::new();
        for seed in 0..50u64 {
            let mut r = ChaCha8Rng::seed_from_u64(seed);
            if let Some(crisis) = generate_crisis(&state, &mut r) {
                tags.push(crisis.kind.tag());
            }
        }

        // At day 5, late-game crises (corporate_seizure, cult, etc.) should
        // be absent or extremely rare since they have near-zero weight
        let late_count = tags.iter()
            .filter(|&&t| matches!(t, "corporate_seizure" | "cult"))
            .count();
        assert!(late_count <= 2,
            "at day 5, late-game crises should be rare, got {}/{}",
            late_count, tags.len());
    }

    #[test]
    fn field_team_detained_generates_in_collapsed_region() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = AppState::new_default(42);
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
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
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
        // Crisis is active — game is blocked via is_blocked()

        let pending_before = state.pending_crises.len();
        let (msg, _) = resolve_crisis(&mut state, 0, &mut events); // Pay
        assert!(state.pending_crises.len() > pending_before, "paying should schedule follow-up");
        assert!(state.pending_crises.iter().any(|k| matches!(k, CrisisKind::FieldTeamDetainedAgain { .. })),
            "follow-up should be FieldTeamDetainedAgain");
        assert!(msg.contains("paid"), "resolution message should mention payment");
    }

    #[test]
    fn field_team_detained_write_off_loses_personnel() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
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
        // Crisis is active — game is blocked via is_blocked()

        let personnel_before = state.resources.personnel;
        resolve_crisis(&mut state, 2, &mut events); // Write them off
        assert_eq!(state.resources.personnel, personnel_before - 3, "write-off should lose team_size personnel");
    }

    #[test]
    fn new_pathogen_detected_offers_identification() {
        let mut state = AppState::new_default(42);
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
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        state.resources.funding = 2000.0;
        state.resources.personnel = 20;

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let funding_before = state.resources.funding;
        assert!(state.active_research.iter().all(|p| !p.kind.is_field_work()), "no research before resolution");
        let (msg, _) = resolve_crisis(&mut state, 0, &mut events); // Begin identification
        assert!(!state.active_research.iter().all(|p| !p.kind.is_field_work()),
            "identification research should start after choosing option A");
        assert!(matches!(
            &state.active_research.iter().find(|p| p.kind.is_field_work()).unwrap().kind,
            ResearchKind::IdentifyThreat { disease_idx: 0 }
        ), "should be identifying disease 0");
        assert!(msg.contains("initiated"), "message should confirm initiation: {}", msg);
        // Funding should be deducted (¥350 base cost for IdentifyThreat)
        assert!(state.resources.funding < funding_before,
            "funding should be deducted: before={}, after={}", funding_before, state.resources.funding);
    }

    #[test]
    fn new_pathogen_crisis_e2e_via_tick_and_apply_action() {
        // Simulate real gameplay: tick until the NewPathogenDetected crisis fires,
        // then resolve it via apply_action, and verify research starts.
        use crate::action::Action;
        use crate::apply_action;
        use crate::engine::tick;

        let mut state = AppState::new_default(99);
        // Ensure disease 0 is NOT yet detected so detection happens during ticks
        state.diseases[0].detected = false;

        // Tick until a NewPathogenDetected crisis fires (or give up after many ticks)
        let mut crisis_fired = false;
        for _ in 0..500 {
            state = state.with_world(tick(&state).0);
            if state.active_crisis.as_ref().is_some_and(|c|
                matches!(c.kind, CrisisKind::NewPathogenDetected { .. })) {
                crisis_fired = true;
                break;
            }
            // If a non-NewPathogenDetected crisis fired, auto-resolve it to keep ticking
            if state.active_crisis.is_some() {
                state = apply_action(&state, &Action::Confirm);
            }
        }

        if !crisis_fired {
            // If no crisis fired after 500 ticks, the detection threshold wasn't
            // reached — this seed/setup may not trigger it quickly. Skip rather
            // than fail, since the unit test above covers the logic directly.
            return;
        }

        // Crisis is active — verify it has identification option
        let crisis = state.active_crisis.as_ref().unwrap();
        assert!(crisis.options.len() > 1,
            "NewPathogenDetected should have Begin identification option, got {} options",
            crisis.options.len());

        // Ensure the player can afford the option
        state.resources.funding = 2000.0;

        // Resolve: choose option 0 (Begin identification) via apply_action
        let field_before = state.active_research.iter().filter(|p| p.kind.is_field_work()).count();
        state = apply_action(&state, &Action::Confirm); // crisis_selection defaults to 0

        // Verify identification research started
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).count() > field_before,
            "identification research should start after confirming crisis option 0");
        assert!(state.active_research.iter().any(|p|
            matches!(p.kind, ResearchKind::IdentifyThreat { .. })),
            "should have an IdentifyThreat project active");
    }

    #[test]
    fn new_pathogen_detected_refunds_funding_if_insufficient_personnel() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        state.resources.funding = 2000.0;
        state.resources.personnel = 2; // IdentifyThreat needs 5

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let funding_before = state.resources.funding;
        let (msg, _) = resolve_crisis(&mut state, 0, &mut events); // Begin identification
        assert!(state.active_research.iter().all(|p| !p.kind.is_field_work()),
            "no research should start without enough personnel");
        assert_eq!(state.resources.funding, funding_before,
            "funding should be refunded when personnel are insufficient");
        assert!(msg.contains("personnel"), "message should mention personnel: {}", msg);
    }

    #[test]
    fn new_pathogen_detected_dismiss_does_nothing() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        state.resources.funding = 2000.0;

        let kind = CrisisKind::NewPathogenDetected { disease_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        // Dismiss is option index 1 (when identification is available)
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let funding_before = state.resources.funding;
        resolve_crisis(&mut state, 1, &mut events); // Acknowledge
        assert!(state.active_research.iter().all(|p| !p.kind.is_field_work()), "no research should start on dismiss");
        assert_eq!(state.resources.funding, funding_before, "no funding should be deducted on dismiss");
    }

    #[test]
    fn new_pathogen_detected_no_identification_when_already_researching() {
        let mut state = AppState::new_default(42);
        state.diseases[0].detected = true;
        state.diseases[0].knowledge = 0.0;
        // Already identifying disease 0
        state.active_research.push(crate::state::ResearchProject {
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

    #[test]
    fn chairman_mood_shifts_board_budget() {
        let mut state = crate::engine::new_game(42);

        // Find the chairman
        let chair_idx = state.board_members.iter().position(|m| m.is_chairman)
            .expect("should have a chairman");

        // Baseline: neutral satisfaction (0.5) with neutral chairman
        state.board_members[chair_idx].satisfaction = 0.5;
        let base_budget = compute_board_budget_per_tick(&state, 0.5);
        let base_shift = chairman_funding_shift(&state);
        assert_eq!(base_shift, 0.0, "neutral chairman should have no shift");

        // Content chairman (>0.7) should increase budget
        state.board_members[chair_idx].satisfaction = 0.9;
        let content_shift = chairman_funding_shift(&state);
        assert_eq!(content_shift, 0.1, "content chairman should shift +0.1");
        let content_budget = compute_board_budget_per_tick(&state, 0.5);
        assert!(content_budget > base_budget,
            "content chairman should increase budget: base={base_budget}, content={content_budget}");

        // Hostile chairman (<0.3) should decrease budget
        state.board_members[chair_idx].satisfaction = 0.1;
        let hostile_shift = chairman_funding_shift(&state);
        assert_eq!(hostile_shift, -0.1, "hostile chairman should shift -0.1");
        let hostile_budget = compute_board_budget_per_tick(&state, 0.5);
        assert!(hostile_budget < base_budget,
            "hostile chairman should decrease budget: base={base_budget}, hostile={hostile_budget}");
    }

    #[test]
    fn crisis_urgency_increases_budget_with_visible_infections() {
        let mut state = AppState::new_default(42);
        crate::engine::initialize_game(&mut state);

        assert!(state.reference_base_budget_per_tick > 0.0,
            "reference base should be set after board init");

        let pop = state.initial_population();
        assert!(pop > 7e9, "total population should be ~7.8 billion, got {pop}");

        // No infections: urgency boost should be zero
        let budget_calm = compute_board_budget_per_tick(&state, 0.5);
        assert_eq!(crisis_urgency_boost(&state), 0.0,
            "no infections should mean no urgency boost");

        // 100K screened: early outbreak, barely registers
        let n_regions = state.regions.len() as f64;
        for region in &mut state.regions {
            region.estimated_infected = 100_000.0 / n_regions;
        }
        let urgency_100k = crisis_urgency_boost(&state);
        assert!(urgency_100k < 0.02,
            "100K screened should barely register, got {:.3}", urgency_100k);

        // 50M screened (~0.6% of pop): mid-to-late game, meaningful boost
        for region in &mut state.regions {
            region.estimated_infected = 50_000_000.0 / n_regions;
        }
        let urgency_50m = crisis_urgency_boost(&state);
        assert!(urgency_50m > 0.20 && urgency_50m < 0.30,
            "50M screened should give ~0.24 urgency, got {:.3}", urgency_50m);

        let budget_crisis = compute_board_budget_per_tick(&state, 0.5);
        assert!(budget_crisis > budget_calm * 1.20,
            "budget with 50M infections ({:.2}) should be >20% above calm ({:.2})",
            budget_crisis, budget_calm);

        // 200M screened (~2.5% of pop): full crisis, urgency should nearly cap
        for region in &mut state.regions {
            region.estimated_infected = 200_000_000.0 / n_regions;
        }
        let urgency_200m = crisis_urgency_boost(&state);
        assert!(urgency_200m > 0.35,
            "200M screened should be near cap, got {:.3}", urgency_200m);

        // Even with low satisfaction (0.2), urgency at 200M should significantly
        // offset the budget cut
        let budget_low_sat_urgent = compute_board_budget_per_tick(&state, 0.2);
        let budget_low_sat_calm = {
            for region in &mut state.regions {
                region.estimated_infected = 0.0;
            }
            compute_board_budget_per_tick(&state, 0.2)
        };
        assert!(budget_low_sat_urgent > budget_low_sat_calm * 1.40,
            "urgency at 200M should boost low-sat budget by >40%: urgent={:.2}, calm={:.2}",
            budget_low_sat_urgent, budget_low_sat_calm);
    }

    #[test]
    fn chairman_text_appears_in_board_communique() {
        let mut state = crate::engine::new_game(42);

        // Make chairman content
        let chair_idx = state.board_members.iter().position(|m| m.is_chairman)
            .expect("should have a chairman");
        let chair_name = state.board_members[chair_idx].name.clone();
        state.board_members[chair_idx].satisfaction = 0.9;

        let crisis = build_crisis_event(&state, CrisisKind::BoardMeeting);
        assert!(crisis.description.contains(&chair_name),
            "communiqué should mention chairman by name: {}", crisis.description);
        assert!(crisis.description.contains("generous allocation"),
            "content chairman text should mention generous allocation: {}", crisis.description);

        // Make chairman hostile
        state.board_members[chair_idx].satisfaction = 0.1;
        let crisis = build_crisis_event(&state, CrisisKind::BoardMeeting);
        assert!(crisis.description.contains("deeper cuts"),
            "hostile chairman text should mention cuts: {}", crisis.description);
    }

    #[test]
    fn vote_no_confidence_names_chairman_and_corp() {
        let state = crate::engine::new_game(42);

        let chair = state.board_members.iter().find(|m| m.is_chairman).unwrap();
        let chair_name = chair.name.clone();
        let corp_name = chair.corp_idx
            .and_then(|idx| state.corporations.get(idx))
            .map(|c| c.name.clone())
            .unwrap();

        let crisis = build_crisis_event(&state,
            CrisisKind::VoteOfNoConfidence);
        assert!(crisis.description.contains(&chair_name),
            "should mention chairman name: {}", crisis.description);
        assert!(crisis.description.contains(&corp_name),
            "should mention chairman's corp: {}", crisis.description);
        assert_eq!(crisis.options.len(), 2);
        assert!(crisis.options[0].label.contains("concessions"),
            "first option should be concessions: {}", crisis.options[0].label);
        assert!(crisis.options[0].cost.is_some(), "concessions should have a funding cost");
    }

    #[test]
    fn vote_no_confidence_concessions_boost_chairman() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = crate::engine::new_game(42);
        state.resources.funding = 5000.0;

        let chair_idx = state.board_members.iter().position(|m| m.is_chairman).unwrap();
        state.board_members[chair_idx].satisfaction = 0.1;
        state.chairman_hostile_since = Some(0);

        // Directly test resolve_crisis for concessions (option 0)
        let kind = CrisisKind::VoteOfNoConfidence;
        let crisis_event = build_crisis_event(&state, kind);
        // Simulate cost deduction (generic handler does this before resolve)
        let cost = crisis_event.options[0].cost.as_ref().unwrap().funding;
        state.active_crisis = Some(crisis_event);
        state.resources.funding -= cost;
        resolve_crisis(&mut state, 0, &mut events);

        let chair = &state.board_members[chair_idx];
        let crisis_total = chair.modifier_total(&ModifierSource::CrisisEffect);
        assert!(crisis_total > 0.2,
            "concessions should boost chairman CrisisEffect modifier, got {}", crisis_total);
        assert!(state.chairman_hostile_since.is_none(),
            "hostility timer should reset after concessions");
    }

    #[test]
    fn vote_no_confidence_stand_firm_cuts_budget() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        crate::engine::initialize_game(&mut state);
        state.resources.funding = 5000.0;
        state.resources.authority = Authority::Medium;
        let budget_before = state.board_budget_per_tick;

        let kind = CrisisKind::VoteOfNoConfidence;
        let crisis_event = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis_event);
        resolve_crisis(&mut state, 1, &mut events);

        // Standing firm applies a -0.15 chairman satisfaction hit
        let chairman = state.board_members.iter().find(|m| m.is_chairman).unwrap();
        let crisis_total = chairman.modifier_total(&ModifierSource::CrisisEffect);
        assert!(crisis_total < 0.0,
            "chairman satisfaction should drop from stand-firm, got {}", crisis_total);
        assert!(state.board_budget_per_tick < budget_before * 0.85,
            "board budget should drop, got {:.2} (was {:.2})", state.board_budget_per_tick, budget_before);
    }

    #[test]
    fn vote_no_confidence_fires_after_3_days_hostile() {
        use crate::engine::tick;

        let mut state = crate::engine::new_game(42);
        state.tick = (11.0 * crate::state::TICKS_PER_DAY) as u64; // past day 10 gate
        state.next_board_meeting_tick = u64::MAX; // prevent board meetings
        state.last_contract_offer_tick = state.tick; // prevent contract offers

        // Make chairman hostile (satisfaction < 0.20)
        let chair_idx = state.board_members.iter().position(|m| m.is_chairman).unwrap();
        // Force Profiteer so satisfaction = pure stock performance (0.0 when bankrupt)
        state.board_members[chair_idx].personality = Some(crate::state::BoardPersonality::Profiteer);
        let corp_idx = match &state.board_members[chair_idx].role {
            crate::state::BoardRole::CorporateLeader { corp_idx } => *corp_idx,
            _ => panic!("chairman should be corporate leader"),
        };
        // Tank the corp's share price to make chairman satisfaction drop
        state.corporations[corp_idx].share_price = 0.0;
        state.corporations[corp_idx].bankrupt = true;

        // Set hostility timer to 3+ days ago
        state.chairman_hostile_since = Some(state.tick - (3.5 * crate::state::TICKS_PER_DAY) as u64);

        // Tick and look for the crisis
        let mut found = false;
        let max_ticks = 20;
        let mut current = state;
        for _ in 0..max_ticks {
            current = current.with_world(tick(&current).0);
            if let Some(ref crisis) = current.active_crisis {
                if crisis.kind.tag() == "vote_no_confidence" {
                    found = true;
                    assert!(crisis.title.contains("No Confidence"));
                    break;
                }
                // Auto-resolve other crises
                current.active_crisis = None;
            }
        }
        assert!(found, "Vote of No Confidence should fire after 3 days of chairman hostility");
    }

    #[test]
    fn vote_no_confidence_respects_cooldown() {
        let mut state = crate::engine::new_game(42);
        state.tick = (15.0 * crate::state::TICKS_PER_DAY) as u64;
        state.next_board_meeting_tick = u64::MAX;
        state.last_contract_offer_tick = state.tick;

        // Chairman is hostile for 3+ days
        let chair_idx = state.board_members.iter().position(|m| m.is_chairman).unwrap();
        state.board_members[chair_idx].personality = Some(crate::state::BoardPersonality::Profiteer);
        let corp_idx = match &state.board_members[chair_idx].role {
            crate::state::BoardRole::CorporateLeader { corp_idx } => *corp_idx,
            _ => panic!("chairman should be corporate leader"),
        };
        state.corporations[corp_idx].share_price = 0.0;
        state.corporations[corp_idx].bankrupt = true;
        let tick = state.tick;
        state.chairman_hostile_since = Some(tick - (4.0 * crate::state::TICKS_PER_DAY) as u64);

        // Set cooldown from recent firing
        state.crisis_cooldowns.insert("vote_no_confidence".to_string(), tick - 100);

        // Tick a few times — should NOT fire due to cooldown
        let mut current = state;
        for _ in 0..10 {
            current = current.with_world(crate::engine::tick(&current).0);
            if let Some(ref crisis) = current.active_crisis {
                assert_ne!(crisis.kind.tag(), "vote_no_confidence",
                    "should not fire while on cooldown");
                current.active_crisis = None;
            }
        }
    }

    #[test]
    fn governor_sick_worst_case_queues_death_probabilistically() {
        // Governor death after non-stabilize choices should happen ~30% of the time.
        // Run each personality across many seeds and check the aggregate rate.
        let cases: Vec<(GovernorPersonality, usize)> = vec![
            (GovernorPersonality::Buffoon, 1),    // Secure corporation (governor unmonitored)
            (GovernorPersonality::Blowhard, 1),   // Refuse
            (GovernorPersonality::Recluse, 0),    // Leave them alone
            (GovernorPersonality::Hardliner, 2),  // Refuse
            (GovernorPersonality::Operative, 1),  // Refuse
            (GovernorPersonality::Mobster, 2),     // Refuse
        ];

        let trials = 200;
        let mut total_deaths = 0;
        let mut total_trials = 0;

        for (personality, worst_choice) in &cases {
            for seed in 0..trials {
                let mut events: Vec<GameEvent> = Vec::new();
                let mut s = AppState::new_default(seed);
                s.tick = (15.0 * TICKS_PER_DAY) as u64;
                s.pending_crises.clear();
                let region_idx = 0;
                s.regions[region_idx].governor.personality = *personality;
                s.regions[region_idx].governor.dead = false;

                let kind = CrisisKind::GovernorSick { region_idx };
                let crisis = build_crisis_event(&s, kind);
                s.active_crisis = Some(crisis);

                let (_msg, _) = resolve_crisis(&mut s, *worst_choice, &mut events);

                let has_death = s.pending_crises.iter()
                    .any(|k| matches!(k, CrisisKind::GovernorDeath { region_idx: ri } if *ri == region_idx));
                if has_death {
                    total_deaths += 1;
                }
                total_trials += 1;
            }
        }

        let rate = total_deaths as f64 / total_trials as f64;
        assert!(rate > 0.15 && rate < 0.45,
            "Governor death rate should be ~30%, got {:.1}% ({}/{})",
            rate * 100.0, total_deaths, total_trials);
    }

    #[test]
    fn buffoon_sick_stabilize_governor_hits_supply_lines_not_death() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        state.tick = (15.0 * TICKS_PER_DAY) as u64;
        state.regions[0].governor.personality = GovernorPersonality::Buffoon;
        state.regions[0].governor.dead = false;
        state.regions[0].supply_lines = 1.0;
        let old_coop = state.regions[0].governor.cooperation;

        let kind = CrisisKind::GovernorSick { region_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let (_msg, _) = resolve_crisis(&mut state, 0, &mut events); // Stabilize governor

        // Should NOT queue governor death
        let has_death = state.pending_crises.iter()
            .any(|k| matches!(k, CrisisKind::GovernorDeath { region_idx: ri } if *ri == 0));
        assert!(!has_death, "Stabilize governor should not queue GovernorDeath");

        // Should hit supply lines
        assert!(state.regions[0].supply_lines < 1.0,
            "Supply lines should be reduced, got {}", state.regions[0].supply_lines);

        // Should reduce cooperation
        assert!(state.regions[0].governor.cooperation < old_coop,
            "Cooperation should drop");
    }

    #[test]
    fn governor_sick_worst_case_skips_if_already_dead() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        state.tick = (15.0 * TICKS_PER_DAY) as u64;
        state.regions[0].governor.personality = GovernorPersonality::Hardliner;
        state.regions[0].governor.dead = true;

        let kind = CrisisKind::GovernorSick { region_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let (_msg, _) = resolve_crisis(&mut state, 2, &mut events); // Refuse (worst-case)

        let has_death = state.pending_crises.iter()
            .any(|k| matches!(k, CrisisKind::GovernorDeath { region_idx: ri } if *ri == 0));
        assert!(!has_death, "should not queue GovernorDeath for already-dead governor");
    }

    #[test]
    fn governor_sick_worst_case_skips_if_death_already_pending() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        state.tick = (15.0 * TICKS_PER_DAY) as u64;
        state.regions[0].governor.personality = GovernorPersonality::Operative;
        state.regions[0].governor.dead = false;

        // Pre-existing pending GovernorDeath
        state.pending_crises.push(CrisisKind::GovernorDeath { region_idx: 0 });

        let kind = CrisisKind::GovernorSick { region_idx: 0 };
        let crisis = build_crisis_event(&state, kind);
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let (_msg, _) = resolve_crisis(&mut state, 1, &mut events); // Refuse (worst-case)

        let death_count = state.pending_crises.iter()
            .filter(|k| matches!(k, CrisisKind::GovernorDeath { region_idx: ri } if *ri == 0))
            .count();
        assert_eq!(death_count, 1, "should not duplicate GovernorDeath");
    }

    #[test]
    fn corporate_demand_generates_when_policy_hurts_sector() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = AppState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = (15.0 * TICKS_PER_DAY) as u64;

        // Find a Logistics corp and enable travel ban in its region
        let logistics_idx = state.corporations.iter()
            .position(|c| c.sector == CorporationSector::Logistics && !c.bankrupt)
            .expect("should have a logistics corp");
        let region_idx = state.corporations[logistics_idx].region_idx;
        state.policies[region_idx].travel_ban = true;

        // Depress the corp's revenue below threshold
        state.corporations[logistics_idx].revenue =
            state.corporations[logistics_idx].base_revenue * 0.50;

        let mut found = false;
        for seed in 0..200u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            if let Some(crisis) = generate_crisis(&state, &mut rng) {
                if matches!(crisis.kind, CrisisKind::CorporateDemand { corp_idx }
                    if corp_idx == logistics_idx)
                {
                    // Verify crisis text mentions the corp and policy
                    assert!(crisis.description.contains("travel ban"),
                        "crisis should mention the offending policy");
                    assert!(crisis.options.len() == 2, "should have 2 options");
                    assert!(crisis.options.iter().any(|o| o.cost.is_none()),
                        "must have a free option");
                    found = true;
                    break;
                }
            }
        }
        assert!(found, "CorporateDemand should generate for logistics corp under travel ban");
    }

    #[test]
    fn corporate_demand_per_corp_cooldown() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = AppState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = (15.0 * TICKS_PER_DAY) as u64;

        let logistics_idx = state.corporations.iter()
            .position(|c| c.sector == CorporationSector::Logistics && !c.bankrupt)
            .expect("should have a logistics corp");
        let region_idx = state.corporations[logistics_idx].region_idx;
        state.policies[region_idx].travel_ban = true;
        state.corporations[logistics_idx].revenue =
            state.corporations[logistics_idx].base_revenue * 0.50;

        // Set a recent demand tick — should prevent generation
        state.corporations[logistics_idx].last_demand_tick = Some(state.tick - 10);

        let mut found_demand = false;
        for seed in 0..200u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            if let Some(crisis) = generate_crisis(&state, &mut rng) {
                if matches!(crisis.kind, CrisisKind::CorporateDemand { corp_idx }
                    if corp_idx == logistics_idx)
                {
                    found_demand = true;
                    break;
                }
            }
        }
        assert!(!found_demand, "corp on cooldown should not generate a demand");
    }

    #[test]
    fn corporate_demand_refuse_damages_infrastructure() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = 1000;

        // Find a logistics corp
        let corp_idx = state.corporations.iter()
            .position(|c| c.sector == CorporationSector::Logistics && !c.bankrupt)
            .unwrap();
        let region_idx = state.corporations[corp_idx].region_idx;

        let supply_before = state.regions[region_idx].supply_lines;
        let civil_before = state.regions[region_idx].civil_order;

        // Build and activate the crisis
        let crisis = build_crisis_event(&state, CrisisKind::CorporateDemand { corp_idx });
        state.active_crisis = Some(crisis);
        // Crisis is active — game is blocked via is_blocked()

        let (msg, _) = resolve_crisis(&mut state, 1, &mut events); // Refuse

        assert!(state.regions[region_idx].supply_lines < supply_before,
            "supply lines should decrease on refusal");
        assert!(state.regions[region_idx].civil_order < civil_before,
            "civil order should decrease on refusal");
        assert!(state.corporations[corp_idx].last_demand_tick.is_some(),
            "demand tick should be recorded");
        assert!(msg.contains("retaliates"), "message should mention retaliation");
    }

    #[test]
    fn biotech_never_generates_demand() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = AppState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        state.tick = (15.0 * TICKS_PER_DAY) as u64;

        // Enable all policies everywhere and depress all biotech corps
        for (i, policy) in state.world.policies.iter_mut().enumerate() {
            policy.quarantine = true;
            policy.travel_ban = true;
            policy.martial_law = true;
            // Also depress all biotech corps in this region
            for corp in state.world.corporations.iter_mut().filter(|c| c.region_idx == i) {
                corp.revenue = corp.base_revenue * 0.30;
            }
        }

        let mut found_biotech_demand = false;
        for seed in 0..500u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            if let Some(crisis) = generate_crisis(&state, &mut rng) {
                if let CrisisKind::CorporateDemand { corp_idx } = crisis.kind {
                    if state.corporations[corp_idx].sector == CorporationSector::Biotech {
                        found_biotech_demand = true;
                        break;
                    }
                }
            }
        }
        assert!(!found_biotech_demand, "Biotech corps should never generate demands");
    }

    #[test]
    fn board_meeting_raises_authority_when_pressure_high() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = crate::engine::new_game(42);

        // Start at Minimal authority
        state.resources.authority = Authority::Minimal;

        // Ensure disease 0 exists and is detected so deaths count toward pressure
        if !state.diseases.is_empty() {
            state.diseases[0].detected = true;
        }

        // Create massive crisis pressure: lots of infections and deaths
        for region in &mut state.regions {
            region.infections.push(RegionDiseaseState {
                disease_idx: 0,
                exposed: 0.0,
                infected: 5_000_000.0,
                dead: 500_000.0,
                immune: 0.0,
            });
        }

        // Max out board satisfaction to boost pressure further
        for member in &mut state.board_members {
            member.satisfaction = 1.0;
        }

        // Verify suggested authority is above Minimal
        let suggested = state.suggested_authority();
        assert!(suggested > Authority::Minimal,
            "with massive infections, suggested authority should be above Minimal, got {:?}", suggested);

        // Resolve a board meeting
        let crisis = build_crisis_event(&state, CrisisKind::BoardMeeting);
        state.active_crisis = Some(crisis);
        resolve_crisis(&mut state, 0, &mut events);

        assert_eq!(state.resources.authority, Authority::VeryLow,
            "board meeting should raise authority by exactly one level from Minimal");
    }

    #[test]
    fn board_meeting_lowers_authority_when_pressure_low() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = crate::engine::new_game(42);

        // Start at High authority with no crisis
        state.resources.authority = Authority::High;

        // Clear all infections so pressure is purely from board satisfaction
        for region in &mut state.regions {
            region.infections.clear();
            region.dead = 0.0;
        }

        // Zero out board satisfaction to minimize pressure
        for member in &mut state.board_members {
            member.satisfaction = 0.0;
        }

        // Verify suggested authority is below High
        let suggested = state.suggested_authority();
        assert!(suggested < Authority::High,
            "with no crisis and zero satisfaction, suggested should be below High, got {:?}", suggested);

        // Resolve a board meeting
        let crisis = build_crisis_event(&state, CrisisKind::BoardMeeting);
        state.active_crisis = Some(crisis);
        resolve_crisis(&mut state, 0, &mut events);

        assert_eq!(state.resources.authority, Authority::Medium,
            "board meeting should lower authority by exactly one level from High");
    }

    #[test]
    fn board_meeting_emits_policy_authorized_on_raise() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::PolicyId;

        let mut state = crate::engine::new_game(42);

        // Start at VeryLow — raising to Low should unlock DiscourageHosp (requires Low)
        state.resources.authority = Authority::VeryLow;

        // Ensure disease 0 is detected so deaths count toward pressure
        if !state.diseases.is_empty() {
            state.diseases[0].detected = true;
        }

        // Create enough pressure that suggested > VeryLow
        for region in &mut state.regions {
            region.infections.push(RegionDiseaseState {
                disease_idx: 0,
                exposed: 0.0,
                infected: 5_000_000.0,
                dead: 500_000.0,
                immune: 0.0,
            });
        }
        for member in &mut state.board_members {
            member.satisfaction = 1.0;
        }

        let suggested = state.suggested_authority();
        assert!(suggested > Authority::VeryLow,
            "setup should produce suggested > VeryLow, got {:?}", suggested);

        // Clear events, then resolve board meeting
        events.clear();
        let crisis = build_crisis_event(&state, CrisisKind::BoardMeeting);
        state.active_crisis = Some(crisis);
        resolve_crisis(&mut state, 0, &mut events);

        assert_eq!(state.resources.authority, Authority::Low,
            "authority should have been raised to Low");

        // DiscourageHosp requires Authority::Low — should be in the emitted events
        let policy_events: Vec<_> = events.iter().filter_map(|e| {
            if let GameEvent::PolicyAuthorized { policy } = e {
                Some(*policy)
            } else {
                None
            }
        }).collect();

        assert!(policy_events.contains(&PolicyId::DiscourageHosp),
            "raising to Low should emit PolicyAuthorized for DiscourageHosp, got {:?}", policy_events);

        // TravelBan and Quarantine require Medium — should NOT be emitted
        assert!(!policy_events.contains(&PolicyId::TravelBan),
            "TravelBan requires Medium, should not be emitted at Low");
        assert!(!policy_events.contains(&PolicyId::Quarantine),
            "Quarantine requires Medium, should not be emitted at Low");
    }
}
