use crate::state::{
    GameEvent, GameState, ScreeningLevel, policy_display_name,
    BORDER_CONTROLS_COST, BORDER_CONTROLS_PERSONNEL,
    HEALTHCARE_INVESTMENT_COST,
    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL,
    MARTIAL_LAW_COST, MARTIAL_LAW_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    QUARANTINE_COST, QUARANTINE_PERSONNEL,
    TICKS_PER_DAY, TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL,
    WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
};

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
    let region_name = state.regions.get(region_idx)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    // Check POL requirement (only when enabling, not disabling)
    let is_currently_active = match policy_idx {
        0..=4 | 8 | 9 => state.policies[region_idx].get_bool(policy_idx),
        5 => state.policies[region_idx].screening == ScreeningLevel::Basic,
        6 => state.policies[region_idx].screening == ScreeningLevel::Antigen,
        7 => state.policies[region_idx].screening == ScreeningLevel::MassRapid,
        10 => state.regions[region_idx].healthcare_invested,
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
        // Healthcare Investment (10): one-time per-region permanent upgrade
        10 => {
            let region = &state.regions[region_idx];
            if region.healthcare_invested {
                (Some(format!("{region_name} already has healthcare infrastructure")), false)
            } else if region.collapsed {
                (Some(format!("{region_name} has collapsed — cannot invest")), false)
            } else if state.resources.funding < HEALTHCARE_INVESTMENT_COST {
                (Some(format!("Not enough funding (need ${:.0})", HEALTHCARE_INVESTMENT_COST)), false)
            } else {
                state.resources.funding -= HEALTHCARE_INVESTMENT_COST;
                state.regions[region_idx].healthcare_invested = true;
                (Some(format!("Healthcare infrastructure built in {region_name} — lethality reduced 25%")), true)
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
        return (Some(format!("Not enough funding (need ${RALLY_COST:.0})")), false);
    }

    state.resources.funding -= RALLY_COST;
    state.resources.last_rally_tick = Some(state.tick);
    state.resources.political_power = (state.resources.political_power + RALLY_POL_GAIN).min(1.0);

    let pol_pct = state.resources.political_power * 100.0;
    (Some(format!("Rally successful! POL +{:.0}% → {pol_pct:.0}%", RALLY_POL_GAIN * 100.0)), true)
}

/// Enact an emergency decree. Permanent, irreversible.
/// Returns (message, success).
pub(super) fn enact_decree(state: &mut GameState, decree_idx: usize, region_idx: Option<usize>) -> (Option<String>, bool) {
    use crate::state::{
        decree_display_name, DECREE_POL_THRESHOLDS,
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

    // POL check
    let pol = state.resources.political_power;
    let threshold = DECREE_POL_THRESHOLDS[decree_idx];
    if pol < threshold {
        return (Some(format!(
            "{} requires {:.0}% Political Power (current: {:.0}%)",
            decree_display_name(decree_idx), threshold * 100.0, pol * 100.0
        )), false);
    }

    match decree_idx {
        0 => {
            // Conscript Researchers: +personnel, permanent income penalty
            state.enacted_decrees.conscript_researchers = true;
            state.resources.personnel += CONSCRIPT_PERSONNEL_GAIN;
            let penalty_per_day = CONSCRIPT_INCOME_PENALTY * TICKS_PER_DAY;
            (Some(format!(
                "⚠ DECREE: Conscript Researchers — +{} personnel, -${:.0}/day income permanently",
                CONSCRIPT_PERSONNEL_GAIN, penalty_per_day
            )), true)
        }
        1 => {
            // Authorize Human Trials: faster clinical trials, risk of adverse events
            state.enacted_decrees.authorize_human_trials = true;
            (Some(
                "⚠ DECREE: Authorize Human Trials — clinical trials 50% faster, risk of adverse events".to_string()
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
            // Clear policies
            if let Some(p) = state.policies.get_mut(r_idx) {
                p.clear_all();
            }
            let bonus_pct = (SACRIFICE_INCOME_BONUS - 1.0) * 100.0;
            (Some(format!(
                "⚠ DECREE: {} sacrificed — +{:.0}% income from remaining regions",
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
        state.policies[0].screening = ScreeningLevel::MassRapid; // $0.6/tick
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
        // Set up: High screening ($0.6/tick) + quarantine ($0.6/tick) = $1.2/tick
        state.policies[0].screening = ScreeningLevel::MassRapid;
        state.policies[0].quarantine = true;
        // Enough for one but not both
        state.resources.funding = 0.8;
        for r in &mut state.regions { r.infections.clear(); }

        state = tick(&state);
        // Both cost $0.6; one should be suspended. The enforcement loop finds
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
        state.regions[0].infections[0].infected = 2_500.0;
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
        state.regions[0].infections[0].infected = 100_000.0;

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
    fn healthcare_investment_reduces_lethality() {
        let mut state = screening_test_state();
        // Set up a region with significant infections for measurable deaths
        state.regions[0].infections[0].infected = 100_000.0;
        state.diseases[0].lethality = 0.01;

        // Run without healthcare investment
        let without = tick(&state);
        let deaths_without = without.regions[0].dead;

        // Now invest in healthcare
        state.regions[0].healthcare_invested = true;
        let with = tick(&state);
        let deaths_with = with.regions[0].dead;

        // Healthcare reduces lethality by 25%, so deaths should be ~75% of baseline
        assert!(deaths_with < deaths_without,
            "healthcare should reduce deaths: {deaths_with:.1} vs {deaths_without:.1}");
        let ratio = deaths_with / deaths_without;
        assert!(ratio > 0.60 && ratio < 0.90,
            "healthcare should reduce deaths by ~25% (ratio: {ratio:.2})");
    }

    #[test]
    fn healthcare_investment_toggle() {
        let mut state = screening_test_state();

        // Purchase healthcare
        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(ok, "should succeed with sufficient funds");
        assert!(state.regions[0].healthcare_invested);
        assert!(msg.unwrap().contains("lethality reduced"));
        let expected_cost = state.resources.funding;
        assert!(expected_cost < 10_000.0, "funding should be deducted");

        // Try to purchase again — should fail
        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(!ok, "should not purchase twice");
        assert!(msg.unwrap().contains("already"));
    }

    #[test]
    fn healthcare_blocked_for_collapsed_regions() {
        let mut state = screening_test_state();
        state.regions[0].collapsed = true;

        let (msg, ok) = toggle_policy(&mut state, 0, 10);
        assert!(!ok, "should not invest in collapsed region");
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
    fn decree_blocked_by_insufficient_pol() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 10_000.0;
        state.resources.political_power = 0.10; // Below all decree thresholds

        for i in 0..crate::state::DECREE_COUNT {
            let (msg, ok) = enact_decree(&mut state, i, None);
            assert!(!ok, "decree {i} should be blocked at low POL");
            assert!(msg.unwrap().contains("Political Power"));
        }
    }

    #[test]
    fn sacrifice_region_collapses_and_boosts_income() {
        let mut state = screening_test_state();
        let income_before = state.funding_income_rate();
        assert!(!state.regions[0].collapsed);

        let (msg, ok) = enact_decree(&mut state, 2, Some(0));
        assert!(ok, "should succeed");
        assert!(msg.unwrap().contains("sacrificed"));
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
}
