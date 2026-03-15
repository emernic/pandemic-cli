use rand::Rng;

use crate::state::{
    CorporationSector,
    CrisisKind, FundingCondition, GameEvent, WorldState, GovernorPersonality,
    ModifierSource, RegionSpecialization, RegionTrait,
    ScreeningLevel,
    DecreeId, PolicyId,
    QUARANTINE_COST, TRAVEL_BAN_COST,
    INFECTION_PRESSURE_CRIT, INFECTION_PRESSURE_HIGH, INFECTION_PRESSURE_MOD,
    INFRA_STRESSED, SUPPLY_STRESSED_COST_MULT,
    REGULATORY_APPARATUS_COST_MULT, SURVEILLANCE_NETWORK_SCREENING_MULT,
    ADVANCED_INTEL_COST, ADVANCED_INTEL_PERSONNEL,
    BARGAIN_BLOWHARD_FUNDING_COST, BARGAIN_BLOWHARD_COOPERATION_GAIN,
    BARGAIN_BUFFOON_APPROVAL_COST,
    BARGAIN_HARDLINER_FUNDING_COST,
    BARGAIN_COOPERATION_GAIN,
    BARGAIN_MOBSTER_BASE_COST,
    BARGAIN_OPERATIVE_INCOME_CUT, MAX_OPERATIVE_INCOME_SKIM,
    BARGAIN_RECLUSE_PERSONNEL_COST,
    BORDER_CONTROLS_PERSONNEL,
    COLLAPSE_DISRUPTION_TICKS,
    REBUILD_INFRA_COST_PER_POINT, REBUILD_INFRA_MAX_REPAIR, REBUILD_INFRA_AUTO_THRESHOLD,
    FIELD_HOSPITAL_COST, FIELD_HOSPITAL_PERSONNEL,
    GOVERNOR_ACTION_INTERVAL, GOVERNOR_HOSTILITY_THRESHOLD,
    DISCOURAGE_HOSP_PERSONNEL,
    INTEL_STATION_COST, INTEL_STATION_PERSONNEL,
    MARTIAL_LAW_PERSONNEL,
    MEDICAL_CENTER_COST, MEDICAL_CENTER_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    QUARANTINE_PERSONNEL,
    SCREENING_DECAY_RATE, SCREENING_RAMP_RATE,
    TICKS_PER_DAY,
    TRAVEL_BAN_PERSONNEL,
    WATER_SANITATION_PERSONNEL,
};

/// Return names of active contracts whose ForbidPolicy condition matches the given policy.
fn conflicting_contract_names(state: &WorldState, policy: PolicyId) -> Vec<String> {
    state.contracts.iter()
        .filter(|c| matches!(c.condition, FundingCondition::ForbidPolicy { policy: p } if p == policy))
        .map(|c| c.name.clone())
        .collect()
}

/// Enforce policy costs: suspend most expensive policies one at a time
/// until affordable, then deduct the total cost. Returns the total
/// policy cost (needed by the caller for funding warning calculations).
pub(super) fn tick_enforce_costs(state: &mut WorldState, events: &mut Vec<GameEvent>) -> f64 {
    let mut policy_cost = state.total_policy_funding_cost();
    while policy_cost > 0.0 && state.resources.funding < policy_cost {
        // Find the most expensive active individual policy across all regions.
        // Applies the same multipliers as total_policy_funding_cost() for consistent ranking.
        let mut best: Option<(usize, PolicyId, f64)> = None;
        for (i, p) in state.policies.iter().enumerate() {
            let region = state.regions.get(i);
            let traits = region.map(|r| r.traits.as_slice()).unwrap_or(&[]);
            // Apply all multipliers matching total_policy_funding_cost()
            let gov_mult = region.map(|r| r.governor.cost_multiplier()).unwrap_or(1.0);
            let supply_mult = region.map(|r| {
                if r.supply_lines < INFRA_STRESSED { SUPPLY_STRESSED_COST_MULT } else { 1.0 }
            }).unwrap_or(1.0);
            let spec_mult = region.map(|r| {
                if r.has_specialization(RegionSpecialization::RegulatoryApparatus) {
                    REGULATORY_APPARATUS_COST_MULT
                } else {
                    1.0
                }
            }).unwrap_or(1.0);
            let auto_bonus = state.sector_bonus(i, CorporationSector::Automation);
            let auto_mult = 1.0 - CorporationSector::Automation.max_bonus_pct() / 100.0 * auto_bonus;
            for (policy, cost) in p.active_policy_costs(traits) {
                let effective = cost * gov_mult * supply_mult * spec_mult * auto_mult;
                if best.is_none() || effective > best.unwrap().2 {
                    best = Some((i, policy, effective));
                }
            }
        }
        if let Some((region_idx, policy, _)) = best {
            let name = if policy == PolicyId::BasicScreening {
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
                state.policies[region_idx].set_bool(policy, false);
                policy.display_name().to_string()
            };
            events.push(GameEvent::PolicySuspended {
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

/// Tick nuclear state transitions. When a nuke in transit reaches its hit_tick,
/// transition to Dropped: kill 99% of the population and any board members in the region.
pub(super) fn tick_nuclear(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let tick = state.tick;
    for region_idx in 0..state.policies.len() {
        if let crate::state::NuclearState::Dropping { hit_tick } = state.policies[region_idx].nuclear_state {
            if tick >= hit_tick {
                // Transition to Dropped
                state.policies[region_idx].nuclear_state = crate::state::NuclearState::Dropped;

                // Kill 99% of remaining alive population
                let region = &mut state.regions[region_idx];
                let alive = region.alive();
                let killed = alive * 0.99;
                region.dead += killed;
                // Attribute nuke deaths proportionally across disease pools
                let total_inf_dead: f64 = region.infections.iter().map(|i| i.dead).sum();
                let num_infections = region.infections.len().max(1) as f64;
                for inf in &mut region.infections {
                    let share = if total_inf_dead > 0.0 { inf.dead / total_inf_dead } else { 1.0 / num_infections };
                    inf.dead += killed * share;
                    inf.infected = 0.0;
                    inf.immune = 0.0;
                }

                // Kill governor in the region (if alive and not evacuated)
                if !state.regions[region_idx].governor.dead {
                    state.regions[region_idx].governor.dead = true;
                    state.regions[region_idx].governor.succession_tick = None; // no succession from a nuked region
                }

                // Kill board members physically in the region (not evacuated)
                for member in &mut state.board_members {
                    if member.region_idx == Some(region_idx) {
                        member.dead = true;
                    }
                }

                events.push(crate::state::GameEvent::NuclearImpact {
                    region_idx,
                    killed,
                });
            }
        }
    }
}

/// Toggle a policy for a region. Returns (message, success, gdp_policy_region) where success
/// indicates the toggle actually happened (vs being rejected), and gdp_policy_region is
/// Some(region_idx) when a GDP-hurting policy was enacted (for board notification).
/// Does not touch UI state.
pub(super) fn toggle_policy(state: &mut WorldState, region_idx: usize, policy: PolicyId) -> (Option<String>, bool, Option<usize>) {
    let (msg, success) = toggle_policy_inner(state, region_idx, policy);
    // Signal GDP-hurting policy enactment for board notification by the orchestrator.
    // A GDP-hurting policy was enacted if the toggle succeeded and the policy is now active.
    let gdp_region = if success && matches!(policy, PolicyId::TravelBan | PolicyId::Quarantine | PolicyId::MartialLaw) {
        let is_now_active = match policy {
            PolicyId::MartialLaw => state.policies.get(region_idx).is_some_and(|p| p.martial_law),
            _ => state.policies.get(region_idx).is_some_and(|p| p.get_bool(policy)),
        };
        if is_now_active { Some(region_idx) } else { None }
    } else {
        None
    };
    (msg, success, gdp_region)
}

fn toggle_policy_inner(state: &mut WorldState, region_idx: usize, policy: PolicyId) -> (Option<String>, bool) {
    if region_idx >= state.policies.len() {
        return (None, false);
    }
    // Collapsed regions: only nuclear annihilation is available
    if state.regions.get(region_idx).is_some_and(|r| r.collapsed) {
        if policy != PolicyId::NuclearOption {
            let region_name = state.regions[region_idx].name.as_str();
            return (Some(format!("{region_name} has collapsed. Policies unavailable.")), false);
        }
    }
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    // Check POL requirement (only when enabling, not disabling)
    let is_currently_active = match policy {
        PolicyId::TravelBan | PolicyId::Quarantine | PolicyId::DiscourageHosp
        | PolicyId::BorderControls | PolicyId::WaterSanitation
        | PolicyId::MartialLaw | PolicyId::NuclearOption => state.policies[region_idx].get_bool(policy),
        PolicyId::BasicScreening => state.policies[region_idx].screening == ScreeningLevel::Basic,
        PolicyId::AntigenScreening => state.policies[region_idx].screening == ScreeningLevel::Antigen,
        PolicyId::MassRapidScreen => state.policies[region_idx].screening == ScreeningLevel::MassRapid,
        PolicyId::FieldHospital => state.regions[region_idx].hospital_level >= 2, // fully built = "active"
        PolicyId::IntelStation => false,
        PolicyId::RebuildInfra => false, // action-based, never "active"
    };
    if !is_currently_active && !state.policy_unlocked(region_idx, policy) {
        if !state.policy_research_met(policy) {
            let tech = policy.research_prerequisite().unwrap();
            return (Some(format!(
                "{} requires {} research",
                policy.display_name(), tech.name()
            )), false);
        }
        let required = state.effective_authority_requirement(region_idx, policy);
        if let Some(req) = required {
            return (Some(format!(
                "{} requires {} Authority (current: {})",
                policy.display_name(), req.label(), state.resources.authority.label()
            )), false);
        }
    }
    let available_personnel = state.personnel_available();
    let region_traits = state.regions.get(region_idx).map(|r| r.traits.as_slice()).unwrap_or(&[]);
    let low_infra = region_traits.contains(&RegionTrait::LowInfrastructure);
    match policy {
        // Boolean policies: identical toggle logic, different metadata.
        PolicyId::TravelBan | PolicyId::Quarantine | PolicyId::DiscourageHosp
        | PolicyId::BorderControls | PolicyId::WaterSanitation => {
            let (name, personnel, on_msg, off_msg) = match policy {
                PolicyId::TravelBan => ("Travel Ban",
                      TRAVEL_BAN_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Travel Ban enacted",
                      "Travel Ban lifted"),
                PolicyId::Quarantine => ("Quarantine",
                      QUARANTINE_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Quarantine imposed",
                      "Quarantine lifted"),
                PolicyId::DiscourageHosp => ("Discourage Hospitalization",
                      DISCOURAGE_HOSP_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Hospitalization discouraged",
                      "Hospitalization restrictions lifted"),
                PolicyId::BorderControls => ("Border Controls",
                      BORDER_CONTROLS_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Border Controls established",
                      "Border Controls removed"),
                PolicyId::WaterSanitation => ("Water Sanitation",
                      WATER_SANITATION_PERSONNEL + if low_infra { 1 } else { 0 },
                      "Water Sanitation active",
                      "Water Sanitation suspended"),
                _ => unreachable!(),
            };
            if state.policies[region_idx].get_bool(policy) {
                state.policies[region_idx].set_bool(policy, false);
                (Some(format!("{region_name}: {off_msg}")), true)
            } else if available_personnel >= personnel {
                state.policies[region_idx].set_bool(policy, true);
                let conflicts = conflicting_contract_names(state, policy);
                let msg = if conflicts.is_empty() {
                    format!("{region_name}: {on_msg}")
                } else {
                    format!("{region_name}: {on_msg} (violates {})", conflicts.join(", "))
                };
                (Some(msg), true)
            } else {
                (Some(format!(
                    "Not enough personnel for {} (need {personnel})", name.to_lowercase()
                )), false)
            }
        }
        // Screening tiers — mutually exclusive.
        // Selecting the current level disables screening; selecting a different
        // level upgrades/downgrades to that tier.
        PolicyId::BasicScreening | PolicyId::AntigenScreening | PolicyId::MassRapidScreen => {
            let target = match policy {
                PolicyId::BasicScreening => ScreeningLevel::Basic,
                PolicyId::AntigenScreening => ScreeningLevel::Antigen,
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
                        ScreeningLevel::Basic => "rough estimates, faster detection — ramps up over ~4 days",
                        ScreeningLevel::Antigen => "infected + immune data, good accuracy — ramps up over ~4 days",
                        ScreeningLevel::MassRapid => "near-complete data, 25% spread reduction — ramps up over ~4 days",
                        ScreeningLevel::None => unreachable!(),
                    };
                    (Some(format!("{region_name}: {} screening active ({tier_desc})",
                        target.label())), true)
                } else {
                    (Some(format!(
                        "Not enough personnel for {} screening (need {})", target.label().to_lowercase(), needed
                    )), false)
                }
            }
        }
        // Martial Law: normal boolean toggle, pre-collapse only
        PolicyId::MartialLaw => {
            let ml_personnel = MARTIAL_LAW_PERSONNEL + if low_infra { 1 } else { 0 };
            if state.policies[region_idx].martial_law {
                state.policies[region_idx].martial_law = false;
                (Some(format!("{region_name}: Martial Law lifted")), true)
            } else if available_personnel >= ml_personnel {
                state.policies[region_idx].martial_law = true;
                (Some(format!("{region_name}: Martial Law declared (collapse threshold −15%)")), true)
            } else {
                (Some(format!(
                    "Not enough personnel for martial law (need {})", ml_personnel
                )), false)
            }
        }
        // Nuclear Annihilation: one-shot for collapsed regions only
        PolicyId::NuclearOption => {
            if state.policies[region_idx].nuclear_state.is_active() {
                let status = if state.policies[region_idx].nuclear_state.is_dropped() {
                    "annihilated"
                } else {
                    "already targeted"
                };
                (Some(format!("{region_name} has already been {status}")), false)
            } else if !state.regions[region_idx].collapsed {
                (Some("Nuclear annihilation is only available for collapsed regions".to_string()), false)
            } else if state.resources.funding < NUCLEAR_ANNIHILATION_COST {
                (Some(format!("Not enough funding (need ¥{:.0})", NUCLEAR_ANNIHILATION_COST)), false)
            } else {
                // Deduct one-time cost and begin transit
                state.resources.funding -= NUCLEAR_ANNIHILATION_COST;
                let hit_tick = state.tick + crate::state::NUCLEAR_TRANSIT_TICKS;
                state.policies[region_idx].nuclear_state = crate::state::NuclearState::Dropping { hit_tick };

                // Queue evacuation crisis for any board members in the region
                let has_members = state.board_members.iter()
                    .any(|m| m.region_idx == Some(region_idx) && !m.dead);
                if has_members {
                    state.pending_crises.push(
                        CrisisKind::NuclearEvacuation { region_idx },
                    );
                }

                (Some(format!("☢ Nuclear payload inbound to {region_name}. Impact in 12 hours.",
                )), true)
            }
        }
        // Field Hospital / Medical Center: tiered per-region infrastructure
        PolicyId::FieldHospital => {
            let region = &state.regions[region_idx];
            if region.collapsed {
                (Some(format!("{region_name} has collapsed. Cannot build.")), false)
            } else if region.hospital_level == 0 {
                // Build Level 1: Field Hospital
                if state.resources.funding < FIELD_HOSPITAL_COST {
                    (Some(format!("Not enough funding (need ¥{:.0})", FIELD_HOSPITAL_COST)), false)
                } else if available_personnel < FIELD_HOSPITAL_PERSONNEL {
                    (Some(format!("Need {} personnel to staff Field Hospital", FIELD_HOSPITAL_PERSONNEL)), false)
                } else {
                    state.resources.funding -= FIELD_HOSPITAL_COST;
                    state.regions[region_idx].hospital_level = 1;
                    state.regions[region_idx].governor.cooperation = (state.regions[region_idx].governor.cooperation + 10.0).min(100.0);
                    (Some(format!("{region_name}: Field Hospital operational (reduces mortality 25%)")), true)
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
                    state.regions[region_idx].governor.cooperation = (state.regions[region_idx].governor.cooperation + 10.0).min(100.0);
                    (Some(format!("{region_name}: Medical Center operational (mortality -40%, efficacy +25%)")), true)
                }
            } else {
                (Some(format!("{region_name} already has a Medical Center")), false)
            }
        }
        // Intel Station / Advanced Intel: tiered per-region surveillance infrastructure
        PolicyId::IntelStation => {
            let region = &state.regions[region_idx];
            if region.collapsed {
                (Some(format!("{region_name} has collapsed. Cannot build.")), false)
            } else if region.intel_level == 0 {
                // Build Level 1: Intel Station
                if state.resources.funding < INTEL_STATION_COST {
                    (Some(format!("Not enough funding (need ¥{:.0})", INTEL_STATION_COST)), false)
                } else if available_personnel < INTEL_STATION_PERSONNEL {
                    (Some(format!("Need {} personnel to staff Intel Station", INTEL_STATION_PERSONNEL)), false)
                } else {
                    state.resources.funding -= INTEL_STATION_COST;
                    state.regions[region_idx].intel_level = 1;
                    (Some(format!("{region_name}: Intel Station operational (detects new pathogens at 3,000 local infections)")), true)
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
                    (Some(format!("{region_name}: Advanced Intel operational (detects at 1,000 infections, generates briefings)")), true)
                }
            } else {
                (Some(format!("{region_name} already has Advanced Intel")), false)
            }
        }
        PolicyId::RebuildInfra => {
            rebuild_infrastructure(state, region_idx)
        }
    }
}

/// One-shot infrastructure rebuild: repairs up to REBUILD_INFRA_MAX_REPAIR of each degraded stat.
/// Cost is proportional to the total repair needed.
fn rebuild_infrastructure(state: &mut WorldState, region_idx: usize) -> (Option<String>, bool) {
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let region = &state.regions[region_idx];
    if region.collapsed {
        return (Some(format!("{region_name} has collapsed. Cannot rebuild.")), false);
    }

    let hc_repair = (1.0 - region.healthcare_capacity).min(REBUILD_INFRA_MAX_REPAIR).max(0.0);
    let sl_repair = (1.0 - region.supply_lines).min(REBUILD_INFRA_MAX_REPAIR).max(0.0);
    let co_repair = (1.0 - region.civil_order).min(REBUILD_INFRA_MAX_REPAIR).max(0.0);
    let total_repair = hc_repair + sl_repair + co_repair;

    if total_repair < 0.001 {
        return (Some(format!("{region_name}: no infrastructure needs repair")), false);
    }

    let cost = total_repair * REBUILD_INFRA_COST_PER_POINT;
    if state.resources.funding < cost {
        return (Some(format!("Not enough funding (need ¥{:.0})", cost)), false);
    }

    state.resources.funding -= cost;
    state.regions[region_idx].healthcare_capacity = (state.regions[region_idx].healthcare_capacity + hc_repair).min(1.0);
    state.regions[region_idx].supply_lines = (state.regions[region_idx].supply_lines + sl_repair).min(1.0);
    state.regions[region_idx].civil_order = (state.regions[region_idx].civil_order + co_repair).min(1.0);

    let mut repaired = Vec::new();
    if hc_repair > 0.001 { repaired.push(format!("HC +{:.0}%", hc_repair * 100.0)); }
    if sl_repair > 0.001 { repaired.push(format!("SL +{:.0}%", sl_repair * 100.0)); }
    if co_repair > 0.001 { repaired.push(format!("CO +{:.0}%", co_repair * 100.0)); }

    (Some(format!("{region_name}: rebuilt {} (¥{:.0})", repaired.join(", "), cost)), true)
}

/// Spend funds to negotiate with a governor, boosting cooperation.
pub(super) fn negotiate_governor(state: &mut WorldState, region_idx: usize) -> (Option<String>, bool) {
    use crate::state::{NEGOTIATE_COST, NEGOTIATE_COOPERATION_GAIN};

    if region_idx >= state.regions.len() {
        return (None, false);
    }
    if state.regions[region_idx].collapsed {
        let name = &state.regions[region_idx].name;
        return (Some(format!("{name} has collapsed. No governor to negotiate with.")), false);
    }
    if state.regions[region_idx].governor.is_dead() {
        let name = &state.regions[region_idx].name;
        return (Some(format!("{name} is leaderless. No governor to negotiate with.")), false);
    }
    if state.resources.funding < NEGOTIATE_COST {
        return (Some(format!("Not enough funding (need ¥{NEGOTIATE_COST:.0})")), false);
    }
    state.resources.funding -= NEGOTIATE_COST;
    let gov = &mut state.regions[region_idx].governor;
    gov.cooperation = (gov.cooperation + NEGOTIATE_COOPERATION_GAIN).min(100.0);
    let name = &state.regions[region_idx].governor.name;
    let cooperation = state.regions[region_idx].governor.cooperation;
    (Some(format!("Deal reached with {name}. Co-Op now {cooperation:.0}. (-¥{NEGOTIATE_COST:.0})")), true)
}

/// Personality-specific bargain with a hostile governor. Free in funding
/// but costs something else depending on personality.
pub(super) fn bargain_with_governor(state: &mut WorldState, region_idx: usize) -> (Option<String>, bool) {
    if region_idx >= state.regions.len() {
        return (None, false);
    }
    if state.regions[region_idx].collapsed {
        let name = &state.regions[region_idx].name;
        return (Some(format!("{name} has collapsed")), false);
    }
    if state.regions[region_idx].governor.is_dead() {
        let name = &state.regions[region_idx].name;
        return (Some(format!("{name} is leaderless. No governor to bargain with.")), false);
    }
    if !state.regions[region_idx].governor.is_hostile() {
        return (Some("Governor is not hostile. No bargain needed.".into()), false);
    }

    let personality = state.regions[region_idx].governor.personality;
    let gov_name = state.regions[region_idx].governor.name.clone();

    match personality {
        GovernorPersonality::Buffoon => {
            // Public Praise — chairman satisfaction hit, cooperation decays fast (tracked in tick)
            if let Some(chairman) = state.board_members.iter_mut().find(|m| m.is_chairman) {
                chairman.add_modifier(ModifierSource::CrisisEffect, -BARGAIN_BUFFOON_APPROVAL_COST);
            }
            let gov = &mut state.regions[region_idx].governor;
            gov.cooperation = (gov.cooperation + BARGAIN_COOPERATION_GAIN).min(100.0);
            let cooperation = gov.cooperation;
            (Some(format!("{gov_name}: praised publicly. Co-Op {cooperation:.0} (won't last).")), true)
        }
        GovernorPersonality::Blowhard => {
            // Token Concession — small funding, large cooperation gain
            if state.resources.funding < BARGAIN_BLOWHARD_FUNDING_COST {
                return (Some(format!("Not enough funding (need ¥{BARGAIN_BLOWHARD_FUNDING_COST:.0})")), false);
            }
            state.resources.funding -= BARGAIN_BLOWHARD_FUNDING_COST;
            let gov = &mut state.regions[region_idx].governor;
            gov.cooperation = (gov.cooperation + BARGAIN_BLOWHARD_COOPERATION_GAIN).min(100.0);
            let cooperation = gov.cooperation;
            (Some(format!("{gov_name}: given a token victory. Co-Op {cooperation:.0}.")), true)
        }
        GovernorPersonality::Recluse => {
            // Send a Manager — personnel cost
            let cost = BARGAIN_RECLUSE_PERSONNEL_COST;
            if state.resources.personnel < cost {
                return (Some(format!("Not enough personnel (need {cost})")), false);
            }
            state.resources.personnel -= cost;
            let gov = &mut state.regions[region_idx].governor;
            gov.cooperation = (gov.cooperation + BARGAIN_COOPERATION_GAIN).min(100.0);
            let cooperation = gov.cooperation;
            (Some(format!("{gov_name}: manager sent. Co-Op {cooperation:.0}. (-{cost} personnel)")), true)
        }
        GovernorPersonality::Hardliner => {
            // Grant Authority — expensive funding
            if state.resources.funding < BARGAIN_HARDLINER_FUNDING_COST {
                return (Some(format!("Not enough funding (need ¥{BARGAIN_HARDLINER_FUNDING_COST:.0})")), false);
            }
            state.resources.funding -= BARGAIN_HARDLINER_FUNDING_COST;
            let gov = &mut state.regions[region_idx].governor;
            gov.cooperation = (gov.cooperation + BARGAIN_COOPERATION_GAIN).min(100.0);
            let cooperation = gov.cooperation;
            (Some(format!("{gov_name}: granted expanded authority. Co-Op {cooperation:.0}.")), true)
        }
        GovernorPersonality::Operative => {
            // Income Cut: permanent skim on regional income
            let gov = &mut state.regions[region_idx].governor;
            gov.income_skim = (gov.income_skim + BARGAIN_OPERATIVE_INCOME_CUT).min(MAX_OPERATIVE_INCOME_SKIM);
            gov.cooperation = (gov.cooperation + BARGAIN_COOPERATION_GAIN).min(100.0);
            let cooperation = gov.cooperation;
            let total_skim = gov.income_skim * 100.0;
            (Some(format!("{gov_name}: cut agreed. Co-Op {cooperation:.0}. (now skimming {total_skim:.0}% of income)")), true)
        }
        GovernorPersonality::Mobster => {
            // Protection Money — escalating cost
            let count = state.regions[region_idx].governor.bargain_count;
            let cost = BARGAIN_MOBSTER_BASE_COST * 2.0_f64.powi(count as i32);
            if state.resources.funding < cost {
                return (Some(format!("Not enough funding (need ¥{cost:.0})")), false);
            }
            state.resources.funding -= cost;
            let gov = &mut state.regions[region_idx].governor;
            gov.bargain_count += 1;
            gov.cooperation = (gov.cooperation + BARGAIN_COOPERATION_GAIN).min(100.0);
            let cooperation = gov.cooperation;
            (Some(format!("{gov_name}: paid ¥{cost:.0}. Co-Op {cooperation:.0}. Next time will cost more.")), true)
        }
    }
}

/// Tick governor cooperation drift. Called once per tick from tick().
///
/// Cooperation drifts based on infection pressure, cumulative deaths, active
/// restrictive policies, and personality. Governors react to the engine's
/// infection pressure thresholds (INFECTION_PRESSURE_CRIT/HIGH/MOD), which
/// are lower than the UI severity labels the player sees.
pub(super) fn tick_governor_cooperation(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let num_regions = state.regions.len();
    for i in 0..num_regions {
        if state.regions[i].collapsed {
            continue;
        }

        // Handle governor succession: new governor arrives after the waiting period
        if state.regions[i].governor.dead {
            if let Some(succ_tick) = state.regions[i].governor.succession_tick {
                if state.tick >= succ_tick {
                    tick_governor_succession(state, i, events);
                }
            }
            // Dead governors don't drift, don't fire crises, don't do anything
            continue;
        }

        let policy = &state.policies[i];
        let personality = state.regions[i].governor.personality;
        let current = state.regions[i].governor.cooperation;

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
        // infection pressure thresholds from state.rs.
        let severity_drain = if infected > INFECTION_PRESSURE_CRIT {
            -0.015 // CRIT: ~1.8/day — mid-game hostility in ~14 days at this level
        } else if infected > INFECTION_PRESSURE_HIGH {
            -0.008 // HIGH: ~0.96/day
        } else if infected > INFECTION_PRESSURE_MOD {
            -0.002 // MOD: ~0.24/day
        } else {
            0.0
        };

        // Death drain: cumulative deaths erode trust (linear, not sqrt)
        let death_drain = -death_frac * 0.03; // ~0.036/day at 1% dead, ~0.36/day at 10%

        // Policy pressure: each restrictive policy drains cooperation
        let policy_drain = -restrictive_count * 0.005; // ~0.6/day per policy

        // Personality modifiers
        let personality_mod = match personality {
            GovernorPersonality::Buffoon => {
                // High passive decay — they forget promises quickly.
                // At low severity: bribe effect fades in ~5 weeks. At CRIT: ~8 days.
                -0.005 // ~0.6/day passive decay
            }
            GovernorPersonality::Blowhard => {
                // Hates restrictive policies — extra drain. Happy when things are calm.
                let restriction_anger = -restrictive_count * 0.004;
                let calm_bonus = if restrictive_count == 0.0 && infected <= INFECTION_PRESSURE_HIGH {
                    0.003
                } else {
                    0.0
                };
                restriction_anger + calm_bonus
            }
            GovernorPersonality::Recluse => {
                // Doesn't care much about anything. Low drift in any direction.
                // Slightly annoyed by attention (policies = someone's paying attention)
                -restrictive_count * 0.001
            }
            GovernorPersonality::Hardliner => {
                // Zero-sum nationalist. Less pliable baseline, plus angry when
                // their region is doing worse than the average of other regions.
                let baseline_stubbornness = -0.003; // ~0.36/day passive decay
                // Compare this region's infection burden to the average of others
                let other_infected: f64 = (0..num_regions)
                    .filter(|&j| j != i && !state.regions[j].collapsed)
                    .map(|j| state.regions[j].total_infected())
                    .sum::<f64>()
                    / (num_regions.saturating_sub(1).max(1) as f64);
                // Happy when their region has fewer infections than others
                let relative_standing = if infected > other_infected * 2.0 {
                    -0.004 // their region is much worse off — furious
                } else if infected > other_infected {
                    -0.002
                } else {
                    0.001 // their region is better off — slightly pleased
                };
                baseline_stubbornness + relative_standing
            }
            GovernorPersonality::Operative => {
                // Passive cooperation gain when being paid (income_skim > 0).
                // Otherwise neutral.
                let skim = state.regions[i].governor.income_skim;
                if skim > 0.0 { 0.002 } else { 0.0 }
            }
            GovernorPersonality::Mobster => {
                // Cooperation decays constantly — always wants more money.
                // Decays faster the more bargains you've made (addiction escalation).
                let count = state.regions[i].governor.bargain_count as f64;
                -0.002 * (1.0 + count * 0.5) // ~0.24/day base, grows with each bargain
            }
        };

        let total_drift = base_drift + severity_drain + death_drain + policy_drain + personality_mod;
        let new_cooperation = (current + total_drift).clamp(0.0, 100.0);
        state.regions[i].governor.cooperation = new_cooperation;

        // Fire a personality-specific crisis when cooperation first drops below hostility threshold
        if new_cooperation < GOVERNOR_HOSTILITY_THRESHOLD && !state.regions[i].governor.hostility_crisis_fired {
            state.regions[i].governor.hostility_crisis_fired = true;
            let kind = match personality {
                GovernorPersonality::Buffoon => None,
                GovernorPersonality::Blowhard => None,
                GovernorPersonality::Recluse => None,
                GovernorPersonality::Hardliner => Some(CrisisKind::GovernorHardliner { region_idx: i }),
                GovernorPersonality::Operative => Some(CrisisKind::GovernorOperative { region_idx: i }),
                GovernorPersonality::Mobster => Some(CrisisKind::GovernorMobster { region_idx: i }),
            };
            // Schedule for immediate activation (current tick)
            if let Some(kind) = kind {
                state.pending_crises.push(kind);
            }
        }

        // Reset the flag when cooperation recovers above hostility threshold
        if new_cooperation >= GOVERNOR_HOSTILITY_THRESHOLD && state.regions[i].governor.hostility_crisis_fired {
            state.regions[i].governor.hostility_crisis_fired = false;
        }

        // GovernorSick: fire when region has HIGH+ infections and cooldown has passed (~30 days).
        // Skip dead governors.
        if !state.regions[i].collapsed && !state.regions[i].governor.dead {
            let region_infected: f64 = state.regions[i].infections.iter().map(|inf| inf.infected).sum();
            let sick_cooldown = (30.0 * TICKS_PER_DAY) as u64;
            let cooldown_ok = state.regions[i].governor.last_sick_tick
                .map_or(true, |t| state.tick.saturating_sub(t) >= sick_cooldown);
            if region_infected > INFECTION_PRESSURE_HIGH && cooldown_ok {
                state.regions[i].governor.last_sick_tick = Some(state.tick);
                state.pending_crises.push(CrisisKind::GovernorSick { region_idx: i });
            }
        }

        // Governor death: rare event in critically infected regions with significant deaths.
        // Per-tick probability when infected > CRIT threshold AND death_frac > 5%.
        // ~0.0002/tick = ~1.2% per day at CRIT. Only fires once (governor dies).
        // Guard: skip dead governors and don't fire if there's already a pending GovernorDeath.
        if !state.regions[i].governor.dead && infected > INFECTION_PRESSURE_CRIT && death_frac > 0.05 {
            let already_pending = state.pending_crises.iter()
                .any(|k| matches!(k, CrisisKind::GovernorDeath { region_idx: ri } if *ri == i));
            let already_active = state.active_crisis.as_ref()
                .map_or(false, |c| matches!(c.kind, CrisisKind::GovernorDeath { region_idx: ri } if ri == i));
            if !already_pending && !already_active {
                let roll: f64 = state.rng_misc.r#gen();
                if roll < 0.0002 {
                    state.pending_crises.push(CrisisKind::GovernorDeath { region_idx: i });
                }
            }
        }
    }
}

/// Handle governor succession: replace the dead governor with a new one.
fn tick_governor_succession(state: &mut WorldState, region_idx: usize, events: &mut Vec<GameEvent>) {
    use crate::state::{GovernorPersonality, SUCCESSOR_COOPERATION};

    // Pick a random personality (different from the deceased)
    let old_personality = state.regions[region_idx].governor.personality;
    let personalities = [
        GovernorPersonality::Buffoon,
        GovernorPersonality::Blowhard,
        GovernorPersonality::Recluse,
        GovernorPersonality::Hardliner,
        GovernorPersonality::Operative,
        GovernorPersonality::Mobster,
    ];
    let candidates: Vec<_> = personalities.iter()
        .filter(|&&p| p != old_personality)
        .copied()
        .collect();
    let new_personality = candidates[state.rng_misc.gen_range(0..candidates.len())];

    // Generate a successor name based on region
    let successor_names: &[&str] = match state.regions[region_idx].name.as_str() {
        "North America" => &["Gov. Reyes", "Gov. Mitchell", "Gov. Park", "Gov. Dubois"],
        "South America" => &["Gov. Silva", "Gov. Mendoza", "Gov. Herrera", "Gov. Aguiar"],
        "Europe" => &["Gov. Müller", "Gov. Johansson", "Gov. Moretti", "Gov. Kowalski"],
        "Africa" => &["Gov. Diallo", "Gov. Mensah", "Gov. Ndung'u", "Gov. Balogun"],
        "Asia" => &["Gov. Nakamura", "Gov. Singh", "Gov. Chen", "Gov. Patel"],
        "Oceania" => &["Gov. Campbell", "Gov. Aroha", "Gov. Dawson", "Gov. Talbot"],
        _ => &["Gov. Unknown"],
    };
    let old_name = state.regions[region_idx].governor.name.clone();
    // Filter out the deceased governor's name, then pick randomly
    let name_candidates: Vec<&&str> = successor_names.iter()
        .filter(|&&n| n != old_name)
        .collect();
    let new_name = if name_candidates.is_empty() {
        successor_names[0].to_string()
    } else {
        name_candidates[state.rng_misc.gen_range(0..name_candidates.len())].to_string()
    };

    // Replace the governor
    let gov = &mut state.regions[region_idx].governor;
    gov.name = new_name.clone();
    gov.personality = new_personality;
    gov.cooperation = SUCCESSOR_COOPERATION;
    gov.dead = false;
    gov.succession_tick = None;
    gov.hostility_crisis_fired = false;
    gov.last_action_tick = state.tick;
    gov.bargain_count = 0;
    gov.income_skim = 0.0;
    gov.last_sick_tick = None;

    // Update the board member if this governor sits on the board
    for member in &mut state.board_members {
        if matches!(member.role, crate::state::BoardRole::RegionGovernor { region_idx: ri } if ri == region_idx) {
            member.name = new_name.clone();
        }
    }

    events.push(GameEvent::GovernorSucceeded {
        region_idx,
        name: new_name,
    });
}

/// Tick autonomous governor actions. Hostile governors periodically act against
/// the player based on personality. Called from tick().
pub(super) fn tick_governor_actions(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let tick = state.tick;
    let num_regions = state.regions.len();

    for i in 0..num_regions {
        if state.regions[i].collapsed {
            continue;
        }
        let gov = &state.regions[i].governor;
        // Dead governors can't take autonomous actions
        if gov.dead {
            continue;
        }
        if gov.cooperation >= GOVERNOR_HOSTILITY_THRESHOLD {
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
            GovernorPersonality::Buffoon => {
                // Accidentally breaks a random policy or wastes funding
                let policy = &state.policies[i];
                let active_policies: Vec<&str> = [
                    (policy.travel_ban, "travel_ban"),
                    (policy.quarantine, "quarantine"),
                    (policy.discourage_hosp, "discourage_hosp"),
                    (policy.border_controls, "border_controls"),
                ].iter()
                    .filter(|(active, _)| *active)
                    .map(|(_, name)| *name)
                    .collect();
                if !active_policies.is_empty() {
                    // Pick a random policy to accidentally disable
                    let idx = (tick as usize) % active_policies.len();
                    let target = active_policies[idx];
                    let label = match target {
                        "travel_ban" => { state.policies[i].travel_ban = false; "Travel Ban" }
                        "quarantine" => { state.policies[i].quarantine = false; "Quarantine" }
                        "discourage_hosp" => { state.policies[i].discourage_hosp = false; "Discourage Hospitalization" }
                        "border_controls" => { state.policies[i].border_controls = false; "Border Controls" }
                        _ => unreachable!(),
                    };
                    Some(format!("{gov_name} accidentally cancelled {label} in {region_name}"))
                } else {
                    // No policies to break — waste some funding instead
                    let waste = (state.resources.funding * 0.05).min(150.0);
                    if waste >= 10.0 {
                        state.resources.funding -= waste;
                        Some(format!("{gov_name} misspent ¥{waste:.0} on a publicity stunt in {region_name}"))
                    } else {
                        None
                    }
                }
            }
            GovernorPersonality::Blowhard => {
                // Small funding drain + alarming messages (mostly hollow)
                let drain = (state.resources.funding * 0.03).min(100.0);
                if drain >= 5.0 {
                    state.resources.funding -= drain;
                    Some(format!("{gov_name} demanded ¥{drain:.0} for \"emergency PR\" in {region_name}"))
                } else {
                    None
                }
            }
            GovernorPersonality::Recluse => {
                // Passive neglect: the governor has completely checked out.
                // Mechanical consequence is through policy_effectiveness (0.4x vs 0.7x
                // for other hostile governors) — policies barely work in this region.
                Some(format!("{gov_name} is unreachable in {region_name}. Policies barely enforced."))
            }
            GovernorPersonality::Hardliner => {
                // Unilaterally activates a restrictive policy the player didn't set
                let policy = &state.policies[i];
                let inactive: Vec<&str> = [
                    (!policy.quarantine, "quarantine"),
                    (!policy.border_controls, "border_controls"),
                    (!policy.martial_law, "martial_law"),
                ].iter()
                    .filter(|(inactive, _)| *inactive)
                    .map(|(_, name)| *name)
                    .collect();
                if let Some(&target) = inactive.first() {
                    let (label, pid) = match target {
                        "quarantine" => { state.policies[i].quarantine = true; ("Quarantine", Some(PolicyId::Quarantine)) }
                        "border_controls" => { state.policies[i].border_controls = true; ("Border Controls", Some(PolicyId::BorderControls)) }
                        "martial_law" => { state.policies[i].martial_law = true; ("Martial Law", None) }
                        _ => unreachable!(),
                    };
                    let conflicts = pid.map(|p| conflicting_contract_names(state, p)).unwrap_or_default();
                    let suffix = if conflicts.is_empty() {
                        String::new()
                    } else {
                        format!(" (violates {})", conflicts.join(", "))
                    };
                    Some(format!("{gov_name} imposed {label} in {region_name} without authorization{suffix}"))
                } else {
                    None // All restrictive policies already active
                }
            }
            GovernorPersonality::Operative => {
                // Continuous funding drain that grows over time
                let skim = state.regions[i].governor.income_skim;
                let drain = (state.resources.funding * (0.05 + skim)).min(300.0);
                if drain >= 10.0 {
                    state.resources.funding -= drain;
                    Some(format!("{gov_name} siphoned ¥{drain:.0} from operations in {region_name}"))
                } else {
                    None
                }
            }
            GovernorPersonality::Mobster => {
                // Lump-sum demands that increase each time
                let count = state.regions[i].governor.bargain_count;
                let demand = 100.0 * 2.0_f64.powi(count as i32);
                if state.resources.funding >= demand {
                    state.resources.funding -= demand;
                    Some(format!("{gov_name} extorted ¥{demand:.0} from {region_name}"))
                } else {
                    // Can't pay — chairman satisfaction hit instead
                    if let Some(chairman) = state.board_members.iter_mut().find(|m| m.is_chairman) {
                        chairman.add_modifier(ModifierSource::CrisisEffect, -0.05);
                    }
                    Some(format!("{gov_name} made threats in {region_name}. International embarrassment."))
                }
            }
        };

        if let Some(desc) = action_desc {
            state.regions[i].governor.last_action_tick = tick;
            events.push(GameEvent::GovernorAction {
                region_idx: i,
                description: desc,
            });
        }
    }
}

/// Enact an emergency decree. Permanent, irreversible.
/// Returns (message, success).
pub(super) fn enact_decree(state: &mut WorldState, decree: DecreeId, region_idx: Option<usize>, events: &mut Vec<GameEvent>) -> (Option<String>, bool) {
    use crate::state::{
        CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY,
        SACRIFICE_INCOME_BONUS, DecreeId,
    };

    // Already enacted?
    if state.enacted_decrees.is_enacted(decree) {
        return (Some(format!("{} has already been enacted", decree.display_name())), false);
    }

    // Severity check: decrees require sufficiently dire conditions to justify them.
    if !state.decree_unlocked(decree) {
        return (Some(format!(
            "{} requires a more severe crisis before it can be enacted.",
            decree.display_name(),
        )), false);
    }

    let chairman_cost = decree.chairman_cost();

    let (msg, success) = match decree {
        DecreeId::ConscriptResearchers => {
            // Conscript Researchers: +personnel, permanent income penalty
            state.enacted_decrees.conscript_researchers = true;
            state.resources.personnel += CONSCRIPT_PERSONNEL_GAIN;
            let penalty_per_day = CONSCRIPT_INCOME_PENALTY * TICKS_PER_DAY;
            (Some(format!(
                "⚠ DECREE: Conscript Researchers enacted. +{} personnel. Income reduced ¥{:.0}/day, permanently.",
                CONSCRIPT_PERSONNEL_GAIN, penalty_per_day
            )), true)
        }
        DecreeId::AuthorizeHumanTrials => {
            // Authorize Human Trials: faster clinical trials, risk of adverse events
            state.enacted_decrees.authorize_human_trials = true;
            (Some(
                "⚠ DECREE: Human Trials authorized. Clinical trials 50% faster. Adverse event risk elevated, permanently.".to_string()
            ), true)
        }
        DecreeId::SacrificeRegion => {
            // Sacrifice Region: voluntarily abandon a region for income bonus
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
            // Collapse the sacrificed region
            state.regions[r_idx].collapsed = true;
            state.regions[r_idx].collapsed_at_tick = Some(state.tick);
            state.regions[r_idx].hospital_level = 0;
            state.regions[r_idx].intel_level = 0;
            // Clear policies — this immediately frees all personnel assigned there
            if let Some(p) = state.policies.get_mut(r_idx) {
                p.clear_all();
            }
            // Notify the UI
            events.push(GameEvent::RegionCollapsed { region_idx: r_idx, personnel_lost: 0 });
            // Apply network disruption to connected non-collapsed regions (same as natural collapse)
            let disruption_end = state.tick + COLLAPSE_DISRUPTION_TICKS;
            let connected: Vec<usize> = state.regions[r_idx].connections.clone();
            for &c in &connected {
                if !state.regions[c].collapsed {
                    state.regions[c].disrupted_until = Some(
                        state.regions[c].disrupted_until.map_or(disruption_end, |t| t.max(disruption_end))
                    );
                    events.push(GameEvent::NetworkDisruption {
                        disrupted_region_idx: c,
                        collapsed_region_idx: r_idx,
                    });
                }
            }
            // Schedule refugee wave toward a non-collapsed neighbor (if any)
            let neighbors: Vec<usize> = connected.iter()
                .filter(|&&c| !state.regions[c].collapsed)
                .copied()
                .collect();
            if let Some(&to) = neighbors.first() {
                let wave = state.regions.iter().filter(|r| r.collapsed).count() as u8;
                state.pending_crises.push(CrisisKind::RefugeeWave {
                    from_region: r_idx,
                    to_region: to,
                    wave,
                });
            }
            let bonus_pct = (SACRIFICE_INCOME_BONUS - 1.0) * 100.0;
            (Some(format!(
                "⚠ DECREE: {} designated a sacrifice zone. Abandoned. Remaining regions: +{:.0}% income.",
                region_name, bonus_pct
            )), true)
        }
        DecreeId::FortifyRegion => {
            // Fortify Region: restore one region's infrastructure, penalize all others
            use crate::state::FORTIFY_INFRA_PENALTY;
            let Some(r_idx) = region_idx else {
                return (Some("Select a region to fortify".to_string()), false);
            };
            if r_idx >= state.regions.len() {
                return (None, false);
            }
            if state.regions[r_idx].collapsed {
                return (Some(format!("{} is already collapsed", state.regions[r_idx].name)), false);
            }
            let region_name = state.regions[r_idx].name.clone();
            state.enacted_decrees.fortified_region = Some(r_idx);
            // Restore target region's infrastructure to 100%
            state.regions[r_idx].healthcare_capacity = 1.0;
            state.regions[r_idx].supply_lines = 1.0;
            state.regions[r_idx].civil_order = 1.0;
            // Penalize all other non-collapsed regions
            for i in 0..state.regions.len() {
                if i != r_idx && !state.regions[i].collapsed {
                    state.regions[i].healthcare_capacity =
                        (state.regions[i].healthcare_capacity - FORTIFY_INFRA_PENALTY).max(0.0);
                    state.regions[i].supply_lines =
                        (state.regions[i].supply_lines - FORTIFY_INFRA_PENALTY).max(0.0);
                    state.regions[i].civil_order =
                        (state.regions[i].civil_order - FORTIFY_INFRA_PENALTY).max(0.0);
                }
            }
            let penalty_pct = (FORTIFY_INFRA_PENALTY * 100.0) as u32;
            (Some(format!(
                "⚠ DECREE: {} designated as fortified zone. Infrastructure restored. All other regions: -{}% infrastructure.",
                region_name, penalty_pct
            )), true)
        }
        DecreeId::EmergencyCountermeasure => {
            // Emergency Countermeasure: reduce disease parameters, kill population
            use crate::state::{
                COUNTERMEASURE_KILL_FRACTION, COUNTERMEASURE_SPREAD_WITHIN_MULT,
                COUNTERMEASURE_SPREAD_MULT,
            };
            state.enacted_decrees.emergency_countermeasure = true;
            // Reduce all disease within-region and cross-region spread
            for disease in &mut state.diseases {
                disease.within_region_spread *= COUNTERMEASURE_SPREAD_WITHIN_MULT;
                disease.cross_region_spread *= COUNTERMEASURE_SPREAD_MULT;
            }
            // Kill a fraction of the alive population in every non-collapsed region
            let mut total_killed = 0.0_f64;
            for region in &mut state.regions {
                if region.collapsed {
                    continue;
                }
                let alive = region.population as f64 - region.dead;
                let killed = alive * COUNTERMEASURE_KILL_FRACTION;
                region.dead += killed;
                total_killed += killed;
            }
            let killed_str = if total_killed >= 1_000_000_000.0 {
                format!("{:.1}B", total_killed / 1_000_000_000.0)
            } else if total_killed >= 1_000_000.0 {
                format!("{:.1}M", total_killed / 1_000_000.0)
            } else {
                format!("{:.0}", total_killed)
            };
            (Some(format!(
                "⚠ DECREE: Emergency countermeasure deployed. Within-region spread halved. Cross-region spread reduced 75%. Casualties: {}.",
                killed_str
            )), true)
        }
    };

    // Apply chairman satisfaction cost only on successful enactment
    if success {
        if let Some(chairman) = state.board_members.iter_mut().find(|m| m.is_chairman) {
            chairman.add_modifier(ModifierSource::CrisisEffect, chairman_cost);
        }
    }

    (msg, success)
}

/// Execute standing orders for policy automation. Fires each tick.
/// Auto-enables policies for regions that cross infection pressure thresholds,
/// provided the policy isn't already active and the player has the
/// required chairman approval and personnel.
/// Returns region indices where GDP-hurting policies were enacted (for board notification).
pub(super) fn tick_standing_orders(state: &mut WorldState, events: &mut Vec<GameEvent>) -> Vec<usize> {
    // Affordability guard: don't try to auto-enable policies when the player can't
    // sustain the current cost load. Prevents oscillation where cost enforcement
    // suspends a policy and this function immediately re-enables it.
    let mut gdp_regions = Vec::new();
    let current_cost = state.total_policy_funding_cost();
    let num_regions = state.regions.len();
    for region_idx in 0..num_regions {
        if state.regions[region_idx].collapsed {
            continue;
        }
        let infected: f64 = state.regions[region_idx].total_infected();

        // Auto-quarantine at HIGH (10K+)
        if state.standing_orders.auto_quarantine_at_high
            && infected > INFECTION_PRESSURE_HIGH
            && !state.policies[region_idx].quarantine
            && state.resources.funding > current_cost + QUARANTINE_COST
        {
            let (_, ok, gdp_region) = toggle_policy(state, region_idx, PolicyId::Quarantine);
            if ok {
                if let Some(r) = gdp_region { gdp_regions.push(r); }
                let region_name = state.regions[region_idx].name.clone();
                let conflicts = conflicting_contract_names(state, PolicyId::Quarantine);
                let suffix = if conflicts.is_empty() {
                    String::new()
                } else {
                    format!(" (violates {})", conflicts.join(", "))
                };
                events.push(GameEvent::PolicyAutoActivated {
                    region_idx,
                    policy_name: format!("Quarantine in {region_name}{suffix}"),
                });
            }
        }

        // Auto-travel-ban at CRIT (100K+)
        if state.standing_orders.auto_travel_ban_at_crit
            && infected > INFECTION_PRESSURE_CRIT
            && !state.policies[region_idx].travel_ban
            && state.resources.funding > current_cost + TRAVEL_BAN_COST
        {
            let (_, ok, gdp_region) = toggle_policy(state, region_idx, PolicyId::TravelBan);
            if ok {
                if let Some(r) = gdp_region { gdp_regions.push(r); }
                let region_name = state.regions[region_idx].name.clone();
                let conflicts = conflicting_contract_names(state, PolicyId::TravelBan);
                let suffix = if conflicts.is_empty() {
                    String::new()
                } else {
                    format!(" (violates {})", conflicts.join(", "))
                };
                events.push(GameEvent::PolicyAutoActivated {
                    region_idx,
                    policy_name: format!("Travel Ban in {region_name}{suffix}"),
                });
            }
        }
    }
    gdp_regions
}

/// Auto-rebuild infrastructure for regions with auto_rebuild_infra enabled.
/// Fires once per day (every TICKS_PER_DAY ticks) when any infra stat drops below threshold.
pub(super) fn tick_auto_rebuild(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    // Only fire once per day to avoid draining funds every tick
    if state.tick % (TICKS_PER_DAY as u64) != 0 {
        return;
    }
    let num_regions = state.regions.len();
    for region_idx in 0..num_regions {
        if !state.policies[region_idx].auto_rebuild_infra {
            continue;
        }
        if state.regions[region_idx].collapsed {
            continue;
        }
        let region = &state.regions[region_idx];
        let needs_repair = region.healthcare_capacity < REBUILD_INFRA_AUTO_THRESHOLD
            || region.supply_lines < REBUILD_INFRA_AUTO_THRESHOLD
            || region.civil_order < REBUILD_INFRA_AUTO_THRESHOLD;
        if needs_repair {
            let (msg, ok) = rebuild_infrastructure(state, region_idx);
            if ok {
                if let Some(m) = msg {
                    events.push(GameEvent::PolicyAutoActivated {
                        region_idx,
                        policy_name: format!("Auto-rebuild: {}", m),
                    });
                }
            }
        }
    }
}

/// Update screening infrastructure progress and estimated infection counts.
///
/// Each tick:
/// 1. Ramp screening_progress up/down based on whether screening is active.
/// 2. Converge each region's estimated_infected toward real detected infected.
///    Convergence rate depends on screening level and progress — without screening,
///    the estimate lags days behind reality (genuine fog of war). With Mass Rapid
///    at full progress, it tracks near-real-time.
pub(super) fn tick_screening(state: &mut WorldState) {
    let none_rate = ScreeningLevel::None.convergence_rate();

    for i in 0..state.regions.len() {
        // Update screening progress
        let screening = state.policies[i].screening;
        let progress = if screening != ScreeningLevel::None {
            (state.policies[i].screening_progress + SCREENING_RAMP_RATE).min(1.0)
        } else {
            (state.policies[i].screening_progress - SCREENING_DECAY_RATE).max(0.0)
        };
        state.policies[i].screening_progress = progress;

        // Compute effective convergence rate
        let level_rate = screening.convergence_rate();
        let mut effective_rate = none_rate + (level_rate - none_rate) * progress;
        // SurveillanceNetwork specialization: screening converges 50% faster
        if state.regions[i].has_specialization(RegionSpecialization::SurveillanceNetwork) {
            effective_rate *= SURVEILLANCE_NETWORK_SCREENING_MULT;
        }
        // DataInfra sector bonus: screening convergence faster
        let data_bonus = state.sector_bonus(i, crate::state::CorporationSector::DataInfra);
        effective_rate *= 1.0 + crate::state::CorporationSector::DataInfra.max_bonus_pct() / 100.0 * data_bonus;

        // Get real detected infected for this region.
        // Without antigen-level screening, exposed (incubating) people are invisible —
        // they show no symptoms, so only symptomatic cases contribute to the estimate.
        // Gate on progress > 0.5 to match the UI's screening_shows_exposed() — the
        // infrastructure needs meaningful ramp-up before it can detect incubating cases.
        let real = if screening.shows_exposed() && progress > 0.5 {
            state.regions[i].detected_infected(&state.diseases)
        } else {
            state.regions[i].detected_symptomatic(&state.diseases)
        };

        // Apply per-region noise bias. The estimate converges toward a biased target
        // (real * (1 + bias * strength)) rather than toward truth. Noise strength
        // scales with visibility_rate so each screening tier meaningfully improves
        // accuracy: None=100% noise, Basic=69%, Antigen=25%, MassRapid=0%.
        // This means low-screening data is genuinely wrong, not just stale.
        let none_vis = ScreeningLevel::None.visibility_rate();
        let max_vis = ScreeningLevel::MassRapid.visibility_rate();
        let effective_vis = none_vis + (screening.visibility_rate() - none_vis) * progress;
        let noise_strength = 1.0 - ((effective_vis - none_vis) / (max_vis - none_vis)).clamp(0.0, 1.0);
        let bias = state.regions[i].screening_noise_bias;
        let target = real * (1.0 + bias * noise_strength);

        // Converge estimate toward biased target
        let estimated = state.regions[i].estimated_infected;
        let delta = target - estimated;
        state.regions[i].estimated_infected = (estimated + delta * effective_rate).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Authority, DecreeId, GameState, PolicyId, ScreeningLevel};
    use crate::engine::tick;

    /// Helper: set up a state with full POL and plenty of personnel for screening tests.
    fn screening_test_state() -> GameState {
        let mut state = GameState::new_default(42);
        crate::engine::initialize_game(&mut state);
        state.resources.authority = Authority::Maximum;
        state.resources.funding = 10_000.0;
        // Unlock research prerequisites for advanced screening tiers
        state.unlocked_techs.push(crate::state::BasicTech::RapidSequencing);
        state.unlocked_techs.push(crate::state::BasicTech::MetagenomicSurveillance);
        // Unlock all decrees by satisfying every severity threshold:
        // - 3 collapses → unlocks decrees 3,4 (Fortify needs 1+, Emergency needs 3+)
        // - 900K infected across 3 regions → unlocks decree 0 (500K+ infected)
        //   and provides 3 CRIT regions → unlocks decree 1
        // We collapse regions 3-5 and infect regions 0,1,2 to avoid breaking tests
        // that operate on early regions.
        state.regions[3].collapsed = true;
        state.regions[4].collapsed = true;
        state.regions[5].collapsed = true;
        state.regions[0].get_or_create_infection(0).infected = 300_000.0;
        state.regions[1].get_or_create_infection(0).infected = 300_000.0;
        state.regions[2].get_or_create_infection(0).infected = 300_000.0;
        state
    }

    #[test]
    fn screening_mutual_exclusivity() {
        let mut state = screening_test_state();
        // Enable Low screening on region 0
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::BasicScreening);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::Basic);

        // Switch to Medium — should replace Low, not stack
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::AntigenScreening);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::Antigen);

        // Switch to High — replaces Medium
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::MassRapidScreen);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::MassRapid);

        // Toggle High again — disables screening
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::MassRapidScreen);
        assert!(ok);
        assert_eq!(state.policies[0].screening, ScreeningLevel::None);
    }

    #[test]
    fn screening_authority_gating() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        // Basic screening has no authority requirement — always available
        state.resources.authority = Authority::Minimal;
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::BasicScreening);
        assert!(ok, "Basic screening should work at any authority level");

        // Antigen requires RapidSequencing research — blocked without it
        state.resources.authority = Authority::Medium;
        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::AntigenScreening);
        assert!(!ok, "Antigen screening should be blocked without RapidSequencing");
        assert!(msg.unwrap().contains("research"), "should mention research prerequisite");

        // Unlock research but drop authority — blocked by authority
        state.unlocked_techs.push(crate::state::BasicTech::RapidSequencing);
        state.resources.authority = Authority::Minimal;
        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::AntigenScreening);
        assert!(!ok, "Antigen screening should be blocked at Minimal authority");
        assert!(msg.unwrap().contains("Authority"));

        // With research AND enough authority, Antigen should work
        state.resources.authority = Authority::Low;
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::AntigenScreening);
        assert!(ok, "Antigen screening should work with research + Low authority");
    }

    #[test]
    fn screening_upgrade_frees_personnel_from_current_tier() {
        let mut state = screening_test_state();
        // Start with Low screening (1 personnel)
        toggle_policy(&mut state, 0, PolicyId::BasicScreening);
        assert_eq!(state.policies[0].screening, ScreeningLevel::Basic);

        // Use up all remaining personnel except 1 (which is committed to Low screening)
        // Medium needs 2 personnel. With 1 freed from Low, we need 1 available.
        let busy = state.personnel_busy();
        // Set personnel so that available = 0 but we have 1 in Low screening
        state.resources.personnel = busy; // exactly enough for current commitments

        // Upgrade to Medium: needs 2, frees 1 from Low, so needs 1 more available
        // With available=0 and freed=1, effective_available=1 < needed=2 → should fail
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::AntigenScreening);
        assert!(!ok, "should fail: 0 available + 1 freed = 1 < 2 needed");

        // Give 1 more personnel: available=1, freed=1 from Low → effective=2 >= 2
        state.resources.personnel = busy + 1;
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::AntigenScreening);
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

        let tick_result = tick(&state);
        state = state.with_world(tick_result.0);
        let tick_events = tick_result.1;
        assert_eq!(state.policies[0].screening, ScreeningLevel::None,
            "High screening should be suspended when unaffordable");
        assert!(tick_events.iter().any(|e|
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

        state = state.with_world(tick(&state).0);
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
        // High screening at full progress: threshold = 10,000 * 0.15 = 1,500
        state.policies[0].screening = ScreeningLevel::MassRapid;
        state.policies[0].screening_progress = 1.0;
        state.resources.funding = 10_000.0;
        // Set infections to 2,500 (above 1,500 threshold but below 10,000 default)
        state.regions[0].get_or_create_infection(0).infected = 2_500.0;
        // Clear other regions so total is just 2,500
        for r in &mut state.regions[1..] { r.infections.clear(); }

        let (after, _) = tick(&state);
        assert!(after.diseases[0].detected,
            "disease should be detected at 2,500 infected with High screening (threshold 2,000)");

        // Without screening, same infection level should NOT trigger detection
        let mut state2 = state.clone();
        state2.policies[0].screening = ScreeningLevel::None;
        let (after2, _) = tick(&state2);
        assert!(!after2.diseases[0].detected,
            "disease should NOT be detected at 2,500 infected without screening (threshold 10,000)");
    }

    #[test]
    fn screening_convergence_tracks_reality_faster_with_higher_tier() {
        let mut state = screening_test_state();
        // Set a known infection level
        state.regions[0].get_or_create_infection(0).infected = 100_000.0;
        state.regions[0].estimated_infected = 0.0; // start with no estimate

        // Run 120 ticks (~1 day) with no screening — estimate converges slowly
        for _ in 0..120 {
            tick_screening(&mut state);
        }
        let estimate_no_screening = state.regions[0].estimated_infected;

        // Reset and run with Mass Rapid at full progress — converges fast
        let mut state2 = screening_test_state();
        state2.regions[0].get_or_create_infection(0).infected = 100_000.0;
        state2.regions[0].estimated_infected = 0.0;
        state2.policies[0].screening = ScreeningLevel::MassRapid;
        state2.policies[0].screening_progress = 1.0; // fully ramped up
        for _ in 0..120 {
            tick_screening(&mut state2);
        }
        let estimate_mass_rapid = state2.regions[0].estimated_infected;

        assert!(estimate_mass_rapid > estimate_no_screening,
            "Mass Rapid screening should give a higher estimate after 1 day: {estimate_mass_rapid:.0} vs {estimate_no_screening:.0}");

        // Mass Rapid should be very close to reality (100K) — convergence rate 0.15
        assert!(estimate_mass_rapid > 99_000.0,
            "Mass Rapid at full progress should be near-perfect after 1 day: {estimate_mass_rapid:.0}");

        // No screening should be far behind — convergence rate 0.0007 means
        // after 120 ticks: 1 - 0.9993^120 ≈ 8.1% of reality
        assert!(estimate_no_screening < 20_000.0,
            "No screening should lag far behind reality: {estimate_no_screening:.0}");
    }

    #[test]
    fn screening_rampup_prevents_peek_exploit() {
        // Toggling screening on for a single tick should give negligible benefit
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 100_000.0;
        state.regions[0].estimated_infected = 0.0;

        // Enable Mass Rapid screening for 1 tick (the "peek" exploit)
        state.policies[0].screening = ScreeningLevel::MassRapid;
        tick_screening(&mut state);
        let after_peek = state.regions[0].estimated_infected;

        // Disable it immediately
        state.policies[0].screening = ScreeningLevel::None;
        tick_screening(&mut state);

        // The estimate should be nearly zero — ramp-up prevents instant benefit
        assert!(after_peek < 1000.0,
            "Single-tick peek should give negligible info: {after_peek:.0}");
        // Progress should be near-zero
        assert!(state.policies[0].screening_progress < 0.01,
            "Progress should decay when screening disabled");
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

        let (without, _) = tick(&state);
        let deaths_without = without.regions[0].dead;

        // Level 1: Field Hospital — 25% lethality reduction
        state.regions[0].hospital_level = 1;
        let (with_l1, _) = tick(&state);
        let deaths_l1 = with_l1.regions[0].dead;
        let ratio_l1 = deaths_l1 / deaths_without;
        assert!(ratio_l1 > 0.60 && ratio_l1 < 0.90,
            "Field Hospital should reduce deaths by ~25% (ratio: {ratio_l1:.2})");

        // Level 2: Medical Center — 40% lethality reduction
        state.regions[0].hospital_level = 2;
        let (with_l2, _) = tick(&state);
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
        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::FieldHospital);
        assert!(ok, "should succeed with sufficient funds");
        assert_eq!(state.regions[0].hospital_level, 1);
        assert!(msg.unwrap().contains("Field Hospital"));

        let funds_after_l1 = state.resources.funding;
        assert!(funds_after_l1 < 10_000.0, "funding should be deducted");

        // Upgrade to Level 2: Medical Center
        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::FieldHospital);
        assert!(ok, "upgrade should succeed");
        assert_eq!(state.regions[0].hospital_level, 2);
        assert!(msg.unwrap().contains("Medical Center"));
        assert!(state.resources.funding < funds_after_l1, "upgrade should cost funds");

        // Try again — already maxed
        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::FieldHospital);
        assert!(!ok, "should not build past level 2");
        assert!(msg.unwrap().contains("already"));
    }

    #[test]
    fn field_hospital_blocked_for_collapsed_regions() {
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::FieldHospital);
        assert!(!ok, "should not build in collapsed region");
        assert!(msg.unwrap().contains("collapsed"));
    }

    #[test]
    fn conscript_researchers_grants_personnel_and_penalizes_income() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        let personnel_before = state.resources.personnel;
        let income_before = state.funding_income_rate();

        let (msg, ok) = enact_decree(&mut state, DecreeId::ConscriptResearchers, None, &mut events);
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
        let (_, ok) = enact_decree(&mut state, DecreeId::ConscriptResearchers, None, &mut events);
        assert!(!ok, "should not enact twice");
    }

    #[test]
    fn decree_blocked_by_insufficient_severity() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        // Fresh game: no deaths, no collapses — all decrees should be locked

        for decree in DecreeId::ALL {
            let (msg, ok) = enact_decree(&mut state, decree, None, &mut events);
            assert!(!ok, "decree {decree:?} should be blocked when severity is low");
            assert!(msg.unwrap().contains("more severe crisis"), "error message should mention severity");
        }
    }

    #[test]
    fn sacrifice_region_collapses_and_boosts_income() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        let income_before = state.funding_income_rate();
        assert!(!state.regions[0].collapsed);

        let (msg, ok) = enact_decree(&mut state, DecreeId::SacrificeRegion, Some(0), &mut events);
        assert!(ok, "should succeed");
        assert!(msg.unwrap().contains("sacrifice zone"));
        assert!(state.regions[0].collapsed);
        assert!(state.regions[0].collapsed, "sacrificed region should be collapsed");
        assert_eq!(state.enacted_decrees.sacrificed_region, Some(0));
        // Refugee wave should be scheduled
        assert!(state.pending_crises.iter().any(|k| matches!(k, crate::state::CrisisKind::RefugeeWave { from_region: 0, .. })),
            "refugee wave should be scheduled from sacrificed region");
        // RegionCollapsed event should be fired
        assert!(events.iter().any(|e| matches!(e, crate::state::GameEvent::RegionCollapsed { region_idx: 0, .. })),
            "RegionCollapsed event should be fired");

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
        let (_, ok) = enact_decree(&mut state, DecreeId::SacrificeRegion, Some(1), &mut events);
        assert!(!ok, "should not sacrifice twice");
    }

    #[test]
    fn sacrifice_region_requires_region_idx() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();

        let (msg, ok) = enact_decree(&mut state, DecreeId::SacrificeRegion, None, &mut events);
        assert!(!ok, "should require region selection");
        assert!(msg.unwrap().contains("Select"));
    }

    #[test]
    fn sacrifice_region_rejects_already_collapsed() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok) = enact_decree(&mut state, DecreeId::SacrificeRegion, Some(0), &mut events);
        assert!(!ok, "should not sacrifice already collapsed region");
        assert!(msg.unwrap().contains("collapsed"));
    }

    #[test]
    fn negotiate_governor_boosts_cooperation() {
        let mut state = screening_test_state();
        state.regions[0].governor.cooperation = 50.0;
        let funding_before = state.resources.funding;

        let (msg, ok) = negotiate_governor(&mut state, 0);
        assert!(ok, "should succeed with sufficient funds");
        assert!(msg.unwrap().contains("Deal reached"));
        assert!((state.regions[0].governor.cooperation - 65.0).abs() < 0.01);
        assert!((state.resources.funding - (funding_before - crate::state::NEGOTIATE_COST)).abs() < 0.01);
    }

    #[test]
    fn negotiate_governor_blocked_by_insufficient_funds() {
        let mut state = screening_test_state();
        state.resources.funding = 50.0;

        let (_, ok) = negotiate_governor(&mut state, 0);
        assert!(!ok, "should fail without funds");
    }

    #[test]
    fn negotiate_governor_blocked_for_collapsed_region() {
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok) = negotiate_governor(&mut state, 0);
        assert!(!ok, "should fail for collapsed region");
        assert!(msg.unwrap().contains("collapsed"));
    }

    #[test]
    fn negotiate_governor_caps_at_100() {
        let mut state = screening_test_state();
        state.regions[0].governor.cooperation = 95.0;

        let (_, ok) = negotiate_governor(&mut state, 0);
        assert!(ok);
        assert!((state.regions[0].governor.cooperation - 100.0).abs() < 0.01,
            "cooperation should cap at 100: got {}", state.regions[0].governor.cooperation);
    }

    #[test]
    fn governor_cooperation_drifts_with_restrictive_policies() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].governor.cooperation = 60.0;
        state.policies[0].travel_ban = true;
        state.policies[0].quarantine = true;
        state.policies[0].martial_law = true;

        let before = state.regions[0].governor.cooperation;
        // Tick cooperation for ~1 day (120 ticks)
        for _ in 0..120 {
            tick_governor_cooperation(&mut state, &mut events);
        }
        assert!(state.regions[0].governor.cooperation < before,
            "cooperation should decrease with restrictive policies: was {before}, now {}",
            state.regions[0].governor.cooperation);
    }

    #[test]
    fn governor_cooperation_drops_fast_in_crit_region() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].governor.cooperation = 70.0;
        // Put >100K infected so severity = CRIT
        state.regions[0].get_or_create_infection(0).infected = 200_000.0;

        // Tick for 20 days
        for _ in 0..(120 * 20) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        assert!(state.regions[0].governor.cooperation < 45.0,
            "CRIT region should drive cooperation well below 45 in 20 days, got {}",
            state.regions[0].governor.cooperation);
    }

    #[test]
    fn hardliner_governor_drops_faster_than_operative() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 200_000.0;

        // Test Hardliner — less pliable baseline + zero-sum standing pressure
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Hardliner;
        state.regions[0].governor.cooperation = 70.0;
        for _ in 0..(120 * 15) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let hardliner_cooperation = state.regions[0].governor.cooperation;

        // Test Operative — neutral when no income skim
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Operative;
        state.regions[0].governor.cooperation = 70.0;
        for _ in 0..(120 * 15) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let operative_cooperation = state.regions[0].governor.cooperation;

        assert!(hardliner_cooperation < operative_cooperation,
            "Hardliner ({hardliner_cooperation:.1}) should lose cooperation faster than Operative ({operative_cooperation:.1}) in a CRIT region");
    }

    #[test]
    fn governor_hostility_reduces_policy_effectiveness() {
        use crate::state::GOVERNOR_HOSTILITY_THRESHOLD;

        let mut state = screening_test_state();
        state.regions[0].governor.cooperation = GOVERNOR_HOSTILITY_THRESHOLD - 1.0;
        assert!(state.regions[0].governor.is_hostile());
        assert!(state.regions[0].policy_effectiveness() < 1.0);

        state.regions[0].governor.cooperation = GOVERNOR_HOSTILITY_THRESHOLD + 1.0;
        assert!(!state.regions[0].governor.is_hostile());
        assert!((state.regions[0].policy_effectiveness() - 1.0).abs() < 0.001);
    }

    #[test]
    fn governor_cooperation_reduces_costs() {
        use crate::state::GOVERNOR_COOPERATION_THRESHOLD;

        let mut state = screening_test_state();
        state.policies[0].quarantine = true;

        // Normal cooperation — full cost
        state.regions[0].governor.cooperation = 50.0;
        let normal_cost = state.total_policy_funding_cost();

        // Cooperative cooperation — reduced cost
        state.regions[0].governor.cooperation = GOVERNOR_COOPERATION_THRESHOLD + 1.0;
        let coop_cost = state.total_policy_funding_cost();

        assert!(coop_cost < normal_cost,
            "cooperative governor should reduce costs: normal={normal_cost}, coop={coop_cost}");
    }

    #[test]
    fn blowhard_governor_hates_restrictions() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 200_000.0; // CRIT
        state.policies[0].quarantine = true;
        state.policies[0].travel_ban = true;

        // Blowhard with restrictions — extra drain
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Blowhard;
        state.regions[0].governor.cooperation = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let blowhard_cooperation = state.regions[0].governor.cooperation;

        // Operative with same restrictions (baseline — neutral personality mod)
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Operative;
        state.regions[0].governor.cooperation = 70.0;
        for _ in 0..(120 * 10) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let operative_cooperation = state.regions[0].governor.cooperation;

        assert!(blowhard_cooperation < operative_cooperation,
            "Blowhard ({blowhard_cooperation:.1}) should lose cooperation faster than Operative ({operative_cooperation:.1}) with restrictions");
    }

    #[test]
    fn blowhard_governor_happy_without_restrictions() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        // Low infections, no restrictions — blowhard's calm bonus kicks in
        state.regions[0].get_or_create_infection(0).infected = 100.0;
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Blowhard;
        state.regions[0].governor.cooperation = 50.0;

        for _ in 0..(120 * 5) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        assert!(state.regions[0].governor.cooperation > 50.0,
            "Blowhard should gain cooperation with no restrictions and low infections, got {}",
            state.regions[0].governor.cooperation);
    }

    #[test]
    fn mobster_cooperation_decays_faster_with_bargains() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Mobster;

        // No bargains — base decay
        state.regions[0].governor.cooperation = 70.0;
        state.regions[0].governor.bargain_count = 0;
        for _ in 0..(120 * 10) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let no_bargain_cooperation = state.regions[0].governor.cooperation;

        // After 3 bargains — faster decay
        state.regions[0].governor.cooperation = 70.0;
        state.regions[0].governor.bargain_count = 3;
        for _ in 0..(120 * 10) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let many_bargain_cooperation = state.regions[0].governor.cooperation;

        assert!(many_bargain_cooperation < no_bargain_cooperation,
            "Mobster with 3 bargains ({many_bargain_cooperation:.1}) should decay faster than with 0 ({no_bargain_cooperation:.1})");
    }

    #[test]
    fn operative_gains_cooperation_when_skimming() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        state.regions[0].get_or_create_infection(0).infected = 100.0; // low infections
        state.regions[0].governor.personality = crate::state::GovernorPersonality::Operative;

        // Without income skim — neutral
        state.regions[0].governor.income_skim = 0.0;
        state.regions[0].governor.cooperation = 50.0;
        for _ in 0..(120 * 10) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let no_skim_cooperation = state.regions[0].governor.cooperation;

        // With income skim — passive cooperation gain
        state.regions[0].governor.income_skim = 0.10;
        state.regions[0].governor.cooperation = 50.0;
        for _ in 0..(120 * 10) {
            tick_governor_cooperation(&mut state, &mut events);
        }
        let skim_cooperation = state.regions[0].governor.cooperation;

        assert!(skim_cooperation > no_skim_cooperation,
            "Operative with income skim ({skim_cooperation:.1}) should have higher cooperation than without ({no_skim_cooperation:.1})");
    }

    // --- Governor autonomous action tests ---

    fn hostile_governor_state(personality: GovernorPersonality) -> GameState {
        let mut state = GameState::new_default(42);
        state.regions[0].governor.personality = personality;
        state.regions[0].governor.cooperation = 20.0; // well below hostility threshold (40)
        state.regions[0].governor.last_action_tick = 0;
        state.tick = GOVERNOR_ACTION_INTERVAL + 1; // past cooldown
        state
    }

    #[test]
    fn buffoon_governor_breaks_policy() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Buffoon);
        state.policies[0].border_controls = true;

        tick_governor_actions(&mut state, &mut events);

        // Buffoon should accidentally disable the active policy
        assert!(!state.policies[0].border_controls,
            "Buffoon governor should accidentally cancel a policy");
        assert!(events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("accidentally cancelled"))
        ));
    }

    #[test]
    fn blowhard_governor_drains_funding() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Blowhard);
        state.resources.funding = 1000.0;
        let before = state.resources.funding;

        tick_governor_actions(&mut state, &mut events);

        assert!(state.resources.funding < before,
            "Blowhard governor should drain funding for PR");
        assert!(events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("emergency PR"))
        ));
    }

    #[test]
    fn recluse_governor_no_direct_resource_impact() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Recluse);
        let funding_before = state.resources.funding;
        let personnel_before = state.resources.personnel;

        tick_governor_actions(&mut state, &mut events);

        // Recluse doesn't directly drain resources — consequence is through
        // policy_effectiveness (0.4x for Recluse vs 0.7x standard)
        assert_eq!(state.resources.funding, funding_before);
        assert_eq!(state.resources.personnel, personnel_before);
        assert!(events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("unreachable"))
        ));
    }

    #[test]
    fn hardliner_governor_imposes_policy() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Hardliner);
        assert!(!state.policies[0].quarantine);

        tick_governor_actions(&mut state, &mut events);

        assert!(state.policies[0].quarantine,
            "Hardliner governor should unilaterally impose a restrictive policy");
        assert!(events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("imposed"))
        ));
    }

    #[test]
    fn operative_governor_siphons_funding() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Operative);
        state.resources.funding = 1000.0;
        let before = state.resources.funding;

        tick_governor_actions(&mut state, &mut events);

        assert!(state.resources.funding < before,
            "Operative governor should siphon funding");
        assert!(events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("siphoned"))
        ));
    }

    #[test]
    fn mobster_governor_extorts_funding() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Mobster);
        state.resources.funding = 1000.0;
        let before = state.resources.funding;

        tick_governor_actions(&mut state, &mut events);

        assert!(state.resources.funding < before,
            "Mobster governor should extort funding");
        assert!(events.iter().any(|e|
            matches!(e, GameEvent::GovernorAction { description, .. } if description.contains("extorted"))
        ));
    }

    #[test]
    fn recluse_hostile_policy_effectiveness_is_lower() {
        use crate::state::{GOVERNOR_HOSTILITY_EFFECTIVENESS, RECLUSE_HOSTILITY_EFFECTIVENESS};
        let mut state = hostile_governor_state(GovernorPersonality::Recluse);
        assert!(state.regions[0].policy_effectiveness() < GOVERNOR_HOSTILITY_EFFECTIVENESS,
            "Recluse policy effectiveness ({}) should be lower than standard hostility ({})",
            state.regions[0].policy_effectiveness(), GOVERNOR_HOSTILITY_EFFECTIVENESS);
        assert!((state.regions[0].policy_effectiveness() - RECLUSE_HOSTILITY_EFFECTIVENESS).abs() < 0.001);

        // Compare with a non-Recluse hostile governor
        state.regions[0].governor.personality = GovernorPersonality::Hardliner;
        assert!((state.regions[0].policy_effectiveness() - GOVERNOR_HOSTILITY_EFFECTIVENESS).abs() < 0.001,
            "Non-Recluse hostile governor should use standard effectiveness");
    }

    #[test]
    fn governor_actions_respect_cooldown() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Hardliner);
        state.regions[0].governor.last_action_tick = state.tick; // just acted

        tick_governor_actions(&mut state, &mut events);

        // Should not act when on cooldown
        assert!(!events.iter().any(|e| matches!(e, GameEvent::GovernorAction { .. })),
            "Governor should not act when still on cooldown");
    }

    #[test]
    fn governor_actions_only_fire_when_hostile() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Hardliner);
        state.regions[0].governor.cooperation = 50.0; // above threshold

        tick_governor_actions(&mut state, &mut events);

        assert!(!events.iter().any(|e| matches!(e, GameEvent::GovernorAction { .. })),
            "Governor above hostility threshold should not act");
    }

    #[test]
    fn standing_order_auto_quarantine_fires_at_high() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.standing_orders.auto_quarantine_at_high = true;

        // Simulate a region at HIGH severity
        state.regions[0].get_or_create_infection(0).infected = INFECTION_PRESSURE_HIGH + 1.0;
        assert!(!state.policies[0].quarantine, "Quarantine should not be active yet");

        tick_standing_orders(&mut state, &mut events);

        assert!(state.policies[0].quarantine, "Standing order should have enabled quarantine");
        assert!(events.iter().any(|e| matches!(e, GameEvent::PolicyAutoActivated { .. })),
            "PolicyAutoActivated event should have fired");
    }

    #[test]
    fn standing_order_auto_quarantine_does_not_fire_below_threshold() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.standing_orders.auto_quarantine_at_high = true;

        // Below HIGH severity
        state.regions[0].get_or_create_infection(0).infected = 100.0;

        tick_standing_orders(&mut state, &mut events);

        assert!(!state.policies[0].quarantine, "Quarantine should not fire below HIGH threshold");
    }

    #[test]
    fn standing_order_auto_quarantine_skips_already_active() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.standing_orders.auto_quarantine_at_high = true;
        state.policies[0].quarantine = true; // already active
        state.regions[0].get_or_create_infection(0).infected = INFECTION_PRESSURE_HIGH + 1.0;

        tick_standing_orders(&mut state, &mut events);

        // Should not have toggled (would disable it — we only auto-enable)
        assert!(state.policies[0].quarantine, "Should not disable already-active quarantine");
        assert!(!events.iter().any(|e| matches!(e, GameEvent::PolicyAutoActivated { .. })),
            "Should not fire event for already-active policy");
    }

    #[test]
    fn standing_order_auto_travel_ban_fires_at_crit() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.standing_orders.auto_travel_ban_at_crit = true;

        state.regions[0].get_or_create_infection(0).infected = INFECTION_PRESSURE_CRIT + 1.0;
        assert!(!state.policies[0].travel_ban);

        tick_standing_orders(&mut state, &mut events);

        assert!(state.policies[0].travel_ban, "Standing order should have enabled travel ban");
        assert!(events.iter().any(|e| matches!(e, GameEvent::PolicyAutoActivated { .. })),
            "PolicyAutoActivated event should have fired");
    }

    #[test]
    fn standing_order_disabled_does_not_fire() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        // Both standing orders OFF (default)
        state.regions[0].get_or_create_infection(0).infected = INFECTION_PRESSURE_CRIT + 1.0;

        tick_standing_orders(&mut state, &mut events);

        assert!(!state.policies[0].quarantine);
        assert!(!state.policies[0].travel_ban);
        assert!(!events.iter().any(|e| matches!(e, GameEvent::PolicyAutoActivated { .. })));
    }

    #[test]
    fn toggle_policy_warns_about_contract_conflict() {
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.contracts.push(crate::state::FundingContract {
            name: "Hospitality Protection Fund".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::ForbidPolicy { policy: PolicyId::Quarantine },
            template_id: 1,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
            last_bonus_tick: 0,
        });

        // Enable quarantine — should succeed but warn about the contract
        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::Quarantine);
        assert!(ok);
        let msg = msg.unwrap();
        assert!(msg.contains("Quarantine"), "should mention the policy: {msg}");
        assert!(msg.contains("Hospitality Protection Fund"), "should mention the conflicting contract: {msg}");
    }

    #[test]
    fn standing_order_auto_activation_warns_about_contract_conflict() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = GameState::new_default(42);
        state.resources.authority = Authority::Maximum;
        state.standing_orders.auto_quarantine_at_high = true;
        state.contracts.push(crate::state::FundingContract {
            name: "Hospitality Protection Fund".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::ForbidPolicy { policy: PolicyId::Quarantine },
            template_id: 1,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
            last_bonus_tick: 0,
        });
        state.regions[0].get_or_create_infection(0).infected = INFECTION_PRESSURE_HIGH + 1.0;

        tick_standing_orders(&mut state, &mut events);

        assert!(state.policies[0].quarantine);
        let event = events.iter().find(|e| matches!(e, GameEvent::PolicyAutoActivated { .. }));
        assert!(event.is_some(), "PolicyAutoActivated event should have fired");
        if let GameEvent::PolicyAutoActivated { policy_name, .. } = event.unwrap() {
            assert!(policy_name.contains("Hospitality Protection Fund"),
                "auto-activation event should mention conflicting contract: {policy_name}");
        }
    }

    #[test]
    fn fortify_region_restores_target_penalizes_others() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        // Set low infra on target region
        state.regions[0].healthcare_capacity = 0.3;
        state.regions[0].supply_lines = 0.4;
        state.regions[0].civil_order = 0.5;
        // Set high infra on others
        for i in 1..state.regions.len() {
            state.regions[i].healthcare_capacity = 1.0;
            state.regions[i].supply_lines = 1.0;
            state.regions[i].civil_order = 1.0;
        }

        let (msg, ok) = enact_decree(&mut state, DecreeId::FortifyRegion, Some(0), &mut events);
        assert!(ok, "should succeed");
        assert!(msg.unwrap().contains("fortified"));
        assert_eq!(state.enacted_decrees.fortified_region, Some(0));

        // Target region should be at 100%
        assert!((state.regions[0].healthcare_capacity - 1.0).abs() < 0.01);
        assert!((state.regions[0].supply_lines - 1.0).abs() < 0.01);
        assert!((state.regions[0].civil_order - 1.0).abs() < 0.01);

        // Other regions should be penalized by 25%
        let penalty = crate::state::FORTIFY_INFRA_PENALTY;
        for i in 1..state.regions.len() {
            if !state.regions[i].collapsed {
                assert!((state.regions[i].healthcare_capacity - (1.0 - penalty)).abs() < 0.01,
                    "region {} HC should be {}, got {}", i, 1.0 - penalty, state.regions[i].healthcare_capacity);
            }
        }

        // Cannot fortify again
        let (_, ok) = enact_decree(&mut state, DecreeId::FortifyRegion, Some(1), &mut events);
        assert!(!ok, "should not fortify twice");
    }

    #[test]
    fn fortify_region_requires_region_idx() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();

        let (msg, ok) = enact_decree(&mut state, DecreeId::FortifyRegion, None, &mut events);
        assert!(!ok, "should require region selection");
        assert!(msg.unwrap().contains("Select"));
    }

    #[test]
    fn emergency_countermeasure_reduces_disease_params_and_kills_population() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = screening_test_state();
        // Set up disease parameters
        state.diseases[0].within_region_spread = 1.0;
        state.diseases[0].cross_region_spread = 0.5;
        // Reset deaths but collapse 3 regions to keep decree 5 unlocked
        for region in &mut state.regions {
            region.dead = 0.0;
        }
        for i in 3..6 {
            state.regions[i].collapsed = true;
        }
        let total_alive_before: f64 = state.regions.iter()
            .filter(|r| !r.collapsed)
            .map(|r| r.population as f64 - r.dead)
            .sum();

        let (msg, ok) = enact_decree(&mut state, DecreeId::EmergencyCountermeasure, None, &mut events);
        assert!(ok, "should succeed");
        assert!(msg.unwrap().contains("countermeasure"));
        assert!(state.enacted_decrees.emergency_countermeasure);

        // Disease params should be halved/quartered
        let inf_mult = crate::state::COUNTERMEASURE_SPREAD_WITHIN_MULT;
        let spread_mult = crate::state::COUNTERMEASURE_SPREAD_MULT;
        assert!((state.diseases[0].within_region_spread - 1.0 * inf_mult).abs() < 0.01);
        assert!((state.diseases[0].cross_region_spread - 0.5 * spread_mult).abs() < 0.01);

        // Population should have been killed
        let total_dead: f64 = state.regions.iter()
            .filter(|r| !r.collapsed)
            .map(|r| r.dead)
            .sum();
        let kill_frac = crate::state::COUNTERMEASURE_KILL_FRACTION;
        let expected_dead = total_alive_before * kill_frac;
        assert!((total_dead - expected_dead).abs() / expected_dead < 0.01,
            "should kill {:.0} people, got {:.0}", expected_dead, total_dead);

        // Cannot enact again
        let (_, ok) = enact_decree(&mut state, DecreeId::EmergencyCountermeasure, None, &mut events);
        assert!(!ok, "should not enact twice");
    }

    // --- Governor death and succession tests ---

    #[test]
    fn dead_governor_reduces_policy_effectiveness() {
        use crate::state::LEADERLESS_EFFECTIVENESS;
        let mut state = GameState::new_default(42);
        assert!(!state.regions[0].governor.is_dead());
        assert!((state.regions[0].policy_effectiveness() - 1.0).abs() < 0.001);

        state.regions[0].governor.dead = true;
        assert!(state.regions[0].governor.is_dead());
        assert!((state.regions[0].policy_effectiveness() - LEADERLESS_EFFECTIVENESS).abs() < 0.001);
    }

    #[test]
    fn dead_governor_is_not_hostile() {
        let mut state = GameState::new_default(42);
        state.regions[0].governor.cooperation = 10.0; // well below threshold
        assert!(state.regions[0].governor.is_hostile());

        state.regions[0].governor.dead = true;
        assert!(!state.regions[0].governor.is_hostile());
        assert!(!state.regions[0].governor.is_cooperative());
    }

    #[test]
    fn dead_governor_no_autonomous_actions() {
        let mut events: Vec<GameEvent> = Vec::new();
        let mut state = hostile_governor_state(GovernorPersonality::Hardliner);
        state.regions[0].governor.dead = true;
        tick_governor_actions(&mut state, &mut events);

        assert!(events.is_empty(),
            "Dead governor should not take autonomous actions");
    }

    #[test]
    fn governor_succession_replaces_dead_governor() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::SUCCESSOR_COOPERATION;
        let mut state = GameState::new_default(42);
        state.regions[0].governor.dead = true;
        state.regions[0].governor.succession_tick = Some(100);
        state.tick = 100;

        tick_governor_cooperation(&mut state, &mut events);

        assert!(!state.regions[0].governor.is_dead(),
            "Governor should be replaced after succession tick");
        assert!((state.regions[0].governor.cooperation - SUCCESSOR_COOPERATION).abs() < 0.001);
        assert!(events.iter().any(|e| matches!(e, GameEvent::GovernorSucceeded { .. })),
            "Should fire GovernorSucceeded event");
    }

    #[test]
    fn negotiate_blocked_for_dead_governor() {
        let mut state = GameState::new_default(42);
        state.regions[0].governor.dead = true;
        state.resources.funding = 10000.0;

        let (msg, ok) = negotiate_governor(&mut state, 0);
        assert!(!ok, "Should not be able to negotiate with a dead governor");
        assert!(msg.unwrap().contains("leaderless"));
    }

    #[test]
    fn bargain_blocked_for_dead_governor() {
        let mut state = GameState::new_default(42);
        state.regions[0].governor.dead = true;
        state.regions[0].governor.cooperation = 10.0;

        let (msg, ok) = bargain_with_governor(&mut state, 0);
        assert!(!ok, "Should not be able to bargain with a dead governor");
        assert!(msg.unwrap().contains("leaderless"));
    }

    #[test]
    fn rebuild_infra_repairs_degraded_systems() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        state.regions[0].healthcare_capacity = 0.80;
        state.regions[0].supply_lines = 0.70;
        state.regions[0].civil_order = 1.0; // already full

        let (msg, ok, _) = toggle_policy(&mut state, 0, PolicyId::RebuildInfra);
        assert!(ok, "rebuild should succeed: {:?}", msg);
        // HC should repair by 10% (0.80 → 0.90), SL by 10% (0.70 → 0.80), CO unchanged
        assert!((state.regions[0].healthcare_capacity - 0.90).abs() < 0.01,
            "HC: {}", state.regions[0].healthcare_capacity);
        assert!((state.regions[0].supply_lines - 0.80).abs() < 0.01,
            "SL: {}", state.regions[0].supply_lines);
        assert!((state.regions[0].civil_order - 1.0).abs() < 0.01,
            "CO should be unchanged: {}", state.regions[0].civil_order);
    }

    #[test]
    fn rebuild_infra_proportional_cost() {
        use crate::state::REBUILD_INFRA_COST_PER_POINT;
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        let before = state.resources.funding;

        // Only HC degraded by 5%
        state.regions[0].healthcare_capacity = 0.95;
        state.regions[0].supply_lines = 1.0;
        state.regions[0].civil_order = 1.0;

        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::RebuildInfra);
        assert!(ok);
        let cost = before - state.resources.funding;
        let expected = 0.05 * REBUILD_INFRA_COST_PER_POINT; // only 5% of one system
        assert!((cost - expected).abs() < 1.0,
            "cost should be proportional: expected {}, got {}", expected, cost);
    }

    #[test]
    fn rebuild_infra_rejects_when_nothing_to_repair() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        // All at 100%
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::RebuildInfra);
        assert!(!ok, "should reject when nothing needs repair");
    }

    #[test]
    fn rebuild_infra_rejects_when_insufficient_funds() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 1.0; // nearly broke
        state.regions[0].healthcare_capacity = 0.50;
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::RebuildInfra);
        assert!(!ok, "should reject when insufficient funds");
    }

    #[test]
    fn rebuild_infra_rejects_collapsed_region() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        state.regions[0].collapsed = true;
        state.regions[0].healthcare_capacity = 0.0;
        let (_, ok, _) = toggle_policy(&mut state, 0, PolicyId::RebuildInfra);
        assert!(!ok, "should reject for collapsed region");
    }

    #[test]
    fn auto_rebuild_fires_when_infra_below_threshold() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::TICKS_PER_DAY;
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        state.policies[0].auto_rebuild_infra = true;
        state.regions[0].healthcare_capacity = 0.85; // below 90% threshold

        // Set tick to a day boundary so auto-rebuild fires
        state.tick = TICKS_PER_DAY as u64;
        tick_auto_rebuild(&mut state, &mut events);

        assert!(state.regions[0].healthcare_capacity > 0.85,
            "HC should have been repaired: {}", state.regions[0].healthcare_capacity);
    }

    #[test]
    fn auto_rebuild_skips_when_infra_above_threshold() {
        let mut events: Vec<GameEvent> = Vec::new();
        use crate::state::TICKS_PER_DAY;
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        state.policies[0].auto_rebuild_infra = true;
        // All above 90%
        state.regions[0].healthcare_capacity = 0.95;
        state.regions[0].supply_lines = 0.95;
        state.regions[0].civil_order = 0.95;
        let funding_before = state.resources.funding;

        state.tick = TICKS_PER_DAY as u64;
        tick_auto_rebuild(&mut state, &mut events);

        assert!((state.resources.funding - funding_before).abs() < 0.01,
            "should not spend funds when all infra above threshold");
    }
}
