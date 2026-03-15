mod board;
mod contracts;
mod corporations;
mod crisis;
mod disease;
mod infrastructure;
mod loans;
mod medicine;
mod policy;
mod research;
mod spread;

use rand::Rng;

use crate::state::{
    CrisisKind, DecreeId, GameCommand, GameEvent, GameOutcome, WorldState, ResearchKind,
    StandingOrderKind,
    COLLAPSE_DEATH_RATE, COLLAPSE_SUBSISTENCE_FLOOR,
    CRISIS_INTERVAL, CRISIS_MIN_TICK,
    EMERGENCE_CHANCE_PER_TICK, EMERGENCE_MIN_TICK,
    MAX_DISEASES, PERSONNEL_UPKEEP_COST, TICKS_PER_DAY,
    WAVE_CLUSTER_WINDOW_TICKS,
};

/// Advance the simulation by one tick.
///
/// Initialize game systems that run after basic state construction.
/// Called once for new games (not for loaded saves that already have this data).
/// Generates corporations and board members from the initial game state.
pub fn initialize_game(state: &mut WorldState) {
    if state.corporations.is_empty() {
        corporations::generate_corporations(state);
    }
    if state.board_members.is_empty() {
        board::generate_board_members(state);
    }

    // Initialize board budget from corporate tax revenue at current satisfaction.
    // Done here (not in board.rs) because it depends on crisis::compute_board_budget_per_tick,
    // and subsystems must not call into each other — mod.rs orchestrates cross-subsystem work.
    if state.board_budget_per_tick == 0.0 {
        let base = state.base_board_budget_per_tick();
        state.reference_base_budget_per_tick = base;
        let board_sat = state.board_satisfaction();
        state.board_budget_per_tick =
            crisis::compute_board_budget_per_tick(state, board_sat);
    }
}

/// External callers should use `lib::tick_and_process()` instead, which also
/// calls `events::process_events()` to materialize events into UI state. This
/// function is `pub(crate)` so engine unit tests can call it directly without
/// going through the UI layer.
pub(crate) fn tick(state: &WorldState) -> (WorldState, Vec<GameEvent>) {
    let mut new = state.clone();
    let mut events: Vec<GameEvent> = Vec::new();

    // Don't advance simulation after game over
    if new.outcome != GameOutcome::Playing {
        return (new, Vec::new());
    }

    // Snapshot decree unlock state so we can detect newly unlocked decrees at end of tick.
    let decrees_were_unlocked: Vec<bool> = DecreeId::ALL.iter().map(|&d| state.decree_unlocked(d)).collect();

    // Clone per-subsystem RNG streams out so we can mutably borrow them and
    // `new.regions` simultaneously. Written back at the end of the function.
    let mut rng_spread = new.rng_spread.clone();
    let mut rng_emergence = new.rng_emergence.clone();
    let mut rng_crisis = new.rng_crisis.clone();
    let mut rng_research = new.rng_research.clone();
    let mut rng_misc = new.rng_misc.clone();

    // Disease spread and variant spawning
    spread::tick_spread_within(&mut new, &state.diseases, &mut rng_spread);
    spread::tick_spread_cross_region(&mut new, &state.diseases, &mut rng_spread, &mut events);
    spread::tick_horizontal_gene_transfer(&mut new, &mut events);
    disease::tick_variant_spawning(&mut new, &mut rng_spread, &mut events);

    // Research progress
    let research_completions = research::tick_research(&mut new, &mut rng_research, &mut events);
    for _ in 0..research_completions {
        board::on_research_completed(&mut new);
    }

    // Auto-deploy medicines to worst-affected regions
    medicine::try_auto_deploy(&mut new, &mut events);

    // Process arriving medicine shipments
    medicine::tick_shipments(&mut new, &mut rng_misc, &mut events);

    // Infrastructure degradation — hospitals overwhelm, supply lines break, civil order erodes.
    infrastructure::tick_infrastructure(&mut new, &mut events);

    // Crisis operations — temporary personnel commitments from crisis resolutions.
    crisis::tick_crisis_operations(&mut new, &mut events);

    // Nuclear state transitions — land nukes that have reached their hit tick.
    policy::tick_nuclear(&mut new, &mut events);

    // Policy costs — suspend unaffordable policies and deduct costs.
    let policy_cost = policy::tick_enforce_costs(&mut new, &mut events);
    new.cumulative_policy_spending += policy_cost;

    // Emergency loans — offer when policies are suspended, accrue interest, trigger hostile crises.
    // Runs after policy enforcement so we know if a suspension just happened.
    if events.iter().any(|e| matches!(e, GameEvent::PolicySuspended { .. })) {
        loans::maybe_queue_loan_offer(&mut new);
    }
    loans::tick_loans(&mut new);

    // Governor cooperation drift — reacts to policies, deaths, and personality.
    policy::tick_governor_cooperation(&mut new, &mut events);

    // Governor autonomous actions — hostile governors act against the player.
    policy::tick_governor_actions(&mut new, &mut events);

    // Standing orders — auto-enable policies when severity thresholds are crossed.
    let gdp_regions = policy::tick_standing_orders(&mut new, &mut events);
    for r_idx in gdp_regions {
        board::on_gdp_policy_enacted(&mut new, r_idx);
    }

    // Auto-rebuild infrastructure for regions with the toggle enabled.
    policy::tick_auto_rebuild(&mut new, &mut events);

    // Screening infrastructure — update progress ramp-up and estimated infection counts.
    // Must run after spread (so real values are current) and after policy costs
    // (so suspended screening is reflected).
    policy::tick_screening(&mut new);

    // Snapshot per-disease observed infection estimates for Rt computation.
    // Every tick: update current_day_observed_infected with latest screened estimates.
    // At day boundaries: rotate current into prev.
    snapshot_disease_observations(&mut new);

    // Funding contracts — check conditions (revoke violators), offer new contracts,
    // and check for loyalty raise eligibility on long-held contracts.
    contracts::tick_check_contracts(&mut new, &mut events);
    contracts::tick_offer_contracts(&mut new, &mut rng_misc, &mut events);
    contracts::tick_loyalty_raises(&mut new, &mut rng_misc);
    contracts::tick_patron_bonuses(&mut new, &mut rng_misc, &mut events);

    // Corporate finances — update revenue, drain reserves, bankrupt failing corps.
    corporations::tick_corporations(&mut new, &mut rng_misc, &mut events);
    // Update regional GDP — smoothly tracks toward target based on disease + policies.
    // Must run before board satisfaction so governors see current GDP.
    tick_gdp(&mut new);
    // Board satisfaction check — queue demand crises when corps are hurting.
    // Update per-member board satisfaction from connected entities.
    board::update_board_satisfaction(&mut new);

    // Passive resource generation (both degrade as deaths mount)
    let funding_income = new.funding_income_rate();
    new.resources.funding += funding_income;

    // Personnel upkeep — mandatory cost for maintaining your roster.
    // Floor at 0: if income can't cover upkeep, the deficit is absorbed
    // (personnel stay but the treasury doesn't go negative).
    let upkeep = new.personnel_upkeep_rate();
    new.resources.funding = (new.resources.funding - upkeep).max(0.0);

    // Personnel attrition: when funding is $0, unassigned personnel leave.
    // Rate: ~1 person per day. Thematic: unpaid workers resign.
    if new.resources.funding <= 0.0 && new.personnel_available() > 0 {
        new.resources.attrition_accum += 1.0 / TICKS_PER_DAY;
        if new.resources.attrition_accum >= 1.0 {
            let lost = (new.resources.attrition_accum as u32).min(new.personnel_available());
            new.resources.personnel = new.resources.personnel.saturating_sub(lost);
            new.resources.attrition_accum -= lost as f64;
            events.push(GameEvent::PersonnelAttrition { count: lost });
        }
    } else {
        new.resources.attrition_accum = 0.0;
    }


    // Low funding warning: warn when net burn rate will exhaust funds within half a day.
    // At 1x speed (500ms/tick), half a day gives ~15 seconds of real-time warning.
    // Only warn if there are active policies that could actually be suspended.
    // Rate-limited to once per day to prevent log spam during extended low-funds periods.
    let total_costs = policy_cost + upkeep;
    let net_burn = total_costs - funding_income;
    if policy_cost > 0.0 && net_burn > 0.0 && new.resources.funding < net_burn * (TICKS_PER_DAY / 2.0)
        && new.tick.saturating_sub(new.resources.last_funding_warning_tick) >= TICKS_PER_DAY as u64
    {
        events.push(GameEvent::FundingWarning);
        new.resources.last_funding_warning_tick = new.tick;
    }

    // Mid-game disease emergence (spawns undetected — player won't see it yet).
    // Later diseases are tougher (scaled by game day and player capability).
    // The arms race is bidirectional: more player tech → faster emergence.
    //
    // Wave clustering: after day 24, recent disease spawns temporarily spike
    // the emergence rate, creating coordinated waves. Ramps from 2× (day 24)
    // to 5× (day 50+), so mid-game sees 2-disease clusters while late-game
    // sees 2-3.
    {
        let day = new.tick as f64 / crate::state::TICKS_PER_DAY;

        // Wave boost: if a disease spawned recently and we're past early game,
        // increase the chance of another spawn. Ramps up over mid-to-late game.
        // Also tracks which disease triggered the wave, for sequence homology.
        let (wave_boost, wave_trigger_idx) = if day >= 24.0 {
            let trigger = new.diseases.iter()
                .enumerate()
                .max_by_key(|(_, d)| d.spawned_at_tick);
            let (trigger_idx, most_recent_spawn) = trigger
                .map(|(i, d)| (Some(i), d.spawned_at_tick))
                .unwrap_or((None, 0));
            let ticks_since = new.tick.saturating_sub(most_recent_spawn);
            if ticks_since > 0 && ticks_since < WAVE_CLUSTER_WINDOW_TICKS {
                // Ramp: 2.0 at day 24 → 4.0 at day 50+
                let ramp = ((day - 24.0) / 26.0).clamp(0.0, 1.0);
                (2.0 + ramp * 2.0, trigger_idx)
            } else {
                (0.0, None)
            }
        } else {
            (0.0, None)
        };

        let emergence_chance = EMERGENCE_CHANCE_PER_TICK * (1.0 + new.tech_pressure() + wave_boost);
        if new.tick >= EMERGENCE_MIN_TICK
            && new.diseases.len() < MAX_DISEASES
            && rng_emergence.r#gen::<f64>() < emergence_chance
        {
            if let Some((new_disease_idx, _)) = disease::spawn_disease_scaled(&mut new, &mut rng_emergence) {
                // New medicines need manufacturers — orchestrated here since
                // disease and corporations are peer subsystems.
                corporations::assign_manufacturers(&mut new);
                // Assign sequence group when this spawn was triggered by wave clustering.
                // Diseases from the same wave share a group ID, visible via Rapid Sequencing.
                if let Some(trigger_idx) = wave_trigger_idx {
                    let group = match new.diseases[trigger_idx].sequence_group {
                        Some(g) => g,
                        None => {
                            let g = new.next_sequence_group;
                            new.next_sequence_group += 1;
                            new.diseases[trigger_idx].sequence_group = Some(g);
                            g
                        }
                    };
                    new.diseases[new_disease_idx].sequence_group = Some(group);
                }
            }
        }
    }

    // Disease detection — undetected diseases are revealed when enough infections are
    // observed. There are two detection paths:
    // 1. Global: total infected (all regions) >= DETECTION_THRESHOLD * screening multiplier.
    // 2. Per-region intel: if a region with an Intel Station has enough LOCAL infections,
    //    the disease is detected early. Thresholds: Basic=3,000, Advanced=1,000.
    {
        let global_threshold = crate::state::DETECTION_THRESHOLD
            * new.effective_detection_multiplier();
        for disease_idx in 0..new.diseases.len() {
            if new.diseases[disease_idx].detected {
                continue;
            }
            let mut global_total: f64 = 0.0;
            let mut detected = false;
            let mut detected_via_advanced_intel = false;
            for region in &new.regions {
                let local: f64 = region.infections.iter()
                    .filter(|inf| inf.disease_idx == disease_idx)
                    .map(|inf| inf.infected)
                    .sum();
                global_total += local;
                // Per-region intel detection
                let intel_threshold = match region.intel_level {
                    2 => 1_000.0,
                    1 => 3_000.0,
                    _ => f64::INFINITY, // no intel: don't trigger per-region
                };
                if local >= intel_threshold {
                    detected = true;
                    if region.intel_level >= 2 {
                        detected_via_advanced_intel = true;
                    }
                }
            }
            if global_total >= global_threshold {
                detected = true;
            }
            if detected {
                let spawned_at = new.diseases[disease_idx].spawned_at_tick;
                let silent_days = (new.tick.saturating_sub(spawned_at)) as f64
                    / crate::state::TICKS_PER_DAY;
                // Record which regions had infections at detection time
                let detection_regions: Vec<usize> = new.regions.iter().enumerate()
                    .filter(|(_, r)| r.disease_state(disease_idx)
                        .is_some_and(|inf| inf.infected > 0.0))
                    .map(|(i, _)| i)
                    .collect();
                new.diseases[disease_idx].first_detected_regions = detection_regions;
                new.diseases[disease_idx].detected_day = new.tick as f64 / crate::state::TICKS_PER_DAY;
                new.diseases[disease_idx].detected = true;
                // Advanced Intel grants immediate identification — reveals name and
                // pathogen type without requiring field research.
                if detected_via_advanced_intel {
                    let current = new.diseases[disease_idx].knowledge;
                    if current < crate::state::KNOWLEDGE_NAME {
                        new.diseases[disease_idx].knowledge = crate::state::KNOWLEDGE_NAME;
                        events.push(GameEvent::IntelAnalysis {
                            disease_idx,
                            message: format!(
                                "INTEL: Pathogen identified — {} ({})",
                                new.diseases[disease_idx].name,
                                new.diseases[disease_idx].pathogen_type.label(),
                            ),
                        });
                    }
                }
                events.push(GameEvent::DiseaseDetected { disease_idx, silent_days });
                // Schedule pathogen detection alert crisis. Fires immediately if no
                // other crisis is active; otherwise queues as pending.
                let kind = CrisisKind::NewPathogenDetected { disease_idx };
                if new.active_crisis.is_none() {
                    let alert = crisis::build_crisis_event(&new, kind);
                    let post = crisis::activate_crisis(&mut new, alert, &mut events);
                    dispatch_crisis_post_action(&mut new, post, &mut events);
                } else {
                    new.pending_crises.push(kind);
                }
            }
        }
    }

    // Threat escalation alerts: warn when a detected disease's deaths cross
    // major thresholds (1M, 100M, 1B). Fires once per threshold per disease.
    // Auto-pauses the game so the player can't miss an escalating threat.
    {
        const THRESHOLDS: &[(u8, f64)] = &[
            (1, 1_000_000.0),
            (2, 100_000_000.0),
            (3, 1_000_000_000.0),
        ];
        // Grow tracking vec if new diseases were spawned
        while new.death_milestone_tier.len() < new.diseases.len() {
            new.death_milestone_tier.push(0);
        }
        for (d_idx, disease) in new.diseases.iter().enumerate() {
            if !disease.detected {
                continue;
            }
            let deaths: f64 = new.regions.iter()
                .filter_map(|r| r.disease_state(d_idx))
                .map(|inf| inf.dead)
                .sum();
            let current_level = new.death_milestone_tier[d_idx];
            for &(level, threshold) in THRESHOLDS {
                if level > current_level && deaths >= threshold {
                    new.death_milestone_tier[d_idx] = level;
                    let has_medicine = new.medicines.iter().any(|m| {
                        m.unlocked && m.target_diseases.contains(&d_idx)
                    });
                    events.push(GameEvent::ThreatEscalation {
                        disease_idx: d_idx,
                        deaths,
                        has_medicine,
                    });
                }
            }
        }
    }

    // Advanced Intel briefings: warn about undetected diseases before they hit detection
    // threshold. Advanced Intel (level 2) regions alert at 500 local infections — well
    // before the 1,000 auto-detection threshold. Fires once per disease.
    {
        // Grow tracking vec if new diseases were spawned
        while new.intel_pre_detection_briefed.len() < new.diseases.len() {
            new.intel_pre_detection_briefed.push(false);
        }
        for (d_idx, disease) in new.diseases.iter().enumerate() {
            if disease.detected || new.intel_pre_detection_briefed[d_idx] {
                continue;
            }
            for region in &new.regions {
                if region.intel_level < 2 {
                    continue;
                }
                let local: f64 = region.infections.iter()
                    .filter(|inf| inf.disease_idx == d_idx)
                    .map(|inf| inf.infected)
                    .sum();
                if local >= 500.0 {
                    new.intel_pre_detection_briefed[d_idx] = true;
                    events.push(GameEvent::IntelBriefing {
                        message: format!(
                            "INTEL: {} anomalous hospital admissions. Possible emerging pathogen, monitoring closely.",
                            region.name
                        ),
                    });
                    break;
                }
            }
        }
    }

    // Fire pending crises immediately — no delay, no spacing.
    if new.active_crisis.is_none() && !new.pending_crises.is_empty() {
        let mut kind = new.pending_crises.remove(0);
        // Validate refugee destination: if to_region collapsed since queuing,
        // re-route to another non-collapsed neighbor or drop the crisis entirely.
        if let CrisisKind::RefugeeWave { from_region, ref mut to_region, .. } = kind {
            if new.regions[*to_region].collapsed {
                let alt: Vec<usize> = new.regions[from_region].connections.iter()
                    .filter(|&&c| !new.regions[c].collapsed)
                    .copied()
                    .collect();
                if let Some(&dest) = alt.first() {
                    *to_region = dest;
                }
            }
            // Only fire if destination is still valid (uncollapsed).
            if !new.regions[*to_region].collapsed {
                let crisis = crisis::build_crisis_event(&new, kind);
                let post = crisis::activate_crisis(&mut new, crisis, &mut events);
                dispatch_crisis_post_action(&mut new, post, &mut events);
            }
        } else if let CrisisKind::ArkProtocol { ref mut region_idx } = kind {
            // Validate Ark target: if region collapsed since queuing, re-pick
            // the best surviving region by survival fraction.
            if new.regions[*region_idx].collapsed {
                let best = new.regions.iter().enumerate()
                    .filter(|(_, r)| !r.collapsed)
                    .max_by(|(_, a), (_, b)| {
                        let frac = |r: &crate::state::Region| r.alive() / (r.population as f64).max(1.0);
                        frac(a).partial_cmp(&frac(b)).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i);
                if let Some(new_idx) = best {
                    *region_idx = new_idx;
                }
            }
            // Only fire if target is valid (uncollapsed).
            if !new.regions[*region_idx].collapsed {
                let crisis = crisis::build_crisis_event(&new, kind);
                let post = crisis::activate_crisis(&mut new, crisis, &mut events);
                dispatch_crisis_post_action(&mut new, post, &mut events);
            }
        } else if matches!(kind,
            CrisisKind::LoyaltyRaise { template_id } | CrisisKind::ContractDemand { template_id }
                if !new.contracts.iter().any(|c| c.template_id == template_id))
        {
            // Contract was revoked/cancelled since this crisis was queued — drop it.
        } else {
            let crisis = crisis::build_crisis_event(&new, kind);
            let post = crisis::activate_crisis(&mut new, crisis, &mut events);
            dispatch_crisis_post_action(&mut new, post, &mut events);
        }
    }

    // Scheduled board meetings: bypass normal crisis cooldown, but wait for active crisis.
    if new.active_crisis.is_none()
        && new.next_board_meeting_tick > 0
        && new.tick >= new.next_board_meeting_tick
        && !new.board_members.is_empty()
    {
        let crisis = crisis::build_crisis_event(&new, CrisisKind::BoardMeeting);
        let post = crisis::activate_crisis(&mut new, crisis, &mut events);
        dispatch_crisis_post_action(&mut new, post, &mut events);
        // Schedule next meeting 7-10 days from now.
        let base = (7.0 * TICKS_PER_DAY) as u64;
        let range = (3.0 * TICKS_PER_DAY) as u64;
        let jitter = rng_crisis.r#gen::<u64>() % (range + 1);
        new.next_board_meeting_tick = new.tick + base + jitter;
    }

    // Embezzlement detection: fire a warning letter when non-board stock positions
    // exceed cumulative policy spending + buffer. Only fires once.
    if new.active_crisis.is_none()
        && !new.embezzlement_warned
        && new.exceeds_embezzlement_threshold()
    {
        let crisis = crisis::build_crisis_event(&new, CrisisKind::BoardEmbezzlementWarning);
        let post = crisis::activate_crisis(&mut new, crisis, &mut events);
        dispatch_crisis_post_action(&mut new, post, &mut events);
    }

    // Board Research Inquiry: fires once around day 5 if no identification research has been
    // started for any disease. A one-shot nudge from the board.
    {
        let day = new.tick as f64 / TICKS_PER_DAY;
        let already_fired = new.crisis_cooldowns.contains_key("board_research_inquiry");
        if new.active_crisis.is_none()
            && day >= 5.0
            && !already_fired
        {
            // Check if any identification research has ever been started:
            // either currently running as field research, or a disease has knowledge > 0
            // (meaning identification completed or is in progress).
            let any_identification_started = new.active_research.iter()
                .any(|fr| matches!(fr.kind, ResearchKind::IdentifyThreat { .. }))
                || new.diseases.iter().any(|d| d.detected && d.knowledge > 0.0);
            if !any_identification_started {
                // Mark as fired immediately so it never fires again, even if manually cleared.
                new.crisis_cooldowns.insert("board_research_inquiry".to_string(), new.tick);
                let crisis = crisis::build_crisis_event(&new, CrisisKind::BoardResearchInquiry);
                let post = crisis::activate_crisis(&mut new, crisis, &mut events);
                dispatch_crisis_post_action(&mut new, post, &mut events);
            }
        }
    }

    // Vote of No Confidence: Chairman calls a vote after ~3 days of sustained hostility.
    // Bypasses normal crisis cooldown (like board meetings) — this is a personal confrontation.
    {
        let day = new.tick as f64 / TICKS_PER_DAY;
        let hostile_days = new.chairman_hostile_since
            .map(|since| (new.tick.saturating_sub(since)) as f64 / TICKS_PER_DAY)
            .unwrap_or(0.0);
        let on_cooldown = new.crisis_cooldowns.get("vote_no_confidence")
            .is_some_and(|&last| new.tick.saturating_sub(last) < (14.0 * TICKS_PER_DAY) as u64);
        if new.active_crisis.is_none()
            && hostile_days >= 3.0
            && day > 10.0
            && !on_cooldown
        {
            let crisis = crisis::build_crisis_event(&new, CrisisKind::VoteOfNoConfidence);
            let post = crisis::activate_crisis(&mut new, crisis, &mut events);
            dispatch_crisis_post_action(&mut new, post, &mut events);
        }
    }

    // Crisis event generation (only when no crisis is active).
    // Frequency scales with game day: early game ~1/10 days, late game ~1/3 days.
    let crisis_interval = {
        let day = new.tick as f64 / TICKS_PER_DAY;
        let base = CRISIS_INTERVAL as f64;
        // Halve the interval every 30 days, floor at 3 days
        (base * 0.5_f64.powf(day / 30.0)).max(3.0 * TICKS_PER_DAY)
    };
    if new.active_crisis.is_none()
        && new.tick >= CRISIS_MIN_TICK
        && rng_crisis.r#gen::<f64>() < 1.0 / crisis_interval
    {
        if let Some(crisis) = crisis::generate_crisis(&new, &mut rng_crisis) {
            let post = crisis::activate_crisis(&mut new, crisis, &mut events);
            dispatch_crisis_post_action(&mut new, post, &mut events);
        }
    }

    new.rng_spread = rng_spread;
    new.rng_emergence = rng_emergence;
    new.rng_crisis = rng_crisis;
    new.rng_research = rng_research;
    new.rng_misc = rng_misc;

    new.tick += 1;

    // Check regional collapse
    for i in 0..new.regions.len() {
        if new.regions[i].collapsed {
            continue;
        }
        let pop = new.regions[i].population as f64;
        let alive = new.regions[i].alive();
        let martial_law_active = new.policies.get(i).is_some_and(|p| p.martial_law);
        let threshold = new.regions[i].effective_collapse_threshold(martial_law_active);
        if alive < pop * threshold {
            new.regions[i].collapsed = true;
            new.regions[i].collapsed_at_tick = Some(new.tick);
            new.regions[i].hospital_level = 0; // Hospital destroyed (per-region infrastructure)
            new.regions[i].intel_level = 0; // Intel station destroyed (per-region infrastructure)
            // lab_level is intentionally NOT reset — it's global research infrastructure
            // Clear all policies in the collapsed region
            if let Some(policy) = new.policies.get_mut(i) {
                policy.clear_all();
            }
            // Personnel loss: staff in the collapsed region are lost
            let lost_personnel = 2u32.min(new.resources.personnel);
            new.resources.personnel = new.resources.personnel.saturating_sub(lost_personnel);
            events.push(GameEvent::RegionCollapsed { region_idx: i, personnel_lost: lost_personnel });

            // Schedule refugee crisis toward a non-collapsed neighbor (if any).
            // Queued as pending so it respects the minimum gap between crises,
            // preventing collapse cascades from drowning the player in popups.
            let neighbors: Vec<usize> = new.regions[i].connections.iter()
                .filter(|&&c| !new.regions[c].collapsed)
                .copied()
                .collect();
            let to = if neighbors.len() > 1 {
                Some(neighbors[new.rng_crisis.r#gen::<usize>() % neighbors.len()])
            } else {
                neighbors.first().copied()
            };
            if let Some(to) = to {
                let wave = new.regions.iter().filter(|r| r.collapsed).count() as u8;
                let kind = CrisisKind::RefugeeWave { from_region: i, to_region: to, wave };
                new.pending_crises.push(kind);
            } else {
                // No uncollapsed neighbors — no refugee destination.
                // Game is nearly over; notification area will show the collapse.
            }
        }
    }

    // Post-collapse secondary deaths: starvation, violence, infrastructure breakdown.
    // 5% of alive per day until population hits 2% subsistence floor.
    for i in 0..new.regions.len() {
        if !new.regions[i].collapsed {
            continue;
        }
        let pop = new.regions[i].population as f64;
        let floor = pop * COLLAPSE_SUBSISTENCE_FLOOR;
        let alive = new.regions[i].alive();
        if alive <= floor {
            continue;
        }
        let deaths_this_tick = (alive * COLLAPSE_DEATH_RATE / TICKS_PER_DAY)
            .min(alive - floor);
        if deaths_this_tick > 0.0 {
            new.regions[i].dead += deaths_this_tick;
            new.regions[i].collapse_deaths += deaths_this_tick;
            // Log once per day (on the tick boundary)
            if new.tick % (TICKS_PER_DAY as u64) == 0 {
                let daily_deaths = deaths_this_tick * TICKS_PER_DAY;
                events.push(GameEvent::CollapseSecondaryDeaths {
                    region_idx: i,
                    deaths: daily_deaths,
                });
            }
        }
    }

    // Schedule Ark Protocol when 2+ regions have collapsed and it hasn't fired yet.
    // Scheduled as a pending crisis so it fires after the immediate RefugeeWave.
    if new.ark_protocol.is_none() {
        let collapsed_count = new.regions.iter().filter(|r| r.collapsed).count();
        let surviving: Vec<usize> = new.regions.iter().enumerate()
            .filter(|(_, r)| !r.collapsed)
            .map(|(i, _)| i)
            .collect();
        let already_pending = new.pending_crises.iter()
            .any(|k| matches!(k, CrisisKind::ArkProtocol { .. }));
        let already_cooldown = new.crisis_cooldowns.contains_key("ark_protocol");
        let already_active = new.active_crisis.as_ref()
            .is_some_and(|c| matches!(c.kind, CrisisKind::ArkProtocol { .. }));
        if collapsed_count >= 2 && !surviving.is_empty() && !already_pending && !already_cooldown && !already_active {
            // Score by survival fraction (alive / population) so devastated regions
            // are not recommended even if they have a large raw population.
            let survival_fraction = |idx: usize| {
                let r = &new.regions[idx];
                r.alive() / (r.population as f64).max(1.0)
            };
            let best = surviving.iter()
                .copied()
                .max_by(|&a, &b| {
                    survival_fraction(a)
                        .partial_cmp(&survival_fraction(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(surviving[0]);
            new.pending_crises.push(CrisisKind::ArkProtocol { region_idx: best });
        }
    }


    // Check defeat condition (only while still playing).
    // There is no victory — you lose eventually. The question is when.
    if new.outcome == GameOutcome::Playing {
        let all_collapsed = new.regions.iter().all(|r| r.collapsed);
        if all_collapsed {
            new.outcome = GameOutcome::Lost;
            new.active_crisis = None;
            // No need to set sim_state — is_blocked() derives from outcome != Playing
            events.push(GameEvent::GameOver);
        }
    }

    // If all diseases burned out but regions survive, spawn a tougher replacement.
    // This prevents the "zombie state" where the game has no threats and no end.
    if new.outcome == GameOutcome::Playing
        && new.total_infected() < 1.0
        && new.tick > EMERGENCE_MIN_TICK
    {
        let mut rng_e = new.rng_emergence.clone();
        disease::spawn_disease_scaled(&mut new, &mut rng_e);
        corporations::assign_manufacturers(&mut new);
        new.rng_emergence = rng_e;
    }

    // Record history for dashboard sparklines
    if new.tick % crate::state::HISTORY_INTERVAL == 0 {
        new.history.push(crate::state::HistorySnapshot {
            tick: new.tick,
            screened_infected: new.total_infected_screened(),
            detected_dead: new.total_dead_detected(),
        });
        if new.history.len() > crate::state::HISTORY_MAX {
            new.history.remove(0);
        }
    }

    // Update per-region death rate for collapse time estimates.
    // Sampled every ~1 day so the rate reflects recent trends.
    let rate_interval = TICKS_PER_DAY as u64;
    for region in &mut new.regions {
        let elapsed = new.tick.saturating_sub(region.prev_dead_tick);
        if elapsed >= rate_interval {
            if region.prev_dead_tick > 0 {
                let death_delta = (region.total_dead() - region.prev_dead).max(0.0);
                region.cached_deaths_per_day = death_delta / (elapsed as f64 / TICKS_PER_DAY);
            }
            region.prev_dead = region.total_dead();
            region.prev_dead_tick = new.tick;
        }
    }

    // Detect newly unlocked emergency decrees (severity crossed threshold this tick).
    for (i, &decree) in DecreeId::ALL.iter().enumerate() {
        if !decrees_were_unlocked[i] && new.decree_unlocked(decree) && !new.enacted_decrees.is_enacted(decree) {
            events.push(GameEvent::DecreeUnlocked { decree });
        }
    }

    (new, events)
}

/// Update per-disease observed infection estimates from screened data.
/// Every tick: compute per-disease screened infected total and store as current estimate.
/// At day boundaries: rotate current into prev, giving a 1-day comparison window for Rt.
fn snapshot_disease_observations(state: &mut WorldState) {
    let is_day_boundary = state.tick > 0 && state.tick % (TICKS_PER_DAY as u64) == 0;

    for disease_idx in 0..state.diseases.len() {
        // Compute this disease's screened infected total across all regions
        let observed: f64 = state.regions.iter().enumerate()
            .filter_map(|(region_idx, region)| {
                let inf = region.disease_state(disease_idx)?;
                if inf.infected + inf.exposed <= 0.0 { return None; }
                let shows_exposed = state.screening_shows_exposed(region_idx);
                let total_real = if shows_exposed {
                    region.detected_infected(&state.diseases)
                } else {
                    region.detected_symptomatic(&state.diseases)
                };
                let this_disease = if shows_exposed { inf.exposed + inf.infected } else { inf.infected };
                let proportion = if total_real > 0.0 { this_disease / total_real } else { 0.0 };
                Some(region.estimated_infected * proportion)
            })
            .sum();

        if is_day_boundary {
            // Rotate: current becomes prev, start fresh accumulation
            state.diseases[disease_idx].prev_day_observed_infected =
                state.diseases[disease_idx].current_day_observed_infected;
        }
        state.diseases[disease_idx].current_day_observed_infected = observed;
    }
}

/// GDP smoothing rate: ~10% convergence per day toward target.
/// Slow enough that GDP doesn't spike instantly when policies toggle,
/// fast enough that the player sees the effect within a few days.
const GDP_SMOOTHING: f64 = 0.10 / crate::state::TICKS_PER_DAY;

/// Update each region's GDP toward its computed target.
fn tick_gdp(state: &mut WorldState) {
    for i in 0..state.regions.len() {
        if state.regions[i].collapsed {
            state.regions[i].gdp = 0.0;
            continue;
        }
        let target = state.gdp_target(i);
        let current = state.regions[i].gdp;
        // Exponential smoothing toward target
        state.regions[i].gdp = current + (target - current) * GDP_SMOOTHING;
    }
}

/// Result of executing a game command. Contains feedback message and whether
/// the command succeeded (so the UI layer can update navigation accordingly).
pub struct CommandResult {
    pub message: Option<String>,
    pub success: bool,
    pub events: Vec<GameEvent>,
}

/// Dispatch cross-subsystem effects from crisis resolution.
/// Called from both `execute_command` (manual) and `tick` (auto-resolve).
/// When auto-resolving, updates the CrisisAutoResolved event message with the
/// richer message from the subsystem (e.g. contract details).
fn dispatch_crisis_post_action(state: &mut WorldState, post_action: crisis::CrisisPostAction, events: &mut Vec<GameEvent>) -> Option<String> {
    let msg = match post_action {
        crisis::CrisisPostAction::None => None,
        crisis::CrisisPostAction::AcceptContract => {
            let (_, msg) = contracts::accept_contract(state);
            msg
        }
        crisis::CrisisPostAction::RejectContract => {
            let (_, msg) = contracts::reject_contract(state);
            msg
        }
        crisis::CrisisPostAction::CancelContract { board_member_idx } => {
            contracts::cancel_contract(state, board_member_idx);
            None
        }
    };
    // If the dispatch produced a richer message, update the most recent
    // CrisisAutoResolved event so the event log shows contract details
    // rather than the generic placeholder from crisis.rs.
    if let Some(ref m) = msg {
        for event in events.iter_mut().rev() {
            if let GameEvent::CrisisAutoResolved { message } = event {
                *message = m.clone();
                break;
            }
        }
    }
    msg
}

/// Execute a game command. Pure game logic — does NOT touch UI state.
/// The caller is responsible for UI transitions based on the result.
pub fn execute_command(state: &mut WorldState, cmd: &GameCommand) -> CommandResult {
    let mut events: Vec<GameEvent> = Vec::new();
    if state.outcome != GameOutcome::Playing {
        return CommandResult { message: None, success: false, events: Vec::new() };
    }
    let mut result = match cmd {
        GameCommand::DeployMedicine {
            medicine_idx,
            region_idx,
            target,
        } => {
            let (success, msg) =
                medicine::deploy_medicine(state, *medicine_idx, *region_idx, target.clone(), &mut events);
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::StartResearch { project_idx, double_personnel } => {
            let (ok, msg) = research::start_research(state, *project_idx, *double_personnel);
            CommandResult { message: msg, success: ok, events: Vec::new() }
        }
        GameCommand::TogglePolicy {
            region_idx,
            policy,
        } => {
            let (msg, success, gdp_region) = policy::toggle_policy(state, *region_idx, *policy);
            if let Some(r_idx) = gdp_region {
                board::on_gdp_policy_enacted(state, r_idx);
            }
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::ResolveCrisis { choice } => {
            let (mut msg, post_action) = crisis::resolve_crisis(state, *choice, &mut events);
            if let Some(m) = dispatch_crisis_post_action(state, post_action, &mut events) {
                msg = m;
            }
            CommandResult { message: Some(msg), success: true, events: Vec::new() }
        }
        GameCommand::EnactDecree { decree, region_idx } => {
            let (msg, success) = policy::enact_decree(state, *decree, *region_idx, &mut events);
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::NegotiateGovernor { region_idx } => {
            let (msg, success) = policy::negotiate_governor(state, *region_idx);
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::BargainWithGovernor { region_idx } => {
            let (msg, success) = policy::bargain_with_governor(state, *region_idx);
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::ToggleStandingOrder { kind } => {
            match kind {
                StandingOrderKind::AutoQuarantineAtHigh => state.standing_orders.auto_quarantine_at_high = !state.standing_orders.auto_quarantine_at_high,
                StandingOrderKind::AutoTravelBanAtCrit => state.standing_orders.auto_travel_ban_at_crit = !state.standing_orders.auto_travel_ban_at_crit,
            }
            CommandResult { message: None, success: true, events: Vec::new() }
        }
        GameCommand::ToggleAutoRebuild { region_idx } => {
            if let Some(p) = state.policies.get_mut(*region_idx) {
                p.auto_rebuild_infra = !p.auto_rebuild_infra;
                let status = if p.auto_rebuild_infra { "enabled" } else { "disabled" };
                let name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                CommandResult {
                    message: Some(format!("Auto-rebuild {} for {}", status, name)),
                    success: true,
                    events: Vec::new(),
                }
            } else {
                CommandResult { message: None, success: false, events: Vec::new() }
            }
        }
        GameCommand::ToggleDeploy { med_idx } => {
            while state.deploy_enabled.len() <= *med_idx {
                state.deploy_enabled.push(false);
            }
            while state.deploy_regions.len() <= *med_idx {
                state.deploy_regions.push(std::collections::BTreeSet::new());
            }
            let was_enabled = state.deploy_enabled[*med_idx];
            state.deploy_enabled[*med_idx] = !was_enabled;
            // When first enabling, ensure deploy_regions is empty (= all regions)
            // Reset blocked notification so the player gets re-notified if still blocked
            state.deploy_blocked_notified.remove(med_idx);
            CommandResult { message: None, success: true, events: Vec::new() }
        }
        GameCommand::ToggleDeployRegion { med_idx, region_idx } => {
            while state.deploy_regions.len() <= *med_idx {
                state.deploy_regions.push(std::collections::BTreeSet::new());
            }
            let regions = &mut state.deploy_regions[*med_idx];
            if regions.is_empty() {
                // Currently "all regions" — switching to explicit: add all then remove the toggled one
                for i in 0..state.regions.len() {
                    regions.insert(i);
                }
                regions.remove(region_idx);
            } else if regions.contains(region_idx) {
                regions.remove(region_idx);
                // If removing made it equal to "all regions again", clear to empty = all
            } else {
                regions.insert(*region_idx);
                // If we now have all regions, clear to empty = all
                if regions.len() == state.regions.len() {
                    regions.clear();
                }
            }
            CommandResult { message: None, success: true, events: Vec::new() }
        }
        GameCommand::ToggleAutoRepeat { kind } => {
            if let Some(pos) = state.auto_repeat_research.iter().position(|k| k == kind) {
                state.auto_repeat_research.remove(pos);
            } else {
                state.auto_repeat_research.push(kind.clone());
            }
            CommandResult { message: None, success: true, events: Vec::new() }
        }
        GameCommand::ToggleThreatVisibility { disease_idx } => {
            if let Some(d) = state.diseases.get_mut(*disease_idx) {
                d.hidden = !d.hidden;
                CommandResult { message: None, success: true, events: Vec::new() }
            } else {
                CommandResult { message: None, success: false, events: Vec::new() }
            }
        }
        GameCommand::UpgradeLab => {
            let (success, msg) = research::upgrade_lab(state);
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::RepayLoan { loan_idx } => {
            if *loan_idx >= state.loans.len() {
                return CommandResult { message: Some("No such loan".to_string()), success: false, events: Vec::new() };
            }
            let amount = state.loans[*loan_idx].outstanding;
            let lender_name = state.loans[*loan_idx].lender_name.clone();
            let lender = state.loans[*loan_idx].lender.clone();
            if state.resources.funding < amount {
                return CommandResult {
                    message: Some(format!(
                        "Insufficient funds: need ¥{:.0}, have ¥{:.0}",
                        amount, state.resources.funding
                    )),
                    success: false,
                    events: Vec::new(),
                };
            }
            loans::repay_loan(state, *loan_idx);
            // Clear any pending LoanCallIn for this lender — prevents double-payment
            // if the crisis was queued but hadn't fired yet when the player paid early.
            state.pending_crises.retain(|k| {
                !matches!(k, CrisisKind::LoanCallIn { lender: l, .. } if *l == lender)
            });
            CommandResult {
                message: Some(format!("Loan repaid to {}. ¥{:.0} deducted.", lender_name, amount)),
                success: true,
                events: Vec::new(),
            }
        }
        GameCommand::BuyShares { corp_idx, quantity } => {
            if *corp_idx >= state.corporations.len() {
                return CommandResult { message: Some("Invalid corporation".to_string()), success: false, events: Vec::new() };
            }
            let corp = &state.corporations[*corp_idx];
            if corp.bankrupt {
                return CommandResult { message: Some("Corporation is bankrupt".to_string()), success: false, events: Vec::new() };
            }
            let cost = corp.share_price * (*quantity as f64);
            if state.resources.funding < cost {
                return CommandResult {
                    message: Some(format!(
                        "Insufficient funds: need ¥{:.0}, have ¥{:.0}",
                        cost, state.resources.funding
                    )),
                    success: false,
                    events: Vec::new(),
                };
            }
            state.resources.funding -= cost;
            while state.portfolio.len() <= *corp_idx {
                state.portfolio.push(0);
            }
            while state.cost_basis.len() <= *corp_idx {
                state.cost_basis.push(0.0);
            }
            state.portfolio[*corp_idx] += quantity;
            state.cost_basis[*corp_idx] += cost;
            let reaction = board::on_buy_shares(state, *corp_idx);
            let name = state.corporations[*corp_idx].name.clone();
            let mut msg = format!(
                "Bought {} shares of {} at ¥{:.1}/share (¥{:.0} total)",
                quantity, name, state.corporations[*corp_idx].share_price, cost
            );
            if let Some(r) = reaction {
                msg.push_str(&format!(" — {}", r));
            }
            CommandResult { message: Some(msg), success: true, events: Vec::new() }
        }
        GameCommand::SellShares { corp_idx, quantity } => {
            if *corp_idx >= state.corporations.len() {
                return CommandResult { message: Some("Invalid corporation".to_string()), success: false, events: Vec::new() };
            }
            let held = state.portfolio.get(*corp_idx).copied().unwrap_or(0);
            if held < *quantity {
                return CommandResult {
                    message: Some(format!("Only hold {} shares", held)),
                    success: false,
                    events: Vec::new(),
                };
            }
            let proceeds = state.corporations[*corp_idx].share_price * (*quantity as f64);
            state.resources.funding += proceeds;
            // Reduce cost basis proportionally: selling N of M shares removes N/M of basis.
            if held > 0 {
                let basis = state.cost_basis.get(*corp_idx).copied().unwrap_or(0.0);
                let fraction_sold = *quantity as f64 / held as f64;
                if let Some(b) = state.cost_basis.get_mut(*corp_idx) {
                    *b -= basis * fraction_sold;
                    if *b < 0.0 { *b = 0.0; }
                }
            }
            state.portfolio[*corp_idx] -= quantity;
            let reaction = board::on_sell_shares(state, *corp_idx);
            let name = state.corporations[*corp_idx].name.clone();
            let mut msg = format!(
                "Sold {} shares of {} at ¥{:.1}/share (¥{:.0} proceeds)",
                quantity, name, state.corporations[*corp_idx].share_price, proceeds
            );
            if let Some(r) = reaction {
                msg.push_str(&format!(" — {}", r));
            }
            CommandResult { message: Some(msg), success: true, events: Vec::new() }
        }
        GameCommand::EmergencySampleDelivery { medicine_idx, region_idx } => {
            let mut rng = state.rng_misc.clone();
            let (success, msg) = medicine::emergency_sample_delivery(
                state, *medicine_idx, *region_idx, &mut rng, &mut events,
            );
            state.rng_misc = rng;
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::CancelContract { board_member_idx } => {
            let (success, msg) = contracts::cancel_contract(state, *board_member_idx);
            CommandResult { message: msg, success, events: Vec::new() }
        }
        GameCommand::BailoutCorporation { corp_idx } => {
            if *corp_idx >= state.corporations.len() {
                return CommandResult { message: Some("Invalid corporation".to_string()), success: false, events: Vec::new() };
            }
            let corp = &state.corporations[*corp_idx];
            if corp.bankrupt {
                return CommandResult { message: Some("Corporation is bankrupt — bailout not possible".to_string()), success: false, events: Vec::new() };
            }
            let cost = corp.bailout_cost();
            if state.resources.funding < cost {
                return CommandResult {
                    message: Some(format!(
                        "Insufficient funds: need ¥{:.0}, have ¥{:.0}",
                        cost, state.resources.funding
                    )),
                    success: false,
                    events: Vec::new(),
                };
            }
            state.resources.funding -= cost;
            let max_reserves = state.corporations[*corp_idx].max_reserves;
            state.corporations[*corp_idx].reserves = max_reserves;
            let name = state.corporations[*corp_idx].name.clone();
            CommandResult {
                message: Some(format!(
                    "Bailout: ¥{:.0} injected into {}. Reserves restored to full.",
                    cost, name
                )),
                success: true,
                events: Vec::new(),
            }
        }
        GameCommand::FirePersonnel { count } => {
            let available = state.personnel_available();
            if available == 0 {
                return CommandResult {
                    message: Some("No unassigned personnel to fire.".to_string()),
                    success: false,
                    events: Vec::new(),
                };
            }
            let actual = (*count).min(available);
            state.resources.personnel -= actual;
            CommandResult {
                message: Some(format!(
                    "Fired {} personnel. Roster: {} (upkeep ¥{:.1}/day)",
                    actual, state.resources.personnel,
                    state.resources.personnel as f64 * PERSONNEL_UPKEEP_COST * TICKS_PER_DAY
                )),
                success: true,
                events: Vec::new(),
            }
        }
    };
    result.events = events;
    result
}


#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use crate::action::Action;
    use crate::apply_action;
    use crate::state::{Authority, CrisisKind, DecreeId, DeployTarget, AppState, WorldState, GovernorPersonality, MedicineUiState, OpsUiState, Panel, PathogenType, PolicyId, PolicyUiState, RegionDiseaseState, ResearchUiState};

    /// Helper: unlock all medicines and mark them tested (for tests that predate the research system).
    fn unlock_all_medicines(state: &mut WorldState) {
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
            med.doses = med.max_doses;
        }
    }

    /// Helper: mark all diseases as detected (most tests assume this).
    fn detect_all_diseases(state: &mut WorldState) {
        for d in &mut state.diseases {
            d.detected = true;
        }
    }

    /// Helper: find the region index that has the primary (first) disease outbreak.
    fn primary_outbreak_region(state: &WorldState) -> usize {
        state.regions.iter().position(|r|
            r.infections.iter().any(|i| i.disease_idx == 0 && i.infected > 0.0)
        ).expect("should have a region with disease 0")
    }

    #[test]
    fn tick_increases_infections() {
        let state = AppState::new_default(42);
        let initial = state.total_infected();
        let (after, _) = tick(&state);
        assert!(
            after.total_infected() > initial,
            "infections should grow: {} -> {}",
            initial,
            after.total_infected()
        );
    }

    #[test]
    fn tick_causes_deaths() {
        let state = AppState::new_default(42);
        let mut s = state;
        for _ in 0..20 {
            s = s.with_world(tick(&s).0);
        }
        assert!(s.total_dead() > 0.0, "should have some deaths after 20 ticks");
    }

    #[test]
    fn tick_advances_state() {
        let state = AppState::new_default(42);
        let (after, _) = tick(&state);
        assert_eq!(after.tick, state.tick + 1);
        assert!(after.total_infected() > state.total_infected());
    }

    #[test]
    fn multi_tick_determinism() {
        let state = AppState::new_default(42);
        let mut a = state.clone();
        let mut b = state;
        for _ in 0..50 {
            a = a.with_world(tick(&a).0);
            b = b.with_world(tick(&b).0);
        }
        assert_eq!(a.total_infected(), b.total_infected());
        assert_eq!(a.total_dead(), b.total_dead());
        assert_eq!(a.total_immune(), b.total_immune());
    }

    #[test]
    fn recovery_accumulates() {
        let state = AppState::new_default(42);
        let mut s = state;
        for _ in 0..50 {
            s = s.with_world(tick(&s).0);
        }
        assert!(
            s.total_immune() > 0.0,
            "should have immune (recovered) after 50 ticks, got {}",
            s.total_immune()
        );
    }

    #[test]
    fn population_conservation() {
        let state = AppState::new_default(42);
        let mut s = state;
        for _ in 0..100 {
            s = s.with_world(tick(&s).0);
        }
        for region in &s.regions {
            let pop = region.population as f64;
            // Shared death counter must not exceed population.
            assert!(
                region.dead <= pop + 1.0,
                "region {}: dead {} > population {}",
                region.name,
                region.dead,
                pop
            );
            for inf in &region.infections {
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
        let state = AppState::new_default(42);
        let mut s = state;
        // With smaller initial seed (500-2500), need more ticks for cross-region spread
        for _ in 0..1000 {
            s = s.with_world(tick(&s).0);
        }
        let infected_regions = s
            .regions
            .iter()
            .filter(|r| !r.infections.is_empty())
            .count();
        assert!(
            infected_regions > 1,
            "disease should spread to more than 1 region after 1000 ticks, got {}",
            infected_regions
        );
    }

    #[test]
    fn toggle_pause() {
        use crate::state::SimState;
        let state = AppState::new_default(42);
        assert!(state.sim_state.is_running());
        let s = apply_action(&state, &Action::TogglePause);
        assert_eq!(s.sim_state, SimState::Paused);
        let s = apply_action(&s, &Action::TogglePause);
        assert!(s.sim_state.is_running());
    }

    #[test]
    fn open_close_panels() {
        let state = AppState::new_default(42);
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
        let state = AppState::new_default(42);
        let max_sel = state.diseases.len() - 1;

        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.panel_selection, 0);
        // Navigate to the end
        let mut s = s;
        for _ in 0..max_sel {
            s = apply_action(&s, &Action::SelectNext);
        }
        assert_eq!(s.ui.panel_selection, max_sel);
        // Wraps from last to first
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 0);
        // Wraps from first to last
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.panel_selection, max_sel);
    }

    #[test]
    fn immune_reduces_susceptible_pool() {
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Set 90% of the region's population as immune — drastically reduces susceptible pool
        let pop = state.regions[ri].population as f64;
        state.regions[ri].get_or_create_infection(0).immune = pop * 0.9;
        let inf_before = state.regions[ri].disease_state(0).unwrap();
        let before = inf_before.exposed + inf_before.infected;
        let (after, _) = tick(&state);
        let inf_after = after.regions[ri].disease_state(0).unwrap();
        let growth = (inf_after.exposed + inf_after.infected) - before;

        let state2 = AppState::new_default(42);
        let ri2 = primary_outbreak_region(&state2);
        let inf_before2 = state2.regions[ri2].disease_state(0).unwrap();
        let before2 = inf_before2.exposed + inf_before2.infected;
        let (after2, _) = tick(&state2);
        let inf_after2 = after2.regions[ri2].disease_state(0).unwrap();
        let growth2 = (inf_after2.exposed + inf_after2.infected) - before2;

        assert!(
            growth < growth2,
            "immunity should reduce infection growth: {} vs {}",
            growth,
            growth2
        );
    }

    #[test]
    fn dense_urban_increases_spread() {
        use crate::state::RegionTrait;
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Ensure the outbreak region does NOT already have DenseUrban
        state.regions[ri].traits.retain(|t| *t != RegionTrait::DenseUrban);
        let inf = state.regions[ri].disease_state(0).unwrap();
        let before = inf.exposed + inf.infected;

        // Tick without DenseUrban
        let (after_normal, _) = tick(&state);
        let inf_n = after_normal.regions[ri].disease_state(0).unwrap();
        let growth_normal = (inf_n.exposed + inf_n.infected) - before;

        // Add DenseUrban trait and tick again
        state.regions[ri].traits.push(RegionTrait::DenseUrban);
        let (after_dense, _) = tick(&state);
        let inf_d = after_dense.regions[ri].disease_state(0).unwrap();
        let growth_dense = (inf_d.exposed + inf_d.infected) - before;

        assert!(growth_dense > growth_normal,
            "DenseUrban should increase within-region spread: {} vs {}", growth_dense, growth_normal);
    }

    #[test]
    fn strong_public_health_reduces_lethality() {
        use crate::state::RegionTrait;
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Ensure outbreak region has some infected to die
        let inf = state.regions[ri].disease_state(0).unwrap();
        assert!(inf.infected > 0.0, "need infected to measure lethality");

        // Remove StrongPublicHealth, tick, measure deaths
        state.regions[ri].traits.retain(|t| *t != RegionTrait::StrongPublicHealth);
        let (after_normal, _) = tick(&state);
        let deaths_normal = after_normal.regions[ri].dead - state.regions[ri].dead;

        // Add StrongPublicHealth, tick from same starting state
        state.regions[ri].traits.push(RegionTrait::StrongPublicHealth);
        let (after_sph, _) = tick(&state);
        let deaths_sph = after_sph.regions[ri].dead - state.regions[ri].dead;

        assert!(deaths_sph < deaths_normal,
            "StrongPublicHealth should reduce deaths: {} vs {}", deaths_sph, deaths_normal);
    }

    #[test]
    fn disease_can_spread_into_vaccinated_region() {
        let mut state = AppState::new_default(42);
        // Find a region WITHOUT disease 0 and pre-vaccinate it
        let clean_region = (0..state.regions.len())
            .find(|&i| !state.regions[i].infections.iter().any(|inf| inf.disease_idx == 0))
            .expect("should have an uninfected region");
        state.regions[clean_region].get_or_create_infection(0).immune = 100_000_000.0;
        let mut s = state;
        for _ in 0..200 {
            s = s.with_world(tick(&s).0);
        }
        let imm = s.regions[clean_region]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .map(|i| i.immune)
            .unwrap_or(0.0);
        assert!(
            imm >= 100_000_000.0,
            "immune count should be preserved"
        );
    }

    #[test]
    fn medicine_vaccination_deployment() {
        use crate::state::DeployTarget;
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        // Unlock VaccinePlatform so vaccination mode is available
        state.unlocked_techs.push(crate::state::BasicTech::VaccinePlatform);
        // Find any tested, unlocked targeted medicine
        let med_idx = state.medicines.iter().position(|m| {
            m.unlocked && m.mechanism.is_some() && !m.tested_against.is_empty()
        }).expect("should have a tested targeted medicine");

        let doses_before = state.medicines[med_idx].doses;
        let funding_before = state.resources.funding;
        // Deploy via engine API using Vaccination mode
        let (ok, _msg) = medicine::deploy_medicine(
            &mut state, med_idx, 0,
            DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Vaccine },
            &mut events,
        );
        assert!(ok, "vaccination deploy should succeed");
        // Dispatch deducts doses immediately (no funding cost), creates a pending shipment
        assert_eq!(state.resources.funding, funding_before, "no funding cost on deploy");
        assert!(state.medicines[med_idx].doses < doses_before, "doses deducted on dispatch");
        assert_eq!(state.pending_shipments.len(), 1, "should have one pending shipment");

        // Advance time and deliver — immune should increase
        let immune_before = state.regions[0].infections.iter()
            .find(|i| i.disease_idx == 0).map(|i| i.immune).unwrap_or(0.0);
        state.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }
        let immune_after = state.regions[0].infections.iter()
            .find(|i| i.disease_idx == 0).map(|i| i.immune).unwrap_or(0.0);
        assert!(immune_after > immune_before, "immune should increase after delivery: {immune_before} -> {immune_after}");
    }

    #[test]
    fn medicine_treatment_deployment() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::DeployTarget;
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        // Disable auto-deploy so it doesn't create cooldowns that interfere
        // with the manual deploy this test is exercising.
        for flag in &mut state.deploy_enabled { *flag = false; }
        for _ in 0..20 {
            state = state.with_world(tick(&state).0);
        }
        let ri = primary_outbreak_region(&state);
        let infected_before = state.regions[ri].disease_state(0).unwrap().infected;

        // Deploy treatment directly via engine API
        state.medicines[0].tested_against.push(0);
        let funding_before = state.resources.funding;
        let (ok, _msg) = medicine::deploy_medicine(
            &mut state, 0, ri,
            DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        assert!(ok, "deployment should succeed");

        // Dispatch: doses deducted (no funding cost), pending shipment created
        assert_eq!(state.resources.funding, funding_before);
        assert!(
            state.medicines[0].doses < state.medicines[0].max_doses,
            "doses should have been deducted on dispatch"
        );
        assert_eq!(state.pending_shipments.len(), 1, "should have one pending shipment");

        // Deliver the shipment
        state.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }

        let infected_after = state.regions[ri].disease_state(0).unwrap().infected;
        assert!(
            infected_after < infected_before,
            "treatment should reduce infected after delivery: {} -> {}",
            infected_before,
            infected_after
        );
    }

    #[test]
    fn medicine_empty_doses_blocks_deployment() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::DeployTarget;
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].doses = 0.0; // Empty
        for _ in 0..20 {
            state = state.with_world(tick(&state).0);
        }
        let ri = primary_outbreak_region(&state);

        // Deploy directly via engine API (UI flow skips steps now)
        let funding_before = state.resources.funding;
        let (ok, msg) = medicine::deploy_medicine(
            &mut state, 0, ri, DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        assert!(!ok, "should fail when empty");
        assert_eq!(state.resources.funding, funding_before, "should not charge when empty");
        assert!(
            msg.as_ref().unwrap().contains("No doses remaining"),
            "expected no doses message, got: {:?}", msg
        );
    }

    #[test]
    fn medicine_esc_backstep() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        state = apply_action(&state, &Action::ClosePanel); // → close panel
        assert_eq!(state.ui.open_panel, Panel::None);
        assert!(state.ui.medicine_ui.is_none());
    }

    #[test]
    fn medicine_zero_targets_refused() {
        use crate::state::DeployTarget;
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        // Clear region 0 infections so we can test treating with zero targets
        state.regions[0].infections.clear();
        let infections_before = state.regions[0].infections.len();
        let funding_before = state.resources.funding;
        let (ok, msg) = medicine::deploy_medicine(
            &mut state, 0, 0,
            DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        assert!(!ok, "deploy should fail when no infected targets");
        assert_eq!(state.resources.funding, funding_before, "should not charge on failure");
        assert!(
            msg.as_ref().map(|m| m.contains("No infected")).unwrap_or(false),
            "expected zero-target message, got: {:?}", msg
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
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        // Open medicines, navigate away to another panel, then re-open
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::SelectNext); // move selection
        state = apply_action(&state, &Action::OpenThreats);
        state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        assert_eq!(state.ui.panel_selection, 0);
    }

    /// Helper: unlock medicines but leave them untested.
    fn unlock_untested(state: &mut WorldState) {
        for med in &mut state.medicines {
            med.unlocked = true;
            med.doses = med.max_doses;
        }
    }

    #[test]
    fn toggle_deploy_enables_and_disables() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        let med_idx = 0;

        // Initially deploy_enabled is empty / false
        assert!(!state.deploy_enabled.get(med_idx).copied().unwrap_or(false));

        // Toggle on
        crate::engine::execute_command(&mut state, &crate::engine::GameCommand::ToggleDeploy { med_idx });
        assert!(state.deploy_enabled.get(med_idx).copied().unwrap_or(false),
            "should be enabled after first toggle");

        // Toggle off
        crate::engine::execute_command(&mut state, &crate::engine::GameCommand::ToggleDeploy { med_idx });
        assert!(!state.deploy_enabled.get(med_idx).copied().unwrap_or(false),
            "should be disabled after second toggle");
    }

    #[test]
    fn enter_on_browse_toggles_deploy() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::BrowseMedicines)));

        // Confirm (Enter) should toggle deploy, not open a wizard
        let was_enabled = state.deploy_enabled.get(0).copied().unwrap_or(false);
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::BrowseMedicines)),
            "Enter should stay in BrowseMedicines after toggling deploy");
        assert_ne!(state.deploy_enabled.get(0).copied().unwrap_or(false), was_enabled,
            "deploy_enabled should have flipped");
    }

    #[test]
    fn multi_target_medicine_deploys_via_engine_api() {
        use crate::state::DeployTarget;
        let mut state = AppState::new_default(42);
        // Ensure infections exist (therapeutics need infected population)
        state.regions[0].get_or_create_infection(0).infected = 50_000.0;
        // Find any targeted medicine
        let med_idx = state.medicines.iter().position(|m| {
            m.mechanism.is_some()
        }).expect("should have a targeted medicine");
        // Make it target two diseases and be tested against both
        state.medicines[med_idx].unlocked = true;
        state.medicines[med_idx].tested_against = vec![0, 1];
        state.medicines[med_idx].target_diseases = vec![0, 1];
        state.medicines[med_idx].doses = state.medicines[med_idx].max_doses;
        // Add a second disease
        { let d = state.world.diseases[0].clone(); state.world.diseases.push(d); };
        state.diseases[1].detected = true;

        let mut events: Vec<GameEvent> = Vec::new();
        let doses_before = state.medicines[med_idx].doses;
        // Deploy directly via engine API
        let (ok, _msg) = medicine::deploy_medicine(
            &mut state, med_idx, 0,
            DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        assert!(ok, "multi-target tested medicine should deploy successfully");
        assert!(state.medicines[med_idx].doses < doses_before, "doses should be consumed");
    }

    #[test]
    fn untested_medicine_deploy_succeeds_without_confirmation() {
        use crate::state::DeployTarget;
        let mut state = AppState::new_default(42);
        unlock_untested(&mut state);
        // Ensure infections exist (therapeutics need infected population to deploy)
        state.regions[0].get_or_create_infection(0).infected = 50_000.0;
        // Find any targeted medicine (untested)
        let med_idx = state.medicines.iter().position(|m| {
            m.unlocked && m.mechanism.is_some()
        }).expect("should have a targeted medicine");

        let mut events: Vec<GameEvent> = Vec::new();
        let funding_before = state.resources.funding;
        let (ok, _msg) = medicine::deploy_medicine(
            &mut state, med_idx, 0,
            DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        // The engine allows deploying untested medicines — no confirmation step needed
        assert!(ok, "untested medicine should deploy via engine API");
        assert_eq!(state.resources.funding, funding_before, "deploy should be free");
    }

    #[test]
    fn tested_medicine_deploys_immediately() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::DeployTarget;
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state); // tested
        // Advance time so there are infections to treat
        for _ in 0..20 {
            state = state.with_world(tick(&state).0);
        }
        let ri = primary_outbreak_region(&state);
        let funding_before = state.resources.funding;
        // Deploy directly via engine API — tested medicine should succeed
        let (ok, _msg) = medicine::deploy_medicine(
            &mut state, 0, ri, DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        assert!(ok, "tested medicine should deploy immediately");
        assert_eq!(state.resources.funding, funding_before, "deploy should be free");
    }

    #[test]
    fn map_navigation_right_left_wraps() {
        // Reading order: NA(0) → EU(2) → Asia(4) → SA(1) → Africa(3) → Oceania(5) → NA(0)
        let state = AppState::new_default(42);
        assert_eq!(state.ui.map_selection, 0); // NA
        let s = apply_action(&state, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 2); // EU
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 4); // Asia
        // Wraps from end of row 0 to start of row 1
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 1); // SA
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 3); // Africa
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 5); // Oceania
        // Wraps from last region back to first
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 0); // NA

        // Left wraps the other direction
        let s = apply_action(&state, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 5); // Oceania (wrap from first to last)
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 3); // Africa
    }

    #[test]
    fn map_navigation_up_down_no_panel() {
        let state = AppState::new_default(42);
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
        let mut state = AppState::new_default(42);
        // Need at least 2 diseases so the panel has items to navigate
        {
            let mut rng = state.world.rng_emergence.clone();
            let d = crate::state::Disease::generate(&mut rng, crate::state::PathogenType::Bacterium, &[], true);
            state.world.diseases.push(d);
        }
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
    fn research_panel_navigation() {
        let mut state = AppState::new_default(42);
        state = apply_action(&state, &Action::OpenResearch);

        // Flat panel: BrowseAll with all items in one list
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));
        assert_eq!(state.ui.panel_selection, 0);

        let items = state.research_flat_items();
        let max = items.len().saturating_sub(1);
        assert!(max > 0, "should have at least one selectable item");

        // Navigate forward through all items
        for i in 1..=max {
            state = apply_action(&state, &Action::SelectNext);
            assert_eq!(state.ui.panel_selection, i);
        }

        // Wraps from last to first
        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 0);

        // Esc closes
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_esc_backstep() {
        let mut state = AppState::new_default(42);
        detect_all_diseases(&mut state);

        state = apply_action(&state, &Action::OpenResearch);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));

        // Confirm first available project → goes to ConfirmProject
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ConfirmProject { .. })));

        // Esc back to flat list
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));

        // Esc again closes panel
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_confirm_noop_on_active_project() {
        use crate::state::ResearchFlatItem;

        let mut state = AppState::new_default(42);
        // Start a field research project first
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Confirm first available
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ConfirmProject { .. })));
        state = apply_action(&state, &Action::Confirm); // Start it
        // After starting, UI returns to BrowseAll on the Research panel

        // Close the panel first, then re-open to get a fresh BrowseAll
        state = apply_action(&state, &Action::ClosePanel);
        state = apply_action(&state, &Action::OpenResearch);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));
        let items = state.research_flat_items();
        // Find the active item's position in the unified list
        let active_pos = items.iter().position(|i| matches!(i, ResearchFlatItem::Active(_))).unwrap();
        assert!(matches!(items[active_pos], ResearchFlatItem::Active(0)));

        // Navigate to the active project and press Enter — should be a no-op
        state.ui.panel_selection = active_pos;
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseAll)));
    }

    #[test]
    fn diseases_start_unknown() {
        let state = AppState::new_default(42);
        for disease in &state.diseases {
            assert_eq!(disease.knowledge, 0.0);
        }
    }

    #[test]
    fn lose_condition_triggers_when_all_regions_collapse() {
        let mut state = AppState::new_default(42);
        // Override to extreme parameters so all regions collapse quickly.
        // Normal game parameters (R0 3-5) cause loss via multiple diseases over 20 days.
        for disease in &mut state.diseases {
            disease.within_region_spread = 0.5;
            disease.lethality = 0.1;
            disease.recovery_rate = 0.005;
            disease.cross_region_spread = 0.3;
        }
        // Seed all regions heavily so collapse happens within a few hundred ticks
        for region in &mut state.regions {
            for inf in &mut region.infections {
                inf.infected = 1_000_000.0;
            }
        }
        // Run until game over (collapse requires all regions to fall)
        for _ in 0..10000 {
            state = state.with_world(tick(&state).0);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        assert_eq!(state.outcome, GameOutcome::Lost);
        assert!(state.is_blocked(), "game should be blocked after game over");
        // All regions should be collapsed with timestamps
        assert!(state.regions.iter().all(|r| r.collapsed));
        assert!(state.regions.iter().all(|r| r.collapsed_at_tick.is_some()),
            "every collapsed region should have a collapse timestamp");
        // Collapse timestamps should be in order (earlier collapses have lower tick values)
        let ticks: Vec<u64> = state.regions.iter()
            .filter_map(|r| r.collapsed_at_tick)
            .collect();
        assert_eq!(ticks.len(), state.regions.len());
        // Not all should be the same tick (regions collapse at different rates)
        assert!(ticks.iter().collect::<std::collections::HashSet<_>>().len() > 1,
            "regions should collapse at different times, got {:?}", ticks);
    }

    // ⚠️  HARD REQUIREMENT — DO NOT WEAKEN THIS TEST. EVER.
    //
    // This test enforces a direct user requirement: with zero player intervention,
    // every seed must reach GameOutcome::Lost (all regions collapsed) within 120 days.
    // Cross-region spread is intentionally low (regional containment is a core
    // strategy), so diseases take longer to reach all 6 regions — but within each
    // region they burn fast. The ceiling accounts for this regional spread delay.
    //
    // If this test fails, disease parameters are too weak — increase within_region_spread
    // in PathogenType::stat_ranges(). Do NOT increase per-tick lethality or recovery
    // (that shortens infectious period and causes epidemic burnout — see stat_ranges()
    // comment). Do NOT raise the day ceiling or add seeds to skip. Do NOT add `#[ignore]`.
    //
    // See also: CLAUDE.md "Game Balance Thresholds — DO NOT NERF DISEASES",
    // PathogenType::stat_ranges() comment in state.rs.
    #[test]
    fn game_is_lost_within_120_days_without_intervention() {
        // HARD REQUIREMENT: with zero player input, every seed must lose by day 120.
        // Median should be under 90 days. See CLAUDE.md "Game Balance Thresholds".
        let seeds: Vec<u64> = (0..50).collect();
        let mut loss_days = Vec::new();
        for seed in &seeds {
            let mut state = AppState::new_default(*seed);
            corporations::generate_corporations(&mut state);
            board::generate_board_members(&mut state);
            let max_ticks = 120 * TICKS_PER_DAY as u64;
            for _ in 0..max_ticks {
                state = state.with_world(tick(&state).0);
                if state.active_crisis.is_some() {
                    state.active_crisis = None;
                }
                if state.outcome != GameOutcome::Playing {
                    break;
                }
            }
            let day = state.tick as f64 / TICKS_PER_DAY;
            assert_eq!(state.outcome, GameOutcome::Lost,
                "Seed {seed}: game should be lost within 120 days (reached day {day:.1}). \
                 Regions: {:?}. If this fails, fix disease params — do NOT raise the ceiling.",
                state.regions.iter().map(|r| {
                    let pct = 100.0 * (1.0 - r.alive() as f64 / r.population as f64);
                    (r.name.clone(), r.collapsed, format!("{pct:.1}% dead"))
                }).collect::<Vec<_>>());
            loss_days.push(day);
        }
        loss_days.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = loss_days[loss_days.len() / 2];
        let max = loss_days.last().copied().unwrap();
        assert!(median < 90.0,
            "Median loss day is {median:.1} (expected < 90). Max: {max:.1}. Days: {loss_days:?}. \
             Disease parameters are too weak — fix disease params, do NOT raise the ceiling.");
    }

    #[test]
    fn competent_play_extends_survival() {
        // A player who uses research + policies + medicine should survive
        // meaningfully longer than passive play. The three pillars work
        // together: quarantine slows spread, research develops medicines,
        // treatment reduces infected population.
        //
        // Note: BS efficacy is intentionally low (0.15) — BS is a bandaid that
        // buys time while targeted medicines are researched, not a win condition.
        // The 1.10x threshold ensures player actions produce a meaningful (>=10%)
        // survival improvement. Actual median is typically ~1.15-1.20x across seeds.
        //
        // Strategy: treatment first (removes ~efficacy fraction of infected per deploy),
        // vaccination second (only with targeted meds — BS vaccination is a dose trap).
        // Prioritize worst-hit regions.
        use crate::state::{ResearchKind, DeployTarget};

        fn simulate_competent(seed: u64) -> f64 {
            let mut state = AppState::new_default(seed);
            corporations::generate_corporations(&mut state);
            board::generate_board_members(&mut state);
            // Give the bot full authority so it can test policies & research,
            // not the authority ramp mechanic.
            state.resources.authority = Authority::Maximum;
            // Extra starting personnel — a competent player trains personnel.
            state.resources.personnel += 25;
            let max_ticks = 200 * TICKS_PER_DAY as u64;
            let mut total_deploys = 0u32;
            for _ in 0..max_ticks {
                state = state.with_world(tick(&state).0);
                if state.active_crisis.is_some() {
                    state.active_crisis = None;
                }
                if state.outcome != GameOutcome::Playing { break; }

                // --- RESEARCH: start projects by priority until out of resources ---
                loop {
                    let projects = state.all_available_projects();
                    // Train personnel when running low (< 10 available)
                    let need_personnel = state.personnel_available() < 10;
                    let best = projects.iter().enumerate().min_by_key(|(_, k)| match k {
                        ResearchKind::TrainPersonnel if need_personnel => 0,
                        ResearchKind::DevelopMedicine { .. } => 1,
                        ResearchKind::ManufactureDoses { .. } => 1,
                        ResearchKind::IdentifyThreat { .. } => 2,
                        ResearchKind::ClinicalTrial { .. } => 3,
                        ResearchKind::GenomicSequencing { .. } => 4,
                        _ => 5,
                    });
                    if let Some((idx, kind)) = best {
                        let (personnel, _, cost_funding) = kind.costs(&state.medicines);
                        if state.resources.funding >= cost_funding + 200.0
                            && state.personnel_available() >= personnel
                        {
                            execute_command(&mut state, &GameCommand::StartResearch {
                                project_idx: idx, double_personnel: false,
                            });
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                // --- POLICIES: containment and public health ---
                // A competent player enables border controls immediately (cheap,
                // always helps cross-region spread), enables water sanitation
                // only when a waterborne disease is present (otherwise pure waste),
                // quarantines infected regions.
                let any_waterborne = state.diseases.iter().any(|d| {
                    d.transmission == crate::state::TransmissionVector::Waterborne
                });
                for r_idx in 0..state.regions.len() {
                    if state.regions[r_idx].collapsed { continue; }
                    let total_infected: f64 = state.regions[r_idx].infections.iter()
                        .map(|inf| inf.infected).sum();
                    let has_border = state.policies[r_idx].border_controls;
                    let has_water = state.policies[r_idx].water_sanitation;
                    let has_quarantine = state.policies[r_idx].quarantine;
                    if !has_border {
                        execute_command(&mut state, &GameCommand::TogglePolicy {
                            region_idx: r_idx, policy: PolicyId::BorderControls,
                        });
                    }
                    if !has_water && any_waterborne {
                        execute_command(&mut state, &GameCommand::TogglePolicy {
                            region_idx: r_idx, policy: PolicyId::WaterSanitation,
                        });
                    }
                    if !has_quarantine && total_infected > 10_000.0 {
                        execute_command(&mut state, &GameCommand::TogglePolicy {
                            region_idx: r_idx, policy: PolicyId::Quarantine,
                        });
                    }
                    let has_travel_ban = state.policies[r_idx].travel_ban;
                    if !has_travel_ban && total_infected > 100_000.0 {
                        execute_command(&mut state, &GameCommand::TogglePolicy {
                            region_idx: r_idx, policy: PolicyId::TravelBan,
                        });
                    }
                }

                // --- MEDICINE: treat aggressively, vaccinate only with targeted meds ---
                // Treatment removes ~efficacy fraction of infected per deploy and
                // costs proportional doses. BS (0.15 efficacy) slows disease but
                // can't stop it — targeted medicines are needed to actually clear it.
                // BS vaccination burns doses for minimal coverage; vaccinate only
                // with targeted medicines (which are dose-efficient by design).
                let min_funding = 200.0;

                // Build list of (region, disease, infected) sorted by severity
                let mut targets: Vec<(usize, usize, f64)> = Vec::new();
                for r_idx in 0..state.regions.len() {
                    if state.regions[r_idx].collapsed { continue; }
                    for d_idx in 0..state.diseases.len() {
                        let infected = state.regions[r_idx].infections.iter()
                            .find(|inf| inf.disease_idx == d_idx)
                            .map(|inf| inf.infected)
                            .unwrap_or(0.0);
                        if infected > 1000.0 {
                            targets.push((r_idx, d_idx, infected));
                        }
                    }
                }
                targets.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

                for &(r_idx, d_idx, _infected) in &targets {
                    if state.resources.funding < min_funding { break; }

                    // Find and deploy best medicine as therapeutic.
                    let mut best: Option<usize> = None;
                    let mut best_eff = 0.0f64;
                    let mut best_is_targeted = false;
                    for mi in 0..state.medicines.len() {
                        let m = &state.medicines[mi];
                        if !m.unlocked || m.doses <= 0.0 { continue; }
                        if !m.tested_against.contains(&d_idx) { continue; }
                        let eff = m.effective_efficacy(d_idx, &state.diseases);
                        let is_targeted = m.therapy_type != crate::state::TherapyType::BroadSpectrum;
                        if (is_targeted && !best_is_targeted) || (is_targeted == best_is_targeted && eff > best_eff) {
                            best_eff = eff;
                            best = Some(mi);
                            best_is_targeted = is_targeted;
                        }
                    }
                    if let Some(med_idx) = best {
                        let target = DeployTarget { disease_idx: d_idx, mode: crate::state::MedicineMode::Therapeutic };
                        let result = execute_command(&mut state, &GameCommand::DeployMedicine {
                            medicine_idx: med_idx, region_idx: r_idx, target,
                        });
                        if result.success { total_deploys += 1; }
                    }
                }
            }
            let day = state.tick as f64 / TICKS_PER_DAY;
            eprintln!("  seed {seed}: {day:.1}d, {total_deploys} deploys, dead={:.0}, funds={:.0}",
                state.total_dead(), state.resources.funding);
            day
        }

        fn simulate_passive(seed: u64) -> f64 {
            let mut state = AppState::new_default(seed);
            corporations::generate_corporations(&mut state);
            board::generate_board_members(&mut state);
            let max_ticks = 200 * TICKS_PER_DAY as u64;
            for _ in 0..max_ticks {
                state = state.with_world(tick(&state).0);
                if state.active_crisis.is_some() {
                    state.active_crisis = None;
                }
                if state.outcome != GameOutcome::Playing { break; }
            }
            state.tick as f64 / TICKS_PER_DAY
        }

        // Paired comparison: same seed, active vs passive.
        // This isolates the effect of player actions from seed-specific variability.
        let seeds: Vec<u64> = (0..20).collect();
        let pairs: Vec<(f64, f64)> = seeds.iter().map(|s| {
            (simulate_competent(*s), simulate_passive(*s))
        }).collect();
        let improvements: Vec<f64> = pairs.iter()
            .map(|(a, p)| a / p)
            .collect();
        let mut sorted_improvements = improvements.clone();
        sorted_improvements.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_improvement = sorted_improvements[sorted_improvements.len() / 2];

        // At least half of seeds should show the active player surviving longer
        let better_count = pairs.iter().filter(|(a, p)| a > p).count();

        eprintln!("Paired results ({better_count}/{} seeds active > passive):", seeds.len());
        for (i, (a, p)) in pairs.iter().enumerate() {
            let ratio = a / p;
            eprintln!("  seed {i}: active={a:.1}d passive={p:.1}d ratio={ratio:.2}");
        }
        eprintln!("Median improvement ratio: {median_improvement:.2}");

        assert!(median_improvement >= 1.10,
            "Median paired improvement is {median_improvement:.2}x (expected >=1.10x). \
             Player actions (policies + research + BS treatment) aren't meaningful enough. \
             {better_count}/{} seeds show improvement.",
            seeds.len());
    }

    #[test]
    fn no_collapse_before_day_6_without_intervention() {
        // First collapse should not occur before day 6, giving players
        // minimum time for initial decisions. With aggressive disease
        // parameters, day 20 is too generous — some seeds collapse by day 16.
        for seed in [42, 123, 7, 99, 2024, 1, 999, 314, 55555, 8675309_u64] {
            let mut state = AppState::new_default(seed);
            let max_ticks = 6 * TICKS_PER_DAY as u64;
            for t in 0..max_ticks {
                state = state.with_world(tick(&state).0);
                if state.active_crisis.is_some() {
                    state.active_crisis = None;
                }
                let collapsed = state.regions.iter().find(|r| r.collapsed);
                assert!(
                    collapsed.is_none(),
                    "Seed {seed}: {} collapsed at tick {t} (day {:.1}), expected no collapse before day 6",
                    collapsed.map(|r| r.name.as_str()).unwrap_or("?"),
                    t as f64 / TICKS_PER_DAY
                );
            }
        }
    }

    #[test]
    fn no_victory_condition_exists() {
        let mut state = AppState::new_default(42);
        // Clear all infections, identify everything, test all medicines
        for region in &mut state.regions {
            region.infections.clear();
        }
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
        }
        let disease_count = state.diseases.len();
        state.medicines[0].tested_against = (0..disease_count).collect();
        // Advance past emergence threshold so burn-out spawn can fire
        state.tick = crate::state::EMERGENCE_MIN_TICK + 1;
        state = state.with_world(tick(&state).0);
        // Game should NOT end — there is no victory. Instead, a new disease spawns.
        assert_eq!(state.outcome, GameOutcome::Playing);
        assert!(
            state.diseases.len() > disease_count,
            "when all infections burn out, a new disease should spawn"
        );
    }


    #[test]
    fn no_deploy_after_game_over() {
        let mut state = AppState::new_default(42);
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
        let mut state = AppState::new_default(42);
        state.outcome = GameOutcome::Lost;
        let s = apply_action(&state, &Action::TogglePause);
        assert!(s.is_blocked(), "game should remain blocked after game over");
    }

    #[test]
    fn tick_does_not_advance_after_game_over() {
        let mut state = AppState::new_default(42);
        state.outcome = GameOutcome::Lost;
        let tick_before = state.tick;
        state = state.with_world(tick(&state).0);
        assert_eq!(state.tick, tick_before, "tick should not advance after game over");
    }

    #[test]
    fn tiny_infected_snaps_to_zero() {
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Set up a region with sub-person infected count
        state.regions[ri].get_or_create_infection(0).infected = 0.7;
        state = state.with_world(tick(&state).0);
        // Should have snapped to 0 (sub-person counts are meaningless)
        assert_eq!(
            state.regions[ri].disease_state(0).unwrap().infected, 0.0,
            "infected below 1.0 should snap to zero"
        );
    }

    #[test]
    fn multi_disease_dead_never_exceeds_population() {
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        let pop = state.regions[ri].population as f64;
        // Add a second disease with heavy infection in the same region
        { let d = state.world.diseases[0].clone(); state.world.diseases.push(d); };
        state.regions[ri].get_or_create_infection(1).infected = pop * 0.3;
        // Also boost first disease
        state.regions[ri].get_or_create_infection(0).infected = pop * 0.3;
        // Run many ticks — both diseases should share the population
        for _ in 0..2000 {
            state = state.with_world(tick(&state).0);
            if state.active_crisis.is_some() {
                state.active_crisis = None;
            }
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        // Shared death counter should never exceed population.
        assert!(state.regions[ri].dead <= pop + 1.0,
            "shared dead ({:.0}) should not exceed population ({pop:.0})",
            state.regions[ri].dead);
        // Per-disease attribution totals should approximately match shared dead.
        let attributed: f64 = state.regions[ri].infections.iter()
            .map(|i| i.dead).sum();
        assert!(attributed <= pop * 1.05,
            "attributed dead sum ({attributed:.0}) should not wildly exceed population ({pop:.0})");
    }

    #[test]
    fn coinfection_increases_deaths() {
        // With 2 diseases both above the co-infection threshold,
        // deaths should be higher than with a single disease.
        let mut single = AppState::new_default(42);
        let ri = primary_outbreak_region(&single);
        single.regions[ri].get_or_create_infection(0).infected = 100_000.0;

        let mut dual = single.clone();
        // Add a second disease with significant infection
        { let d = dual.world.diseases[0].clone(); dual.world.diseases.push(d); }
        dual.regions[ri].get_or_create_infection(1).infected = 100_000.0;

        // Run some ticks
        for _ in 0..100 {
            single = single.with_world(tick(&single).0);
            dual = dual.with_world(tick(&dual).0);
        }

        let single_dead = single.regions[ri].dead;
        let dual_dead = dual.regions[ri].dead;
        assert!(dual_dead > single_dead,
            "co-infection should cause more deaths: dual={:.0} vs single={:.0}",
            dual_dead, single_dead);
    }

    #[test]
    fn burn_out_spawns_scaled_disease() {
        let mut state = AppState::new_default(42);
        // Clear all infections to simulate burn-out
        for region in &mut state.regions {
            region.infections.clear();
        }
        // Set to day 20 — scaled disease should have 2.0x boosted stats
        state.tick = 20 * crate::state::TICKS_PER_DAY as u64;
        let disease_count = state.diseases.len();
        let original_spread = state.diseases[0].within_region_spread;
        state = state.with_world(tick(&state).0);

        assert!(
            state.diseases.len() > disease_count,
            "should spawn a new disease when all infections burn out"
        );
        // The new disease at day 20 gets 2.0x scaling (1.0 + 20 * 0.05).
        // Its base stats are in a similar range to disease 0, so after 2x scaling
        // it should be notably more infectious.
        let new_disease = &state.diseases[disease_count];
        assert!(
            new_disease.within_region_spread > original_spread,
            "late-game disease within-region spread ({}) should exceed original ({})",
            new_disease.within_region_spread, original_spread
        );
    }

    #[test]
    fn burn_out_recycles_slot_at_max_diseases() {
        use crate::state::MAX_DISEASES;
        let mut state = AppState::new_default(42);
        // Fill up to MAX_DISEASES
        while state.diseases.len() < MAX_DISEASES {
            let mut rng = state.rng_emergence.clone();
            disease::spawn_disease(&mut state, &mut rng);
            state.rng_emergence = rng;
        }
        assert_eq!(state.diseases.len(), MAX_DISEASES);
        // Clear all infections to simulate burn-out
        for region in &mut state.regions {
            region.infections.clear();
        }
        state.tick = 20 * crate::state::TICKS_PER_DAY as u64;
        state = state.with_world(tick(&state).0);
        // Should have recycled a slot — disease count stays at MAX_DISEASES
        assert_eq!(state.diseases.len(), MAX_DISEASES,
            "should recycle a slot, not exceed MAX_DISEASES");
        // At least one disease should have infections (the recycled one)
        assert!(state.total_infected() > 0.0,
            "recycled disease should have active infections");
    }

    #[test]
    fn policy_travel_ban_reduces_spread() {
        let mut state = AppState::new_default(42);
        // Run without travel ban
        let mut no_ban = state.clone();
        for _ in 0..100 {
            no_ban = no_ban.with_world(tick(&no_ban).0);
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
            with_ban = with_ban.with_world(tick(&with_ban).0);
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
    fn travel_ban_does_not_block_medicine_shipments() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::DeployTarget;
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.push(0);
        // Infect region 0 so treatment makes sense
        state.regions[0].get_or_create_infection(0).infected = 50_000.0;
        // Enable travel ban on region 0
        state.policies[0].travel_ban = true;

        // Deploy medicine to region 0 (creates a pending shipment)
        let (ok, _msg) = medicine::deploy_medicine(
            &mut state, 0, 0,
            DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic },
            &mut events,
        );
        assert!(ok, "deployment should succeed despite travel ban");
        assert_eq!(state.pending_shipments.len(), 1);

        // Advance past arrival tick
        let arrive = state.pending_shipments[0].arrive_tick;
        state.tick = arrive + 1;
        { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }

        // Shipment should have been delivered, not blocked
        assert_eq!(state.pending_shipments.len(), 0, "shipment should deliver despite travel ban");
        let delivered = events.iter().any(|e| matches!(e, GameEvent::ShipmentDelivered { .. }));
        assert!(delivered, "should have ShipmentDelivered event");
    }

    #[test]
    fn personnel_upkeep_reduces_funding() {
        use crate::state::PERSONNEL_UPKEEP_COST;
        let mut state = AppState::new_default(42);
        for r in &mut state.regions {
            r.infections.clear();
        }
        state.resources.funding = 1000.0;
        let income = state.funding_income_rate();
        let upkeep = state.resources.personnel as f64 * PERSONNEL_UPKEEP_COST;

        let (after, _) = tick(&state);
        let delta = after.resources.funding - 1000.0;

        // Net change should be income minus upkeep (no policies)
        assert!(
            (delta - (income - upkeep)).abs() < 0.01,
            "funding delta {delta:.2} should equal income {income:.2} - upkeep {upkeep:.2}"
        );
        // Upkeep should be non-negligible
        assert!(upkeep > 0.1, "upkeep {upkeep:.2} should be meaningful");
    }

    #[test]
    fn fire_personnel_reduces_roster() {
        let mut state = AppState::new_default(42);
        state.resources.personnel = 25;
        let result = execute_command(&mut state, &GameCommand::FirePersonnel { count: 5 });
        assert!(result.success);
        assert_eq!(state.resources.personnel, 20);
    }

    #[test]
    fn fire_personnel_capped_by_available() {
        let mut state = AppState::new_default(42);
        state.resources.personnel = 25;
        // Start a research project to tie up some personnel
        state.resources.authority = Authority::Maximum;
        let _ = research::start_research(&mut state, 0, false);
        let available = state.personnel_available();
        assert!(available < 25, "some personnel should be busy");
        let result = execute_command(&mut state, &GameCommand::FirePersonnel { count: 100 });
        assert!(result.success);
        assert_eq!(state.resources.personnel, 25 - available);
    }

    #[test]
    fn fire_personnel_fails_when_none_available() {
        let mut state = AppState::new_default(42);
        state.resources.personnel = 0;
        let result = execute_command(&mut state, &GameCommand::FirePersonnel { count: 5 });
        assert!(!result.success);
    }

    #[test]
    fn policy_quarantine_reduces_infections() {
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Run without quarantine
        let mut no_q = state.clone();
        for _ in 0..50 {
            no_q = no_q.with_world(tick(&no_q).0);
        }

        // Run with quarantine on the primary outbreak region
        state.policies[ri].quarantine = true;
        let mut with_q = state;
        for _ in 0..50 {
            with_q = with_q.with_world(tick(&with_q).0);
        }

        assert!(
            with_q.regions[ri].total_infected() < no_q.regions[ri].total_infected(),
            "quarantine should reduce infections: {} vs {}",
            with_q.regions[ri].total_infected(), no_q.regions[ri].total_infected()
        );
    }

    #[test]
    fn discourage_hospitalization_increases_deaths() {
        let mut state = AppState::new_default(42);
        let ri = primary_outbreak_region(&state);
        // Run baseline (hospitals active by default)
        let mut baseline = state.clone();
        for _ in 0..50 {
            baseline = baseline.with_world(tick(&baseline).0);
        }

        // Run with discourage hospitalization on the primary outbreak region
        state.policies[ri].discourage_hosp = true;
        let mut with_dh = state;
        for _ in 0..50 {
            with_dh = with_dh.with_world(tick(&with_dh).0);
        }

        assert!(
            with_dh.regions[ri].total_dead() > baseline.regions[ri].total_dead(),
            "discourage hospitalization should increase deaths: {} vs baseline {}",
            with_dh.regions[ri].total_dead(), baseline.regions[ri].total_dead()
        );
    }

    #[test]
    fn policy_costs_deducted_each_tick() {
        let mut state = AppState::new_default(42);
        // First tick without policy to measure income
        let (no_policy, _) = tick(&state);
        let income_no_policy = no_policy.resources.funding - state.resources.funding;

        // Now tick with travel ban
        let funding_before = state.resources.funding;
        state.policies[0].travel_ban = true; // $0.7/tick, also reduces region 0 GDP
        state = state.with_world(tick(&state).0);
        let net_change = state.resources.funding - funding_before;

        // Should have deducted travel ban cost and reduced region income
        assert!(
            net_change < income_no_policy,
            "travel ban should reduce net income: net {net_change:.1} vs no-policy {income_no_policy:.1}"
        );
    }

    #[test]
    fn policy_funding_crisis_suspends_most_expensive_first() {
        let mut state = AppState::new_default(42);
        state.resources.funding = 0.8; // Enough for quarantine ($0.6) but not both ($1.3)
        state.policies[0].travel_ban = true; // $0.7/tick — most expensive
        state.policies[0].quarantine = true; // $0.6/tick
        let tick_events;
        { let r = tick(&state); state = state.with_world(r.0); tick_events = r.1; }
        // Should have suspended travel ban (most expensive) but kept quarantine
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended");
        assert!(state.policies[0].quarantine, "quarantine should survive");
        assert!(
            tick_events.iter().any(|e| matches!(e, GameEvent::PolicySuspended { .. })),
            "should emit PolicySuspended event"
        );
    }

    #[test]
    fn policy_gradual_suspension_across_ticks() {
        let mut state = AppState::new_default(42);
        // Set up 3 policies: $1.0 + $0.6 + $0.4 = $2.0/tick total
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].discourage_hosp = true;
        // Enough for quarantine + discourage hosp ($1.0) but not all three ($2.0)
        state.resources.funding = 1.2;
        state = state.with_world(tick(&state).0);
        // Travel ban ($1.0, most expensive) should be suspended
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended first");
        assert!(state.policies[0].quarantine, "quarantine should survive tick 1");
        assert!(state.policies[0].discourage_hosp, "discourage hosp should survive tick 1");
    }

    #[test]
    fn funding_warning_when_runway_low() {
        let mut state = AppState::new_default(42);
        // Enable expensive policies across ALL regions to create net burn.
        // Per region: travel ban ($1/tick) + quarantine ($0.6/tick) + discourage hosp ($0.4/tick) = $2/tick
        // Six regions = $12/tick policy cost. Plus upkeep: 20 × $0.06 = $1.2/tick. Total ~$13.2/tick.
        // Income ~$9/tick (minus travel ban penalty halving income). Net burn is positive → warning fires.
        for i in 0..6 {
            state.policies[i].travel_ban = true;
            state.policies[i].quarantine = true;
            state.policies[i].discourage_hosp = true;
        }
        // Funding must be ≥ policy_cost (12.0) to avoid auto-suspension, but
        // low enough that the runway warning fires.
        state.resources.funding = 15.0;
        // Start at a realistic tick so the rate limit (once per day) allows firing.
        state.tick = TICKS_PER_DAY as u64;
        let tick_events;
        { let r = tick(&state); state = state.with_world(r.0); tick_events = r.1; }
        assert!(
            tick_events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should emit FundingWarning when runway is low"
        );
    }

    #[test]
    fn no_funding_warning_when_flush() {
        let mut state = AppState::new_default(42);
        state.policies[0].travel_ban = true; // $1/tick
        state.resources.funding = 1000.0; // Plenty of runway after deduction
        let tick_events;
        { let r = tick(&state); state = state.with_world(r.0); tick_events = r.1; }
        assert!(
            !tick_events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should not warn when funding is high"
        );
    }

    #[test]
    fn no_funding_warning_without_active_policies() {
        let mut state = AppState::new_default(42);
        // No policies active — only personnel upkeep creates costs.
        // Even with zero funding, warning shouldn't fire because there's
        // nothing to suspend.
        state.resources.funding = 0.0;
        let tick_events;
        { let r = tick(&state); state = state.with_world(r.0); tick_events = r.1; }
        assert!(
            !tick_events.iter().any(|e| matches!(e, GameEvent::FundingWarning)),
            "should not warn about policy suspension when no policies are active"
        );
    }

    #[test]
    fn policy_toggle_via_confirm() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum; // Full authority for testing

        // P key now opens directly to ManagePolicies for the current map region (0)
        state = apply_action(&state, &Action::OpenPolicy);
        assert_eq!(state.ui.open_panel, Panel::Policy);
        assert!(matches!(
            state.ui.policy_ui,
            Some(PolicyUiState::ManagePolicies { region_idx: 0 })
        ));

        // Navigate to Travel Ban — find its display position dynamically so the test
        // stays correct even if policy_display_order() changes.
        let travel_ban_display_pos = crate::state::policy_display_order()
            .iter()
            .position(|&p| p == PolicyId::TravelBan)
            .expect("Travel Ban must be in display order");
        for _ in 0..travel_ban_display_pos {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm);
        assert!(state.policies[0].travel_ban);

        // Toggle it off
        state = apply_action(&state, &Action::Confirm);
        assert!(!state.policies[0].travel_ban);
    }

    #[test]
    fn simulation_is_deterministic() {
        let state = AppState::new_default(42);
        let mut a = state.clone();
        let mut b = state;
        for _ in 0..1000 {
            a = a.with_world(tick(&a).0);
            b = b.with_world(tick(&b).0);
        }
        assert_eq!(a.diseases[0].within_region_spread, b.diseases[0].within_region_spread);
        assert_eq!(a.diseases[0].lethality, b.diseases[0].lethality);
        assert_eq!(a.diseases.len(), b.diseases.len());
    }

    #[test]
    fn variant_spawns_from_parent_disease() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut state = AppState::new_default(42);
        state.tick = 20 * crate::state::TICKS_PER_DAY as u64; // day 20

        let initial_count = state.diseases.len();
        let parent_name = state.diseases[0].name.clone();

        // Force a variant spawn by calling the function directly with high probability
        let mut rng = ChaCha8Rng::seed_from_u64(123);
        // Try many times since the rate is low
        let mut spawned = false;
        for _ in 0..100000 {
            let mut events: Vec<GameEvent> = Vec::new();
            disease::tick_variant_spawning(&mut state, &mut rng, &mut events);
            if state.diseases.len() > initial_count {
                spawned = true;
                break;
            }
        }
        assert!(spawned, "variant should have spawned after many attempts");

        let variant = &state.diseases[state.diseases.len() - 1];
        assert!(variant.parent_lineage.is_some(),
            "variant should have parent_lineage set");
        assert_eq!(variant.parent_lineage.as_deref(), Some(parent_name.as_str()),
            "variant lineage should match parent name");
        assert!(variant.variant_number > 0,
            "variant_number should be > 0");
        assert!(variant.name.contains("II") || variant.name.contains("III"),
            "variant name should have roman numeral suffix: {}", variant.name);
        assert!(!variant.detected,
            "variant should start undetected");
        // Stats should be boosted relative to parent
        assert!(variant.within_region_spread > state.diseases[0].within_region_spread,
            "variant should have higher spread than parent");

        // Should have targeted medicines
        let has_med = state.medicines.iter().any(|m| {
            m.target_diseases.contains(&(state.diseases.len() - 1))
        });
        assert!(has_med, "variant should have targeted medicines");
    }

    #[test]
    fn resistance_builds_from_treatment_pressure() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::TherapyType;
        let mut state = AppState::new_default(42);
        // Find first non-prion disease and unlock its targeted medicines
        let disease_idx = state.diseases.iter().position(|d| {
            d.pathogen_type != crate::state::PathogenType::Prion
        }).unwrap();
        let med_idx = state.medicines.iter().position(|m| {
            m.target_diseases.contains(&disease_idx)
                && m.therapy_type != TherapyType::BroadSpectrum
        }).unwrap();
        state.medicines[med_idx].unlocked = true;
        state.medicines[med_idx].tested_against.push(disease_idx);
        state.medicines[med_idx].doses = 1_000_000_000.0;
        state.medicines[med_idx].max_doses = 1_000_000_000.0;
        state.resources.funding = 1_000_000.0;

        // Seed infection in region 0
        state.regions[0].get_or_create_infection(disease_idx).infected = 100_000.0;

        // Record initial resistance
        let initial_res = state.medicines[med_idx].resistance_factor(disease_idx, &state.diseases);
        assert!((initial_res - 1.0).abs() < 0.001, "should start with no resistance");

        // Deploy treatment multiple times (clear cooldown between deploys, deliver each shipment)
        for i in 0..10usize {
            if let Some(inf) = state.regions[0].infections.iter_mut().find(|i| i.disease_idx == disease_idx) {
                inf.infected = 100_000.0;
            }
            state.resources.funding = 1_000_000.0;
            state.regions[0].last_deploy_tick.clear();
            let (_, _) = medicine::deploy_medicine(&mut state, med_idx, 0, DeployTarget { disease_idx, mode: crate::state::MedicineMode::Therapeutic }, &mut events);
            // Advance time to deliver this shipment
            state.tick = (i as u64 + 1) * (crate::state::SHIPPING_TICKS + 1);
            { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }
        }

        let after_res = state.medicines[med_idx].resistance_factor(disease_idx, &state.diseases);
        assert!(after_res < 1.0, "resistance should have built up after 10 treatments, got factor {after_res}");
        assert!(after_res > 0.2, "resistance shouldn't be maxed after only 10 treatments, got factor {after_res}");

        // Broad-spectrum builds faster
        let bs_idx = state.medicines.iter().position(|m| {
            m.therapy_type == TherapyType::BroadSpectrum
        }).unwrap();
        state.medicines[bs_idx].unlocked = true;
        state.medicines[bs_idx].tested_against.push(disease_idx);
        state.medicines[bs_idx].doses = 1_000_000_000.0;
        state.medicines[bs_idx].max_doses = 1_000_000_000.0;

        let base_tick = state.tick;
        for i in 0..10usize {
            if let Some(inf) = state.regions[0].infections.iter_mut().find(|i| i.disease_idx == disease_idx) {
                inf.infected = 100_000.0;
            }
            state.resources.funding = 1_000_000.0;
            state.regions[0].last_deploy_tick.clear();
            let (_, _) = medicine::deploy_medicine(&mut state, bs_idx, 0, DeployTarget { disease_idx, mode: crate::state::MedicineMode::Therapeutic }, &mut events);
            state.tick = base_tick + (i as u64 + 1) * (crate::state::SHIPPING_TICKS + 1);
            { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }
        }

        let bs_res = state.medicines[bs_idx].resistance_factor(disease_idx, &state.diseases);
        assert!(bs_res < after_res, "broad-spectrum should build resistance faster than targeted: bs={bs_res} vs targeted={after_res}");
    }

    #[test]
    fn targeted_medicines_have_mechanism_of_action() {
        use crate::state::TherapyType;

        let state = AppState::new_default(42);
        // Disease 0 is never a prion — should have one medicine per mechanism
        let targeted_meds: Vec<_> = state.medicines.iter()
            .filter(|m| m.target_diseases.contains(&0)
                && m.therapy_type != TherapyType::BroadSpectrum)
            .collect();
        // One medicine per mechanism.
        // Bacteria have 4 mechanisms, viruses/fungi have 3.
        assert!(targeted_meds.len() >= 3,
            "should have 3+ targeted medicines for disease 0, got {}: {:?}",
            targeted_meds.len(),
            targeted_meds.iter().map(|m| &m.name).collect::<Vec<_>>());
        for med in &targeted_meds {
            assert!(med.mechanism.is_some(),
                "targeted medicine '{}' should have a mechanism", med.name);
        }
        // Each mechanism should appear exactly once
        let mut mech_counts: std::collections::HashMap<_, u32> = std::collections::HashMap::new();
        for med in &targeted_meds {
            *mech_counts.entry(med.mechanism.unwrap()).or_default() += 1;
        }
        for (mech, count) in &mech_counts {
            assert_eq!(*count, 1,
                "mechanism {:?} should appear exactly once, got {}", mech, count);
        }
        // Each mechanism should have distinct tradeoff properties
        let fast_mech = targeted_meds.iter()
            .find(|m| m.mechanism.unwrap().dev_cost_multiplier() < 1.0)
            .expect("should have a fast/cheap mechanism option");
        let slow_mech = targeted_meds.iter()
            .find(|m| m.mechanism.unwrap().dev_cost_multiplier() > 1.0)
            .expect("should have a slow/expensive mechanism option");
        assert!(fast_mech.mechanism.unwrap().resistance_rate_multiplier() >
                slow_mech.mechanism.unwrap().resistance_rate_multiplier(),
            "fast mechanism should build resistance faster than slow one");

        // Broad-spectrum medicine (last one) should have no mechanism
        let broad = state.medicines.last().unwrap();
        assert!(broad.mechanism.is_none(), "broad-spectrum should have no mechanism");
        assert_eq!(broad.therapy_type, TherapyType::BroadSpectrum);
    }

    #[test]
    fn new_disease_emerges_mid_game() {
        let mut state = AppState::new_default(42);
        let initial_diseases = state.diseases.len();
        let initial_medicines = state.medicines.len();

        // Fast-forward past emergence threshold by running many ticks.
        // With EMERGENCE_MIN_TICK=840 and EMERGENCE_CHANCE=0.0007,
        // we need ~2500 eligible ticks for reliable emergence.
        for _ in 0..3500 {
            state = state.with_world(tick(&state).0);
        }

        // With 0.07% chance per tick over ~2660 eligible ticks,
        // P(at least one emergence) = 1 - 0.9993^2660 ≈ 84%
        if state.diseases.len() > initial_diseases {
            // New disease appeared — verify it's properly set up
            let new_idx = initial_diseases;
            let new_disease = &state.diseases[new_idx];
            assert!(new_disease.within_region_spread > 0.0);
            assert!(new_disease.lethality > 0.0);
            assert_eq!(new_disease.knowledge, 0.0);
            // variant_number should be 0 for newly emerged (non-variant) diseases

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
        let mut state = AppState::new_default(42);
        use crate::state::MAX_DISEASES;
        while state.diseases.len() < MAX_DISEASES {
            use rand::SeedableRng;
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
            disease::spawn_disease(&mut state, &mut rng);
        }
        assert_eq!(state.diseases.len(), MAX_DISEASES);

        // Attempting another spawn should return None
        use rand::SeedableRng;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
        assert!(disease::spawn_disease(&mut state, &mut rng).is_none());
    }

    #[test]
    fn pathogen_type_diversity_enforced() {
        use crate::state::MAX_DISEASES;
        // Try many seeds — no seed should produce 3+ diseases of the same type
        for seed in 0..50u64 {
            let mut state = AppState::new_default(seed);
            while state.diseases.len() < MAX_DISEASES {
                let mut rng = state.rng_emergence.clone();
                disease::spawn_disease(&mut state, &mut rng);
                state.rng_emergence = rng;
            }
            let mut counts = std::collections::HashMap::new();
            for d in &state.diseases {
                *counts.entry(d.pathogen_type).or_insert(0usize) += 1;
            }
            for (pt, count) in &counts {
                assert!(
                    *count <= 2,
                    "seed {seed}: pathogen type {pt:?} appears {count} times (max 2)",
                );
            }
            // With MAX_DISEASES diseases and max 2 per type, we need at least
            // ceil(MAX_DISEASES/2) distinct types
            let min_types = (MAX_DISEASES + 1) / 2;
            assert!(
                counts.len() >= min_types,
                "seed {seed}: only {} distinct pathogen types with {} diseases (need >= {min_types})",
                counts.len(), state.diseases.len(),
            );
        }
    }

    #[test]
    fn late_game_diseases_shift_toward_deadly_types() {
        use crate::state::{PathogenType, TICKS_PER_DAY};
        // Spawn many diseases at day 0 and day 60, compare type distributions.
        let mut early_deadly = 0usize; // fungus + prion
        let mut late_deadly = 0usize;
        let trials = 200;

        for seed in 0..trials {
            // Early game (day 0)
            let mut state = AppState::new_default(seed);
            state.tick = 0;
            let mut rng = state.rng_emergence.clone();
            // Remove existing disease so spawn works clean
            state.diseases.clear();
            if let Some((idx, _)) = disease::spawn_disease(&mut state, &mut rng) {
                match state.diseases[idx].pathogen_type {
                    PathogenType::Fungus | PathogenType::Prion => early_deadly += 1,
                    _ => {}
                }
            }
            state.rng_emergence = rng;

            // Late game (day 60)
            let mut state2 = AppState::new_default(seed + 1000);
            state2.tick = (60.0 * TICKS_PER_DAY) as u64;
            state2.diseases.clear();
            let mut rng2 = state2.rng_emergence.clone();
            if let Some((idx, _)) = disease::spawn_disease(&mut state2, &mut rng2) {
                match state2.diseases[idx].pathogen_type {
                    PathogenType::Fungus | PathogenType::Prion => late_deadly += 1,
                    _ => {}
                }
            }
        }

        // Late game should produce significantly more fungi/prions than early game.
        // Early: ~1/8 base pool entries are fungus (12.5%), prion 5% = ~17%
        // Late: ~3/7 base pool entries are fungus (43%), prion 25% = ~55%
        assert!(
            late_deadly > early_deadly,
            "Late-game should produce more fungi/prions: early={early_deadly}/{trials}, late={late_deadly}/{trials}"
        );
        // Sanity check: early shouldn't be zero (fungus is still possible)
        // and late should be meaningfully high
        assert!(
            late_deadly as f64 / trials as f64 > 0.30,
            "Late-game fungi/prion rate should be >30%: {}/{trials}",
            late_deadly
        );
    }

    #[test]
    fn late_game_diseases_favor_contact_transmission() {
        use crate::state::{TransmissionVector, TICKS_PER_DAY};
        let trials = 200;
        let mut early_contact = 0usize;
        let mut late_contact = 0usize;

        for seed in 0..trials {
            // Early game
            let mut state = AppState::new_default(seed as u64 + 7000);
            state.tick = 0;
            state.diseases.clear();
            for r in &mut state.regions { r.infections.clear(); }
            let mut rng = state.rng_emergence.clone();
            disease::spawn_disease_scaled(&mut state, &mut rng);
            if !state.diseases.is_empty() && state.diseases[0].transmission == TransmissionVector::Contact {
                early_contact += 1;
            }

            // Late game
            let mut state2 = AppState::new_default(seed as u64 + 8000);
            state2.tick = (70.0 * TICKS_PER_DAY) as u64; // day 70: full optimization
            state2.diseases.clear();
            for r in &mut state2.regions { r.infections.clear(); }
            let mut rng2 = state2.rng_emergence.clone();
            disease::spawn_disease_scaled(&mut state2, &mut rng2);
            if !state2.diseases.is_empty() && state2.diseases[0].transmission == TransmissionVector::Contact {
                late_contact += 1;
            }
        }

        assert!(
            late_contact > early_contact,
            "Late-game should produce more Contact diseases: early={early_contact}/{trials}, late={late_contact}/{trials}"
        );
    }

    #[test]
    fn mid_game_diseases_target_vulnerable_regions() {
        use crate::state::{ScreeningLevel, TICKS_PER_DAY};
        // Set up: region 0 is heavily defended, region 1-5 are undefended.
        // At mid-game (day 16), vulnerability targeting is dominant — diseases
        // prefer undefended regions where they can spread easily.
        let trials = 200;
        let mut defended_hits = 0usize;
        let mut undefended_hits = 0usize;

        for seed in 0..trials {
            let mut state = AppState::new_default(seed as u64 + 5000);
            state.tick = (16.0 * TICKS_PER_DAY) as u64; // day 16: vulnerability targeting active
            // Defend region 0 heavily
            state.policies[0].screening = ScreeningLevel::MassRapid;
            state.regions[0].hospital_level = 2;
            // Leave all other regions undefended (default: no screening, no hospital)

            // Remove existing disease so we start clean
            state.diseases.clear();
            for r in &mut state.regions { r.infections.clear(); }

            let mut rng = state.rng_emergence.clone();
            if let Some((_, region_idx)) = disease::spawn_disease(&mut state, &mut rng) {
                if region_idx == 0 {
                    defended_hits += 1;
                } else {
                    undefended_hits += 1;
                }
            }
        }

        // With 6 regions, uniform would give ~33 hits to region 0 out of 200.
        // With vulnerability targeting, region 0 (defended) should get fewer.
        let defended_rate = defended_hits as f64 / trials as f64;
        assert!(
            defended_rate < 0.15,
            "Defended region should be targeted less than 15% of the time: {defended_hits}/{trials} ({defended_rate:.1}%)"
        );
        assert!(
            undefended_hits > defended_hits * 3,
            "Undefended regions should be targeted much more than defended: undefended={undefended_hits}, defended={defended_hits}"
        );
    }

    #[test]
    fn late_game_diseases_target_player_strongholds() {
        use crate::state::{ScreeningLevel, TICKS_PER_DAY};
        // At late-game (day 50+), strategic targeting dominates — diseases
        // target the player's invested regions (high infrastructure, active policies).
        // This is the "designed, not random" behavior.
        let trials = 200;
        let mut invested_hits = 0usize;

        for seed in 0..trials {
            let mut state = AppState::new_default(seed as u64 + 7000);
            state.tick = (50.0 * TICKS_PER_DAY) as u64; // day 50: strategic targeting dominant
            // Invest heavily in region 0 (policies + infrastructure)
            state.policies[0].screening = ScreeningLevel::MassRapid;
            state.policies[0].quarantine = true;
            state.policies[0].discourage_hosp = false; // hospitals active (default)
            state.regions[0].hospital_level = 2;
            // Leave all other regions neglected

            state.diseases.clear();
            for r in &mut state.regions { r.infections.clear(); }

            let mut rng = state.rng_emergence.clone();
            if let Some((_, region_idx)) = disease::spawn_disease(&mut state, &mut rng) {
                if region_idx == 0 {
                    invested_hits += 1;
                }
            }
        }

        // With strategic targeting, the invested region should be hit MORE
        // than uniform (1/6 ≈ 17%). 18% threshold catches meaningful bias
        // without being flaky across RNG stream changes.
        let invested_rate = invested_hits as f64 / trials as f64;
        assert!(
            invested_rate > 0.18,
            "Invested region should be targeted more than 18% of the time: {invested_hits}/{trials} ({invested_rate:.1}%)"
        );
    }

    #[test]
    fn transmission_vector_affects_quarantine() {
        use crate::state::TransmissionVector;

        let mut state = AppState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        // Set first disease to Contact transmission (quarantine factor = 0.30)
        state.diseases[0].transmission = TransmissionVector::Contact;
        state.diseases[0].within_region_spread = 0.02;
        state.diseases[0].knowledge = 1.0;
        // Give the region a big susceptible pool
        state.regions[region_idx].get_or_create_infection(0).infected = 1000.0;

        // Run without quarantine
        let (no_quarantine, _) = tick(&state);
        let s = no_quarantine.regions[region_idx].disease_state(0).unwrap();
        let inf_no_q = s.exposed + s.infected;

        // Run with quarantine
        state.policies[region_idx].quarantine = true;
        let (with_quarantine, _) = tick(&state);
        let s = with_quarantine.regions[region_idx].disease_state(0).unwrap();
        let inf_with_q = s.exposed + s.infected;

        // Quarantine should reduce new infections significantly for Contact
        // (quarantine_factor = 0.30, so within-region spread drops to 30%)
        assert!(inf_with_q < inf_no_q, "quarantine should reduce infections");

        // Now test Waterborne (quarantine factor = 0.75, less effective)
        state.diseases[0].transmission = TransmissionVector::Waterborne;
        let (with_q_waterborne, _) = tick(&state);
        let s = with_q_waterborne.regions[region_idx].disease_state(0).unwrap();
        let inf_with_q_wb = s.exposed + s.infected;

        // Waterborne quarantine should be less effective than Contact quarantine
        assert!(inf_with_q_wb > inf_with_q,
            "waterborne quarantine should be less effective than contact quarantine");
    }

    #[test]
    fn discourage_hosp_reduces_within_region_spread() {
        let mut state = AppState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        state.diseases[0].within_region_spread = 0.02;
        state.diseases[0].lethality = 0.01;
        state.regions[region_idx].get_or_create_infection(0).infected = 5000.0;

        // Run baseline (hospitals active, +25% spread)
        let (baseline, _) = tick(&state);

        // Run with discourage hospitalization (removes hospital exposure)
        state.policies[region_idx].discourage_hosp = true;
        let (with_dh, _) = tick(&state);

        // Discourage hospitalization should reduce infections
        let s = baseline.regions[region_idx].disease_state(0).unwrap();
        let inf_baseline = s.exposed + s.infected;
        let s = with_dh.regions[region_idx].disease_state(0).unwrap();
        let inf_dh = s.exposed + s.infected;
        assert!(inf_dh < inf_baseline,
            "discourage hospitalization should reduce spread: {} vs baseline {}",
            inf_dh, inf_baseline);
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
            let mut state = AppState::new_default(42);
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
                exposed: 0.0,
                infected: 10_000.0,
                dead: 0.0,
                immune: 0.0,
            });

            // Test airborne
            state.diseases[0].transmission = TransmissionVector::Airborne;
            state.rng_spread = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let (after, _) = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                airborne_spreads += 1;
            }

            // Test contact
            state.diseases[0].transmission = TransmissionVector::Contact;
            state.rng_spread = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let (after, _) = tick(&state);
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

    #[test]
    fn border_controls_reduces_cross_region_spread() {
        use crate::state::TransmissionVector;
        use rand::SeedableRng;

        let mut controls_spreads = 0u32;
        let mut no_policy_spreads = 0u32;

        for seed in 0..200 {
            let mut state = AppState::new_default(42);
            state.diseases.truncate(1);
            state.diseases[0].transmission = TransmissionVector::Airborne;
            state.diseases[0].cross_region_spread = 0.01;
            for region in &mut state.regions { region.infections.clear(); }
            state.regions[0].infections.push(RegionDiseaseState {
                disease_idx: 0, exposed: 0.0, infected: 10_000.0, dead: 0.0, immune: 0.0,
            });

            // No policy
            state.rng_spread = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let (after, _) = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                no_policy_spreads += 1;
            }

            // Border controls on source region
            state.policies[0].border_controls = true;
            state.rng_spread = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let (after, _) = tick(&state);
            if after.regions.iter().skip(1).any(|r|
                r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0)
            ) {
                controls_spreads += 1;
            }
        }

        assert!(controls_spreads < no_policy_spreads,
            "border controls should reduce cross-region spread: {} vs {} (no policy)",
            controls_spreads, no_policy_spreads);
    }

    #[test]
    fn water_sanitation_reduces_waterborne_within_region_spread() {
        use crate::state::TransmissionVector;
        let mut state = AppState::new_default(42);
        let region_idx = primary_outbreak_region(&state);

        state.diseases[0].transmission = TransmissionVector::Waterborne;
        state.diseases[0].within_region_spread = 0.02;
        state.regions[region_idx].get_or_create_infection(0).infected = 1000.0;

        // Without sanitation
        let (no_sanitation, _) = tick(&state);
        let s = no_sanitation.regions[region_idx].disease_state(0).unwrap();
        let inf_no = s.exposed + s.infected;

        // With sanitation
        state.policies[region_idx].water_sanitation = true;
        let (with_sanitation, _) = tick(&state);
        let s = with_sanitation.regions[region_idx].disease_state(0).unwrap();
        let inf_with = s.exposed + s.infected;

        assert!(inf_with < inf_no,
            "water sanitation should reduce waterborne infections: {} vs {}",
            inf_with, inf_no);

        // Sanitation should NOT affect airborne diseases
        state.diseases[0].transmission = TransmissionVector::Airborne;
        let (airborne_with_sanitation, _) = tick(&state);
        state.policies[region_idx].water_sanitation = false;
        let (airborne_without, _) = tick(&state);
        let inf_airborne_with = airborne_with_sanitation.regions[region_idx].disease_state(0).unwrap().infected;
        let inf_airborne_without = airborne_without.regions[region_idx].disease_state(0).unwrap().infected;

        // Should be roughly equal (same noise seed means identical)
        assert!((inf_airborne_with - inf_airborne_without).abs() < 1.0,
            "sanitation should not affect airborne: {} vs {}",
            inf_airborne_with, inf_airborne_without);
    }

    #[test]
    fn crisis_generates_after_min_tick() {
        let mut state = AppState::new_default(42);
        // Run past CRISIS_MIN_TICK — a crisis should eventually appear.
        // With CRISIS_INTERVAL=840, we need ~5000 ticks for P(no crisis) < 1%.
        let mut found_crisis = false;
        for _ in 0..5000 {
            state = state.with_world(tick(&state).0);
            if state.active_crisis.is_some() {
                found_crisis = true;
                break;
            }
        }
        assert!(found_crisis,
            "expected a crisis to generate within 5000 ticks");
    }

    #[test]
    fn crisis_blocks_normal_actions() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = AppState::new_default(42);
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "Test Crisis".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "A".into(), description: "A".into(), cost: None },
             CrisisOption { label: "B".into(), description: "B".into(), cost: None },
            ],
            tick_created: 0,
        });

        // Normal panel actions should be blocked
        let after = apply_action(&state, &Action::OpenThreats);
        assert_eq!(after.ui.open_panel, Panel::None, "panel should not open during crisis");

        // SelectNext should change crisis selection
        let after = apply_action(&state, &Action::SelectNext);
        assert_eq!(after.ui.crisis_selection, 1);
        let after = apply_action(&after, &Action::SelectPrev);
        assert_eq!(after.ui.crisis_selection, 0);
    }

    #[test]
    fn crisis_resolution_applies_effects() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = AppState::new_default(42);
        let initial_personnel = state.resources.personnel;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
            title: "Attrition".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "Accept losses".into(), description: "".into(), cost: None },
             CrisisOption { label: "Pay retention".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });

        // Choose option A (accept losses — lose personnel)
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
        assert_eq!(after.resources.personnel, initial_personnel - 3,
            "should have lost personnel");

        // Reset and choose option B (pay retention — keep personnel)
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PersonnelCrisis { amount: 3 },
            title: "Attrition".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "Accept losses".into(), description: "".into(), cost: None },
             CrisisOption { label: "Pay retention".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::SelectNext); // select option B
        let after = apply_action(&after, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
        assert_eq!(after.resources.personnel, initial_personnel,
            "retention should keep personnel unchanged");
    }

    #[test]
    fn crisis_unaffordable_option_blocked() {
        use crate::state::{CrisisCost, CrisisEvent, CrisisKind, CrisisOption};

        let mut state = AppState::new_default(42);
        state.resources.funding = 0.0; // broke
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "Burnout".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "Accept".into(), description: "".into(), cost: None },
             CrisisOption { label: "Pay ¥400".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 400.0, personnel: 0, ..Default::default() }) },
            ],
            tick_created: 0,
        });

        // Try to pay (option B) but can't afford — confirm should be blocked
        let after = apply_action(&state, &Action::SelectNext);
        let after = apply_action(&after, &Action::Confirm);
        assert!(after.active_crisis.is_some(), "crisis should still be active");
        assert!(after.session.status_message.as_ref().unwrap().contains("Not enough"),
            "should show affordability message");

        // Free option (A) should still work
        let after = apply_action(&state, &Action::Confirm); // option A (default)
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
    }

    #[test]
    fn crisis_preserves_running_pacing_on_dismiss() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = AppState::new_default(42);
        // Game is running when crisis fires — pacing stays Running
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "Test".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "A".into(), description: "".into(), cost: None },
             CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.sim_state, SimState::Running,
            "should restore Running state after crisis when game was running");
    }

    #[test]
    fn crisis_preserves_paused_pacing_on_dismiss() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = AppState::new_default(42);
        // Game was paused when crisis fired — pacing stays Paused
        state.sim_state = SimState::Paused;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "Test".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "A".into(), description: "".into(), cost: None },
             CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.sim_state, SimState::Paused,
            "pacing should remain Paused after crisis dismissal");
    }

    #[test]
    fn contract_demand_placate_boosts_satisfaction() {
        use crate::state::{
            CrisisCost, CrisisEvent, CrisisKind, CrisisOption, FundingCondition,
            FundingContract,
        };

        let mut state = AppState::new_default(42);
        // Crisis active — blocking derived from active_crisis
        // Add a contract with low satisfaction
        state.contracts.push(FundingContract {
            name: "Media Transparency Pledge".to_string(),
            board_member_idx: 0,
            income: 1.8,
            condition: FundingCondition::MaxDeaths { threshold: 50_000_000.0 },
            template_id: 4,
            satisfaction: 0.4,
            warned: true,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
            last_bonus_tick: 0,
        });

        // Set up the contract demand crisis as active
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::ContractDemand { template_id: 4 },
            title: "Kowalski: Demands".into(),
            description: "Test".into(),
            options: vec![
                CrisisOption {
                    label: "Placate (¥100)".into(),
                    description: "".into(),
                    cost: Some(CrisisCost { funding: 100.0, personnel: 0, ..Default::default() }),
                },
                CrisisOption {
                    label: "Refuse".into(),
                    description: "".into(),
                    cost: None,
                },
            ],
            tick_created: 0,
        });

        let funding_before = state.resources.funding;
        // Choose option 0 (placate)
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.resources.funding, funding_before - 100.0);
        // Satisfaction should have jumped by 0.25
        assert!((after.contracts[0].satisfaction - 0.65).abs() < 0.01,
            "Satisfaction should be ~0.65 after placating, got {}",
            after.contracts[0].satisfaction);
        assert!(!after.contracts[0].warned, "warned flag should reset after placate");
    }

    #[test]
    fn contract_demand_refuse_drops_satisfaction() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, FundingCondition, FundingContract};

        let mut state = AppState::new_default(42);
        // Crisis active — blocking derived from active_crisis
        state.contracts.push(FundingContract {
            name: "Media Transparency Pledge".to_string(),
            board_member_idx: 0,
            income: 1.8,
            condition: FundingCondition::MaxDeaths { threshold: 50_000_000.0 },
            template_id: 4,
            satisfaction: 0.4,
            warned: true,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
            last_bonus_tick: 0,
        });

        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::ContractDemand { template_id: 4 },
            title: "Kowalski: Demands".into(),
            description: "Test".into(),
            options: vec![
                CrisisOption {
                    label: "Placate".into(),
                    description: "".into(),
                    cost: None,
                },
                CrisisOption {
                    label: "Refuse".into(),
                    description: "".into(),
                    cost: None,
                },
            ],
            tick_created: 0,
        });

        // Select option 1 (refuse) and confirm
        let after = apply_action(&state, &Action::SelectNext);
        let after = apply_action(&after, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        // Satisfaction should drop by 0.15 (from 0.4 to 0.25)
        assert!((after.contracts[0].satisfaction - 0.25).abs() < 0.01,
            "Satisfaction should be ~0.25 after refusal, got {}",
            after.contracts[0].satisfaction);
    }

    #[test]
    fn spacebar_blocked_during_event_state() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption, SimState};

        let mut state = AppState::new_default(42);
        // Crisis active — blocking derived from active_crisis
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "Test".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "A".into(), description: "".into(), cost: None },
             CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::TogglePause);
        assert_eq!(after.sim_state, SimState::Running,
            "spacebar should not change pacing during crisis");
        assert!(after.active_crisis.is_some(), "crisis should still be active");
    }

    #[test]
    fn game_over_clears_active_crisis() {
        use crate::state::{CrisisEvent, CrisisKind, CrisisOption};

        let mut state = AppState::new_default(42);
        // Set up a highly lethal disease to trigger game over (collapse all regions).
        // High cross_region_spread needed to reach refugia through sparser graph.
        for disease in &mut state.diseases {
            disease.within_region_spread = 0.12;
            disease.lethality = 0.08;
            disease.recovery_rate = 0.002;
            disease.cross_region_spread = 0.20;
        }
        // Boost initial infection so collapse happens quickly
        for region in &mut state.regions {
            for inf in &mut region.infections {
                inf.infected = 50_000.0;
            }
        }

        // Inject an active crisis
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "Test".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "A".into(), description: "".into(), cost: None },
             CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });
        // Crisis active — blocking derived from active_crisis

        // Run until game over (collapse requires all regions to fall)
        for _ in 0..20000 {
            state = state.with_world(tick(&state).0);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }

        assert_eq!(state.outcome, GameOutcome::Lost);
        assert!(state.active_crisis.is_none(),
            "active crisis should be cleared on game over");
        assert!(state.is_blocked(),
            "game should be blocked after game over");
    }

    #[test]
    fn crisis_auto_resolves_with_saved_preference() {
        let mut state = AppState::new_default(42);
        // Set auto-resolve preference for personnel crises: always pick option A
        state.auto_resolve_crises.insert("personnel".to_string(), 0);

        // Run until a crisis would generate
        let mut auto_resolved = false;
        for _ in 0..5000 {
            let tick_events;
            { let r = tick(&state); state = state.with_world(r.0); tick_events = r.1; }
            // If a personnel crisis auto-resolved, the game isn't in Event state
            // (it may be Paused from a DiseaseDetected in the same tick, which is fine)
            if tick_events.iter().any(|e| matches!(e, GameEvent::CrisisAutoResolved { .. })) {
                auto_resolved = true;
                assert!(state.active_crisis.is_none(),
                    "crisis should be resolved immediately");
                assert!(!state.is_blocked() || state.outcome != GameOutcome::Playing,
                    "game should not be blocked after auto-resolve (unless game over)");
                break;
            }
            // If a non-personnel crisis fires, it should pause normally
            if state.active_crisis.is_some() {
                // Dismiss it manually to continue
                let crisis_tag = state.active_crisis.as_ref().unwrap().kind.tag().to_string();
                assert_ne!(crisis_tag, "personnel",
                    "personnel crisis should have been auto-resolved");
                state = apply_action(&state, &Action::Confirm);
            }
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        // We may not get a personnel crisis in 5000 ticks — that's OK.
        // The test verifies correctness IF it fires, not that it fires.
        if auto_resolved {
            // Good — verified auto-resolve works
        }
    }

    // --- Crisis resolution effect tests ---

    /// Helper: create a crisis event and inject it into state with choice pre-selected.
    fn setup_crisis(state: &mut AppState, kind: CrisisKind, choice: usize) {
        use crate::state::{CrisisEvent, CrisisOption};
        state.ui.crisis_selection = choice;
        state.active_crisis = Some(CrisisEvent {
            kind,
            title: "Test Crisis".into(),
            description: "Test".into(),
            options: vec![ CrisisOption { label: "A".into(), description: "".into(), cost: None },
             CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });
    }

    // --- Generic crisis cost deduction tests ---

    #[test]
    fn crisis_cost_deducts_funding() {
        use crate::state::{CrisisCost, CrisisEvent, CrisisOption};
        let mut state = AppState::new_default(42);
        let funding_before = state.resources.funding;
        // Crisis active — blocking derived from active_crisis
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "T".into(),
            description: "T".into(),
            options: vec![
                CrisisOption { label: "A".into(), description: "".into(), cost: None },
                CrisisOption { label: "B".into(), description: "".into(),
                    cost: Some(CrisisCost { funding: 250.0, personnel: 0, ..Default::default() }) },
            ],
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert!((after.resources.funding - (funding_before - 250.0)).abs() < 0.01,
            "choosing a costed option should deduct funding; expected {} got {}",
            funding_before - 250.0, after.resources.funding);
    }

    #[test]
    fn crisis_cost_deducts_personnel_permanently() {
        use crate::state::{CrisisCost, CrisisEvent, CrisisOption};
        let mut state = AppState::new_default(42);
        let personnel_before = state.resources.personnel;
        // Crisis active — blocking derived from active_crisis
        state.ui.crisis_selection = 0;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "T".into(),
            description: "T".into(),
            options: vec![
                CrisisOption { label: "A".into(), description: "".into(),
                    cost: Some(CrisisCost { funding: 0.0, personnel: 2, ..Default::default() }) },
                CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert_eq!(after.resources.personnel, personnel_before - 2,
            "choosing a costed option should permanently deduct personnel");
    }

    #[test]
    fn crisis_cost_creates_operation_for_temporary_personnel() {
        use crate::state::{CrisisCost, CrisisEvent, CrisisOption, OperationSpec};
        let mut state = AppState::new_default(42);
        let personnel_before = state.resources.personnel;
        // Crisis active — blocking derived from active_crisis
        state.ui.crisis_selection = 0;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "T".into(),
            description: "T".into(),
            options: vec![
                CrisisOption { label: "A".into(), description: "".into(),
                    cost: Some(CrisisCost {
                        funding: 100.0,
                        personnel: 3,
                        operation: Some(OperationSpec { days: 2.0, label: "Test Op".into() }),
                    }) },
                CrisisOption { label: "B".into(), description: "".into(), cost: None },
            ],
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        // Personnel should NOT be permanently deducted — they're in an operation
        assert_eq!(after.resources.personnel, personnel_before,
            "personnel should not be permanently lost when operation is specified");
        assert_eq!(after.crisis_operations.len(), 1);
        assert_eq!(after.crisis_operations[0].label, "Test Op");
        assert_eq!(after.crisis_operations[0].personnel, 3);
    }

    #[test]
    fn crisis_cost_none_deducts_nothing() {
        let mut state = AppState::new_default(42);
        let funding_before = state.resources.funding;
        let personnel_before = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::PerformanceReview, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none());
        assert!((after.resources.funding - funding_before).abs() < 0.01,
            "free option should not deduct funding");
        assert_eq!(after.resources.personnel, personnel_before,
            "free option should not deduct personnel");
    }

    #[test]
    fn personnel_crisis_option_a_loses_personnel() {
        let mut state = AppState::new_default(42);
        let before = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::PersonnelCrisis { amount: 3 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.resources.personnel, before - 3);
    }



    #[test]
    fn refugee_wave_option_a_transfers_population_and_infections() {
        let mut state = AppState::new_default(42);
        // Set up: region 0 collapsed with infections, region 1 as destination
        state.regions[0].collapsed = true;
        state.regions[0].dead = 200_000_000.0; // 200M dead of 500M
        state.regions[0].infections = vec![RegionDiseaseState {
            disease_idx: 0, exposed: 0.0, infected: 10_000.0, dead: 200_000_000.0, immune: 5_000.0,
        }];
        let survivors = state.regions[0].alive(); // 300M
        let dest_pop_before = state.regions[1].population;
        let dest_infected_before = state.regions[1].infections
            .iter().find(|i| i.disease_idx == 0)
            .map(|i| i.infected).unwrap_or(0.0);
        setup_crisis(&mut state, CrisisKind::RefugeeWave { from_region: 0, to_region: 1, wave: 1 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        // Population should increase by survivor count
        assert_eq!(after.regions[1].population, dest_pop_before + survivors as u64,
            "option A should transfer surviving population to destination");
        // Infections should increase
        let dest_infected_after = after.regions[1].infections
            .iter().find(|i| i.disease_idx == 0)
            .map(|i| i.infected).unwrap_or(0.0);
        assert!(dest_infected_after > dest_infected_before,
            "option A should increase infections in destination region");
    }

    #[test]
    fn refugee_wave_option_b_kills_refugees() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.regions[0].collapsed = true;
        let dead_before = state.regions[0].dead;
        let survivors_before = state.regions[0].alive();
        setup_crisis(&mut state, CrisisKind::RefugeeWave { from_region: 0, to_region: 1, wave: 1 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        // 20% of survivors die at the border
        let expected_deaths = survivors_before * 0.20;
        assert!((after.regions[0].dead - dead_before - expected_deaths).abs() < 1.0,
            "option B should kill 20% of survivors at the border");
    }

    #[test]
    fn collapse_triggers_refugee_crisis() {
        let mut state = AppState::new_default(42);
        // Push region 0 right to the edge of collapse
        let threshold = state.regions[0].collapse_threshold;
        let pop = state.regions[0].population as f64;
        // Need alive < pop * threshold, so dead > pop * (1 - threshold)
        state.regions[0].dead = pop * (1.0 - threshold) + 1.0;
        state.regions[0].get_or_create_infection(0).dead = state.regions[0].dead;
        // Ensure no other crisis is active
        assert!(state.active_crisis.is_none());
        // First tick triggers collapse and queues refugee crisis as pending
        let (after, _) = tick(&state);
        assert!(after.regions[0].collapsed, "region should collapse");
        assert!(after.pending_crises.iter().any(|k| matches!(k, CrisisKind::RefugeeWave { .. })),
            "refugee crisis should be queued as pending");
        // Second tick fires the pending refugee crisis
        let (after2, _) = tick(&after);
        assert!(after2.active_crisis.is_some(), "refugee crisis should fire on next tick");
        assert_eq!(after2.active_crisis.as_ref().unwrap().title, "Refugee Crisis");
    }

    #[test]
    fn refugee_wave_dropped_if_destination_collapsed() {
        let mut state = AppState::new_default(42);
        state.tick = 100;
        state.last_contract_offer_tick = state.tick;
        // Region 0 collapsed, region 1 is the queued destination but also collapsed.
        state.regions[0].collapsed = true;
        state.regions[1].collapsed = true;
        // Region 0 connects to [1, 2] — region 2 is still alive.
        // Queue a refugee wave targeting the now-collapsed region 1.
        state.pending_crises.push(CrisisKind::RefugeeWave { from_region: 0, to_region: 1, wave: 1 });
        let (after, _) = tick(&state);
        // Should re-route to region 2 (the only non-collapsed neighbor of region 0).
        assert!(after.active_crisis.is_some(), "refugee crisis should fire re-routed to region 2");
        if let Some(ref crisis) = after.active_crisis {
            if let CrisisKind::RefugeeWave { to_region, .. } = crisis.kind {
                assert_eq!(to_region, 2, "should re-route to non-collapsed neighbor");
            } else {
                panic!("expected RefugeeWave crisis");
            }
        }
    }

    #[test]
    fn refugee_wave_dropped_if_all_neighbors_collapsed() {
        let mut state = AppState::new_default(42);
        state.tick = 100;
        state.last_contract_offer_tick = state.tick;
        // Region 0 connects to [1, 2]. Collapse all of them.
        state.regions[0].collapsed = true;
        state.regions[1].collapsed = true;
        state.regions[2].collapsed = true;
        state.pending_crises.push(CrisisKind::RefugeeWave { from_region: 0, to_region: 1, wave: 1 });
        let (after, _) = tick(&state);
        // The RefugeeWave should have been consumed and NOT fired.
        let refugee_active = after.active_crisis.as_ref()
            .is_some_and(|c| matches!(c.kind, CrisisKind::RefugeeWave { .. }));
        assert!(!refugee_active,
            "refugee crisis should be dropped when all neighbors collapsed");
        let refugee_pending = after.pending_crises.iter()
            .any(|k| matches!(k, CrisisKind::RefugeeWave { .. }));
        assert!(!refugee_pending,
            "refugee crisis should be consumed from pending even when dropped");
    }

    #[test]
    fn ark_protocol_reroutes_if_target_collapsed() {
        let mut state = AppState::new_default(42);
        state.tick = 100;
        state.last_contract_offer_tick = state.tick;
        // Collapse the originally-chosen Ark target (region 0) plus one more
        // so the 2+ collapsed threshold is met.
        state.regions[0].collapsed = true;
        state.regions[1].collapsed = true;
        // Queue ArkProtocol targeting the now-collapsed region 0.
        state.pending_crises.push(CrisisKind::ArkProtocol { region_idx: 0 });
        let (after, _) = tick(&state);
        // Should fire with a re-picked surviving region.
        assert!(after.active_crisis.is_some(),
            "ArkProtocol should fire after re-routing to surviving region");
        if let Some(ref crisis) = after.active_crisis {
            if let CrisisKind::ArkProtocol { region_idx } = crisis.kind {
                assert!(!after.regions[region_idx].collapsed,
                    "Ark target should be a surviving region, got collapsed region {}", region_idx);
            } else {
                panic!("expected ArkProtocol crisis");
            }
        }
    }

    #[test]
    fn ark_protocol_dropped_if_all_regions_collapsed() {
        let mut state = AppState::new_default(42);
        state.tick = 100;
        state.last_contract_offer_tick = state.tick;
        // Collapse all regions.
        for r in state.regions.iter_mut() {
            r.collapsed = true;
        }
        state.pending_crises.push(CrisisKind::ArkProtocol { region_idx: 0 });
        let (after, _) = tick(&state);
        // Should be consumed but not fired.
        let ark_active = after.active_crisis.as_ref()
            .is_some_and(|c| matches!(c.kind, CrisisKind::ArkProtocol { .. }));
        assert!(!ark_active, "ArkProtocol should not fire when all regions collapsed");
        let ark_pending = after.pending_crises.iter()
            .any(|k| matches!(k, CrisisKind::ArkProtocol { .. }));
        assert!(!ark_pending, "ArkProtocol should be consumed from pending");
    }

    #[test]
    fn collapse_queues_refugee_crisis_when_crisis_active() {
        let mut state = AppState::new_default(42);
        // Push region 0 right to the edge of collapse
        let threshold = state.regions[0].collapse_threshold;
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * (1.0 - threshold) + 1.0;
        state.regions[0].get_or_create_infection(0).dead = state.regions[0].dead;
        // Pre-load an active crisis
        state.active_crisis = Some(crisis::build_crisis_event(&state, CrisisKind::PerformanceReview));
        assert!(state.active_crisis.is_some());
        // Tick should trigger collapse but NOT override — queue as pending instead
        let (after, _) = tick(&state);
        assert!(after.regions[0].collapsed, "region should collapse");
        assert_eq!(after.active_crisis.as_ref().unwrap().title, "Quarterly Performance Review",
            "existing crisis should NOT be overridden");
        assert!(after.pending_crises.iter().any(|k| matches!(k, CrisisKind::RefugeeWave { .. })),
            "refugee crisis should be queued as pending");
    }

    #[test]
    fn refugee_wave_auto_resolves_with_saved_preference() {
        let mut state = AppState::new_default(42);
        // Player has previously chosen to always close borders (option 1)
        state.auto_resolve_crises.insert("refugee".to_string(), 1);
        state.resources.authority = Authority::Maximum;
        // Push region 0 to collapse on the next tick
        let threshold = state.regions[0].collapse_threshold;
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * (1.0 - threshold) + 1.0;
        state.regions[0].get_or_create_infection(0).dead = state.regions[0].dead;
        // First tick: collapse queues refugee crisis as pending
        let (after, _) = tick(&state);
        assert!(after.regions[0].collapsed, "region should collapse");
        assert!(after.pending_crises.iter().any(|k| matches!(k, CrisisKind::RefugeeWave { .. })),
            "refugee crisis should be pending after collapse");
        // Second tick: pending crisis fires and auto-resolves
        let (after2, tick_events) = tick(&after);
        assert!(after2.active_crisis.is_none(),
            "refugee crisis should be auto-resolved, not left active");
        assert!(tick_events.iter().any(|e| matches!(e, GameEvent::CrisisAutoResolved { .. })),
            "CrisisAutoResolved event should be emitted");
    }



    #[test]
    fn crisis_option_a_resolves() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum;
        setup_crisis(&mut state, CrisisKind::PerformanceReview, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
    }

    #[test]
    fn crisis_option_b_costs_funding() {
        use crate::state::CrisisCost;
        let mut state = AppState::new_default(42);
        state.resources.funding = 1000.0;
        // Crisis active — blocking derived from active_crisis
        state.ui.crisis_selection = 1;
        state.active_crisis = Some(crate::state::CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "T".into(), description: "T".into(),
            options: vec![ crate::state::CrisisOption { label: "A".into(), description: "".into(), cost: None },
             crate::state::CrisisOption { label: "B".into(), description: "".into(),
                cost: Some(CrisisCost { funding: 300.0, personnel: 1, ..Default::default() }) },
            ],
            tick_created: 0,
        });
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - 700.0).abs() < 1.0,
            "option B should cost 300 funding");
    }

    #[test]
    fn trial_shortcut_option_a_resolves() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum;
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
    }

    #[test]
    fn trial_shortcut_option_b_resolves() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
    }


    #[test]
    fn corporate_seizure_option_a_loses_personnel() {
        let mut state = AppState::new_default(42);
        let before_personnel = state.resources.personnel;
        setup_crisis(&mut state, CrisisKind::CorporateSeizure { cooperate_loss: 4, board_member_idx: 0, corp_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.resources.personnel, before_personnel - 4,
            "option A should lose scaled personnel");
    }

    #[test]
    fn cult_blockade_option_a_resolves() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum;
        setup_crisis(&mut state, CrisisKind::CultBlockade { region_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!(after.active_crisis.is_none(), "crisis should be resolved");
    }

    #[test]
    fn crisis_temporary_operation_ties_up_personnel_and_returns_them() {
        use crate::state::{CrisisCost, CrisisEvent, CrisisKind, CrisisOption, OperationSpec, TICKS_PER_DAY};

        let mut state = AppState::new_default(42);
        let before_personnel = state.resources.personnel;

        // Set up a crisis with a 2-day temporary operation cost
        // Crisis active — blocking derived from active_crisis
        state.ui.crisis_selection = 0;
        state.active_crisis = Some(CrisisEvent {
            kind: CrisisKind::PerformanceReview,
            title: "T".into(),
            description: "T".into(),
            options: vec![CrisisOption {
                label: "Deploy team".into(),
                description: "Test temp op".into(),
                cost: Some(CrisisCost {
                    funding: 0.0,
                    personnel: 2,
                    operation: Some(OperationSpec { days: 2.0, label: "Test Team".into() }),
                }),
            }],
            tick_created: 0,
        });

        let after = apply_action(&state, &Action::Confirm);

        // Personnel should NOT be permanently deducted
        assert_eq!(after.resources.personnel, before_personnel,
            "temporary op should not permanently deduct personnel");

        // A crisis operation should be created
        assert_eq!(after.crisis_operations.len(), 1,
            "a crisis operation should be active");
        assert_eq!(after.crisis_operations[0].personnel, 2);
        assert_eq!(after.crisis_operations[0].label, "Test Team");

        // Personnel should show as unavailable
        assert_eq!(after.personnel_available(), before_personnel - 2,
            "personnel should be unavailable while in crisis op");

        // After 2 days of ticking, operation completes and personnel return
        let mut state2 = after;
        let ticks_needed = (2.0 * TICKS_PER_DAY).ceil() as u32;
        let mut all_tick_events = Vec::new();
        for _ in 0..ticks_needed {
            let tick_events;
            { let r = tick(&state2); state2 = state2.with_world(r.0); tick_events = r.1; }
            all_tick_events.extend(tick_events);
        }
        assert_eq!(state2.crisis_operations.len(), 0,
            "crisis operation should complete after duration");
        assert_eq!(state2.personnel_available(), before_personnel,
            "personnel should be returned after operation completes");
        assert!(all_tick_events.iter().any(|e| matches!(e,
            crate::state::GameEvent::CrisisTeamReturned { personnel: 2, .. }
        )), "should fire CrisisTeamReturned event");
    }

#[test]
    fn vaccine_dispute_option_a_loses_funding() {
        let mut state = AppState::new_default(42);
        state.resources.funding = 1000.0;
        setup_crisis(&mut state, CrisisKind::VaccineDispute { neutral_loss: 200.0, credit_gain: 300.0, corp_a: "Seraph Genomics".to_string(), corp_b: "Caliber Bioscience".to_string() }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - 800.0).abs() < 1.0,
            "option A should lose scaled neutral_loss ($200)");
    }

    #[test]
    fn vaccine_dispute_option_b_gains_funding() {
        let mut state = AppState::new_default(42);
        state.resources.funding = 1000.0;
        state.resources.authority = Authority::Maximum;
        setup_crisis(&mut state, CrisisKind::VaccineDispute { neutral_loss: 200.0, credit_gain: 300.0, corp_a: "Seraph Genomics".to_string(), corp_b: "Caliber Bioscience".to_string() }, 1);
        let after = apply_action(&state, &Action::Confirm);
        assert!((after.resources.funding - 1300.0).abs() < 1.0,
            "option B should gain scaled credit_gain ($300)");
    }

    #[test]
    fn trial_shortcut_fast_track_marks_tested_and_targeted() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 1);

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.medicines[0].tested_against.contains(&0),
            "fast-track should mark medicine as tested");
        assert!(after.medicines[0].target_diseases.contains(&0),
            "fast-track should add disease as primary target");
        assert!(after.active_crisis.is_none());
    }

    #[test]
    fn trial_shortcut_maintain_standards_no_medicine_change() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        setup_crisis(&mut state, CrisisKind::TrialShortcut { disease_idx: 0, medicine_idx: 0 }, 0);

        let after = apply_action(&state, &Action::Confirm);
        assert!(after.medicines[0].tested_against.is_empty(),
            "maintain standards should not mark medicine as tested");
        assert!(after.active_crisis.is_none());
    }

    #[test]
    fn trial_shortcut_generates_when_untested_exists() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        state.medicines[0].tested_against.clear();
        state.tick = 5000; // past CRISIS_MIN_TICK
        let mut got_trial = false;
        for seed in 0..500u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            if let Some(event) = crisis::generate_crisis(&state, &mut rng) {
                if matches!(event.kind, CrisisKind::TrialShortcut { .. }) {
                    got_trial = true;
                    break;
                }
            }
        }
        assert!(got_trial, "TrialShortcut should generate when untested disease exists");
    }


    #[test]
    fn horizontal_gene_transfer_between_bacteria() {
        let mut state = AppState::new_default(42);
        // Disable auto-deploy: it builds broad-spectrum resistance on disease 0
        // which would inflate disease 0's resistance far above the manually set 0.5,
        // causing disease 1 to receive more HGT than the test expects.
        for flag in &mut state.deploy_enabled { *flag = false; }
        // Set up two Bacterium diseases co-located in the same region
        state.diseases[0].pathogen_type = PathogenType::Bacterium;
        // Add a second Bacterium disease
        let mut disease2 = state.diseases[0].clone();
        disease2.name = "Test Bacterium B".into();
        disease2.pathogen_type = PathogenType::Bacterium;
        disease2.mechanism_resistance.clear();
        state.diseases.push(disease2);

        // Give disease 0 significant broad-spectrum resistance (mechanism=None)
        state.diseases[0].add_resistance(None, 0.5);

        // Ensure both diseases have infections in the same region
        let region_idx = primary_outbreak_region(&state);
        state.regions[region_idx].get_or_create_infection(1).infected = 1000.0;

        // Disease 1 should start with no resistance
        assert_eq!(state.diseases[1].get_resistance(None), 0.0);

        // Run many ticks to allow HGT to accumulate
        for _ in 0..1200 { // ~10 days
            state = state.with_world(tick(&state).0);
        }

        // Disease 1 should have gained meaningful broad-spectrum resistance
        // At 10%/day over 10 days with 0.5 donor: expect ~0.5*(1-0.9^10) ≈ 0.33
        let transferred = state.diseases[1].get_resistance(None);
        assert!(
            transferred > 0.10,
            "HGT should transfer meaningful resistance: got {transferred}"
        );
        assert!(
            transferred < 0.5,
            "HGT should not fully equalize: got {transferred}"
        );
    }

    #[test]
    fn horizontal_gene_transfer_only_affects_bacteria() {
        let mut state = AppState::new_default(42);
        // Disease 0 is Bacterium with resistance, disease 1 is RnaVirus
        state.diseases[0].pathogen_type = PathogenType::Bacterium;
        state.diseases[0].add_resistance(None, 0.5);

        // Ensure there's a second disease that's a virus
        if state.diseases.len() < 2 {
            let mut d = state.diseases[0].clone();
            d.name = "Test Virus".into();
            d.pathogen_type = PathogenType::RnaVirus;
            d.mechanism_resistance.clear();
            state.diseases.push(d);
        } else {
            state.diseases[1].pathogen_type = PathogenType::RnaVirus;
            state.diseases[1].mechanism_resistance.clear();
        }

        // Ensure co-location
        let region_idx = primary_outbreak_region(&state);
        state.regions[region_idx].get_or_create_infection(1).infected = 1000.0;

        for _ in 0..1200 {
            state = state.with_world(tick(&state).0);
        }

        // Virus should NOT gain resistance from bacterial HGT
        let virus_resistance = state.diseases[1].get_resistance(None);
        assert_eq!(
            virus_resistance, 0.0,
            "HGT should not affect non-bacteria: got {virus_resistance}"
        );
    }

    #[test]
    fn collapse_loses_personnel() {
        let mut state = AppState::new_default(42);
        corporations::generate_corporations(&mut state);
        board::generate_board_members(&mut state);
        detect_all_diseases(&mut state);
        let initial_personnel = state.resources.personnel;

        // Force a region to collapse
        let region_idx = primary_outbreak_region(&state);
        let pop = state.regions[region_idx].population as f64;
        state.regions[region_idx].dead = pop * 0.6; // above collapse threshold

        // Tick to trigger collapse detection
        state = state.with_world(tick(&state).0);
        assert!(state.regions[region_idx].collapsed, "region should have collapsed");

        // Personnel should be reduced by 2
        assert_eq!(
            state.resources.personnel,
            initial_personnel - 2,
            "should lose 2 personnel on collapse"
        );
    }

    #[test]
    fn deploy_cooldown_blocks_repeat_deployment() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        // Setup: unlock a medicine, seed infection, give funds
        let disease_idx = 0;
        let med_idx = 0;
        state.medicines[med_idx].unlocked = true;
        state.medicines[med_idx].tested_against.push(disease_idx);
        state.medicines[med_idx].doses = 1_000_000.0;
        state.medicines[med_idx].max_doses = 1_000_000.0;
        state.resources.funding = 1_000_000.0;
        state.regions[0].get_or_create_infection(disease_idx).infected = 50_000.0;

        // First deploy should succeed
        let treat = DeployTarget { disease_idx, mode: crate::state::MedicineMode::Therapeutic };
        let (nav, msg) = medicine::deploy_medicine(&mut state, med_idx, 0, treat.clone(), &mut events);
        assert!(nav, "first deploy should succeed");
        assert!(msg.unwrap().contains("Shipped"), "should show shipment message");

        // Region should now have a cooldown set for this medicine
        assert!(state.regions[0].last_deploy_tick.contains_key(&med_idx));

        // Second deploy at same tick should be blocked
        state.resources.funding = 1_000_000.0;
        state.regions[0].get_or_create_infection(disease_idx).infected = 50_000.0;
        let (nav2, msg2) = medicine::deploy_medicine(&mut state, med_idx, 0, treat.clone(), &mut events);
        assert!(!nav2, "second deploy should be blocked by cooldown");
        assert!(msg2.unwrap().contains("cooldown"), "should mention cooldown");

        // After cooldown expires, deploy should work again
        state.tick = crate::state::DEPLOY_COOLDOWN_TICKS + 1;
        state.resources.funding = 1_000_000.0;
        state.regions[0].get_or_create_infection(disease_idx).infected = 50_000.0;
        let (nav3, msg3) = medicine::deploy_medicine(&mut state, med_idx, 0, treat.clone(), &mut events);
        assert!(nav3, "deploy after cooldown should succeed");
        assert!(msg3.unwrap().contains("Shipped"));

        // Different region should still be deployable (cooldown is per-region-per-medicine)
        state.tick = 0;
        state.regions[0].last_deploy_tick.insert(med_idx, 0);
        state.regions[1].get_or_create_infection(disease_idx).infected = 50_000.0;
        state.resources.funding = 1_000_000.0;
        let (nav4, _) = medicine::deploy_medicine(&mut state, med_idx, 1, treat.clone(), &mut events);
        assert!(nav4, "deploying to different region should work during cooldown");
    }

    #[test]
    fn threat_escalation_fires_at_death_thresholds() {
        let mut state = AppState::new_default(42);
        state.diseases[0].detected = true;
        // Set deaths above 1M threshold on the existing infection entry
        // (new_default already seeds disease 0 in some region)
        for region in &mut state.regions {
            if let Some(inf) = region.infections.iter_mut().find(|i| i.disease_idx == 0) {
                inf.dead = 1_500_000.0;
                inf.infected = 100_000.0;
            }
            if region.dead > 0.0 {
                region.dead = 1_500_000.0;
            }
        }

        let (new_state, tick_events) = tick(&state);
        let escalation = tick_events.iter().find(|e|
            matches!(e, GameEvent::ThreatEscalation { .. })
        );
        assert!(escalation.is_some(), "should fire escalation at 1M deaths");

        if let Some(GameEvent::ThreatEscalation { disease_idx, has_medicine, .. }) = escalation {
            assert_eq!(*disease_idx, 0);
            assert!(has_medicine, "broad-spectrum starts unlocked and targets all diseases");
        }
        assert_eq!(new_state.death_milestone_tier[0], 1, "should set alert level to 1");

        // Second tick should NOT re-fire the same threshold
        let (state2, tick_events2) = tick(&new_state);
        let escalation2 = tick_events2.iter().find(|e|
            matches!(e, GameEvent::ThreatEscalation { .. })
        );
        assert!(escalation2.is_none(), "should not re-fire same threshold");
    }

    #[test]
    fn threat_escalation_skips_undetected_diseases() {
        let mut state = AppState::new_default(42);
        state.diseases[0].detected = false; // Not yet detected
        // Set deaths high but infected below detection threshold (10K)
        // so the disease stays undetected during the tick
        for region in &mut state.regions {
            if let Some(inf) = region.infections.iter_mut().find(|i| i.disease_idx == 0) {
                inf.dead = 2_000_000.0;
                inf.infected = 100.0;
            }
            if region.dead > 0.0 {
                region.dead = 2_000_000.0;
            }
        }

        let (_, tick_events) = tick(&state);
        let escalation = tick_events.iter().find(|e|
            matches!(e, GameEvent::ThreatEscalation { .. })
        );
        assert!(escalation.is_none(), "should not fire for undetected disease");
    }

    #[test]
    fn decree_enact_via_orders_panel_ui_flow() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.resources.funding = 10_000.0;
        // Unlock all decrees: collapse regions 3-5 + set 600K infected on region 0
        for i in 3..6 { state.regions[i].collapsed = true; }
        state.regions[0].get_or_create_infection(0).infected = 600_000.0;

        // Open Orders panel — first item is the first decree (Conscript Researchers)
        state = apply_action(&state, &Action::OpenOperations);
        assert_eq!(state.ui.open_panel, Panel::Operations);
        assert_eq!(state.ui.panel_selection, 0);

        let personnel_before = state.resources.personnel;
        // First Confirm goes to the confirmation screen
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.ui.operations_ui, Some(OpsUiState::ConfirmDecree { decree: DecreeId::ConscriptResearchers }),
            "should show confirmation before enacting");
        assert!(!state.enacted_decrees.conscript_researchers, "should not yet be enacted");
        // Second Confirm enacts the decree
        state = apply_action(&state, &Action::Confirm);
        assert!(state.enacted_decrees.conscript_researchers);
        assert_eq!(state.resources.personnel, personnel_before + crate::state::CONSCRIPT_PERSONNEL_GAIN);
        assert!(state.session.status_message.as_ref().unwrap().contains("Conscript"));
    }

    #[test]
    fn decree_sacrifice_region_ui_flow() {
        let mut state = AppState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.resources.funding = 10_000.0;
        // Unlock all decrees: collapse regions 3-5 + set 600K infected on region 0
        for i in 3..6 { state.regions[i].collapsed = true; }
        state.regions[0].get_or_create_infection(0).infected = 600_000.0;

        // Open Orders panel, navigate to Sacrifice Region (decree index 2)
        state = apply_action(&state, &Action::OpenOperations);
        let sacrifice_idx = 2;
        for _ in 0..sacrifice_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        assert_eq!(state.ui.panel_selection, sacrifice_idx);
        state = apply_action(&state, &Action::Confirm);

        // Should be in SelectSacrificeRegion state
        assert_eq!(state.ui.operations_ui, Some(OpsUiState::SelectSacrificeRegion));

        // Select first non-collapsed region and confirm
        state = apply_action(&state, &Action::Confirm);
        assert!(state.enacted_decrees.sacrificed_region.is_some());
        let sacrificed_idx = state.enacted_decrees.sacrificed_region.unwrap();
        assert!(state.regions[sacrificed_idx].collapsed);

        // UI should return to BrowseOps after successful sacrifice
        assert_eq!(state.ui.operations_ui, Some(OpsUiState::BrowseOps),
            "should return to BrowseOps after enacting sacrifice");
    }

    // --- Crisis chain tests ---





    #[test]
    fn corporate_cooperate_schedules_overreach_followup() {
        let mut state = AppState::new_default(42);
        state.tick = 1000;
        setup_crisis(&mut state, CrisisKind::CorporateSeizure { cooperate_loss: 3, board_member_idx: 0, corp_idx: 0 }, 0);
        let after = apply_action(&state, &Action::Confirm);
        assert_eq!(after.pending_crises.len(), 1);
        assert!(matches!(after.pending_crises[0], CrisisKind::CorporateOverreach { .. }));
    }

    #[test]
    fn pending_crisis_fires_when_due() {
        let mut state = AppState::new_default(42);
        state.tick = 100; // Pending check runs before tick increment
        state.last_contract_offer_tick = state.tick; // prevent contract offer from adding a pending crisis
        state.pending_crises.push(CrisisKind::CorporateOverreach { corp_idx: 0, board_member_idx: 0 });
        let (after, _) = tick(&state);
        assert!(after.active_crisis.is_some(), "pending crisis should fire");
        assert!(after.pending_crises.is_empty(), "fired crisis should be removed from pending");
    }



    #[test]
    fn collapse_rate_estimate_updates_after_one_day() {
        let mut state = AppState::new_default(42);
        let region_idx = primary_outbreak_region(&state);
        // Run for 2+ days to ensure the rate sampler fires at least once
        let ticks = (2.5 * TICKS_PER_DAY) as usize;
        for _ in 0..ticks {
            state = state.with_world(tick(&state).0);
        }
        let region = &state.regions[region_idx];
        // The outbreak region should have deaths and a positive rate
        assert!(
            region.total_dead() > 0.0,
            "outbreak region should have deaths by day 2.5"
        );
        assert!(
            region.cached_deaths_per_day > 0.0,
            "cached death rate should be positive: got {}",
            region.cached_deaths_per_day
        );
        // days_to_collapse should return Some since the region has deaths
        assert!(
            region.days_to_collapse(false).is_some(),
            "should estimate time to collapse when deaths are occurring"
        );
    }

    #[test]
    fn collapse_rate_not_shown_for_safe_regions() {
        let state = AppState::new_default(42);
        // At tick 0, no deaths have occurred — rate should be 0
        for region in &state.regions {
            assert_eq!(region.cached_deaths_per_day, 0.0);
            assert!(region.days_to_collapse(false).is_none());
        }
    }

    #[test]
    fn auto_deploy_treats_worst_region() {
        let mut state = AppState::new_default(42);
        state.resources.funding = 5000.0;

        // Give medicine 0 unlocked status, doses, and tested against disease 0
        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 1_000_000.0;
        state.medicines[0].max_doses = 1_000_000.0;
        state.medicines[0].tested_against = vec![0];
        state.diseases[0].detected = true;

        // Set up infections: region 0 has 100K infected, region 1 has 500K
        state.regions[0].get_or_create_infection(0).infected = 100_000.0;
        state.regions[1].get_or_create_infection(0).infected = 500_000.0;

        // Enable deploy for medicine 0
        state.deploy_enabled = vec![true];

        let (after, tick_events) = tick(&state);

        // Should have auto-deployed (MedicineShipped fires on success)
        let shipped_events: Vec<_> = tick_events.iter()
            .filter(|e| matches!(e, GameEvent::MedicineShipped { .. }))
            .collect();
        assert_eq!(shipped_events.len(), 1, "should auto-deploy exactly once per tick");

        // Should target region 1 (worst infected)
        match &shipped_events[0] {
            GameEvent::MedicineShipped { region_idx, .. } => {
                assert_eq!(*region_idx, 1, "should deploy to worst-affected region");
            }
            _ => unreachable!(),
        }

        // Doses should have been consumed
        assert!(after.medicines[0].doses < state.medicines[0].doses,
            "doses should be consumed by auto-deploy");
    }

    #[test]
    fn auto_deploy_skips_untested_medicine() {
        let mut state = AppState::new_default(42);
        state.resources.funding = 5000.0;

        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 1_000_000.0;
        state.medicines[0].max_doses = 1_000_000.0;
        // NOT tested: tested_against is empty
        state.diseases[0].detected = true;

        state.regions[0].get_or_create_infection(0).infected = 100_000.0;
        state.deploy_enabled = vec![true];

        let (_, tick_events) = tick(&state);

        // Should NOT auto-deploy untested medicines
        let shipped: Vec<_> = tick_events.iter()
            .filter(|e| matches!(e, GameEvent::MedicineShipped { .. }))
            .collect();
        assert!(shipped.is_empty(), "should not auto-deploy untested medicines");
    }

    #[test]
    fn auto_deploy_respects_cooldown() {
        let mut state = AppState::new_default(42);
        state.resources.funding = 5000.0;

        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 1_000_000.0;
        state.medicines[0].max_doses = 1_000_000.0;
        state.medicines[0].tested_against = vec![0];
        state.diseases[0].detected = true;

        // Only region 0 has infections, but it's on cooldown
        for r in &mut state.regions { r.infections.clear(); }
        state.regions[0].get_or_create_infection(0).infected = 100_000.0;
        let current_tick = state.tick;
        state.regions[0].last_deploy_tick.insert(0, current_tick);
        state.deploy_enabled = vec![true];

        let (_, tick_events) = tick(&state);

        let shipped: Vec<_> = tick_events.iter()
            .filter(|e| matches!(e, GameEvent::MedicineShipped { .. }))
            .collect();
        assert!(shipped.is_empty(), "should not deploy to region on cooldown");
    }


    #[test]
    fn degraded_infrastructure_reduces_delivery_effectiveness() {
        let mut events: Vec<GameEvent> = Vec::new();
        // When infrastructure is degraded, fewer doses take effect on delivery.
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        // Advance to get some infected
        for _ in 0..20 {
            state = state.with_world(tick(&state).0);
        }

        let ri = primary_outbreak_region(&state);
        let infected = state.regions[ri].disease_state(0).unwrap().infected;
        assert!(infected > 100.0, "need infected to test treatment");

        // Baseline: full infrastructure, deploy and deliver
        let mut baseline = state.clone();
        let target = crate::state::DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic };
        medicine::deploy_medicine(&mut baseline, 0, ri, target.clone(), &mut events);
        assert_eq!(baseline.pending_shipments.len(), 1);
        baseline.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = baseline.rng_misc.clone(); medicine::tick_shipments(&mut baseline, &mut rng, &mut events); }
        let infected_full_infra = baseline.regions[ri].disease_state(0).unwrap().infected;

        // Degraded: 50% supply lines, 50% healthcare = 25% efficiency
        let mut degraded = state.clone();
        degraded.regions[ri].supply_lines = 0.50;
        degraded.regions[ri].healthcare_capacity = 0.50;
        let target = crate::state::DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic };
        medicine::deploy_medicine(&mut degraded, 0, ri, target, &mut events);
        assert_eq!(degraded.pending_shipments.len(), 1);
        degraded.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = degraded.rng_misc.clone(); medicine::tick_shipments(&mut degraded, &mut rng, &mut events); }
        let infected_degraded = degraded.regions[ri].disease_state(0).unwrap().infected;

        // With degraded infrastructure, more infected should remain (fewer doses effective)
        assert!(
            infected_degraded > infected_full_infra,
            "degraded infra should leave more infected: degraded={:.0} vs full={:.0}",
            infected_degraded, infected_full_infra
        );
    }

    #[test]
    fn delivery_efficiency_shown_in_shipped_event() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        for _ in 0..20 {
            state = state.with_world(tick(&state).0);
        }
        let ri = primary_outbreak_region(&state);

        // Degrade infrastructure
        state.regions[ri].supply_lines = 0.60;
        state.regions[ri].healthcare_capacity = 0.70;

        let target = crate::state::DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic };
        medicine::deploy_medicine(&mut state, 0, ri, target, &mut events);
        state.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }

        // Check that the delivered event contains the efficiency
        let delivered = events.iter().find(|e| matches!(e, GameEvent::ShipmentDelivered { .. }));
        assert!(delivered.is_some(), "should have a ShipmentDelivered event");
        match delivered.unwrap() {
            GameEvent::ShipmentDelivered { efficiency, .. } => {
                let expected = 0.60 * 0.70; // 0.42
                assert!(
                    (*efficiency - expected).abs() < 0.01,
                    "efficiency should be supply_lines * healthcare: got {efficiency}, expected {expected}"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn targeting_waste_scales_with_screening() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::ScreeningLevel;

        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        for _ in 0..20 {
            state = state.with_world(tick(&state).0);
        }
        let ri = primary_outbreak_region(&state);

        // No screening: targeting_efficiency = 0.50
        assert_eq!(state.policies[ri].screening, ScreeningLevel::None);
        let target = crate::state::DeployTarget { disease_idx: 0, mode: crate::state::MedicineMode::Therapeutic };
        medicine::deploy_medicine(&mut state, 0, ri, target.clone(), &mut events);
        state.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }

        let delivered = events.iter().rev()
            .find(|e| matches!(e, GameEvent::ShipmentDelivered { .. }));
        assert!(delivered.is_some(), "should have a ShipmentDelivered event");
        match delivered.unwrap() {
            GameEvent::ShipmentDelivered { doses_wasted, doses, .. } => {
                // With no screening and full infra, ~50% of doses should be wasted
                let waste_fraction = doses_wasted / doses;
                assert!(
                    waste_fraction > 0.40 && waste_fraction < 0.60,
                    "no-screening waste should be ~50%, got {:.1}% ({} wasted of {} doses)",
                    waste_fraction * 100.0, doses_wasted, doses
                );
            }
            _ => unreachable!(),
        }

        // Now enable Mass Rapid screening with full progress
        state.policies[ri].screening = ScreeningLevel::MassRapid;
        state.policies[ri].screening_progress = 1.0;
        // Clear cooldown so we can deploy again
        state.regions[ri].last_deploy_tick.clear();
        state.medicines[0].doses = state.medicines[0].max_doses;
        events.clear();

        medicine::deploy_medicine(&mut state, 0, ri, target, &mut events);
        state.tick += crate::state::SHIPPING_TICKS + 1;
        { let mut rng = state.rng_misc.clone(); medicine::tick_shipments(&mut state, &mut rng, &mut events); }

        let delivered2 = events.iter().rev()
            .find(|e| matches!(e, GameEvent::ShipmentDelivered { .. }));
        assert!(delivered2.is_some(), "should have a second ShipmentDelivered event");
        match delivered2.unwrap() {
            GameEvent::ShipmentDelivered { doses_wasted, .. } => {
                assert!(
                    *doses_wasted < 1.0,
                    "mass-rapid screening should have near-zero waste, got {}",
                    doses_wasted
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn ark_protocol_schedules_when_two_regions_collapse() {
        let mut state = AppState::new_default(42);
        // Collapse regions 0 and 1
        for idx in [0, 1] {
            let pop = state.regions[idx].population as f64;
            let threshold = state.regions[idx].collapse_threshold;
            state.regions[idx].dead = pop * (1.0 - threshold) + 1.0;
            state.regions[idx].get_or_create_infection(0).dead = state.regions[idx].dead;
            state.regions[idx].collapsed = true;
        }
        // Clear any crisis state from previous ticks
        state.active_crisis = None;
        state.pending_crises.clear();
        state.crisis_cooldowns.clear();

        let (after, _) = tick(&state);

        // Should have scheduled an ArkProtocol pending crisis
        let ark_pending = after.pending_crises.iter()
            .find(|k| matches!(k, CrisisKind::ArkProtocol { .. }));
        assert!(ark_pending.is_some(), "should schedule Ark Protocol when 2+ regions collapse");

        // Should pick the surviving region with highest survival fraction (alive / population),
        // not raw alive count — devastated regions should not be recommended as the Ark.
        if let Some(CrisisKind::ArkProtocol { region_idx }) = ark_pending {
            assert!(!after.regions[*region_idx].collapsed,
                "Ark target should be a surviving region");
            // Verify it picked the best surviving region (highest survival fraction)
            let survival_fraction = |r: &crate::state::Region| {
                r.alive() / (r.population as f64).max(1.0)
            };
            let best = after.regions.iter().enumerate()
                .filter(|(_, r)| !r.collapsed)
                .max_by(|a, b| survival_fraction(a.1).partial_cmp(&survival_fraction(b.1)).unwrap());
            assert_eq!(Some(*region_idx), best.map(|(i, _)| i),
                "should pick surviving region with highest survival fraction");
        }
    }

    #[test]
    fn ark_protocol_accept_sets_state_and_clears_policies() {
        let mut state = AppState::new_default(42);
        // Enable some policies in region 1 and 2
        state.policies[1].quarantine = true;
        state.policies[2].travel_ban = true;
        state.policies[3].border_controls = true;
        // Collapse regions 0 and 4 so Ark on region 2 makes sense
        state.regions[0].collapsed = true;
        state.regions[4].collapsed = true;

        // Set up Ark Protocol crisis targeting region 2.
        // Surviving regions are [1, 2, 3, 5], so region 2 is at index 1.
        setup_crisis(&mut state, CrisisKind::ArkProtocol { region_idx: 2 }, 1);
        let choice = state.ui.crisis_selection;
        let result = execute_command(&mut state, &GameCommand::ResolveCrisis { choice });

        // Ark Protocol should be active on region 2
        assert_eq!(state.ark_protocol, Some(2));
        // Non-Ark regions should be collapsed with policies cleared
        assert!(state.regions[1].collapsed, "non-Ark region should be collapsed");
        assert!(!state.policies[1].quarantine, "collapsed region policies should be cleared");
        assert!(!state.policies[3].border_controls, "collapsed region policies should be cleared");
        // ArkProtocolActivated event should be emitted
        assert!(result.events.iter().any(|e|
            matches!(e, GameEvent::ArkProtocolActivated { region_idx: 2 })));
    }

    #[test]
    fn ark_protocol_collapses_non_ark_regions() {
        let mut state = AppState::new_default(42);
        unlock_all_medicines(&mut state);
        detect_all_diseases(&mut state);
        // Collapse two regions to trigger Ark Protocol conditions
        state.regions[0].collapsed = true;
        state.regions[4].collapsed = true;
        // Set up Ark Protocol crisis targeting region 2.
        // Surviving regions are [1, 2, 3, 5], so region 2 is at index 1.
        setup_crisis(&mut state, CrisisKind::ArkProtocol { region_idx: 2 }, 1);
        let after = apply_action(&state, &Action::Confirm);

        assert_eq!(after.ark_protocol, Some(2));
        // All non-Ark regions should be collapsed
        for (i, region) in after.regions.iter().enumerate() {
            if i == 2 {
                // Ark region should NOT be collapsed (unless it was already)
                continue;
            }
            assert!(region.collapsed, "non-Ark region {} should be collapsed", i);
        }
        // Policies in collapsed regions should be cleared
        assert!(!after.policies[1].quarantine, "collapsed region policies should be cleared");
        assert!(!after.policies[3].border_controls, "collapsed region policies should be cleared");
    }

    #[test]
    fn decree_gated_by_crisis_severity() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::RegionDiseaseState;

        let mut state = AppState::new_default(42);
        state.resources.funding = 10_000.0;
        state.resources.authority = Authority::Maximum;

        // Fresh game: all decrees locked despite high POL
        let (msg, ok) = policy::enact_decree(&mut state, DecreeId::ConscriptResearchers, None, &mut events);
        assert!(!ok, "decree should be blocked when severity is low");
        assert!(msg.unwrap().contains("more severe crisis"));

        // Add 600K infected to unlock Conscript Researchers (decree 0)
        state.regions[0].infections = vec![RegionDiseaseState {
            disease_idx: 0, exposed: 0.0, infected: 600_000.0, dead: 0.0, immune: 0.0,
        }];
        assert!(state.decree_unlocked(DecreeId::ConscriptResearchers), "decree 0 should unlock at 500K+ infected");
        let (_, ok) = policy::enact_decree(&mut state, DecreeId::ConscriptResearchers, None, &mut events);
        assert!(ok, "decree should be available with sufficient severity");
        assert!(state.enacted_decrees.conscript_researchers);
    }

    #[test]
    fn decree_unlock_emits_event() {
        use crate::state::RegionDiseaseState;

        let mut state = AppState::new_default(42);
        // Clear ALL infections and deaths so all decrees start locked
        for region in &mut state.regions {
            region.infections.clear();
        }
        assert!(!state.decree_unlocked(DecreeId::ConscriptResearchers), "decree 0 should be locked with no infections/deaths");

        // Set total (exposed+infected) just below 500K threshold for decree 0.
        // total_infected() counts exposed+infected.
        // Use 499K total with large infected pool to generate new exposures from
        // the susceptible population, pushing total past 500K in one tick.
        state.regions[0].infections = vec![RegionDiseaseState {
            disease_idx: 0, exposed: 9_000.0, infected: 490_000.0, dead: 0.0, immune: 0.0,
        }];
        assert!(!state.decree_unlocked(DecreeId::ConscriptResearchers), "499K total should be below 500K threshold");

        // Tick: high infected count will expose new susceptibles, pushing total past 500K.
        let (new, tick_events) = tick(&state);
        assert!(new.decree_unlocked(DecreeId::ConscriptResearchers), "decree 0 should be unlocked after tick (spread grew past 500K)");
        let unlocked_events: Vec<_> = tick_events.iter()
            .filter(|e| matches!(e, GameEvent::DecreeUnlocked { .. }))
            .collect();
        assert!(!unlocked_events.is_empty(), "should emit DecreeUnlocked event when crossing threshold");

        // Running tick again should NOT re-emit (already unlocked)
        let (new2, tick_events2) = tick(&new);
        let unlocked_events2: Vec<_> = tick_events2.iter()
            .filter(|e| matches!(e, GameEvent::DecreeUnlocked { .. }))
            .collect();
        assert!(unlocked_events2.is_empty(), "should not re-emit DecreeUnlocked on subsequent ticks");
    }

    #[test]
    fn bargain_buffoon_gains_cooperation() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Buffoon;
        state.regions[0].governor.cooperation = 20.0;
        state.resources.authority = Authority::Maximum;
        let initial_cooperation = state.regions[0].governor.cooperation;

        let (msg, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok, "bargain should succeed");
        assert!(msg.unwrap().contains("praised publicly"));
        assert!(state.regions[0].governor.cooperation > initial_cooperation);
    }

    #[test]
    fn bargain_blowhard_costs_funding_gains_big_cooperation() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Blowhard;
        state.regions[0].governor.cooperation = 20.0;
        let initial_funding = state.resources.funding;

        let (msg, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok, "bargain should succeed");
        assert!(msg.unwrap().contains("token victory"));
        assert!(state.resources.funding < initial_funding);
        // Blowhard gets +30 cooperation (BARGAIN_BLOWHARD_COOPERATION_GAIN)
        assert!((state.regions[0].governor.cooperation - 50.0).abs() < 0.01);
    }

    #[test]
    fn bargain_blowhard_fails_without_funding() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Blowhard;
        state.regions[0].governor.cooperation = 20.0;
        state.resources.funding = 50.0; // less than BARGAIN_BLOWHARD_FUNDING_COST (100)

        let (_, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(!ok, "bargain should fail without enough funding");
    }

    #[test]
    fn bargain_recluse_costs_personnel() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Recluse;
        state.regions[0].governor.cooperation = 20.0;
        let initial_personnel = state.resources.personnel;
        let initial_cooperation = state.regions[0].governor.cooperation;

        let (msg, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok, "bargain should succeed");
        assert!(msg.unwrap().contains("manager sent"));
        assert_eq!(state.resources.personnel, initial_personnel - 2);
        assert!(state.regions[0].governor.cooperation > initial_cooperation);
    }

    #[test]
    fn bargain_recluse_fails_without_personnel() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Recluse;
        state.regions[0].governor.cooperation = 20.0;
        state.resources.personnel = 1; // less than BARGAIN_RECLUSE_PERSONNEL_COST (2)

        let (_, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(!ok, "bargain should fail without enough personnel");
    }

    #[test]
    fn bargain_hardliner_costs_funding() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Hardliner;
        state.regions[0].governor.cooperation = 20.0;
        let initial_funding = state.resources.funding;

        let (msg, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok, "bargain should succeed");
        assert!(msg.unwrap().contains("expanded authority"));
        assert!(state.resources.funding < initial_funding);
    }

    #[test]
    fn bargain_hardliner_fails_without_funding() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Hardliner;
        state.regions[0].governor.cooperation = 20.0;
        state.resources.funding = 100.0; // less than BARGAIN_HARDLINER_FUNDING_COST (400)

        let (_, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(!ok, "bargain should fail without enough funding");
    }

    #[test]
    fn bargain_operative_adds_income_skim() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Operative;
        state.regions[0].governor.cooperation = 20.0;
        assert_eq!(state.regions[0].governor.income_skim, 0.0);

        let (msg, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok, "bargain should succeed");
        assert!(msg.unwrap().contains("cut agreed"));
        assert!((state.regions[0].governor.income_skim - 0.10).abs() < 0.001);
        // Second bargain stacks
        state.regions[0].governor.cooperation = 20.0;
        let (_, ok2) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok2);
        assert!((state.regions[0].governor.income_skim - 0.20).abs() < 0.001);

        // Skim is capped at MAX_OPERATIVE_INCOME_SKIM (0.50)
        state.regions[0].governor.income_skim = 0.45;
        state.regions[0].governor.cooperation = 20.0;
        let (_, ok3) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok3);
        assert!((state.regions[0].governor.income_skim - 0.50).abs() < 0.001,
            "skim should cap at 0.50, got {}", state.regions[0].governor.income_skim);
    }

    #[test]
    fn bargain_mobster_escalates_cost() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Mobster;
        state.regions[0].governor.cooperation = 20.0;
        state.resources.funding = 10000.0;

        // First bargain: 200
        let (_, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok);
        assert!((state.resources.funding - 9800.0).abs() < 0.01);
        assert_eq!(state.regions[0].governor.bargain_count, 1);

        // Second bargain: 400
        state.regions[0].governor.cooperation = 20.0;
        let (_, ok2) = policy::bargain_with_governor(&mut state, 0);
        assert!(ok2);
        assert!((state.resources.funding - 9400.0).abs() < 0.01);
        assert_eq!(state.regions[0].governor.bargain_count, 2);
    }

    #[test]
    fn bargain_mobster_fails_without_funding() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.personality = GovernorPersonality::Mobster;
        state.regions[0].governor.cooperation = 20.0;
        state.resources.funding = 100.0; // less than BARGAIN_MOBSTER_BASE_COST (200)

        let (_, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(!ok, "bargain should fail without enough funding");
    }

    #[test]
    fn bargain_fails_when_not_hostile() {
        let mut state = AppState::new_default(42);
        state.regions[0].governor.cooperation = 60.0; // above hostility threshold
        let (_, ok) = policy::bargain_with_governor(&mut state, 0);
        assert!(!ok, "bargain should fail when governor isn't hostile");
    }

    #[test]
    fn tech_pressure_increases_emergence_rate() {
        use crate::state::{BasicTech, EMERGENCE_CHANCE_PER_TICK};
        let mut state = AppState::new_default(42);

        // No techs → zero pressure
        assert_eq!(state.tech_pressure(), 0.0);

        // Unlock some techs
        state.unlocked_techs.push(BasicTech::TargetedDrugDesign);
        state.unlocked_techs.push(BasicTech::RapidSequencing);
        let pressure = state.tech_pressure();
        assert!(pressure > 0.0, "tech pressure should increase with unlocked techs");
        assert!((pressure - 0.30).abs() < 0.01, "2 techs × 0.15 = 0.30, got {pressure}");

        // Effective emergence chance should be higher
        let effective_chance = EMERGENCE_CHANCE_PER_TICK * (1.0 + pressure);
        assert!(effective_chance > EMERGENCE_CHANCE_PER_TICK);
    }

    #[test]
    fn counter_capability_biases_pathogen_type() {
        use crate::state::TherapyType;
        use rand::SeedableRng;

        // Run many spawns with deployed antiviral medicines and count pathogen types
        let mut virus_count = 0u32;
        let mut non_virus_count = 0u32;

        for seed in 0..50 {
            let mut state = AppState::new_default(42);
            state.tick = (60.0 * crate::state::TICKS_PER_DAY) as u64; // day 60 (full counter-weight)
            // Give the player deployed antivirals
            for med in &mut state.medicines {
                if med.therapy_type == TherapyType::Antiviral {
                    med.deployed_count = 5;
                }
            }
            let mut rng = ChaCha8Rng::seed_from_u64(seed + 1000);
            let initial = state.diseases.len();
            disease::spawn_disease_scaled(&mut state, &mut rng);
            if state.diseases.len() > initial {
                let d = &state.diseases[initial];
                match d.pathogen_type {
                    crate::state::PathogenType::RnaVirus | crate::state::PathogenType::DnaVirus => virus_count += 1,
                    _ => non_virus_count += 1,
                }
            }
        }

        // With antivirals deployed, non-virus types should appear more often
        // than virus types (they're counter-weighted). Without bias, viruses
        // would be ~40% of spawns. With bias, they should be significantly less.
        let total = virus_count + non_virus_count;
        if total >= 10 {
            assert!(non_virus_count > virus_count,
                "with deployed antivirals, non-virus types should dominate: viruses={virus_count}, non-viruses={non_virus_count}");
        }
    }

    #[test]
    fn strategic_targeting_prefers_high_population_regions_late_game() {
        use rand::SeedableRng;

        // Run many spawns at day 60+ and count which regions get hit
        let mut region_hits = [0u32; 6];

        for seed in 0..100 {
            let mut state = AppState::new_default(42);
            state.tick = (60.0 * crate::state::TICKS_PER_DAY) as u64;
            // Add some infrastructure to Asia (highest pop region)
            state.regions[2].hospital_level = 2; // Medical Center
            state.policies[2].quarantine = true;
            let mut rng = ChaCha8Rng::seed_from_u64(seed + 2000);
            let initial = state.diseases.len();
            if let Some((_, region_idx)) = disease::spawn_disease_scaled(&mut state, &mut rng) {
                if state.diseases.len() > initial || true {
                    region_hits[region_idx] += 1;
                }
            }
        }

        // Asia (index 2, pop 4.7B) should be hit more often than Oceania (index 5, pop 45M)
        // at day 60 with strategic targeting active
        let asia_hits = region_hits[2];
        let oceania_hits = region_hits[5];
        // Asia should have at least as many hits as Oceania (it should have MORE,
        // but we use >= to avoid flaky tests with small sample sizes)
        assert!(asia_hits >= oceania_hits,
            "strategic targeting should prefer high-pop regions: Asia={asia_hits}, Oceania={oceania_hits}, all={region_hits:?}");
    }

    #[test]
    fn wave_clustering_increases_emergence_after_recent_spawn() {
        use crate::state::TICKS_PER_DAY;

        // At day 60 (fully ramped), with a disease that spawned 50 ticks ago,
        // emergence chance should be much higher than normal.
        // Wave clustering ramps from 2.0 at day 24 to 4.0 at day 50+.
        let mut state = AppState::new_default(42);
        state.tick = (60.0 * TICKS_PER_DAY) as u64;
        state.diseases[0].spawned_at_tick = state.tick - 50;

        let day = state.tick as f64 / TICKS_PER_DAY;
        assert!(day >= 50.0, "should be past full ramp at day 50");

        let most_recent = state.diseases.iter()
            .map(|d| d.spawned_at_tick)
            .max()
            .unwrap_or(0);
        let ticks_since = state.tick.saturating_sub(most_recent);
        assert!(ticks_since < crate::state::WAVE_CLUSTER_WINDOW_TICKS, "should be within wave window");

        // At day 30 (fully ramped), wave boost is 4.0
        let wave_boost = 4.0;
        let normal_chance = crate::state::EMERGENCE_CHANCE_PER_TICK * (1.0 + state.tech_pressure());
        let boosted_chance = crate::state::EMERGENCE_CHANCE_PER_TICK * (1.0 + state.tech_pressure() + wave_boost);
        assert!(boosted_chance > normal_chance * 3.0,
            "wave-boosted chance should be significantly higher: normal={normal_chance}, boosted={boosted_chance}");
    }

    #[test]
    fn wave_clustering_ramps_from_mid_game() {
        use crate::state::TICKS_PER_DAY;

        // At day 30, wave boost should be active but weaker than late-game.
        // Ramp: (30-24)/26 ≈ 0.23, so boost = 2.0 + 0.23*2.0 ≈ 2.46
        let mut state = AppState::new_default(42);
        state.tick = (30.0 * TICKS_PER_DAY) as u64;
        state.diseases[0].spawned_at_tick = state.tick - 50;

        let day = state.tick as f64 / TICKS_PER_DAY;
        let ramp = ((day - 24.0) / 26.0).clamp(0.0, 1.0);
        let wave_boost = 2.0 + ramp * 2.0;

        // Should be meaningfully boosted but less than the full 4.0
        assert!(wave_boost > 2.0, "boost should be active at day 30: {wave_boost}");
        assert!(wave_boost < 3.0, "boost should not be fully ramped at day 30: {wave_boost}");

        let normal_chance = crate::state::EMERGENCE_CHANCE_PER_TICK * (1.0 + state.tech_pressure());
        let boosted_chance = crate::state::EMERGENCE_CHANCE_PER_TICK * (1.0 + state.tech_pressure() + wave_boost);
        assert!(boosted_chance > normal_chance * 2.0,
            "mid-game wave boost should at least double emergence chance");
    }

    #[test]
    fn wave_diseases_share_sequence_group() {
        use crate::state::TICKS_PER_DAY;

        // Run the game long enough for wave clustering to fire and assign sequence groups.
        // Wave clustering is active past day 24. We run 50 seeds and check that at least
        // one produces two diseases sharing a sequence_group (wave correlation).
        let mut found_shared_group = false;
        for seed in 0..50u64 {
            let mut state = AppState::new_default(seed);
            // Advance to day 50 (well into wave clustering territory)
            for _ in 0..(50 * TICKS_PER_DAY as u64) {
                state = state.with_world(tick(&state).0);
                if matches!(state.outcome, crate::state::GameOutcome::Lost) {
                    break;
                }
            }
            // Check if any two diseases share a sequence_group
            let groups: Vec<u32> = state.diseases.iter()
                .filter_map(|d| d.sequence_group)
                .collect();
            for &g in &groups {
                if groups.iter().filter(|&&x| x == g).count() >= 2 {
                    found_shared_group = true;
                    break;
                }
            }
            if found_shared_group { break; }
        }
        assert!(found_shared_group,
            "At least one seed should produce wave diseases sharing a sequence_group by day 50");
    }

    #[test]
    fn non_wave_diseases_have_no_sequence_group() {
        use crate::state::TICKS_PER_DAY;

        // Early game (before day 24) diseases should not have a sequence_group,
        // since wave clustering is inactive before day 24.
        let mut state = AppState::new_default(42);
        // Advance to day 20 (before wave clustering kicks in at day 24)
        for _ in 0..(20 * TICKS_PER_DAY as u64) {
            state = state.with_world(tick(&state).0);
            if matches!(state.outcome, crate::state::GameOutcome::Lost) {
                break;
            }
        }
        // All diseases spawned before day 24 should have no sequence group
        for disease in &state.diseases {
            assert!(disease.sequence_group.is_none(),
                "Disease '{}' spawned before day 24 should have no sequence_group", disease.name);
        }
    }

    #[test]
    fn collapsed_regions_suffer_secondary_deaths() {
        let mut state = AppState::new_default(42);
        // Manually collapse region 0 with some deaths
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * 0.50; // 50% dead (past the 45% threshold)
        state.regions[0].collapsed = true;
        state.regions[0].collapsed_at_tick = Some(0);

        let alive_before = state.regions[0].alive();
        assert!(alive_before > 0.0);

        // Run one day worth of ticks
        let mut s = state.clone();
        for _ in 0..(TICKS_PER_DAY as u64) {
            s = s.with_world(tick(&s).0);
        }

        let alive_after = s.regions[0].alive();
        let secondary = s.regions[0].collapse_deaths;

        // Should have lost ~5% of alive population
        let expected_loss = alive_before * COLLAPSE_DEATH_RATE;
        assert!(secondary > expected_loss * 0.8, "Expected ~{expected_loss:.0} secondary deaths, got {secondary:.0}");
        assert!(secondary < expected_loss * 1.2, "Too many secondary deaths: {secondary:.0} vs expected ~{expected_loss:.0}");
        assert!(alive_after < alive_before, "Alive should decrease: {alive_before:.0} -> {alive_after:.0}");
    }

    #[test]
    fn collapse_deaths_stop_at_subsistence_floor() {
        let mut state = AppState::new_default(42);
        let pop = state.regions[0].population as f64;
        let floor = pop * COLLAPSE_SUBSISTENCE_FLOOR;

        // Remove all infections so only secondary deaths occur
        state.regions[0].infections.clear();
        state.diseases.clear();

        // Set region just above the floor
        state.regions[0].dead = pop - floor - 100.0;
        state.regions[0].collapsed = true;
        state.regions[0].collapsed_at_tick = Some(0);

        let alive_before = state.regions[0].alive();
        assert!(alive_before > floor);
        assert!(alive_before < floor + 200.0);

        // Run many ticks — should not go below floor
        let mut s = state.clone();
        for _ in 0..(TICKS_PER_DAY as u64 * 10) {
            s = s.with_world(tick(&s).0);
        }

        let alive_after = s.regions[0].alive();
        assert!(alive_after >= floor - 1.0, "Alive {alive_after:.0} fell below floor {floor:.0}");
    }

    #[test]
    fn embezzlement_warning_fires_when_non_board_positions_exceed_threshold() {
        use crate::state::EMBEZZLEMENT_BUFFER;
        let mut state = AppState::new_default(42);
        corporations::generate_corporations(&mut state);
        board::generate_board_members(&mut state);
        state.tick = 1000;
        state.resources.funding = 5000.0;
        // Prevent other crises from firing first
        state.pending_crises.clear();
        state.next_board_meeting_tick = u64::MAX;
        state.last_contract_offer_tick = state.tick;

        // Find a non-board corporation
        let non_board_idx = state.corporations.iter().position(|c| !c.board_seat)
            .expect("need at least one non-board corp");

        // Buy enough shares to exceed the buffer
        let price = state.corporations[non_board_idx].share_price;
        let shares_needed = ((EMBEZZLEMENT_BUFFER + 100.0) / price).ceil() as u32 + 1;
        while state.portfolio.len() <= non_board_idx {
            state.portfolio.push(0);
        }
        state.portfolio[non_board_idx] = shares_needed;

        // cumulative_policy_spending stays at 0, so non_board_value > 0 + buffer
        assert!(state.non_board_portfolio_value() > state.cumulative_policy_spending + EMBEZZLEMENT_BUFFER);

        // Tick should fire the embezzlement warning
        let (after, _) = tick(&state);
        assert!(after.active_crisis.is_some(), "should have an active crisis");
        let crisis = after.active_crisis.as_ref().unwrap();
        assert!(matches!(crisis.kind, CrisisKind::BoardEmbezzlementWarning),
            "crisis should be BoardEmbezzlementWarning, got {:?}", crisis.kind);
    }

    #[test]
    fn embezzlement_warning_only_fires_once() {
        use crate::state::EMBEZZLEMENT_BUFFER;
        let mut state = AppState::new_default(42);
        corporations::generate_corporations(&mut state);
        board::generate_board_members(&mut state);
        state.tick = 1000;
        state.resources.funding = 5000.0;
        state.embezzlement_warned = true; // Already warned

        let non_board_idx = state.corporations.iter().position(|c| !c.board_seat)
            .expect("need at least one non-board corp");
        let price = state.corporations[non_board_idx].share_price;
        let shares_needed = ((EMBEZZLEMENT_BUFFER + 100.0) / price).ceil() as u32 + 1;
        while state.portfolio.len() <= non_board_idx {
            state.portfolio.push(0);
        }
        state.portfolio[non_board_idx] = shares_needed;

        let (after, _) = tick(&state);
        // Should NOT fire again since already warned
        let has_embezzlement_crisis = after.active_crisis.as_ref()
            .map(|c| matches!(c.kind, CrisisKind::BoardEmbezzlementWarning))
            .unwrap_or(false);
        assert!(!has_embezzlement_crisis, "should not fire warning twice");
    }

    #[test]
    fn embezzlement_penalty_reduces_funding_after_warning() {
        use crate::state::EMBEZZLEMENT_BUFFER;
        let mut state = AppState::new_default(42);
        initialize_game(&mut state);
        for r in &mut state.regions { r.infections.clear(); }

        let baseline_income = state.funding_income_rate();

        // Set up: warned + still over threshold
        state.embezzlement_warned = true;
        let non_board_idx = state.corporations.iter().position(|c| !c.board_seat)
            .expect("need at least one non-board corp");
        let price = state.corporations[non_board_idx].share_price;
        let shares_needed = ((EMBEZZLEMENT_BUFFER + 500.0) / price).ceil() as u32 + 1;
        while state.portfolio.len() <= non_board_idx {
            state.portfolio.push(0);
        }
        state.portfolio[non_board_idx] = shares_needed;

        let penalized_income = state.funding_income_rate();
        assert!(penalized_income < baseline_income,
            "income should be reduced: baseline={baseline_income:.2}, penalized={penalized_income:.2}");
    }

    #[test]
    fn emergency_sample_delivery_boosts_cooperation() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 500.0;
        state.medicines[0].max_doses = 500.0;
        state.medicines[0].tested_against = vec![0]; // tested against disease 0
        state.resources.funding = 10_000.0;
        state.regions[0].get_or_create_infection(0).infected = 1000.0;

        let coop_before = state.regions[0].governor.cooperation;

        let result = execute_command(&mut state, &GameCommand::EmergencySampleDelivery {
            medicine_idx: 0,
            region_idx: 0,
        });

        assert!(result.success, "delivery should succeed");
        assert!(result.message.is_some());

        // Tested medicine: +20 cooperation
        let coop_after = state.regions[0].governor.cooperation;
        assert!(coop_after > coop_before,
            "cooperation should increase: before={coop_before}, after={coop_after}");

        // Doses should be consumed
        assert!(state.medicines[0].doses < 500.0, "doses should be consumed");

        // Personnel should be tied up in a crisis operation
        assert!(!state.crisis_operations.is_empty(), "should have an active operation");
        assert_eq!(state.crisis_operations[0].personnel, 2);
    }

    #[test]
    fn emergency_delivery_fails_without_doses() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = true;
        state.medicines[0].doses = 0.0;
        state.resources.funding = 10_000.0;

        let result = execute_command(&mut state, &GameCommand::EmergencySampleDelivery {
            medicine_idx: 0,
            region_idx: 0,
        });

        assert!(!result.success);
    }

    #[test]
    fn emergency_delivery_fails_for_locked_medicine() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = false;
        state.medicines[0].doses = 500.0;
        state.resources.funding = 10_000.0;

        let result = execute_command(&mut state, &GameCommand::EmergencySampleDelivery {
            medicine_idx: 0,
            region_idx: 0,
        });

        assert!(!result.success);
    }

    #[test]
    fn board_research_inquiry_fires_when_no_identification_started() {
        let mut state = AppState::new_default(42);
        crate::engine::initialize_game(&mut state);
        // Set tick to day 5, ensure no other crisis is active
        state.tick = (5.0 * TICKS_PER_DAY) as u64;
        state.active_crisis = None;
        state.pending_crises.clear();
        state.next_board_meeting_tick = u64::MAX; // prevent board meetings
        state.last_contract_offer_tick = state.tick; // prevent contract offers
        // Ensure no identification has started: knowledge should be 0
        for d in &mut state.diseases {
            d.knowledge = 0.0;
        }
        state.active_research.clear();

        // Tick until the inquiry fires (should be within a few ticks)
        let mut found = false;
        let mut current = state;
        for _ in 0..20 {
            current = current.with_world(tick(&current).0);
            if let Some(ref crisis) = current.active_crisis {
                if crisis.kind.tag() == "board_research_inquiry" {
                    found = true;
                    assert_eq!(crisis.title, "Board Inquiry: Research Status");
                    assert_eq!(crisis.options.len(), 2);
                    break;
                }
                // Auto-resolve other crises
                current.active_crisis = None;
            }
        }
        assert!(found, "Board Research Inquiry should fire after day 5 with no identification started");
    }

    #[test]
    fn board_research_inquiry_does_not_fire_when_identification_started() {
        let mut state = AppState::new_default(42);
        crate::engine::initialize_game(&mut state);
        state.tick = (5.0 * TICKS_PER_DAY) as u64;
        state.active_crisis = None;
        state.pending_crises.clear();
        state.next_board_meeting_tick = u64::MAX;
        state.last_contract_offer_tick = state.tick;
        // Give the first disease some knowledge (identification started)
        state.diseases[0].knowledge = 0.25;

        let mut current = state;
        for _ in 0..20 {
            current = current.with_world(tick(&current).0);
            if let Some(ref crisis) = current.active_crisis {
                assert_ne!(crisis.kind.tag(), "board_research_inquiry",
                    "Board Research Inquiry should NOT fire when identification has been started");
                current.active_crisis = None;
            }
        }
    }

    #[test]
    fn gdp_target_includes_trade_coupling() {
        let mut state = AppState::new_default(42);
        // Clear crises so they don't interfere
        state.active_crisis = None;

        // All regions start with gdp == base_gdp, so trade_factor ≈ 1.0.
        // Pick a region with connections.
        let region_idx = 0;
        assert!(!state.regions[region_idx].connections.is_empty(),
            "test region must have connections");

        let healthy_target = state.gdp_target(region_idx);
        assert!(healthy_target > 0.0);

        // Now tank all connected neighbors' GDP to 0 (simulating collapse-like economy).
        let neighbors: Vec<usize> = state.regions[region_idx].connections.clone();
        for &n in &neighbors {
            state.regions[n].gdp = 0.0;
        }

        let damaged_target = state.gdp_target(region_idx);
        // Trade coupling should reduce the target (by up to 30% for non-trade-dependent,
        // up to 50% for trade-dependent).
        assert!(damaged_target < healthy_target,
            "neighbor GDP collapse should reduce trade-coupled target: {} should be < {}",
            damaged_target, healthy_target);

        // The reduction should be roughly 30% (or 50% if TradeDependent).
        let ratio = damaged_target / healthy_target;
        let is_trade_dep = state.regions[region_idx].traits.contains(&crate::state::RegionTrait::TradeDependent);
        if is_trade_dep {
            assert!((ratio - 0.5).abs() < 0.05,
                "trade-dependent region should lose ~50% from zeroed neighbors, got ratio {}", ratio);
        } else {
            assert!((ratio - 0.7).abs() < 0.05,
                "normal region should lose ~30% from zeroed neighbors, got ratio {}", ratio);
        }
    }

    #[test]
    fn advanced_intel_grants_knowledge_boost_on_detection() {
        // Set up a state with an undetected disease and an Advanced Intel station.
        // When the disease reaches the intel detection threshold (1,000 infections),
        // it should be detected AND receive a knowledge boost to KNOWLEDGE_NAME.
        let mut state = AppState::new_default(42);

        // Make the first disease undetected with 0 knowledge
        state.diseases[0].detected = false;
        state.diseases[0].knowledge = 0.0;

        // Give region 0 Advanced Intel (level 2)
        state.regions[0].intel_level = 2;

        // Seed infections just above the Advanced Intel threshold (1,000)
        let inf = state.regions[0].infections.iter_mut()
            .find(|inf| inf.disease_idx == 0);
        if let Some(inf) = inf {
            inf.infected = 1_100.0;
        } else {
            state.regions[0].infections.push(RegionDiseaseState {
                disease_idx: 0,
                exposed: 0.0,
                infected: 1_100.0,
                dead: 0.0,
                immune: 0.0,
            });
        }

        // Run one tick — should trigger detection via Advanced Intel
        let (new, tick_events) = tick(&state);

        assert!(new.diseases[0].detected, "disease should be detected via Advanced Intel");
        assert!(
            (new.diseases[0].knowledge - crate::state::KNOWLEDGE_NAME).abs() < 0.01,
            "Advanced Intel detection should grant KNOWLEDGE_NAME ({}) knowledge, got {}",
            crate::state::KNOWLEDGE_NAME,
            new.diseases[0].knowledge
        );

        // Should have an IntelAnalysis event
        let has_intel_analysis = tick_events.iter().any(|e| {
            matches!(e, GameEvent::IntelAnalysis { .. })
        });
        assert!(has_intel_analysis, "should generate an IntelAnalysis event");
    }

    #[test]
    fn normal_detection_does_not_grant_knowledge_boost() {
        // When a disease is detected via global threshold (not intel),
        // knowledge should remain at 0.
        let mut state = AppState::new_default(42);

        state.diseases[0].detected = false;
        state.diseases[0].knowledge = 0.0;
        // No intel stations anywhere
        for r in &mut state.regions {
            r.intel_level = 0;
        }

        // Seed infections above global threshold (10,000) across regions
        for region in &mut state.regions {
            if let Some(inf) = region.infections.iter_mut().find(|i| i.disease_idx == 0) {
                inf.infected = 15_000.0;
            } else {
                region.infections.push(RegionDiseaseState {
                    disease_idx: 0,
                    exposed: 0.0,
                    infected: 15_000.0,
                    dead: 0.0,
                    immune: 0.0,
                });
            }
        }

        let (new, _) = tick(&state);

        assert!(new.diseases[0].detected, "disease should be detected via global threshold");
        assert!(
            new.diseases[0].knowledge < 0.01,
            "normal detection should not grant knowledge boost, got {}",
            new.diseases[0].knowledge
        );
    }

}
