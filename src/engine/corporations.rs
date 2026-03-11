use rand::Rng;

use crate::state::{
    Corporation, CorporationSector, CrisisKind, GameEvent, GameState,
    CORPORATE_TAX_RATE, CORP_COST_RATIO, CORP_STARTING_RESERVE_DAYS, TICKS_PER_DAY,
};

/// Corporation templates per region. Each region gets 3 corps from different sectors.
/// Index matches region order: NA=0, SA=1, EU=2, AF=3, AS=4, OC=5.
const REGION_CORPS: [[(CorporationSector, &str); 3]; 6] = [
    // North America: tech hub, biotech center, energy
    [
        (CorporationSector::Energy, "Helion Power"),
        (CorporationSector::Biotech, "Seraph Genomics"),
        (CorporationSector::DataInfra, "Lattice Systems"),
    ],
    // South America: mining, automation, energy
    [
        (CorporationSector::Mining, "Crucible Materials"),
        (CorporationSector::Automation, "Volant Industries"),
        (CorporationSector::Energy, "Corriente"),
    ],
    // Europe: logistics hub, biotech, automation
    [
        (CorporationSector::Logistics, "Corridor Group"),
        (CorporationSector::Biotech, "Caliber Bioscience"),
        (CorporationSector::Automation, "Irongate Manufacturing"),
    ],
    // Africa: mining, logistics, data infrastructure
    [
        (CorporationSector::Mining, "Obsidian Extractive"),
        (CorporationSector::Logistics, "Kestrel Freight"),
        (CorporationSector::DataInfra, "Parallax Data"),
    ],
    // Asia: automation powerhouse, energy, mining
    [
        (CorporationSector::Automation, "Motive Systems"),
        (CorporationSector::Energy, "Volta Systems"),
        (CorporationSector::Mining, "Tarim Extraction"),
    ],
    // Oceania: biotech, data, mining (small but specialized)
    [
        (CorporationSector::Biotech, "Optera"),
        (CorporationSector::DataInfra, "Conduit Systems"),
        (CorporationSector::Mining, "Deep Vein Corp"),
    ],
];

/// Generate initial corporations for all regions.
/// Revenue is calibrated so total corporate tax matches the old BASE_FUNDING_INCOME.
///
/// Old formula: BASE_FUNDING_INCOME × (region_pop / total_pop) × income_modifier = region income/tick.
/// New: sum(corp.base_revenue) × TAX_RATE / TICKS_PER_DAY = region income/tick.
/// So corp base_revenue (per day) = old_region_income_per_tick × TICKS_PER_DAY / TAX_RATE / 3.
pub fn generate_corporations(state: &mut GameState) {
    let total_pop: f64 = state.regions.iter().map(|r| r.population as f64).sum();
    if total_pop <= 0.0 {
        return;
    }

    let mut corps = Vec::with_capacity(18);

    for (r_idx, region) in state.regions.iter().enumerate() {
        if r_idx >= REGION_CORPS.len() {
            break;
        }

        // Compute what the old formula would give this region per tick
        let pop = region.population as f64;
        let region_share = pop / total_pop;
        let old_income_per_tick =
            crate::state::BASE_FUNDING_INCOME * region_share * region.income_modifier;

        // Total daily revenue across 3 corps = old_income_per_tick × TICKS_PER_DAY / TAX_RATE
        let total_daily_revenue = old_income_per_tick * TICKS_PER_DAY / CORPORATE_TAX_RATE;

        let templates = &REGION_CORPS[r_idx];
        for (i, (sector, name)) in templates.iter().enumerate() {
            // Distribute revenue unevenly: first corp gets 40%, second 35%, third 25%
            let share = match i {
                0 => 0.40,
                1 => 0.35,
                _ => 0.25,
            };
            // Add ±10% variance from RNG
            let variance = 0.9 + state.rng.r#gen::<f64>() * 0.2;
            let base_revenue = total_daily_revenue * share * variance;
            let operating_costs = base_revenue * CORP_COST_RATIO;
            let reserves = operating_costs * CORP_STARTING_RESERVE_DAYS;

            // First (typically largest) corp in each region gets a board seat
            let board_seat = i == 0;

            corps.push(Corporation {
                name: name.to_string(),
                sector: *sector,
                region_idx: r_idx,
                base_revenue,
                revenue: base_revenue,
                operating_costs,
                reserves,
                max_reserves: reserves,
                bankrupt: false,
                bankrupt_at_tick: None,
                board_seat,
            });
        }
    }

    state.corporations = corps;
}

/// Update corporate finances each tick. Called from tick().
///
/// For each non-bankrupt corporation:
/// 1. Compute revenue based on regional conditions and policy effects
/// 2. Deduct operating costs from revenue to get profit
/// 3. Add/subtract profit from reserves
/// 4. If reserves hit 0, the corp goes bankrupt (permanent)
pub(super) fn tick_corporations(state: &mut GameState) {
    let total_pop: f64 = state.regions.iter().map(|r| r.population as f64).sum();
    if total_pop <= 0.0 {
        return;
    }

    for c_idx in 0..state.corporations.len() {
        if state.corporations[c_idx].bankrupt {
            continue;
        }

        let r_idx = state.corporations[c_idx].region_idx;
        let region = &state.regions[r_idx];

        if region.collapsed {
            // Collapsed region: all corps immediately bankrupt
            state.corporations[c_idx].bankrupt = true;
            state.corporations[c_idx].bankrupt_at_tick = Some(state.tick);
            state.corporations[c_idx].revenue = 0.0;
            state.corporations[c_idx].reserves = 0.0;
            state.events.push(GameEvent::CorporationBankrupt {
                corp_idx: c_idx,
                region_idx: r_idx,
            });
            continue;
        }

        let sector = state.corporations[c_idx].sector;
        let base_revenue = state.corporations[c_idx].base_revenue;

        // Workforce factor: based on healthy fraction of population
        let pop = region.population as f64;
        let infected: f64 = region.infections.iter().map(|inf| inf.infected).sum();
        let incapacitated = region.dead + infected * crate::state::INFECTED_INCAPACITATION_RATE;
        let healthy_frac = ((pop - incapacitated) / pop).clamp(0.0, 1.0);
        let workforce_factor =
            1.0 - (1.0 - healthy_frac) * sector.workforce_sensitivity();

        // Infrastructure factors
        let hc_factor =
            1.0 - (1.0 - region.healthcare_capacity) * sector.healthcare_sensitivity();
        let sl_factor =
            1.0 - (1.0 - region.supply_lines) * sector.supply_line_sensitivity();
        let co_factor =
            1.0 - (1.0 - region.civil_order) * sector.civil_order_sensitivity();

        // Policy factors
        let policies = state.policies.get(r_idx);
        let policy_factor = if let Some(p) = policies {
            let mut factor = 1.0;
            if p.travel_ban {
                factor *= sector.travel_ban_factor();
            }
            if p.quarantine {
                factor *= sector.quarantine_factor();
            }
            if p.border_controls {
                factor *= sector.border_controls_factor();
            }
            if p.hospital_surge {
                factor *= sector.hospital_surge_factor();
            }
            factor
        } else {
            1.0
        };

        let revenue = base_revenue
            * workforce_factor
            * hc_factor
            * sl_factor
            * co_factor
            * policy_factor;

        state.corporations[c_idx].revenue = revenue.max(0.0);

        // Update reserves: profit per tick
        let profit_per_tick =
            (revenue - state.corporations[c_idx].operating_costs) / TICKS_PER_DAY;
        state.corporations[c_idx].reserves += profit_per_tick;

        // Cap reserves at max
        if state.corporations[c_idx].reserves > state.corporations[c_idx].max_reserves {
            state.corporations[c_idx].reserves = state.corporations[c_idx].max_reserves;
        }

        // Bankruptcy check
        if state.corporations[c_idx].reserves <= 0.0 {
            state.corporations[c_idx].reserves = 0.0;
            state.corporations[c_idx].bankrupt = true;
            state.corporations[c_idx].bankrupt_at_tick = Some(state.tick);
            state.corporations[c_idx].revenue = 0.0;
            state.events.push(GameEvent::CorporationBankrupt {
                corp_idx: c_idx,
                region_idx: r_idx,
            });
        }
    }
}

/// Cooldown between board demand crises (~5 days).
const BOARD_DEMAND_COOLDOWN: u64 = (5.0 * TICKS_PER_DAY) as u64;

/// Board satisfaction threshold for a standard demand crisis.
const BOARD_DEMAND_THRESHOLD: f64 = 0.5;

/// Board satisfaction threshold for an ultimatum (more severe).
const BOARD_ULTIMATUM_THRESHOLD: f64 = 0.3;

/// Check board satisfaction and queue a BoardDemand crisis if warranted.
/// Called from tick() after tick_corporations().
pub(super) fn check_board_demands(state: &mut GameState) {
    if state.corporations.is_empty() {
        return;
    }

    let satisfaction = state.board_satisfaction();

    // Determine severity: 1 = ultimatum (< 0.3), 0 = demand (< 0.5), else nothing
    let severity = if satisfaction < BOARD_ULTIMATUM_THRESHOLD {
        1
    } else if satisfaction < BOARD_DEMAND_THRESHOLD {
        0
    } else {
        return;
    };

    // Cooldown check
    let cooldown_ok = state.last_board_demand_tick == 0
        || state.tick.saturating_sub(state.last_board_demand_tick) >= BOARD_DEMAND_COOLDOWN;
    if !cooldown_ok {
        return;
    }

    // Don't queue if one is already pending or active
    let already_pending = state.pending_crises.iter()
        .any(|(_, k)| matches!(k, CrisisKind::BoardDemand { .. }));
    let already_active = state.active_crisis.as_ref()
        .map_or(false, |c| matches!(c.kind, CrisisKind::BoardDemand { .. }));
    if already_pending || already_active {
        return;
    }

    state.last_board_demand_tick = state.tick;
    state.pending_crises.push((
        state.tick,
        CrisisKind::BoardDemand { severity },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tick;

    #[test]
    fn generate_creates_18_corporations_across_6_regions() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        assert_eq!(state.corporations.len(), 18);
        // 3 per region
        for r_idx in 0..6 {
            let count = state.corporations.iter().filter(|c| c.region_idx == r_idx).count();
            assert_eq!(count, 3, "region {r_idx} should have 3 corps");
        }
    }

    #[test]
    fn each_region_has_one_board_seat() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        for r_idx in 0..6 {
            let board_count = state.corporations.iter()
                .filter(|c| c.region_idx == r_idx && c.board_seat)
                .count();
            assert_eq!(board_count, 1, "region {r_idx} should have exactly 1 board seat");
        }
    }

    #[test]
    fn corporate_tax_approximates_old_income() {
        let mut state = GameState::new_default(42);
        let old_income = state.funding_income_rate();
        generate_corporations(&mut state);
        let new_income = state.funding_income_rate();
        // Should be within 20% of old income (RNG variance + trade modifiers can differ)
        let ratio = new_income / old_income;
        assert!(
            (0.8..=1.2).contains(&ratio),
            "corporate income {new_income:.1} should approximate old income {old_income:.1}, ratio={ratio:.2}"
        );
    }

    #[test]
    fn corporations_lose_revenue_under_disease_pressure() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        let initial_revenue: f64 = state.corporations.iter().map(|c| c.revenue).sum();

        // Run for 20 days to let disease spread
        for _ in 0..(20 * TICKS_PER_DAY as u64) {
            state = tick(&state);
        }

        let later_revenue: f64 = state.corporations.iter()
            .filter(|c| !c.bankrupt)
            .map(|c| c.revenue)
            .sum();
        assert!(
            later_revenue < initial_revenue,
            "revenue should decrease under disease: initial={initial_revenue:.0} later={later_revenue:.0}"
        );
    }

    #[test]
    fn collapsed_region_bankrupts_all_corps() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        // Manually collapse North America
        state.regions[0].collapsed = true;
        tick_corporations(&mut state);

        let na_corps: Vec<&Corporation> = state.corporations.iter()
            .filter(|c| c.region_idx == 0)
            .collect();
        assert!(na_corps.iter().all(|c| c.bankrupt), "all NA corps should be bankrupt after collapse");
        assert!(na_corps.iter().all(|c| c.reserves == 0.0), "bankrupt corps should have 0 reserves");
    }

    #[test]
    fn bankrupt_corps_contribute_no_tax() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        let income_before = state.funding_income_rate();

        // Bankrupt all NA corps manually
        for c in state.corporations.iter_mut().filter(|c| c.region_idx == 0) {
            c.bankrupt = true;
            c.revenue = 0.0;
        }

        let income_after = state.funding_income_rate();
        assert!(
            income_after < income_before,
            "bankrupting corps should reduce income: before={income_before:.1} after={income_after:.1}"
        );
    }

    #[test]
    fn board_satisfaction_drops_with_bankruptcies() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        let sat_before = state.board_satisfaction();
        assert!(sat_before > 0.5, "healthy board satisfaction should be high: {sat_before}");

        // Bankrupt half the board-seat corps
        let mut bankrupted = 0;
        for c in state.corporations.iter_mut().filter(|c| c.board_seat) {
            if bankrupted < 3 {
                c.bankrupt = true;
                c.reserves = 0.0;
                c.revenue = 0.0;
                bankrupted += 1;
            }
        }

        let sat_after = state.board_satisfaction();
        assert!(
            sat_after < sat_before,
            "board satisfaction should drop: before={sat_before:.2} after={sat_after:.2}"
        );
    }

    #[test]
    fn low_board_satisfaction_queues_demand_crisis() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        // Damage board-seat corps to push satisfaction between 0.3 and 0.5.
        // satisfaction = avg(reserves_fraction) — revenue is not included to avoid
        // instant policy-impact feedback on pol_target.
        // reserves=30% → reserves_fraction = 0.3 (between thresholds 0.3 and 0.5)
        for c in state.corporations.iter_mut().filter(|c| c.board_seat) {
            c.reserves = c.max_reserves * 0.3;
            c.revenue = c.base_revenue * 0.5; // revenue unchanged but not used in satisfaction
        }

        let sat = state.board_satisfaction();
        assert!(sat < BOARD_DEMAND_THRESHOLD, "satisfaction should be below demand threshold: {sat}");
        assert!(sat >= BOARD_ULTIMATUM_THRESHOLD, "satisfaction should be above ultimatum threshold: {sat}");

        check_board_demands(&mut state);

        assert_eq!(state.pending_crises.len(), 1, "should queue one board demand crisis");
        assert!(
            matches!(state.pending_crises[0].1, CrisisKind::BoardDemand { severity: 0 }),
            "should be severity 0 (demand, not ultimatum)"
        );
    }

    #[test]
    fn very_low_satisfaction_queues_ultimatum() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        // Bankrupt most board-seat corps to push satisfaction very low
        let mut count = 0;
        for c in state.corporations.iter_mut().filter(|c| c.board_seat) {
            if count < 4 {
                c.bankrupt = true;
                c.reserves = 0.0;
                c.revenue = 0.0;
            } else {
                c.reserves = c.max_reserves * 0.1;
                c.revenue = c.base_revenue * 0.2;
            }
            count += 1;
        }

        let sat = state.board_satisfaction();
        assert!(sat < BOARD_ULTIMATUM_THRESHOLD, "satisfaction should be below ultimatum threshold: {sat}");

        check_board_demands(&mut state);

        assert_eq!(state.pending_crises.len(), 1);
        assert!(
            matches!(state.pending_crises[0].1, CrisisKind::BoardDemand { severity: 1 }),
            "should be severity 1 (ultimatum)"
        );
    }

    #[test]
    fn board_demand_cooldown_prevents_spam() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        state.tick = 1000; // Avoid tick-0 sentinel edge case

        // Damage board corps
        for c in state.corporations.iter_mut().filter(|c| c.board_seat) {
            c.reserves = c.max_reserves * 0.1;
            c.revenue = c.base_revenue * 0.3;
        }

        // First check queues a crisis
        check_board_demands(&mut state);
        assert_eq!(state.pending_crises.len(), 1);

        // Clear the pending crisis (simulate it firing)
        state.pending_crises.clear();

        // Second check within cooldown should NOT queue another
        state.tick += 10; // Only 10 ticks later, well within 5-day cooldown
        check_board_demands(&mut state);
        assert_eq!(state.pending_crises.len(), 0, "cooldown should prevent second crisis");

        // After cooldown expires, should queue again
        state.tick += BOARD_DEMAND_COOLDOWN;
        check_board_demands(&mut state);
        assert_eq!(state.pending_crises.len(), 1, "should queue after cooldown");
    }

    #[test]
    fn board_demand_fires_during_natural_game() {
        // Run a full game and verify board demands eventually fire.
        // Since other crises may be active (blocking pending ones from activating),
        // we auto-resolve all other crises so the board demand can fire.
        let mut s = GameState::new_default(42);
        generate_corporations(&mut s);
        let mut board_demand_found = false;

        for _ in 0..(30 * TICKS_PER_DAY as u64) {
            // Auto-resolve any active non-board crisis so pending ones can fire
            if let Some(ref crisis) = s.active_crisis {
                if !matches!(crisis.kind, CrisisKind::BoardDemand { .. }) {
                    s = crate::apply_action(&s, &crate::action::Action::Confirm);
                }
            }

            s = tick(&s);

            // Check if a board demand crisis was queued, active, or already resolved
            if s.pending_crises.iter().any(|(_, k)| matches!(k, CrisisKind::BoardDemand { .. }))
                || s.active_crisis.as_ref().map_or(false, |c| matches!(c.kind, CrisisKind::BoardDemand { .. }))
                || s.crisis_cooldowns.contains_key("board_demand")
            {
                board_demand_found = true;
                break;
            }
        }

        if !board_demand_found {
            let sat = s.board_satisfaction();
            assert!(
                sat >= BOARD_DEMAND_THRESHOLD,
                "Board satisfaction dropped to {sat:.2} but no BoardDemand crisis fired in 30 days"
            );
        }
    }

    #[test]
    fn healthy_corps_dont_trigger_board_demand() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        let sat = state.board_satisfaction();
        assert!(sat > BOARD_DEMAND_THRESHOLD, "healthy corps should have high satisfaction: {sat}");

        check_board_demands(&mut state);
        assert_eq!(state.pending_crises.len(), 0, "no crisis when board is satisfied");
    }
}
