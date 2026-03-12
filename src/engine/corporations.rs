use rand::Rng;
use rand::seq::SliceRandom;

use crate::state::{
    Corporation, CorporationSector, GameEvent, GameState,
    CORPORATE_TAX_RATE, CORP_COST_RATIO, CORP_STARTING_RESERVE_DAYS, TICKS_PER_DAY,
};

/// Maximum number of daily price samples kept in price_history for sparkline.
const PRICE_HISTORY_MAX: usize = 30;

/// Corporation templates per region. Each region gets 3 corps from different sectors.
/// Tuple: (sector, corp_name, director_surname).
/// Index matches region order: NA=0, SA=1, EU=2, AF=3, AS=4, OC=5.
const REGION_CORPS: [[(CorporationSector, &str, &str); 3]; 6] = [
    // North America
    [
        (CorporationSector::Energy, "Helion Power", "Caldwell"),
        (CorporationSector::Biotech, "Seraph Genomics", "Prewitt"),
        (CorporationSector::DataInfra, "Lattice Systems", "Nakamura"),
    ],
    // South America
    [
        (CorporationSector::Mining, "Crucible Materials", "Ferreira"),
        (CorporationSector::Automation, "Volant Industries", "Salazar"),
        (CorporationSector::Energy, "Corriente", "Arriaga"),
    ],
    // Europe
    [
        (CorporationSector::Logistics, "Corridor Group", "Tessier"),
        (CorporationSector::Biotech, "Caliber Bioscience", "Mertens"),
        (CorporationSector::Automation, "Irongate Manufacturing", "Sokolova"),
    ],
    // Africa
    [
        (CorporationSector::Mining, "Obsidian Extractive", "Diallo"),
        (CorporationSector::Logistics, "Kestrel Freight", "Mensah"),
        (CorporationSector::DataInfra, "Parallax Data", "Okoro"),
    ],
    // Asia
    [
        (CorporationSector::Automation, "Motive Systems", "Fujimoto"),
        (CorporationSector::Energy, "Volta Systems", "Bhandari"),
        (CorporationSector::Mining, "Tarim Extraction", "Kuznetsov"),
    ],
    // Oceania
    [
        (CorporationSector::Biotech, "Optera", "Macalister"),
        (CorporationSector::DataInfra, "Conduit Systems", "Rangi"),
        (CorporationSector::Mining, "Deep Vein Corp", "Whitford"),
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
        for (i, (sector, name, director_surname)) in templates.iter().enumerate() {
            // Distribute revenue unevenly: first corp gets 40%, second 35%, third 25%
            let share = match i {
                0 => 0.40,
                1 => 0.35,
                _ => 0.25,
            };
            // Add ±10% variance from RNG
            let variance = 0.9 + state.rng_misc.r#gen::<f64>() * 0.2;
            let base_revenue = total_daily_revenue * share * variance;
            let operating_costs = base_revenue * CORP_COST_RATIO;
            let reserves = operating_costs * CORP_STARTING_RESERVE_DAYS;

            // Board seat assigned randomly after all corps are created (see below)
            let board_seat = false;

            // IPO price scales with revenue: target ~¥50-200 range
            let ipo_price = (base_revenue * 1.5).clamp(20.0, 500.0);
            corps.push(Corporation {
                name: name.to_string(),
                director_surname: director_surname.to_string(),
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
                share_price: ipo_price,
                ipo_price,
                price_history: vec![ipo_price],
            });
        }
    }

    state.corporations = corps;

    // Randomly assign 6 board seats — no distribution constraints.
    // Multiple board members can stack in the same region, creating
    // strategic asymmetry where some regions matter more to the board.
    let mut all_indices: Vec<usize> = (0..state.corporations.len()).collect();
    all_indices.shuffle(&mut state.rng_misc);
    for &idx in all_indices.iter().take(6) {
        state.corporations[idx].board_seat = true;
    }

    // Assign manufacturing contracts to medicines.
    // Each non-broad-spectrum medicine gets a corporation as its manufacturer.
    // Biotech corps are prioritized but any corp can manufacture.
    // This creates a strategic dimension: choosing a medicine manufactured by a
    // board-connected corp boosts that board member's satisfaction on development.
    assign_manufacturers(state);
}

/// Assign manufacturing corporations to medicines that don't have one yet.
/// Called after corporations are generated, and again when new diseases emerge
/// (which create new medicines). Only touches medicines with `manufacturer_corp_idx: None`
/// that aren't already unlocked.
pub fn assign_manufacturers(state: &mut GameState) {
    if state.corporations.is_empty() {
        return;
    }

    // Build a pool of corporation indices, prioritizing Biotech corps
    // (natural pharma manufacturers) but including others for variety.
    let mut biotech_corps: Vec<usize> = state.corporations.iter()
        .enumerate()
        .filter(|(_, c)| c.sector == CorporationSector::Biotech && !c.bankrupt)
        .map(|(i, _)| i)
        .collect();
    let mut other_corps: Vec<usize> = state.corporations.iter()
        .enumerate()
        .filter(|(_, c)| c.sector != CorporationSector::Biotech && !c.bankrupt)
        .map(|(i, _)| i)
        .collect();

    // Shuffle both pools for per-run variety
    use rand::seq::SliceRandom;
    biotech_corps.shuffle(&mut state.rng_misc);
    other_corps.shuffle(&mut state.rng_misc);

    // Interleave: biotech first, then others, cycling through
    let mut pool: Vec<usize> = Vec::with_capacity(biotech_corps.len() + other_corps.len());
    pool.extend(&biotech_corps);
    pool.extend(&other_corps);

    if pool.is_empty() {
        return;
    }

    let mut pool_idx = 0;
    for med in &mut state.medicines {
        // Skip already-assigned, already-unlocked, or broad-spectrum medicines
        if med.manufacturer_corp_idx.is_some() || med.unlocked {
            continue;
        }
        med.manufacturer_corp_idx = Some(pool[pool_idx % pool.len()]);
        pool_idx += 1;
    }
}

/// Update corporate finances each tick. Called from tick().
///
/// For each non-bankrupt corporation:
/// 1. Compute revenue based on regional conditions and policy effects
/// 2. Deduct operating costs from revenue to get profit
/// 3. Add/subtract profit from reserves
/// 4. If reserves hit 0, the corp goes bankrupt (permanent)
pub(super) fn tick_corporations(state: &mut GameState, rng_misc: &mut rand_chacha::ChaCha8Rng) {
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
            if p.discourage_hosp {
                factor *= sector.discourage_hosp_factor();
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

    // Update share prices once per day based on revenue performance + noise.
    if state.tick > 0 && state.tick % (TICKS_PER_DAY as u64) == 0 {
        tick_share_prices(state, rng_misc);
    }
}

/// Update share prices for all corporations.
/// Price = f(revenue_ratio, reserves_fraction) + random walk noise.
/// Called once per day from tick_corporations.
fn tick_share_prices(state: &mut GameState, rng_misc: &mut rand_chacha::ChaCha8Rng) {
    for c_idx in 0..state.corporations.len() {
        let corp = &state.corporations[c_idx];

        if corp.bankrupt {
            // Bankrupt corps crash to near-zero
            state.corporations[c_idx].share_price = 0.01;
            state.corporations[c_idx].price_history.push(0.01);
            if state.corporations[c_idx].price_history.len() > PRICE_HISTORY_MAX {
                state.corporations[c_idx].price_history.remove(0);
            }
            continue;
        }

        let revenue_ratio = if corp.base_revenue > 0.0 {
            corp.revenue / corp.base_revenue
        } else {
            0.0
        };
        let reserves_frac = corp.reserves_fraction();

        // Fair value: IPO price × weighted performance
        let fair_value = corp.ipo_price * (0.6 * revenue_ratio + 0.4 * reserves_frac);

        // Mean-revert toward fair value with random walk
        let old_price = corp.share_price;
        let reversion = 0.15 * (fair_value - old_price);
        let noise = (rng_misc.r#gen::<f64>() - 0.5) * old_price * 0.08;
        let new_price = (old_price + reversion + noise).max(0.01);

        state.corporations[c_idx].share_price = new_price;
        state.corporations[c_idx].price_history.push(new_price);
        if state.corporations[c_idx].price_history.len() > PRICE_HISTORY_MAX {
            state.corporations[c_idx].price_history.remove(0);
        }
    }
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
    fn six_random_board_seats_assigned() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        let board_count = state.corporations.iter().filter(|c| c.board_seat).count();
        assert_eq!(board_count, 6, "should have exactly 6 board seats");
        // Stacking is allowed — no per-region constraint
    }

    #[test]
    fn board_budget_set_after_initialization() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        crate::engine::board::generate_board_members(&mut state);
        let income = state.funding_income_rate();
        assert!(
            income > 0.0,
            "board budget should produce positive income after init: {income:.4}"
        );
        let budget_day = state.board_budget_per_tick * TICKS_PER_DAY;
        assert!(
            budget_day > 100.0 && budget_day < 1000.0,
            "daily board budget should be reasonable: ¥{budget_day:.0}"
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
        let mut rng = state.rng_misc.clone();
        tick_corporations(&mut state, &mut rng);

        let na_corps: Vec<&Corporation> = state.corporations.iter()
            .filter(|c| c.region_idx == 0)
            .collect();
        assert!(na_corps.iter().all(|c| c.bankrupt), "all NA corps should be bankrupt after collapse");
        assert!(na_corps.iter().all(|c| c.reserves == 0.0), "bankrupt corps should have 0 reserves");
    }

    #[test]
    fn bankrupt_corps_dont_affect_fixed_budget() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);
        crate::engine::board::generate_board_members(&mut state);

        let income_before = state.funding_income_rate();

        // Bankrupt all NA corps — should NOT change income (board budget is fixed)
        for c in state.corporations.iter_mut().filter(|c| c.region_idx == 0) {
            c.bankrupt = true;
            c.revenue = 0.0;
        }

        let income_after = state.funding_income_rate();
        assert!(
            (income_after - income_before).abs() < 0.001,
            "board budget should be fixed: before={income_before:.1} after={income_after:.1}"
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
    fn manufacturers_assigned_to_locked_medicines() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        // Every locked (not-yet-developed) medicine should have a manufacturer
        for (i, med) in state.medicines.iter().enumerate() {
            if !med.unlocked {
                assert!(
                    med.manufacturer_corp_idx.is_some(),
                    "locked medicine {} ({}) should have a manufacturer",
                    i, med.name
                );
                let corp_idx = med.manufacturer_corp_idx.unwrap();
                assert!(
                    corp_idx < state.corporations.len(),
                    "manufacturer index {} out of bounds for medicine {}",
                    corp_idx, med.name
                );
            }
        }
    }

    #[test]
    fn broad_spectrum_has_no_manufacturer() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        // The starting broad-spectrum medicine is unlocked and should have no manufacturer
        let broad = state.medicines.iter().find(|m| m.unlocked).unwrap();
        assert!(
            broad.manufacturer_corp_idx.is_none(),
            "starting broad-spectrum should not have a manufacturer"
        );
    }

    #[test]
    fn some_manufacturers_have_board_seats() {
        // With random board seat assignment, check across multiple seeds
        // that both board-connected and non-board manufacturers exist
        let mut found_board = false;
        let mut found_non_board = false;
        for seed in 0..10 {
            let mut state = GameState::new_default(seed);
            generate_corporations(&mut state);

            for med in &state.medicines {
                if !med.unlocked {
                    if let Some(ci) = med.manufacturer_corp_idx {
                        if state.corporations[ci].board_seat {
                            found_board = true;
                        } else {
                            found_non_board = true;
                        }
                    }
                }
            }
            if found_board && found_non_board { break; }
        }
        assert!(found_board, "across seeds, at least one medicine should have a board-connected manufacturer");
        assert!(found_non_board, "across seeds, at least one medicine should have a non-board manufacturer");
    }

    #[test]
    fn develop_medicine_boosts_board_corp_reserves() {
        use crate::state::{ResearchKind, ResearchProject};

        // Try multiple seeds to find one where a locked medicine has a board-seat manufacturer
        let mut state = GameState::new_default(0);
        let mut med_idx = 0;
        let mut found = false;
        for seed in 0..20 {
            state = GameState::new_default(seed);
            generate_corporations(&mut state);
            state.diseases[0].knowledge = 1.0;
            if let Some((i, _)) = state.medicines.iter().enumerate().find(|(_, m)| {
                !m.unlocked && m.manufacturer_corp_idx.map_or(false, |ci| {
                    state.corporations.get(ci).map_or(false, |c| c.board_seat)
                })
            }) {
                med_idx = i;
                found = true;
                break;
            }
        }
        assert!(found, "should find a seed with a board-seat manufacturer medicine");

        let corp_idx = state.medicines[med_idx].manufacturer_corp_idx.unwrap();

        // Drain reserves to 50% to make the boost visible
        state.corporations[corp_idx].reserves = state.corporations[corp_idx].max_reserves * 0.50;
        let reserves_before = state.corporations[corp_idx].reserves;

        // Complete a DevelopMedicine project
        state.applied_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: med_idx },
            progress: 199.0,
            required_ticks: 200.0,
            personnel_assigned: 5,
        });

        state = tick(&state);

        assert!(state.medicines[med_idx].unlocked, "medicine should be unlocked");
        assert!(
            state.corporations[corp_idx].reserves > reserves_before,
            "board-seat manufacturer reserves should increase: before={}, after={}",
            reserves_before, state.corporations[corp_idx].reserves
        );
    }

    #[test]
    fn new_disease_medicines_get_manufacturers() {
        let mut state = GameState::new_default(42);
        generate_corporations(&mut state);

        let initial_med_count = state.medicines.len();

        // Simulate disease emergence by adding new medicines
        let new_meds = crate::state::Medicine::targeted_medicines(
            state.diseases.len(), crate::state::PathogenType::Bacterium
        );
        state.medicines.extend(new_meds);

        // Assign manufacturers to the new medicines
        assign_manufacturers(&mut state);

        // New medicines should have manufacturers
        for med in &state.medicines[initial_med_count..] {
            assert!(
                med.manufacturer_corp_idx.is_some(),
                "new medicine {} should have a manufacturer after assign_manufacturers",
                med.name
            );
        }

        // Original medicines should still have their manufacturers unchanged
        for med in &state.medicines[..initial_med_count] {
            if !med.unlocked {
                assert!(
                    med.manufacturer_corp_idx.is_some(),
                    "original medicine {} should retain its manufacturer",
                    med.name
                );
            }
        }
    }
}
