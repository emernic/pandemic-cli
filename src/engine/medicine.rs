use rand::Rng;

use crate::state::{
    DeployTarget, GameEvent, GameOutcome, GameState, RegionDiseaseState, Shipment,
    DISRUPTION_MEDICINE_COST_MULT, SHIPPING_TICKS,
};

/// Dispatch a medicine shipment: deduct funds and doses, create in-transit
/// shipment. Effects apply when the shipment arrives (1 day later).
/// Pure game logic — does NOT modify UI state.
///
/// Returns (success, message):
/// - `success`: true if dispatch was attempted
/// - `message`: status feedback to display (if any)
///
/// Adverse reactions are checked at delivery time (see `deliver_shipment`).
pub(super) fn deploy_medicine(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    target: DeployTarget,
) -> (bool, Option<String>) {
    // Block after game over
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    // Block deployment to collapsed regions
    if state.regions.get(region_idx).is_some_and(|r| r.collapsed) {
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} has collapsed. Deployment impossible.")));
    }
    // Block deployment to abandoned regions (Ark Protocol)
    if state.is_abandoned(region_idx) {
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} abandoned. Deploy to the Ark instead.")));
    }
    // Block deployment when supply lines have completely failed
    if state.regions[region_idx].supply_lines <= 0.0 {
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} supply lines collapsed")));
    }
    let med = &state.medicines[medicine_idx];
    let base_cost = med.deploy_cost();
    let disruption_mult = if state.regions[region_idx].is_disrupted(state.tick) {
        DISRUPTION_MEDICINE_COST_MULT
    } else {
        1.0
    };
    let cost = base_cost * disruption_mult * state.deployment_cost_bonus();
    let med_name = med.name.clone();
    let region_name = state.regions[region_idx].name.clone();

    if state.resources.funding < cost {
        return (false, Some(insufficient_funds_message(cost, state.resources.funding)));
    }
    if state.medicines[medicine_idx].doses <= 0.0 {
        return (false, Some(format!("No doses remaining for {med_name}")));
    }

    let disease_idx = match &target {
        DeployTarget::Vaccinate { disease_idx } => *disease_idx,
        DeployTarget::Treat { disease_idx } => *disease_idx,
    };

    // Block deployment during per-disease cooldown
    let cooldown = state.regions[region_idx].deploy_cooldown_remaining(state.tick, disease_idx);
    if cooldown > 0 {
        let days = cooldown as f64 / crate::state::TICKS_PER_DAY;
        return (false, Some(format!("{region_name} on cooldown for this disease, {days:.1} days remaining")));
    }

    // Estimate how many doses to dispatch based on current population
    let efficacy = state.medicines[medicine_idx].effective_efficacy(disease_idx, &state.diseases);
    let vax_mult = state.vaccination_multiplier();
    let region = &state.regions[region_idx];
    let pop = region.population as f64;
    let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
    let infected = existing.map(|i| i.infected).unwrap_or(0.0);
    let dead = region.dead;
    let immune = existing.map(|i| i.immune).unwrap_or(0.0);

    let doses_to_ship = match &target {
        DeployTarget::Vaccinate { .. } => {
            let susceptible = (pop - infected - dead - immune).max(0.0);
            state.medicines[medicine_idx].estimate_vaccination(susceptible, efficacy, vax_mult)
        }
        DeployTarget::Treat { .. } => {
            state.medicines[medicine_idx].estimate_treatment(infected, efficacy)
        }
    };

    if doses_to_ship <= 0.0 {
        return match target {
            DeployTarget::Vaccinate { .. } => (false, Some(format!("No susceptible population in {region_name}"))),
            DeployTarget::Treat { .. } => (false, Some(format!("No infected population in {region_name}"))),
        };
    }

    // Deduct cost and doses
    state.resources.funding -= cost;
    state.medicines[medicine_idx].doses = (state.medicines[medicine_idx].doses - doses_to_ship).max(0.0);
    state.medicines[medicine_idx].deployed_count += 1;
    state.total_doses_deployed += doses_to_ship;
    state.regions[region_idx].last_deploy_tick.insert(disease_idx, state.tick);

    // Create the shipment — supply line degradation slows delivery
    let supply_mult = if state.regions[region_idx].supply_lines < crate::state::INFRA_CRITICAL {
        2.0 // Critical: 2x delivery time
    } else {
        1.0
    };
    let arrive_tick = state.tick + (SHIPPING_TICKS as f64 * supply_mult) as u64;
    state.pending_shipments.push(Shipment {
        medicine_idx,
        region_idx,
        target,
        doses: doses_to_ship,
        cost,
        arrive_tick,
    });
    state.events.push(GameEvent::MedicineShipped { medicine_idx, region_idx, doses: doses_to_ship });

    let doses_str = crate::format_number(doses_to_ship);
    let days = (SHIPPING_TICKS as f64 * supply_mult) / crate::state::TICKS_PER_DAY;
    let efficiency = state.regions[region_idx].delivery_efficiency();
    let eff_warning = if efficiency < 0.90 {
        format!(" ({:.0}% delivery efficiency)", efficiency * 100.0)
    } else {
        String::new()
    };
    let msg = format!(
        "Shipped {doses_str} doses of {med_name} to {region_name} (-¥{cost:.0}). Arriving in {days:.0} day{}.{eff_warning}", if days > 1.5 { "s" } else { "" }
    );
    (true, Some(msg))
}

/// Process arriving shipments. Called each tick. Delivers doses that have
/// arrived and discards shipments to collapsed regions. Travel bans restrict
/// civilian movement but do not block medical supply shipments.
pub(super) fn tick_shipments(state: &mut GameState) {
    let mut i = 0;
    while i < state.pending_shipments.len() {
        let reg_idx = state.pending_shipments[i].region_idx;

        // Discard if region collapsed or abandoned
        let region_gone = state.regions.get(reg_idx)
            .map(|r| r.collapsed)
            .unwrap_or(true)
            || state.is_abandoned(reg_idx);
        if region_gone {
            state.pending_shipments.remove(i);
            continue;
        }

        if state.tick < state.pending_shipments[i].arrive_tick {
            i += 1;
            continue;
        }

        // Deliver the shipment
        let shipment = state.pending_shipments.remove(i);
        deliver_shipment(state, &shipment);
        // don't increment i — the vec shifted
    }
}

/// Apply a shipment's effects to the game state (treatment or vaccination).
///
/// Delivery efficiency is based on regional infrastructure: supply lines
/// determine how many doses physically arrive, healthcare capacity determines
/// how many can be administered. These multiply, so degraded regions receive
/// far fewer effective doses. Wasted doses are lost permanently.
fn deliver_shipment(state: &mut GameState, shipment: &Shipment) {
    let med_idx = shipment.medicine_idx;
    let reg_idx = shipment.region_idx;

    let Some(medicine) = state.medicines.get(med_idx) else { return; };
    if !medicine.unlocked { return; }

    let disease_idx = match &shipment.target {
        DeployTarget::Vaccinate { disease_idx } => *disease_idx,
        DeployTarget::Treat { disease_idx } => *disease_idx,
    };

    // Infrastructure bottlenecks: supply lines × healthcare capacity
    let efficiency = state.regions[reg_idx].delivery_efficiency();
    let effective_doses = shipment.doses * efficiency;
    if effective_doses <= 0.0 { return; }

    let mut efficacy = state.medicines[med_idx].effective_efficacy(disease_idx, &state.diseases);
    if state.regions[reg_idx].hospital_level >= 2 {
        efficacy = (efficacy * (1.0 + crate::state::MEDICAL_CENTER_EFFICACY_BONUS)).min(1.0);
    }
    let vax_mult = state.vaccination_multiplier();
    let region = &state.regions[reg_idx];
    let pop = region.population as f64;
    let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
    let infected = existing.map(|i| i.infected).unwrap_or(0.0);
    let dead = region.dead;
    let immune = existing.map(|i| i.immune).unwrap_or(0.0);

    let is_tested = state.medicines[med_idx].tested_against.contains(&disease_idx);

    let adverse = match &shipment.target {
        DeployTarget::Vaccinate { .. } => {
            let susceptible = (pop - infected - dead - immune).max(0.0);
            // Cap at effective doses (after infrastructure losses)
            let actual = state.medicines[med_idx]
                .estimate_vaccination(susceptible, efficacy, vax_mult)
                .min(effective_doses);
            if actual <= 0.0 { return; }
            let (adverse, adverse_deaths) = adverse_check(&mut state.rng, actual, is_tested, susceptible);
            let inf = state.regions[reg_idx].get_or_create_infection(disease_idx);
            apply_immune_and_deaths(inf, actual, adverse_deaths);
            state.regions[reg_idx].dead += adverse_deaths;
            build_resistance(state, med_idx, disease_idx, false);
            adverse
        }
        DeployTarget::Treat { .. } => {
            let actual = state.medicines[med_idx]
                .estimate_treatment(infected, efficacy)
                .min(effective_doses);
            if actual <= 0.0 { return; }
            let (adverse, adverse_deaths) = adverse_check(&mut state.rng, actual, is_tested, infected);
            let inf = state.regions[reg_idx].get_or_create_infection(disease_idx);
            inf.infected -= actual;
            apply_immune_and_deaths(inf, actual, adverse_deaths);
            state.regions[reg_idx].dead += adverse_deaths;
            build_resistance(state, med_idx, disease_idx, true);
            adverse
        }
    };

    state.events.push(GameEvent::ShipmentDelivered {
        medicine_idx: med_idx,
        region_idx: reg_idx,
        doses: shipment.doses,
        adverse,
        efficiency,
    });
}

/// Roll for adverse reaction on untested medicines.
/// Returns (adverse_occurred, deaths). Deaths are capped at `max_deaths`
/// to prevent killing more people than the target population.
fn adverse_check(rng: &mut impl Rng, actual: f64, is_tested: bool, max_deaths: f64) -> (bool, f64) {
    if !is_tested {
        let roll: f64 = rng.r#gen();
        if roll < 0.25 {
            let deaths = (actual * 0.2).min(max_deaths);
            return (true, deaths);
        }
    }
    (false, 0.0)
}

/// Apply immune gains and adverse deaths to infection state.
/// Caller must separately add adverse_deaths to region.dead.
fn apply_immune_and_deaths(
    inf: &mut RegionDiseaseState,
    actual: f64,
    adverse_deaths: f64,
) {
    if adverse_deaths > 0.0 {
        inf.dead += adverse_deaths;
        inf.immune += actual - adverse_deaths;
    } else {
        inf.immune += actual;
    }
}


/// Build resistance from deployment pressure. Treatment creates much more
/// selection pressure than vaccination. Broad-spectrum drugs build resistance
/// faster (2x) because broad selection pressure accelerates adaptation.
/// Mechanism-specific multipliers further modify: cheap/fast mechanisms
/// have high resistance rates, expensive/durable ones have low rates.
/// CombinationTherapy tech halves all resistance buildup.
fn build_resistance(state: &mut GameState, medicine_idx: usize, disease_idx: usize, is_treatment: bool) {
    let med = &state.medicines[medicine_idx];
    let mechanism = med.mechanism;
    let base = if is_treatment { 0.03 } else { 0.005 };
    let type_mult = match med.therapy_type {
        crate::state::TherapyType::BroadSpectrum => 2.0,
        _ => 1.0,
    };
    let mech_mult = mechanism.map(|m| m.resistance_rate_multiplier()).unwrap_or(1.0);
    let combo_mult = state.resistance_multiplier();
    let gain = base * type_mult * mech_mult * combo_mult;
    // Resistance lives on the disease, keyed by mechanism — so deploying
    // any CellWallInhibitor drug builds resistance that affects ALL
    // CellWallInhibitor drugs against this disease.
    if let Some(disease) = state.diseases.get_mut(disease_idx) {
        disease.add_resistance(mechanism, gain);
    }
}

pub(super) fn insufficient_funds_message(cost: f64, have: f64) -> String {
    format!("Insufficient funds! Need ¥{cost:.0}, have ¥{have:.0}")
}

/// Auto-deploy medicines to the worst-affected regions. Called once per tick.
/// For each medicine with auto_deploy enabled:
/// - Must be unlocked, tested, have doses, and have affordable deploy cost
/// - Finds the region with the most infected (for any target disease) where
///   cooldown is clear
/// - Deploys as treatment (treating infected is the reactive choice;
///   vaccination is strategic and left to the player)
pub(super) fn try_auto_deploy(state: &mut GameState) {
    // Grow auto_deploy vec if new medicines were created
    while state.auto_deploy.len() < state.medicines.len() {
        state.auto_deploy.push(false);
    }

    for med_idx in 0..state.medicines.len() {
        if !state.auto_deploy.get(med_idx).copied().unwrap_or(false) {
            continue;
        }
        let med = &state.medicines[med_idx];
        if !med.unlocked || med.doses <= 0.0 {
            continue;
        }

        // Only auto-deploy against tested diseases (avoid adverse reactions)
        let deployable = med.deployable_diseases(&state.diseases);
        let tested: Vec<usize> = deployable.iter()
            .copied()
            .filter(|d_idx| med.tested_against.contains(d_idx))
            .collect();
        if tested.is_empty() {
            continue;
        }

        // Find the best region to deploy to, respecting deployment priority.
        // Higher priority regions (High > Normal > Low) are served first.
        // CutOff regions are skipped entirely.
        // Within a priority tier, the region with the most infected wins.
        let mut best_region: Option<usize> = None;
        let mut best_priority: u8 = u8::MAX;
        let mut best_infected: f64 = 0.0;
        let mut best_disease_idx: usize = 0;

        for (r_idx, region) in state.regions.iter().enumerate() {
            if region.collapsed {
                continue;
            }
            let priority = region.deploy_priority;
            if priority == crate::state::RegionPriority::CutOff {
                continue;
            }
            let rank = priority.rank();
            for &d_idx in &tested {
                if region.deploy_cooldown_remaining(state.tick, d_idx) > 0 {
                    continue;
                }
                let infected = region.disease_state(d_idx)
                    .map(|inf| inf.infected)
                    .unwrap_or(0.0);
                // Prefer higher priority (lower rank), then most infected
                if rank < best_priority || (rank == best_priority && infected > best_infected) {
                    best_priority = rank;
                    best_infected = infected;
                    best_region = Some(r_idx);
                    best_disease_idx = d_idx;
                }
            }
        }

        if let Some(region_idx) = best_region {
            if best_infected <= 0.0 {
                continue;
            }

            // Check funding (including regional deployment discount)
            let cost = state.medicines[med_idx].deploy_cost()
                * state.deployment_cost_bonus();
            if state.resources.funding < cost {
                continue;
            }

            let target = DeployTarget::Treat { disease_idx: best_disease_idx };

            // deploy_medicine() fires MedicineShipped on success
            deploy_medicine(state, med_idx, region_idx, target);
        }
    }
}

