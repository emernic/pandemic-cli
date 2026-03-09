use crate::state::{
    GameEvent, GameState, ScreeningLevel,
    BORDER_SCREENING_COST, HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL,
    QUARANTINE_COST, QUARANTINE_PERSONNEL, TICKS_PER_DAY, TRAVEL_BAN_COST,
    WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
};

/// Enforce policy costs: suspend most expensive policies one at a time
/// until affordable, then deduct the total cost. Returns the total
/// policy cost (needed by the caller for funding warning calculations).
pub(super) fn tick_enforce_costs(state: &mut GameState) -> f64 {
    let mut policy_cost = state.total_policy_funding_cost();
    while policy_cost > 0.0 && state.resources.funding < policy_cost {
        // Find the most expensive active individual policy across all regions
        let mut best: Option<(usize, &str, f64)> = None;
        for (i, p) in state.policies.iter().enumerate() {
            for (name, active, cost) in [
                ("Travel Ban", p.travel_ban, TRAVEL_BAN_COST),
                ("Quarantine", p.quarantine, QUARANTINE_COST),
                ("Hospital Surge", p.hospital_surge, HOSPITAL_SURGE_COST),
                ("Water Sanitation", p.water_sanitation, WATER_SANITATION_COST),
                ("Border Screening", p.border_screening, BORDER_SCREENING_COST),
            ] {
                if active {
                    if best.is_none() || cost > best.unwrap().2 {
                        best = Some((i, name, cost));
                    }
                }
            }
            // Screening as a single suspendable entry
            let scr_cost = p.screening.funding_cost();
            if scr_cost > 0.0 {
                if best.is_none() || scr_cost > best.unwrap().2 {
                    best = Some((i, "Disease Screening", scr_cost));
                }
            }
        }
        if let Some((region_idx, policy_name, _)) = best {
            match policy_name {
                "Travel Ban" => state.policies[region_idx].travel_ban = false,
                "Quarantine" => state.policies[region_idx].quarantine = false,
                "Hospital Surge" => state.policies[region_idx].hospital_surge = false,
                "Border Screening" => state.policies[region_idx].border_screening = false,
                "Water Sanitation" => state.policies[region_idx].water_sanitation = false,
                "Disease Screening" => state.policies[region_idx].screening = ScreeningLevel::None,
                _ => unreachable!(),
            }
            state.events.push(GameEvent::PolicySuspended {
                region_idx,
                policy_name: policy_name.to_string(),
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
    // Collapsed regions cannot have policies toggled
    if state.regions.get(region_idx).is_some_and(|r| r.collapsed) {
        let region_name = state.regions[region_idx].name.as_str();
        return (Some(format!("{region_name} has collapsed — policies unavailable")), false);
    }
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.as_str())
        .unwrap_or("Unknown");
    // Check POL requirement (only when enabling, not disabling)
    let is_currently_active = match policy_idx {
        0 => state.policies[region_idx].travel_ban,
        1 => state.policies[region_idx].quarantine,
        2 => state.policies[region_idx].hospital_surge,
        3 => state.policies[region_idx].border_screening,
        4 => state.policies[region_idx].water_sanitation,
        5 => state.policies[region_idx].screening == ScreeningLevel::Low,
        6 => state.policies[region_idx].screening == ScreeningLevel::Medium,
        7 => state.policies[region_idx].screening == ScreeningLevel::High,
        _ => false,
    };
    if !is_currently_active && !state.policy_unlocked(region_idx, policy_idx) {
        let threshold = state.effective_pol_threshold(region_idx, policy_idx);
        let policy_name = match policy_idx {
            0 => "Travel Ban",
            1 => "Quarantine",
            2 => "Hospital Surge",
            3 => "Border Screening",
            4 => "Water Sanitation",
            5 => "Low Disease Screening",
            6 => "Medium Disease Screening",
            7 => "High Disease Screening",
            _ => "Policy",
        };
        return (Some(format!(
            "{} requires {:.0}% Political Power (current: {:.0}%)",
            policy_name, threshold * 100.0, state.resources.political_power * 100.0
        )), false);
    }
    let available_personnel = state.personnel_available();
    match policy_idx {
        0 => {
            let new_state = !state.policies[region_idx].travel_ban;
            state.policies[region_idx].travel_ban = new_state;
            if new_state {
                (Some(format!("Travel Ban enabled in {region_name} — ${:.0}/day", TRAVEL_BAN_COST * TICKS_PER_DAY)), true)
            } else {
                (Some(format!("Travel Ban disabled in {region_name}")), true)
            }
        }
        1 => {
            if state.policies[region_idx].quarantine {
                state.policies[region_idx].quarantine = false;
                (Some(format!("Quarantine disabled in {region_name}")), true)
            } else if available_personnel >= QUARANTINE_PERSONNEL {
                state.policies[region_idx].quarantine = true;
                (Some(format!("Quarantine enabled in {region_name} — ${:.0}/day + {} personnel",
                    QUARANTINE_COST * TICKS_PER_DAY, QUARANTINE_PERSONNEL)), true)
            } else {
                (Some(format!(
                    "Not enough personnel for quarantine (need {})", QUARANTINE_PERSONNEL
                )), false)
            }
        }
        2 => {
            if state.policies[region_idx].hospital_surge {
                state.policies[region_idx].hospital_surge = false;
                (Some(format!("Hospital Surge disabled in {region_name}")), true)
            } else if available_personnel >= HOSPITAL_SURGE_PERSONNEL {
                state.policies[region_idx].hospital_surge = true;
                (Some(format!("Hospital Surge enabled in {region_name} — ${:.0}/day + {} personnel",
                    HOSPITAL_SURGE_COST * TICKS_PER_DAY, HOSPITAL_SURGE_PERSONNEL)), true)
            } else {
                (Some(format!(
                    "Not enough personnel for hospital surge (need {})", HOSPITAL_SURGE_PERSONNEL
                )), false)
            }
        }
        3 => {
            let new_state = !state.policies[region_idx].border_screening;
            state.policies[region_idx].border_screening = new_state;
            if new_state {
                (Some(format!("Border Screening enabled in {region_name} — ${:.0}/day",
                    BORDER_SCREENING_COST * TICKS_PER_DAY)), true)
            } else {
                (Some(format!("Border Screening disabled in {region_name}")), true)
            }
        }
        4 => {
            if state.policies[region_idx].water_sanitation {
                state.policies[region_idx].water_sanitation = false;
                (Some(format!("Water Sanitation disabled in {region_name}")), true)
            } else if available_personnel >= WATER_SANITATION_PERSONNEL {
                state.policies[region_idx].water_sanitation = true;
                (Some(format!("Water Sanitation enabled in {region_name} — ${:.0}/day + {} personnel",
                    WATER_SANITATION_COST * TICKS_PER_DAY, WATER_SANITATION_PERSONNEL)), true)
            } else {
                (Some(format!(
                    "Not enough personnel for water sanitation (need {})", WATER_SANITATION_PERSONNEL
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
                // Enable/upgrade
                state.policies[region_idx].screening = target;
                (Some(format!("{} Disease Screening enabled in {region_name} — ${:.0}/day",
                    target.label(), target.funding_cost() * TICKS_PER_DAY)), true)
            }
        }
        _ => (None, false),
    }
}
