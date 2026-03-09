use crate::state::{
    GameEvent, GameState, ScreeningLevel,
    BORDER_CONTROLS_COST, BORDER_CONTROLS_PERSONNEL,
    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL,
    MARTIAL_LAW_COST, MARTIAL_LAW_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    QUARANTINE_COST, QUARANTINE_PERSONNEL,
    TICKS_PER_DAY, TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL,
    WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
};

/// Display name for a policy by index. Shared between enforcement and toggle
/// so the name is defined in exactly one place.
fn policy_display_name(policy_idx: usize) -> &'static str {
    match policy_idx {
        0 => "Travel Ban",
        1 => "Quarantine",
        2 => "Hospital Surge",
        3 => "Border Controls",
        4 => "Water Sanitation",
        5 => "Disease Screening",
        8 => "Martial Law",
        9 => "Nuclear Annihilation",
        _ => "Unknown Policy",
    }
}

/// Enforce policy costs: suspend most expensive policies one at a time
/// until affordable, then deduct the total cost. Returns the total
/// policy cost (needed by the caller for funding warning calculations).
pub(super) fn tick_enforce_costs(state: &mut GameState) -> f64 {
    let mut policy_cost = state.total_policy_funding_cost();
    while policy_cost > 0.0 && state.resources.funding < policy_cost {
        // Find the most expensive active individual policy across all regions.
        // Tracks (region_idx, policy_idx, cost) — no string matching.
        let mut best: Option<(usize, usize, f64)> = None;
        for (i, p) in state.policies.iter().enumerate() {
            let bool_costs: &[(usize, f64)] = &[
                (0, TRAVEL_BAN_COST), (1, QUARANTINE_COST), (2, HOSPITAL_SURGE_COST),
                (3, BORDER_CONTROLS_COST), (4, WATER_SANITATION_COST),
                (8, MARTIAL_LAW_COST),
            ];
            for &(idx, cost) in bool_costs {
                if p.get_bool(idx) && (best.is_none() || cost > best.unwrap().2) {
                    best = Some((i, idx, cost));
                }
            }
            let scr_cost = p.screening.funding_cost();
            if scr_cost > 0.0 && (best.is_none() || scr_cost > best.unwrap().2) {
                best = Some((i, 5, scr_cost));
            }
        }
        if let Some((region_idx, policy_idx, _)) = best {
            if policy_idx <= 4 {
                state.policies[region_idx].set_bool(policy_idx, false);
            } else {
                state.policies[region_idx].screening = ScreeningLevel::None;
            }
            state.events.push(GameEvent::PolicySuspended {
                region_idx,
                policy_name: policy_display_name(policy_idx).to_string(),
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
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    // Check POL requirement (only when enabling, not disabling)
    let is_currently_active = match policy_idx {
        0..=4 | 8 | 9 => state.policies[region_idx].get_bool(policy_idx),
        5 => state.policies[region_idx].screening == ScreeningLevel::Low,
        6 => state.policies[region_idx].screening == ScreeningLevel::Medium,
        7 => state.policies[region_idx].screening == ScreeningLevel::High,
        _ => false,
    };
    if !is_currently_active && !state.policy_unlocked(region_idx, policy_idx) {
        let threshold = state.effective_pol_threshold(region_idx, policy_idx);
        let policy_name = match policy_idx {
            5 => "Low Disease Screening",
            6 => "Medium Disease Screening",
            7 => "High Disease Screening",
            _ => policy_display_name(policy_idx),
        };
        return (Some(format!(
            "{} requires {:.0}% Political Power (current: {:.0}%)",
            policy_name, threshold * 100.0, state.resources.political_power * 100.0
        )), false);
    }
    let available_personnel = state.personnel_available();
    match policy_idx {
        // Boolean policies (0-4): identical toggle logic, different metadata.
        0..=4 => {
            let (name, cost, personnel) = match policy_idx {
                0 => ("Travel Ban", TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL),
                1 => ("Quarantine", QUARANTINE_COST, QUARANTINE_PERSONNEL),
                2 => ("Hospital Surge", HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL),
                3 => ("Border Controls", BORDER_CONTROLS_COST, BORDER_CONTROLS_PERSONNEL),
                4 => ("Water Sanitation", WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL),
                _ => unreachable!(),
            };
            if state.policies[region_idx].get_bool(policy_idx) {
                state.policies[region_idx].set_bool(policy_idx, false);
                (Some(format!("{name} disabled in {region_name}")), true)
            } else if available_personnel >= personnel {
                state.policies[region_idx].set_bool(policy_idx, true);
                (Some(format!("{name} enabled in {region_name} — ${:.0}/day + {personnel} personnel",
                    cost * TICKS_PER_DAY)), true)
            } else {
                (Some(format!(
                    "Not enough personnel for {} (need {personnel})", name.to_lowercase()
                )), false)
            }
        }
        // Screening tiers (5=Low, 6=Medium, 7=High) — mutually exclusive.
        // Selecting the current level disables screening; selecting a different
        // level upgrades/downgrades to that tier.
        5 | 6 | 7 => {
            let target = match policy_idx {
                5 => ScreeningLevel::Low,
                6 => ScreeningLevel::Medium,
                _ => ScreeningLevel::High,
            };
            let current = state.policies[region_idx].screening;
            if current == target {
                // Toggle off
                state.policies[region_idx].screening = ScreeningLevel::None;
                (Some(format!("Disease Screening disabled in {region_name}")), true)
            } else {
                // Check personnel — account for personnel freed from current tier
                let needed = target.personnel_cost();
                let freed = current.personnel_cost();
                let effective_available = available_personnel + freed;
                if effective_available >= needed {
                    state.policies[region_idx].screening = target;
                    (Some(format!("{} Disease Screening enabled in {region_name} — ${:.0}/day + {} personnel",
                        target.label(), target.funding_cost() * TICKS_PER_DAY, needed)), true)
                } else {
                    (Some(format!(
                        "Not enough personnel for {} screening (need {})", target.label().to_lowercase(), needed
                    )), false)
                }
            }
        }
        // Martial Law (8): normal boolean toggle, pre-collapse only
        8 => {
            if state.policies[region_idx].martial_law {
                state.policies[region_idx].martial_law = false;
                (Some(format!("Martial Law lifted in {region_name}")), true)
            } else if available_personnel >= MARTIAL_LAW_PERSONNEL {
                state.policies[region_idx].martial_law = true;
                (Some(format!("Martial Law declared in {region_name} — ${:.0}/day + {} personnel",
                    MARTIAL_LAW_COST * TICKS_PER_DAY, MARTIAL_LAW_PERSONNEL)), true)
            } else {
                (Some(format!(
                    "Not enough personnel for martial law (need {})", MARTIAL_LAW_PERSONNEL
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
                (Some(format!("Not enough funding (need ${:.0})", NUCLEAR_ANNIHILATION_COST)), false)
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
                (Some(format!("☢ Nuclear annihilation in {region_name} — {:.1}M casualties",
                    killed / 1_000_000.0)), true)
            }
        }
        _ => (None, false),
    }
}
