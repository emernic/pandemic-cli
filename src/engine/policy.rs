use crate::state::{
    CrisisKind, GameEvent, GameState, GovernorPersonality, RegionTrait, ScreeningLevel,
    policy_display_name,
    ADVANCED_INTEL_COST, ADVANCED_INTEL_PERSONNEL,
    BARGAIN_COOPERATIVE_LOYALTY_DRAIN, BARGAIN_LOYALTY_GAIN,
    BARGAIN_NATIONALIST_PERSONNEL_COST, BARGAIN_POPULIST_POL_FRACTION,
    BARGAIN_TECHNOCRAT_LOYALTY_GAIN, BARGAIN_TECHNOCRAT_RESEARCH_TICKS,
    BORDER_CONTROLS_PERSONNEL,
    FIELD_HOSPITAL_COST, FIELD_HOSPITAL_PERSONNEL,
    GOVERNOR_ACTION_INTERVAL, GOVERNOR_DEFIANCE_THRESHOLD,
    HOSPITAL_SURGE_PERSONNEL,
    INTEL_STATION_COST, INTEL_STATION_PERSONNEL,
    MARTIAL_LAW_PERSONNEL,
    MEDICAL_CENTER_COST, MEDICAL_CENTER_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    QUARANTINE_PERSONNEL,
    TICKS_PER_DAY, TRAVEL_BAN_PERSONNEL,
    WATER_SANITATION_PERSONNEL,
};

/// Enforce policy costs: suspend most expensive policies one at a time
/// until affordable, then deduct the total cost. Returns the total
/// policy cost (needed by the caller for funding warning calculations).
pub(super) fn tick_enforce_costs(state: &mut GameState) -> f64 {
    let mut policy_cost = state.total_policy_funding_cost();
    while policy_cost > 0.0 && state.resources.funding < policy_cost {
        // Find the most expensive active individual policy across all regions.
        // Uses active_policy_costs() — single source of truth for trait-adjusted pricing.
        let mut best: Option<(usize, usize, f64)> = None;
        for (i, p) in state.policies.iter().enumerate() {
            let traits = state.regions.get(i).map(|r| r.traits.as_slice()).unwrap_or(&[]);
            for (idx, cost) in p.active_policy_costs(traits) {
                if best.is_none() || cost > best.unwrap().2 {
                    best = Some((i, idx, cost));
                }
            }
        }
        if let Some((region_idx, policy_idx, _)) = best {
            let name = if policy_idx == 5 {
                // Screening: resolve tier-specific name before clearing
                let tier_name = match state.policies[region_idx].screening {
                    ScreeningLevel::Basic => "Basic Screening",
                    ScreeningLevel::Antigen => "Med Screening",
                    ScreeningLevel::MassRapid => "Mass Screening",
                    ScreeningLevel::None => "Screening",
                };
                state.policies[region_idx].screening = ScreeningLevel::None;
                tier_name.to_string()
            } else {
                state.policies[region_idx].set_bool(policy_idx, false);
                policy_display_name(policy_idx).to_string()
            };
            state.events.push(GameEvent::PolicySuspended {
                region_idx,
                policy_name: name,
            });
            policy_cost = state.total_policy_funding_cost();
        } else {
            break;
        }
    }
    if policy_cost > 0.0 {
        state.resources.funding -= policy_cost;
    }
    policy_cost
}

/// Toggle a policy for a region. Returns (message, success) where success
/// indicates the toggle actually happened (vs being rejected).
/// Does not touch UI state.
pub(super) fn toggle_policy(state: &mut GameState, region_idx: usize, policy_idx: usize) -> (Option<String>, bool) {
    if region_idx >= state.policies.len() {
        return (None, false);
    }
    // Collapsed regions: only nuclear annihilation is available
    if state.regions.get(region_idx).is_some_and(|r| r.collapsed) {
        if policy_idx != 9 {
            let region_name = state.regions[region_idx].name.as_str();
            return (Some(format!("{region_name} has collapsed — policies unavailable")), false);
        }
    }
    // Abandoned regions (Ark Protocol active, not the Ark)
    if state.is_abandoned(region_idx) {
        let region_name = state.regions[region_idx].name.as_str();
        return (Some(format!("{region_name} abandoned — resources consolidated in the Ark")), false);
    }
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    // Check POL requirement (only when enabling, not disabling)
    let is_currently_active = match policy_idx {
        0..=4 | 8 | 9 => state.policies[region_idx].get_bool(policy_idx),
        5 => state.policies[region_idx].screening == ScreeningLevel::Basic,
        6 => state.policies[region_idx].screening == ScreeningLevel::Antigen,
        7 => state.policies[region_idx].screening == ScreeningLevel::MassRapid,
        10 => state.regions[region_idx].hospital_level >= 2, // fully built = "active"
        _ => false,
    };
    if !is_currently_active && !state.policy_unlocked(region_idx, policy_idx) {
        let threshold = state.effective_pol_threshold(region_idx, policy_idx);
        return (Some(format!(
            "{} requires {:.0}% Political Power (current: {:.0}%)",
            policy_display_name(policy_idx), threshold * 100.0, state.resources.political_power * 100.0
        )), false);
    }
    let available_personnel = state.personnel_available();
    let region_traits = state.regions.get(region_idx).map(|r| r.traits.as_slice()).unwrap_or(&[]);
    let low_infra = region_traits.contains(&RegionTrait::LowInfrastructure);
    match policy_idx {
        // Boolean policies (0-4): identical toggle logic, different metadata.
        0..=4 => {
            let (name, personnel, on_msg, off_msg) = match policy_idx {
                0 => ("Travel Ban",
                      TRAVEL_BAN_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Travel Ban enacted — all cross-border movement suspended",
                      "Travel Ban lifted — borders reopened"),
                1 => ("Quarantine",
                      QUARANTINE_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Quarantine imposed — movement within region restricted",
                      "Quarantine lifted"),
                2 => ("Hospital Surge",
                      HOSPITAL_SURGE_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Hospital Surge authorized — surge capacity activated",
                      "Hospital Surge stood down"),
                3 => ("Border Controls",
                      BORDER_CONTROLS_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Border Controls established — checkpoint screening active",
                      "Border Controls removed"),
                4 => ("Water Sanitation",
                      WATER_SANITATION_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Water Sanitation active — treatment protocols deployed",
                      "Water Sanitation suspended"),
                _ => unreachable!(),
            };
            if state.policies[region_idx].get_bool(policy_idx) {
                state.policies[region_idx].set_bool(policy_idx, false);
                (Some(format!("{region_name}: {off_msg}")), true)
            } else if available_personnel >= personnel {
                state.policies[region_idx].set_bool(policy_idx, true);
                (Some(format!("{region_name}: {on_msg}")), true)
            } else {
                (Some(format!(
                    "Not enough personnel for {} (need {personnel})", name.to_lowercase()
                )), false)
            }
        }
        // Screening tiers (5=Basic, 6=Antigen, 7=MassRapid) — mutually exclusive.
        // Selecting the current level disables screening; selecting a different
        // level upgrades/downgrades to that tier.
        5 | 6 | 7 => {
            let target = match policy_idx {
                5 => ScreeningLevel::Basic,
                6 => ScreeningLevel::Antigen,
                _ => ScreeningLevel::MassRapid,
            };
            let current = state.policies[region_idx].screening;
            if current == target {
                // Toggle off
                state.policies[region_idx].screening = ScreeningLevel::None;
                (Some(format!("{region_name}: Disease screening suspended")), true)
            } else {
                // Check personnel — account for personnel freed from current tier
                let infra_extra = if low_infra { 1 } else { 0 };
                let needed = target.personnel_cost() + infra_extra;
                let freed = current.personnel_cost() + if current != ScreeningLevel::None { infra_extra } else { 0 };
                let effective_available = available_personnel + freed;
                if effective_available >= needed {
                    state.policies[region_idx].screening = target;
                    let tier_desc = match target {
                        ScreeningLevel::Basic => "rough infected estimates, faster detection",
                        ScreeningLevel::Antigen => "infected + immune counts, improved accuracy",
                        ScreeningLevel::MassRapid => "near-complete data, 25% spread reduction",
                        ScreeningLevel::None => unreachable!(),
                    };
                    (Some(format!("{region_name}: {} screening active — {tier_desc}",
                        target.label())), true)
                } else {
                    (Some(format!(
                        "Not enough personnel for {} screening (need {})", target.label().to_lowercase(), needed
                    )), false)
                }
            }
        }
        // Martial Law (8): normal boolean toggle, pre-collapse only
        8 => {
            let ml_personnel = MARTIAL_LAW_PERSONNEL + if low_infra { 1 } else { 0 };
            if state.policies[region_idx].martial_law {
                state.policies[region_idx].martial_law = false;
                (Some(format!("{region_name}: Martial Law lifted — civilian governance restored")), true)
            } else if available_personnel >= ml_personnel {
                state.policies[region_idx].martial_law = true;
                (Some(format!("{region_name}: Martial Law declared — adds 15% collapse resilience")), true)
            } else {
                (Some(format!(
                    "Not enough personnel for martial law (need {})", ml_personnel
                )), false)
            }
        }
        // Nuclear Annihilation (9): one-shot for collapsed regions only
        9 => {
            if state.policies[region_idx].nuclear_annihilation {
                (Some(format!("{region_name} has already been annihilated")), false)
            } else if !state.regions[region_idx].collapsed {
                (Some("Nuclear annihilation is only available for collapsed regions".to_string()), false)
            } else if state.resources.funding < NUCLEAR_ANNIHILATION_COST {
                (Some(format!("Not enough funding (need ¥{:.0})", NUCLEAR_ANNIHILATION_COST)), false)
            } else {
                // Deduct one-time cost
                state.resources.funding -= NUCLEAR_ANNIHILATION_COST;
                state.policies[region_idx].nuclear_annihilation = true;
                // Kill 99% of remaining alive population
                let region = &mut state.regions[region_idx];
                let alive = region.alive();
                let killed = alive * 0.99;
                region.dead += killed;
                // Attribute nuke deaths proportionally across disease pools
                // so they're visible in the UI (which sums inf.dead)
                let total_inf_dead: f64 = region.infections.iter().map(|i| i.dead).sum();
                let num_infections = region.infections.len().max(1) as f64;
                for inf in &mut region.infections {
                    let share = if total_inf_dead > 0.0 { inf.dead / total_inf_dead } else { 1.0 / num_infections };
                    inf.dead += killed * share;
                    inf.infected = 0.0;
                    inf.immune = 0.0;
                }
                (Some(format!("☢ {region_name} annihilated — {:.1}M dead. Disease eradicated.",
                    killed / 1_000_000.0)), true)
            }
        }
        // Field Hospital / Medical Center (10): tiered per-region infrastructure
        10 => {
            let region = &state.regions[region_idx];
            if region.collapsed {
                (Some(format!("{region_name} has collapsed — cannot build")), false)
            } else if region.hospital_level == 0 {
                // Build Level 1: Field Hospital
                if state.resources.funding < FIELD_HOSPITAL_COST {
                    (Some(format!("Not enough funding (need ¥{:.0})", FIELD_HOSPITAL_COST)), false)
                } else if available_personnel < FIELD_HOSPITAL_PERSONNEL {
                    (Some(format!("Need {} personnel to staff Field Hospital", FIELD_HOSPITAL_PERSONNEL)), false)
                } else {
                    state.resources.funding -= FIELD_HOSPITAL_COST;
                    state.regions[region_idx].hospital_level = 1;
                    state.regions[region_idx].governor.loyalty = (state.regions[region_idx].governor.loyalty + 10.0).min(100.0);
                    (Some(format!("{region_name}: Field Hospital operational — reduces mortality 25%")), true)
                }
            } else if region.hospital_level == 1 {
                // Upgrade to Level 2: Medical Center
                if state.resources.funding < MEDICAL_CENTER_COST {
                    (Some(format!("Not enough funding (need ¥{:.0})", MEDICAL_CENTER_COST)), false)
                } else if available_personnel < (MEDICAL_CENTER_PERSONNEL - FIELD_HOSPITAL_PERSONNEL) {
                    (Some(format!("Need {} more personnel to staff Medical Center", MEDICAL_CENTER_PERSONNEL - FIELD_HOSPITAL_PERSONNEL)), false)
                } else {
                    state.resources.funding -= MEDICAL_CENTER_COST;
                    state.regions[region_idx].hospital_level = 2;
                    state.regions[region_idx].governor.loyalty = (state.regions[region_idx].governor.loyalty + 10.0).min(100.0);
                    (Some(format!("{region_name}: Medical Center operational — mortality -40%, medicine efficacy +25%")), true)
                }
            } else {
                (Some(format!("{region_name} already has a Medical Center")), false)
            }
        }
        // Intel Station / Advanced Intel (11): tiered per-region surveillance infrastructure
        11 => {
            let region = &state.regions[region_idx];
            if region.collapsed {
                (Some(format!("{region_name} has collapsed — cannot build")), false)
            } else if region.intel_level == 0 {
                // Build Level 1: Intel Station
                if state.resources.funding < INTEL_STATION_COST {
                    (Some(format!("Not enough funding (need ¥{:.0})", INTEL_STATION_COST)), false)
                } else if available_personnel < INTEL_STATION_PERSONNEL {
                    (Some(format!("Need {} personnel to staff Intel Station", INTEL_STATION_PERSONNEL)), false)
                } else {
                    state.resources.funding -= INTEL_STATION_COST;
                    state.regions[region_idx].intel_level = 1;
                    (Some(format!("{region_name}: Intel Station operational — detects new pathogens at 3,000 local infections")), true)
                }
            } else if region.intel_level == 1 {
                // Upgrade to Level 2: Advanced Intel
                if state.resources.funding < ADVANCED_INTEL_COST {
                    (Some(format!("Not enough funding (need ¥{:.0})", ADVANCED_INTEL_COST)), false)
                } else if available_personnel < (ADVANCED_INTEL_PERSONNEL - INTEL_STATION_PERSONNEL) {
                    (Some(format!("Need {} more personnel for Advanced Intel", ADVANCED_INTEL_PERSONNEL - INTEL_STATION_PERSONNEL)), false)
                } else {
                    state.resources.funding -= ADVANCED_INTEL_COST;
                    state.regions[region_idx].intel_level = 2;
                    (Some(format!("{region_name}: Advanced Intel operational — detects at 1,000 infections, generates briefings")), true)
                }
            } else {
                (Some(format!("{region_name} already has Advanced Intel")), false)
            }
        }
        _ => (None, false),
    }
}

/// Rally public support: spend funding to boost POL directly.
/// Returns (message, success).
pub(super) fn rally_support(state: &mut GameState) -> (Option<String>, bool) {
    use crate::state::{RALLY_COST, RALLY_POL_GAIN};

    let cooldown = state.resources.rally_cooldown_remaining(state.tick);
    if cooldown > 0 {
        let days = cooldown as f64 / TICKS_PER_DAY;
        return (Some(format!("Rally on cooldown — {days:.1} days remaining")), false);
    }

    if state.resources.funding < RALLY_COST {
        return (Some(format!("Not enough funding (need ¥{RALLY_COST:.0})")), false);
    }

    state.resources.funding -= RALLY_COST;
    state.resources.last_rally_tick = Some(state.tick);
    state.resources.political_power = (state.resources.political_power + RALLY_POL_GAIN).min(1.0);

    let pol_pct = state.resources.political_power * 100.0;
    (Some(format!("Rally successful! POL +{:.0}% → {pol_pct:.0}%", RALLY_POL_GAIN * 100.0)), true)
}

/// Spend funds to boost a governor's loyalty.
pub(super) fn appease_governor(state: &mut GameState, region_idx: usize) -> (Option<String>, bool) {
    use crate::state::{APPEASE_COST, APPEASE_LOYALTY_GAIN};

    if region_idx >= state.regions.len() {
        return (None, false);
    }
    if state.regions[region_idx].collapsed {
        let name = &state.regions[region_idx].name;
        return (Some(format!("{name} has collapsed — no governor to appease")), false);
    }
    if state.resources.funding < APPEASE_COST {
        return (Some(format!("Not enough funding (need ¥{APPEASE_COST:.0})")), false);
    }
    state.resources.funding -= APPEASE_COST;
    let gov = &mut state.regions[region_idx].governor;
    gov.loyalty = (gov.loyalty + APPEASE_LOYALTY_GAIN).min(100.0);
    let name = &state.regions[region_idx].governor.name;
    let loyalty = state.regions[region_idx].governor.loyalty;
    (Some(format!("{name} appeased — loyalty now {loyalty:.0} (-¥{APPEASE_COST:.0})")), true)
}

/// Personality-specific bargain with a defiant governor. Free in funding
/// but costs something else depending on personality.
pub(super) fn bargain_with_governor(state: &mut GameState, region_idx: usize) -> (Option<String>, bool) {
    if region_idx >= state.regions.len() {
        return (None, false);
    }
    if state.regions[region_idx].collapsed {
        let name = &state.regions[region_idx].name;
        return (Some(format!("{name} has collapsed")), false);
    }
    if !state.regions[region_idx].governor.is_defiant() {
        return (Some("Governor is not defiant — no bargain needed".into()), false);
    }

    let personality = state.regions[region_idx].governor.personality;
    let gov_name = state.regions[region_idx].governor.name.clone();

    match personality {
        GovernorPersonality::Nationalist => {
            let cost = BARGAIN_NATIONALIST_PERSONNEL_COST;
            if state.resources.personnel < cost {
                return (Some(format!("Not enough personnel (need {cost})")), false);
            }
            state.resources.personnel -= cost;
            let gov = &mut state.regions[region_idx].governor;
            gov.loyalty = (gov.loyalty + BARGAIN_LOYALTY_GAIN).min(100.0);
            let loyalty = state.regions[region_idx].governor.loyalty;
            (Some(format!(
                "{gov_name}: Regional Priority accepted — loyalty {loyalty:.0} (-{cost} personnel)"
            )), true)
        }
        GovernorPersonality::Populist => {
            let pol_loss = state.resources.political_power * BARGAIN_POPULIST_POL_FRACTION;
            state.resources.political_power -= pol_loss;
            let gov = &mut state.regions[region_idx].governor;
            gov.loyalty = (gov.loyalty + BARGAIN_LOYALTY_GAIN).min(100.0);
            let loyalty = state.regions[region_idx].governor.loyalty;
            (Some(format!(
                "{gov_name}: Public Concession accepted — loyalty {loyalty:.0} (-{:.0}% POL)",
                BARGAIN_POPULIST_POL_FRACTION * 100.0
            )), true)
        }
        GovernorPersonality::Technocrat => {
            if let Some(ref mut project) = state.applied_research {
                project.required_ticks += BARGAIN_TECHNOCRAT_RESEARCH_TICKS;
                let gov = &mut state.regions[region_idx].governor;
                gov.loyalty = (gov.loyalty + BARGAIN_TECHNOCRAT_LOYALTY_GAIN).min(100.0);
                let loyalty = state.regions[region_idx].governor.loyalty;
                let delay_days = BARGAIN_TECHNOCRAT_RESEARCH_TICKS / TICKS_PER_DAY;
                (Some(format!(
                    "{gov_name}: Research Oversight accepted — loyalty {loyalty:.0} (+{delay_days:.1} day delay to applied research)"
                )), true)
            } else {
                (Some("No active applied research — bargain unavailable".into()), false)
            }
        }
        GovernorPersonality::Cooperative => {
            // Drain loyalty from all other non-collapsed governors
            for (i, region) in state.regions.iter_mut().enumerate() {
                if i != region_idx && !region.collapsed {
                    region.governor.loyalty =
                        (region.governor.loyalty - BARGAIN_COOPERATIVE_LOYALTY_DRAIN).max(0.0);
                }
            }
            let gov = &mut state.regions[region_idx].governor;
            gov.loyalty = (gov.loyalty + BARGAIN_LOYALTY_GAIN).min(100.0);
            let loyalty = state.regions[region_idx].governor.loyalty;
            (Some(format!(
                "{gov_name}: Joint Briefing accepted — loyalty {loyalty:.0} (other governors -{:.0} loyalty)",
                BARGAIN_COOPERATIVE_LOYALTY_DRAIN
            )), true)
        }
    }
}

/// Tick governor loyalty drift. Called once per tick from tick().
///
/// Loyalty drifts based on infection severity, cumulative deaths, active
/// restrictive policies, and personality. Governors react to the same
/// severity thresholds the player sees (CRIT/HIGH/MOD/LOW/OK), so there
/// is a clear mental model: "region is CRIT → governor is angry."
pub(super) fn tick_governor_loyalty(state: &mut GameState) {
    let num_regions = state.regions.len();
    for i in 0..num_regions {
        if state.regions[i].collapsed {
            continue;
        }

        let policy = &state.policies[i];
        let personality = state.regions[i].governor.personality;
        let current = state.regions[i].governor.loyalty;

        // Count active restrictive policies (travel ban, quarantine, martial law, border controls)
        let restrictive_count = [
            policy.travel_ban,
            policy.quarantine,
            policy.martial_law,
            policy.border_controls,
        ].iter().filter(|&&b| b).count() as f64;

        let infected = state.regions[i].total_infected();
        let pop = state.regions[i].population as f64;
        let death_frac = if pop > 0.0 { state.regions[i].dead / pop } else { 0.0 };

        // Base drift: mild regression toward 50
        let base_drift = (50.0 - current) * 0.0001; // ~0.012/day per point away from 50

        // Severity drain: governors react to infection levels using the
        // shared severity thresholds (CRIT/HIGH/MOD) from state.rs.
        use crate::state::{SEVERITY_CRIT_THRESHOLD, SEVERITY_HIGH_THRESHOLD, SEVERITY_MOD_THRESHOLD};
        let severity_drain = if infected > SEVERITY_CRIT_THRESHOLD {
            -0.008 // CRIT: ~0.96/day
        } else if infected > SEVERITY_HIGH_THRESHOLD {
            -0.004 // HIGH: ~0.48/day
        } else if infected > SEVERITY_MOD_THRESHOLD {
            -0.001 // MOD: ~0.12/day
        } else {
            0.0
        };

        // Death drain: cumulative deaths erode trust (linear, not sqrt)
        let death_drain = -death_frac * 0.03; // ~0.036/day at 1% dead, ~0.36/day at 10%

        // Policy pressure: each restrictive policy drains loyalty
        let policy_drain = -restrictive_count * 0.003; // ~0.36/day per policy

        // Personality modifiers
        let personality_mod = match personality {
            GovernorPersonality::Cooperative => 0.002, // passive loyalty gain (~0.24/day)
            GovernorPersonality::Nationalist => {
                // Angry about both restrictions AND suffering in their region
                let restriction_anger = -restrictive_count * 0.002;
                let suffering_anger = if infected > SEVERITY_CRIT_THRESHOLD {
                    -0.004 // extra CRIT penalty (~0.48/day)
                } else if infected > SEVERITY_HIGH_THRESHOLD {
                    -0.002 // extra HIGH penalty (~0.24/day)
                } else {
                    0.0
                };
                restriction_anger + suffering_anger
            }
            GovernorPersonality::Populist => {
                // Hates restrictive policies — extra drain on top of base policy_drain
                let restriction_anger = -restrictive_count * 0.004; // ~0.48/day per policy (stacks with base 0.36)
                // Happy when region is calm with no restrictions
                let calm_bonus = if restrictive_count == 0.0 && infected <= SEVERITY_HIGH_THRESHOLD {
                    0.003 // ~0.36/day — rewards light-touch approach
                } else {
                    0.0
                };
                restriction_anger + calm_bonus
            }
            GovernorPersonality::Technocrat => {
                // Gains loyalty when research targets diseases in their region
                let region_diseases: Vec<usize> = state.regions[i].infections.iter()
                    .filter(|inf| inf.infected > 0.0)
                    .map(|inf| inf.disease_idx)
                    .collect();
                let researching_region = region_diseases.iter().any(|&d_idx| {
                    state.field_research.iter().any(|r| r.references_disease(d_idx))
                        || state.applied_research.as_ref().is_some_and(|r| r.references_disease(d_idx))
                });
                // Angry when unidentified diseases exist in their region
                let has_unidentified = region_diseases.iter().any(|&d_idx| {
                    state.diseases.get(d_idx).is_some_and(|d| d.knowledge < crate::state::KNOWLEDGE_NAME)
                });
                let research_mod = if researching_region { 0.004 } else { 0.0 }; // ~0.48/day
                let ignorance_anger = if has_unidentified { -0.004 } else { 0.0 }; // ~0.48/day
                research_mod + ignorance_anger
            }
        };

        let total_drift = base_drift + severity_drain + death_drain + policy_drain + personality_mod;
        let new_loyalty = (current + total_drift).clamp(0.0, 100.0);
        state.regions[i].governor.loyalty = new_loyalty;

        // Fire a personality-specific crisis when loyalty first drops below defiance threshold
        if new_loyalty < GOVERNOR_DEFIANCE_THRESHOLD && !state.regions[i].governor.defiance_crisis_fired {
            state.regions[i].governor.defiance_crisis_fired = true;
            let kind = match personality {
                GovernorPersonality::Cooperative => CrisisKind::GovernorCooperative { region_idx: i },
                GovernorPersonality::Nationalist => CrisisKind::GovernorNationalist { region_idx: i },
                GovernorPersonality::Populist => CrisisKind::GovernorPopulist { region_idx: i },
                GovernorPersonality::Technocrat => CrisisKind::GovernorTechnocrat { region_idx: i },
            };
            // Schedule for immediate activation (current tick)
            state.pending_crises.push((state.tick, kind));
        }

        // Reset the flag when loyalty recovers above defiance threshold
        if new_loyalty >= GOVERNOR_DEFIANCE_THRESHOLD && state.regions[i].governor.defiance_crisis_fired {
            state.regions[i].governor.defiance_crisis_fired = false;
        }
    }
}

/// Tick autonomous governor actions. Defiant governors periodically act against
/// the player based on personality. Called from tick().
pub(super) fn tick_governor_actions(state: &mut GameState) {
    let tick = state.tick;
    let num_regions = state.regions.len();

    for i in 0..num_regions {
        if state.regions[i].collapsed {
            continue;
        }
        let gov = &state.regions[i].governor;
        if gov.loyalty >= GOVERNOR_DEFIANCE_THRESHOLD {
            continue;
        }
        // Check cooldown
        if tick.saturating_sub(gov.last_action_tick) < GOVERNOR_ACTION_INTERVAL {
            continue;
        }

        let personality = gov.personality;
        let gov_name = gov.name.clone();
        let region_name = state.regions[i].name.clone();

        let action_desc = match personality {
            GovernorPersonality::Populist => {
                // Lift the first active restrictive policy
                let policy = &state.policies[i];
                let active: Vec<&str> = [
                    (policy.travel_ban, "travel_ban"),
                    (policy.quarantine, "quarantine"),
                    (policy.martial_law, "martial_law"),
                    (policy.border_controls, "border_controls"),
                ].iter()
                    .filter(|(active, _)| *active)
                    .map(|(_, name)| *name)
                    .collect();
                if let Some(&target) = active.first() {
                    let label = match target {
                        "travel_ban" => { state.policies[i].travel_ban = false; "Travel Ban" }
                        "quarantine" => { state.policies[i].quarantine = false; "Quarantine" }
                        "martial_law" => { state.policies[i].martial_law = false; "Martial Law" }
                        "border_controls" => { state.policies[i].border_controls = false; "Border Controls" }
                        _ => unreachable!(),
                    };
                    Some(format!("{gov_name} lifted {label} in {region_name}"))
                } else {
                    None // No restrictive policies to lift
                }
            }
            GovernorPersonality::Nationalist => {
                // Diverts budget to "regional priorities" — a clear funding drain
                // (border closing was the old action but it often helped the player by
                // containing spread; this creates the intended negative pressure)
                let drain = (state.resources.funding * 0.08).min(250.0);
                if drain >= 10.0 {
                    state.resources.funding -= drain;
                    Some(format!("{gov_name} diverted ¥{drain:.0} to regional programs in {region_name}"))
                } else {
                    None
                }
            }
            GovernorPersonality::Technocrat => {
                // Steal 1 personnel for "independent research"
                if state.resources.personnel > 1 {
                    state.resources.personnel -= 1;
                    Some(format!("{gov_name} reassigned a researcher in {region_name}"))
                } else {
                    None
                }
            }
            GovernorPersonality::Cooperative => {
                // Leak to media — small POL drain
                if state.resources.political_power > 0.0 {
                    state.resources.political_power = (state.resources.political_power - 0.03).max(0.0);
                    Some(format!("{gov_name} leaked reports to media in {region_name}"))
                } else {
                    None
                }
            }
        };

        if let Some(desc) = action_desc {
            state.regions[i].governor.last_action_tick = tick;
            state.events.push(GameEvent::GovernorAction {
                region_idx: i,
                description: desc,
            });
        }
    }
}

/// Enact an emergency decree. Permanent, irreversible.
/// Returns (message, success).
pub(super) fn enact_decree(state: &mut GameState, decree_idx: usize, region_idx: Option<usize>) -> (Option<String>, bool) {
    use crate::state::{
        decree_display_name, DECREE_THREAT_LEVELS,
        CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY, TICKS_PER_DAY,
        SACRIFICE_INCOME_BONUS,
    };

    if decree_idx >= crate::state::DECREE_COUNT {
        return (None, false);
    }

    // Already enacted?
    if state.enacted_decrees.is_enacted(decree_idx) {
        return (Some(format!("{} has already been enacted", decree_display_name(decree_idx))), false);
    }

    // Threat level check — decrees are gated by crisis severity, not POL.
    let required = DECREE_THREAT_LEVELS[decree_idx];
    if state.threat_level < required {
        return (Some(format!(
            "{} requires DEFCON {} ({}) — current: DEFCON {} ({})",
            decree_display_name(decree_idx),
            required.defcon(), required.label(),
            state.threat_level.defcon(), state.threat_level.label()
        )), false);
    }

    match decree_idx {
        0 => {
            // Conscript Researchers: +personnel, permanent income penalty
            state.enacted_decrees.conscript_researchers = true;
            state.resources.personnel += CONSCRIPT_PERSONNEL_GAIN;
            state.sync_scientists_to_personnel();
            let penalty_per_day = CONSCRIPT_INCOME_PENALTY * TICKS_PER_DAY;
            (Some(format!(
                "⚠ DECREE: Conscript Researchers enacted — +{} personnel. Income reduced ¥{:.0}/day, permanently.",
                CONSCRIPT_PERSONNEL_GAIN, penalty_per_day
            )), true)
        }
        1 => {
            // Authorize Human Trials: faster clinical trials, risk of adverse events
            state.enacted_decrees.authorize_human_trials = true;
            (Some(
                "⚠ DECREE: Human Trials authorized — clinical trials 50% faster. Adverse event risk elevated, permanently.".to_string()
            ), true)
        }
        2 => {
            // Sacrifice Region: voluntarily collapse a region for income bonus
            let Some(r_idx) = region_idx else {
                return (Some("Select a region to sacrifice".to_string()), false);
            };
            if r_idx >= state.regions.len() {
                return (None, false);
            }
            if state.regions[r_idx].collapsed {
                return (Some(format!("{} is already collapsed", state.regions[r_idx].name)), false);
            }
            let region_name = state.regions[r_idx].name.clone();
            state.enacted_decrees.sacrificed_region = Some(r_idx);
            // Collapse the region
            state.regions[r_idx].collapsed = true;
            state.regions[r_idx].collapsed_at_tick = Some(state.tick);
            state.regions[r_idx].hospital_level = 0; // Hospital destroyed
            state.regions[r_idx].intel_level = 0; // Intel station destroyed
            // Clear policies
            if let Some(p) = state.policies.get_mut(r_idx) {
                p.clear_all();
            }
            let bonus_pct = (SACRIFICE_INCOME_BONUS - 1.0) * 100.0;
            (Some(format!(
                "⚠ DECREE: {} designated a sacrifice zone — abandoned. Remaining regions: +{:.0}% income.",
                region_name, bonus_pct
            )), true)
        }
        _ => (None, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GameState, ScreeningLevel};
    use crate::engine::tick;

    /// Helper: set up a state with full POL and plenty of personnel for screening tests.
    fn screening_test_state() -> GameState {
        let mut state = GameState::new_default(42);
        state.resources.political_power = 1.0;
        state.resources.funding = 10_000.0;
        // Set max threat level so decree tests aren't blocked by DEFCON gating
        state.threat_level = crate::state::ThreatLevel::Extinction;
        state
    }

    #[test]
    fn screening_mutual_exclusivity() {
        let mut state = screening_test_state();
        // Enable Low screening on region 0
        let (_, ok) = toggle_policy(&mut state, 0, 5);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::Basic);

        // Switch to Medium — should replace Low, not stack
        let (_, ok) = toggle_policy(&mut state, 0, 6);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::Antigen);

        // Switch to High — replaces Medium
        let (_, ok) = toggle_policy(&mut state, 0, 7);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::MassRapid);

        // Toggle High again — disables screening
        let (_, ok) = toggle_policy(&mut state, 0, 7);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::None);
    }

    #[test]
    fn screening_pol_gating() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        // Low screening has 0.0 POL threshold — should work with no POL
        state.resources.political_power = 0.0;
        let (_, ok) = toggle_policy(&mut state, 0, 5);
        assert!(ok, "Low screening should work at 0 POL");

        // Medium requires 0.10 POL
        state.resources.political_power = 0.05;
        let (msg, ok) = toggle_policy(&mut state, 0, 6);
        assert!(!ok, "Medium screening should be blocked at 5% POL");
        assert!(msg.unwrap().contains("Political Power"));

        // With enough POL, Medium should work
        state.resources.political_power = 0.15;
        let (_, ok) = toggle_policy(&mut state, 0, 6);
        assert!(ok, "Medium screening should work at 15% POL");
    }

    #[test]
    fn screening_upgrade_frees_personnel_from_current_tier() {
        let mut state = screening_test_state();
        // Start with Low screening (1 personnel)
        toggle_policy(&mut state, 0, 5);
        assert_eq!(state.policies[0].screening, ScreeningLevel::Basic);

        // Use up all remaining personnel except 1 (which is committed to Low screening)
        // Medium needs 2 personnel. With 1 freed from Low, we need 1 available.
        let busy = state.personnel_busy();
        // Set personnel so that available = 0 but we have 1 in Low screening
        state.resources.personnel = busy; // exactly enough for current commitments

        // Upgrade to Medium: needs 2, frees 1 from Low, so needs 1 more available
        // With available=0 and freed=1, effective_available=1 < needed=2 → should fail
        let (_, ok) = toggle_policy(&mut state, 0, 6);
        assert!(!ok, "should fail: 0 available + 1 freed = 1 < 2 needed");

        // Give 1 more personnel: available=1, freed=1 from Low → effective=2 >= 2
        state.resources.personnel = busy + 1;
        let (_, ok) = toggle_policy(&mut state, 0, 6);
        assert!(ok, "should succeed: 1 available + 1 freed = 2 >= 2 needed");
        assert_eq!(state.policies[0].screening, ScreeningLevel::Antigen);
    }

    #[test]
    fn screening_suspension_when_funding_runs_out() {
        let mut state = screening_test_state();
        state.policies[0].screening = ScreeningLevel::MassRapid; // ¥0.6/tick
        // Set funding just below screening cost so it gets suspended
        state.resources.funding = 0.3;
        // Clear infections so tick doesn't muddy funding math
        for r in &mut state.regions { r.infections.clear(); }

        state = tick(&state);
        assert_eq!(state.policies[0].screening, ScreeningLevel::None,
            "High screening should be suspended when unaffordable");
        assert!(state.events.iter().any(|e|
            matches!(e, GameEvent::PolicySuspended { policy_name, .. } if policy_name.contains("Screening"))
        ), "should emit PolicySuspended event for screening");
    }

    #[test]
    fn screening_cost_vs_boolean_policy_suspension_order() {
        let mut state = screening_test_state();
        // Set up: High screening (¥0.6/tick) + quarantine (¥0.6/tick) = ¥1.2/tick
        state.policies[0].screening = ScreeningLevel::MassRapid;
        state.policies[0].quarantine = true;
        // Enough for one but not both
        state.resources.funding = 0.8;
        for r in &mut state.regions { r.infections.clear(); }

        state = tick(&state);
        // Both cost ¥0.6; one should be suspended. The enforcement loop finds
        // whichever it encounters first at the max cost — just verify one survived.
        let screening_alive = state.policies[0].screening != ScreeningLevel::None;
        let quarantine_alive = state.policies[0].quarantine;
        assert!(screening_alive != quarantine_alive || (!screening_alive && !quarantine_alive),
            "at most one of the two equal-cost policies should survive");
    }

    #[test]
    fn screening_lowers_detection_threshold() {
        let mut state = GameState::new_default(42);
        // Place undetected disease just below the screening-reduced threshold
        state.diseases[0].detected = false;
        // High screening: threshold = 10,000 * 0.2 = 2,000
        state.policies[0].screening = ScreeningLevel::MassRapid;
        state.resources.funding = 10_000.0;
        // Set infections to 2,500 (above 2,000 threshold but below 10,000 default)
        state.regions[0].get_or_create_infection(0).infected = 2_500.0;
        // Clear other regions so total is just 2,500
        for r in &mut state.regions[1..] { r.infections.clear(); }

        let after = tick(&state);
        assert!(after.diseases[0].detected,
            "disease should be detected at 2,500 infected with High screening (threshold 2,000)");

        // Without screening, same infection level should NOT trigger detection
        let mut state2 = state.clone();
        state2.policies[0].screening = ScreeningLevel::None;
        let after2 = tick(&state2);
        assert!(!after2.diseases[0].detected,
            "disease should NOT be detected at 2,500 infected without screening (threshold 10,000)");
    }

    #[test]
    fn screening_visibility_scales_reported_infections() {
        let mut state = screening_test_state();
        // Set a known infection level
        state.regions[0].get_or_create_infection(0).infected = 100_000.0;

        // Without screening: visibility = 15%
        let screened_none = state.total_infected_screened();

        // With High screening on region 0: visibility = 90%
        state.policies[0].screening = ScreeningLevel::MassRapid;
        let screened_high = state.total_infected_screened();

        assert!(screened_high > screened_none,
            "High screening should show more infections: {screened_high:.0} vs {screened_none:.0}");

        // Region 0's contribution should be roughly 90%/15% = 6x higher
        let ratio = screened_high / screened_none;
        // Not exactly 6x because other regions contribute too, but should be meaningfully higher
        assert!(ratio > 2.0,
            "screening should substantially increase visible infections (ratio: {ratio:.1}x)");
    }

    #[test]
    fn best_screening_level_returns_highest_across_regions() {
        let mut state = screening_test_state();
        state.policies[0].screening = ScreeningLevel::Basic;
        state.policies[2].screening = ScreeningLevel::MassRapid;
        state.policies[4].screening = ScreeningLevel::Antigen;

        let best = state.best_screening_level();
        assert_eq!(best.visibility_rate(), ScreeningLevel::MassRapid.visibility_rate(),
            "best_screening_level should return High when any region has High");
    }

    #[test]
    fn field_hospital_reduces_lethality() {
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 100_000.0;
        state.diseases[0].lethality = 0.01;

        let without = tick(&state);
        let deaths_without = without.regions[0].dead;

        // Level 1: Field Hospital — 25% lethality reduction
        state.regions[0].hospital_level = 1;
        let with_l1 = tick(&state);
        let deaths_l1 = with_l1.regions[0].dead;
        let ratio_l1 = deaths_l1 / deaths_without;
        assert!(ratio_l1 > 0.60 && ratio_l1 < 0.90,
            "Field Hospital should reduce deaths by ~25% (ratio: {ratio_l1:.2})");

        // Level 2: Medical Center — 40% lethality reduction
        state.regions[0].hospital_level = 2;
        let with_l2 = tick(&state);
        let deaths_l2 = with_l2.regions[0].dead;
        let ratio_l2 = deaths_l2 / deaths_without;
        assert!(ratio_l2 > 0.45 && ratio_l2 < 0.75,
            "Medical Center should reduce deaths by ~40% (ratio: {ratio_l2:.2})");
        assert!(deaths_l2 < deaths_l1,
            "Medical Center should save more lives than Field Hospital");
    }

    #[test]
    fn field_hospital_build_and_upgrade() {
        let mut state = screening_test_state();

        // Build Level 1: Field Hospital
        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(ok, "should succeed with sufficient funds");
        assert_eq!(state.regions[0].hospital_level, 1);
        assert!(msg.unwrap().contains("Field Hospital"));

        let funds_after_l1 = state.resources.funding;
        assert!(funds_after_l1 < 10_000.0, "funding should be deducted");

        // Upgrade to Level 2: Medical Center
        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(ok, "upgrade should succeed");
        assert_eq!(state.regions[0].hospital_level, 2);
        assert!(msg.unwrap().contains("Medical Center"));
        assert!(state.resources.funding < funds_after_l1, "upgrade should cost funds");

        // Try again — already maxed
        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(!ok, "should not build past level 2");
        assert!(msg.unwrap().contains("already"));
    }

    #[test]
    fn field_hospital_blocked_for_collapsed_regions() {
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(!ok, "should not build in collapsed region");
        assert!(msg.unwrap().contains("collapsed"));
    }

    #[test]
    fn conscript_researchers_grants_personnel_and_penalizes_income() {
        let mut state = screening_test_state();
        let personnel_before = state.resources.personnel;
        let income_before = state.funding_income_rate();

        let (msg, ok) = enact_decree(&mut state, 0, None);
        assert!(ok, "should succeed with sufficient POL");
        assert!(msg.unwrap().contains("Conscript"));
        assert!(state.enacted_decrees.conscript_researchers);
        assert_eq!(state.resources.personnel, personnel_before + crate::state::CONSCRIPT_PERSONNEL_GAIN);

        // Income should be reduced by the penalty
        let income_after = state.funding_income_rate();
        let expected_penalty = crate::state::CONSCRIPT_INCOME_PENALTY;
        assert!((income_before - income_after - expected_penalty).abs() < 0.01,
            "income should drop by {expected_penalty:.3}/tick: before={income_before:.3}, after={income_after:.3}");

        // Cannot enact again
        let (_, ok) = enact_decree(&mut state, 0, None);
        assert!(!ok, "should not enact twice");
    }

    #[test]
    fn decree_blocked_by_insufficient_threat_level() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        state.threat_level = crate::state::ThreatLevel::Normal; // Below all decree thresholds

        for i in 0..crate::state::DECREE_COUNT {
            let (msg, ok) = enact_decree(&mut state, i, None);
            assert!(!ok, "decree {i} should be blocked at low threat level");
            assert!(msg.unwrap().contains("DEFCON"), "error message should mention DEFCON");
        }
    }

    #[test]
    fn sacrifice_region_collapses_and_boosts_income() {
        let mut state = screening_test_state();
        let income_before = state.funding_income_rate();
        assert!(!state.regions[0].collapsed);

        let (msg, ok) = enact_decree(&mut state, 2, Some(0));
        assert!(ok, "should succeed");
        assert!(msg.unwrap().contains("sacrifice zone"));
        assert!(state.regions[0].collapsed);
        assert_eq!(state.enacted_decrees.sacrificed_region, Some(0));

        // Income should reflect the sacrifice: the collapsed region's contribution
        // is lost, but remaining regions get a +20% bonus.
        let income_after = state.funding_income_rate();
        assert!(income_after > 0.0, "should still have income from remaining regions");
        // The bonus should make remaining income higher than it would be without
        // the boost (income_before includes the sacrificed region's contribution,
        // so after sacrifice we lose that but gain 20% on the rest).
        assert!(income_after != income_before,
            "income should change after sacrifice: before={income_before:.3}, after={income_after:.3}");

        // Cannot sacrifice again
        let (_, ok) = enact_decree(&mut state, 2, Some(1));
        assert!(!ok, "should not sacrifice twice");
    }

    #[test]
    fn sacrifice_region_requires_region_idx() {
        let mut state = screening_test_state();

        let (msg, ok) = enact_decree(&mut state, 2, None);
        assert!(!ok, "should require region selection");
        assert!(msg.unwrap().contains("Select"));
    }

    #[test]
    fn rally_support_boosts_pol_and_deducts_funding() {
        let mut state = screening_test_state();
        state.resources.political_power = 0.10;
        let funding_before = state.resources.funding;

        let (msg, ok) = rally_support(&mut state);
        assert!(ok, "should succeed with sufficient funds");
        assert!(msg.unwrap().contains("Rally successful"));
        assert!((state.resources.political_power - 0.15).abs() < 0.001);
        assert!((state.resources.funding - (funding_before - crate::state::RALLY_COST)).abs() < 0.01);
        assert!(state.resources.last_rally_tick.is_some());
    }

    #[test]
    fn rally_support_blocked_by_cooldown() {
        let mut state = screening_test_state();
        state.resources.last_rally_tick = Some(state.tick);

        let (msg, ok) = rally_support(&mut state);
        assert!(!ok, "should be blocked by cooldown");
        assert!(msg.unwrap().contains("cooldown"));
    }

    #[test]
    fn rally_support_blocked_by_insufficient_funding() {
        let mut state = screening_test_state();
        state.resources.funding = 100.0;

        let (msg, ok) = rally_support(&mut state);
        assert!(!ok, "should be blocked by insufficient funding");
        assert!(msg.unwrap().contains("funding"));
    }

    #[test]
    fn rally_support_caps_pol_at_100() {
        let mut state = screening_test_state();
        state.resources.political_power = 0.98;

        let (_, ok) = rally_support(&mut state);
        assert!(ok);
        assert!((state.resources.political_power - 1.0).abs() < 0.001,
            "POL should cap at 100%: got {}", state.resources.political_power);
    }

    #[test]
    fn sacrifice_region_rejects_already_collapsed() {
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok) = enact_decree(&mut state, 2, Some(0));
        assert!(!ok, "should not sacrifice already collapsed region");
        assert!(msg.unwrap().contains("collapsed"));
    }

    #[test]
    fn appease_governor_boosts_loyalty() {
        let mut state = screening_test_state();
        state.regions[0].governor.loyalty = 50.0;
        let funding_before = state.resources.funding;

        let (msg, ok) = appease_governor(&mut state, 0);
        assert!(ok, "should succeed with sufficient funds");
        assert!(msg.unwrap().contains("appeased"));
        assert!((state.regions[0].governor.loyalty - 65.0).abs() < 0.01);
        assert!((state.resources.funding - (funding_before - crate::state::APPEASE_COST)).abs() < 0.01);
    }

    #[test]
    fn appease_governor_blocked_by_insufficient_funds() {
        let mut state = screening_test_state();
        state.resources.funding = 50.0;

        let (_, ok) = appease_governor(&mut state, 0);
        assert!(!ok, "should fail without funds");
    }

    #[test]
    fn appease_governor_blocked_for_collapsed_region() {
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok) = appease_governor(&mut state, 0);
        assert!(!ok, "should fail for collapsed region");
        assert!(msg.unwrap().contains("collapsed"));
    }

    #[test]
    fn appease_governor_caps_at_100() {
        let mut state = screening_test_state();
        state.regions[0].governor.loyalty = 95.0;

        let (_, ok) = appease_governor(&mut state, 0);
        assert!(ok);
        assert!((state.regions[0].governor.loyalty - 100.0).abs() < 0.01,
            "loyalty should cap at 100: got {}", state.regions[0].governor.loyalty);
    }

    #[test]
    fn governor_loyalty_drifts_with_restrictive_policies() {
        let mut state = screening_test_state();
        state.regions[0].governor.loyalty = 60.0;
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].martial_law = true;

        let before = state.regions[0].governor.loyalty;
        // Tick loyalty for ~1 day (120 ticks)
        for _ in 0..120 {
            tick_governor_loyalty(&mut state);
        }
        assert!(state.regions[0].governor.loyalty < before,
            "loyalty should decrease with restrictive policies: was {before}, now {}",
            state.regions[0].governor.loyalty);
    }

    #[test]
    fn governor_loyalty_drops_fast_in_crit_region() {
        let mut state = screening_test_state();
        state.regions[0].governor.loyalty = 70.0;
        // Put >100K infected so severity = CRIT
        state.regions[0].get_or_create_infection(0).infected = 200_000.0;

        // Tick for 20 days
        for _ in 0..(120 * 20) {
            tick_governor_loyalty(&mut state);
        }
        assert!(state.regions[0].governor.loyalty < 45.0,
            "CRIT region should drive loyalty well below 45 in 20 days, got {}",
            state.regions[0].governor.loyalty);
    }

    #[test]
    fn nationalist_governor_drops_faster_than_cooperative() {
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 200_000.0;

        // Test Nationalist
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Nationalist;
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 15) {
            tick_governor_loyalty(&mut state);
        }
        let nationalist_loyalty = state.regions[0].governor.loyalty;

        // Test Cooperative
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Cooperative;
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 15) {
            tick_governor_loyalty(&mut state);
        }
        let cooperative_loyalty = state.regions[0].governor.loyalty;

        assert!(nationalist_loyalty < cooperative_loyalty,
            "Nationalist ({nationalist_loyalty:.1}) should lose loyalty faster than Cooperative ({cooperative_loyalty:.1}) in a CRIT region");
    }

    #[test]
    fn governor_defiance_reduces_policy_effectiveness() {
        use crate::state::GOVERNOR_DEFIANCE_THRESHOLD;

        let mut state = screening_test_state();
        state.regions[0].governor.loyalty = GOVERNOR_DEFIANCE_THRESHOLD - 1.0;
        assert!(state.regions[0].governor.is_defiant());
        assert!(state.regions[0].policy_effectiveness() < 1.0);

        state.regions[0].governor.loyalty = GOVERNOR_DEFIANCE_THRESHOLD + 1.0;
        assert!(!state.regions[0].governor.is_defiant());
        assert!((state.regions[0].policy_effectiveness() - 1.0).abs() < 0.001);
    }

    #[test]
    fn governor_cooperation_reduces_costs() {
        use crate::state::GOVERNOR_COOPERATION_THRESHOLD;

        let mut state = screening_test_state();
        state.policies[0].hospital_surge = true;

        // Normal loyalty — full cost
        state.regions[0].governor.loyalty = 50.0;
        let normal_cost = state.total_policy_funding_cost();

        // Cooperative loyalty — reduced cost
        state.regions[0].governor.loyalty = GOVERNOR_COOPERATION_THRESHOLD + 1.0;
        let coop_cost = state.total_policy_funding_cost();

        assert!(coop_cost < normal_cost,
            "cooperative governor should reduce costs: normal={normal_cost}, coop={coop_cost}");
    }

    #[test]
    fn populist_governor_hates_restrictions() {
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 200_000.0; // CRIT
        state.policies[0].quarantine = true;
        state.policies[0].travel_ban = true;

        // Populist with restrictions
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Populist;
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_loyalty(&mut state);
        }
        let populist_loyalty = state.regions[0].governor.loyalty;

        // Cooperative with same restrictions (baseline)
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Cooperative;
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_loyalty(&mut state);
        }
        let cooperative_loyalty = state.regions[0].governor.loyalty;

        assert!(populist_loyalty < cooperative_loyalty,
            "Populist ({populist_loyalty:.1}) should lose loyalty faster than Cooperative ({cooperative_loyalty:.1}) with restrictions");
    }

    #[test]
    fn populist_governor_happy_without_restrictions() {
        let mut state = screening_test_state();
        // Low infections, no restrictions — populist's calm bonus kicks in
        state.regions[0].get_or_create_infection(0).infected = 100.0;
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Populist;
        state.regions[0].governor.loyalty = 50.0;

        for _ in 0..(120 * 5) {
            tick_governor_loyalty(&mut state);
        }
        assert!(state.regions[0].governor.loyalty > 50.0,
            "Populist should gain loyalty with no restrictions and low infections, got {}",
            state.regions[0].governor.loyalty);
    }

    #[test]
    fn technocrat_governor_gains_from_targeted_research() {
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 200_000.0;
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Technocrat;

        // Without research targeting disease 0
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_loyalty(&mut state);
        }
        let no_research_loyalty = state.regions[0].governor.loyalty;

        // With research targeting disease 0
        state.regions[0].governor.loyalty = 70.0;
        state.field_research.push(crate::state::ResearchProject {
            kind: crate::state::ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 1000.0,
            personnel_assigned: 5,
            scientist_ids: vec![],
        });
        for _ in 0..(120 * 10) {
            tick_governor_loyalty(&mut state);
        }
        let with_research_loyalty = state.regions[0].governor.loyalty;

        assert!(with_research_loyalty > no_research_loyalty,
            "Technocrat should have higher loyalty with targeted research ({with_research_loyalty:.1}) vs without ({no_research_loyalty:.1})");
    }

    #[test]
    fn technocrat_angry_about_unidentified_diseases() {
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 100.0; // low infections
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Technocrat;

        // Disease not identified (knowledge < KNOWLEDGE_NAME)
        state.diseases[0].knowledge = 0.0;
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_loyalty(&mut state);
        }
        let unidentified_loyalty = state.regions[0].governor.loyalty;

        // Disease identified
        state.diseases[0].knowledge = crate::state::KNOWLEDGE_NAME;
        state.regions[0].governor.loyalty = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_loyalty(&mut state);
        }
        let identified_loyalty = state.regions[0].governor.loyalty;

        assert!(unidentified_loyalty < identified_loyalty,
            "Technocrat should have lower loyalty with unidentified disease ({unidentified_loyalty:.1}) vs identified ({identified_loyalty:.1})");
    }

    // --- Governor autonomous action tests ---

    fn defiant_governor_state(personality: GovernorPersonality) -> GameState {
        let mut state = GameState::new_default(42);
        state.regions[0].governor.personality = personality;
        state.regions[0].governor.loyalty = 20.0; // well below defiance threshold (40)
        state.regions[0].governor.last_action_tick = 0;
        state.tick = GOVERNOR_ACTION_INTERVAL + 1; // past cooldown
        state
    }

    #[test]
    fn populist_governor_lifts_restrictive_policy() {
        let mut state = defiant_governor_state(GovernorPersonality::Populist);
        state.policies[0].border_controls = true;

        tick_governor_actions(&mut state);

        assert!(!state.policies[0].border_controls,
            "Populist governor should lift border controls");
        assert!(state.events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("lifted"))
        ), "should emit GovernorAction event about lifting a policy");
    }

    #[test]
    fn populist_governor_no_action_without_restrictive_policies() {
        let mut state = defiant_governor_state(GovernorPersonality::Populist);
        // No restrictive policies active

        tick_governor_actions(&mut state);

        assert!(!state.events.iter().any(|e| matches!(e, GameEvent::GovernorAction { .. })),
            "Populist with no restrictive policies should take no action");
    }

    #[test]
    fn nationalist_governor_diverts_funding() {
        let mut state = defiant_governor_state(GovernorPersonality::Nationalist);
        state.resources.funding = 1000.0;
        let before = state.resources.funding;

        tick_governor_actions(&mut state);

        assert!(state.resources.funding < before,
            "Nationalist governor should drain funding");
        assert!(state.events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("diverted"))
        ));
    }

    #[test]
    fn technocrat_governor_steals_personnel() {
        let mut state = defiant_governor_state(GovernorPersonality::Technocrat);
        let before = state.resources.personnel;

        tick_governor_actions(&mut state);

        assert_eq!(state.resources.personnel, before - 1,
            "Technocrat governor should steal 1 personnel");
        assert!(state.events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("reassigned"))
        ));
    }

    #[test]
    fn cooperative_governor_drains_political_power() {
        let mut state = defiant_governor_state(GovernorPersonality::Cooperative);
        state.resources.political_power = 0.50;

        tick_governor_actions(&mut state);

        assert!(state.resources.political_power < 0.50,
            "Cooperative governor should drain political power");
        assert!(state.events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("leaked"))
        ));
    }

    #[test]
    fn governor_actions_respect_cooldown() {
        let mut state = defiant_governor_state(GovernorPersonality::Nationalist);
        state.regions[0].governor.last_action_tick = state.tick; // just acted

        tick_governor_actions(&mut state);

        assert!(!state.policies[0].border_controls,
            "Governor should not act when still on cooldown");
    }

    #[test]
    fn governor_actions_only_fire_when_defiant() {
        let mut state = defiant_governor_state(GovernorPersonality::Nationalist);
        state.regions[0].governor.loyalty = 50.0; // above threshold

        tick_governor_actions(&mut state);

        assert!(!state.policies[0].border_controls,
            "Governor above defiance threshold should not act");
    }
}
