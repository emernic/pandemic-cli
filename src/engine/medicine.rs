use rand::Rng;

use crate::state::{
    CrisisOperation, DeployTarget, GameEvent, GameOutcome, GameState, MedicineMode,
    RegionDiseaseState, Shipment, SHIPPING_TICKS, TICKS_PER_DAY,
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
    // Block deployment when supply lines have completely failed
    if state.regions[region_idx].supply_lines <= 0.0 {
        let region_name = &state.regions[region_idx].name;
        return (false, Some(format!("{region_name} supply lines collapsed")));
    }
    let med = &state.medicines[medicine_idx];
    let med_name = med.name.clone();
    let region_name = state.regions[region_idx].name.clone();

    if state.medicines[medicine_idx].doses <= 0.0 {
        return (false, Some(format!("No doses remaining for {med_name}")));
    }

    let disease_idx = target.disease_idx;
    let deploy_mode = target.mode;

    // Block deployment during per-medicine cooldown
    let cooldown = state.regions[region_idx].deploy_cooldown_remaining(state.tick, medicine_idx);
    if cooldown > 0 {
        let days = cooldown as f64 / crate::state::TICKS_PER_DAY;
        return (false, Some(format!("{region_name} on cooldown for {med_name}, {days:.1} days remaining")));
    }

    // Estimate how many doses to dispatch based on current population
    let efficacy = state.medicines[medicine_idx].effective_efficacy(disease_idx, &state.diseases);
    let vax_mult = state.vaccination_multiplier();
    let region = &state.regions[region_idx];
    let pop = region.population as f64;
    let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
    let exposed = existing.map(|i| i.exposed).unwrap_or(0.0);
    let infected = existing.map(|i| i.infected).unwrap_or(0.0);
    let dead = region.dead;
    let immune = existing.map(|i| i.immune).unwrap_or(0.0);

    let doses_to_ship = match deploy_mode {
        MedicineMode::Vaccine => {
            let susceptible = (pop - exposed - infected - dead - immune).max(0.0);
            state.medicines[medicine_idx].estimate_vaccination(susceptible, efficacy, vax_mult)
        }
        MedicineMode::Therapeutic => {
            state.medicines[medicine_idx].estimate_treatment(infected, efficacy)
        }
    };

    if doses_to_ship < 1.0 {
        return match deploy_mode {
            MedicineMode::Vaccine => (false, Some(format!("No susceptible population in {region_name}"))),
            MedicineMode::Therapeutic => (false, Some(format!("No infected population in {region_name}"))),
        };
    }

    // Deduct doses
    state.medicines[medicine_idx].doses = (state.medicines[medicine_idx].doses - doses_to_ship).max(0.0);
    state.medicines[medicine_idx].deployed_count += 1;
    state.total_doses_deployed += doses_to_ship;
    state.regions[region_idx].last_deploy_tick.insert(medicine_idx, state.tick);

    // Create the shipment — supply line degradation slows delivery
    let supply_mult = if state.regions[region_idx].supply_lines < crate::state::INFRA_CRITICAL {
        2.0 // Critical: 2x delivery time
    } else {
        1.0
    };
    // Logistics sector bonus: delivery faster
    let logistics_bonus = state.sector_bonus(region_idx, crate::state::CorporationSector::Logistics);
    let logistics_mult = 1.0 - crate::state::CorporationSector::Logistics.max_bonus_pct() / 100.0 * logistics_bonus;
    // PharmaHub specialization: 30% faster shipping
    let pharma_mult = if state.regions[region_idx].has_specialization(crate::state::RegionSpecialization::PharmaHub) {
        crate::state::PHARMA_HUB_SHIPPING_MULT
    } else {
        1.0
    };
    let arrive_tick = state.tick + (SHIPPING_TICKS as f64 * supply_mult * logistics_mult * pharma_mult) as u64;
    state.pending_shipments.push(Shipment {
        medicine_idx,
        region_idx,
        target,
        doses: doses_to_ship,
        arrive_tick,
    });
    state.events.push(GameEvent::MedicineShipped { medicine_idx, region_idx, doses: doses_to_ship });

    let doses_str = crate::format_number(doses_to_ship);
    let days = (SHIPPING_TICKS as f64 * supply_mult * logistics_mult * pharma_mult) / crate::state::TICKS_PER_DAY;
    let efficiency = state.regions[region_idx].delivery_efficiency();
    let eff_warning = if efficiency < 0.90 {
        format!(" ({:.0}% delivery efficiency)", efficiency * 100.0)
    } else {
        String::new()
    };
    let msg = format!(
        "Shipped {doses_str} doses of {med_name} to {region_name}. Arriving in {days:.0} day{}.{eff_warning}", if days > 1.5 { "s" } else { "" }
    );
    (true, Some(msg))
}

/// Process arriving shipments. Called each tick. Delivers doses that have
/// arrived and discards shipments to collapsed regions. Travel bans restrict
/// civilian movement but do not block medical supply shipments.
pub(super) fn tick_shipments(state: &mut GameState, rng_misc: &mut rand_chacha::ChaCha8Rng) {
    let mut i = 0;
    while i < state.pending_shipments.len() {
        let reg_idx = state.pending_shipments[i].region_idx;

        // Discard if region collapsed
        let region_gone = state.regions.get(reg_idx)
            .map(|r| r.collapsed)
            .unwrap_or(true);
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
        deliver_shipment(state, &shipment, rng_misc);
        // don't increment i — the vec shifted
    }
}

/// Apply a shipment's effects to the game state (treatment or vaccination).
///
/// Delivery efficiency is based on regional infrastructure: supply lines
/// determine how many doses physically arrive, healthcare capacity determines
/// how many can be administered, and collapsed neighbors reduce throughput
/// further. These multiply, so degraded regions receive far fewer effective
/// doses. Wasted doses are lost permanently.
fn deliver_shipment(state: &mut GameState, shipment: &Shipment, rng_misc: &mut rand_chacha::ChaCha8Rng) {
    let med_idx = shipment.medicine_idx;
    let reg_idx = shipment.region_idx;

    let Some(medicine) = state.medicines.get(med_idx) else { return; };
    if !medicine.unlocked { return; }

    let disease_idx = shipment.target.disease_idx;

    // Infrastructure bottlenecks: supply lines × healthcare capacity
    let efficiency = state.regions[reg_idx].delivery_efficiency();
    // Targeting waste: without surveillance, doses go to the wrong people
    let targeting = state.targeting_efficiency(reg_idx);
    let effective_doses = shipment.doses * efficiency * targeting;
    let doses_wasted = shipment.doses * efficiency * (1.0 - targeting);
    if effective_doses <= 0.0 { return; }

    let mut efficacy = state.medicines[med_idx].effective_efficacy(disease_idx, &state.diseases);
    if state.regions[reg_idx].hospital_level >= 2 {
        efficacy = (efficacy * (1.0 + crate::state::MEDICAL_CENTER_EFFICACY_BONUS)).min(1.0);
    }
    let vax_mult = state.vaccination_multiplier();
    let region = &state.regions[reg_idx];
    let pop = region.population as f64;
    let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
    let exposed = existing.map(|i| i.exposed).unwrap_or(0.0);
    let infected = existing.map(|i| i.infected).unwrap_or(0.0);
    let dead = region.dead;
    let immune = existing.map(|i| i.immune).unwrap_or(0.0);

    let is_tested = state.medicines[med_idx].tested_against.contains(&disease_idx);

    let deploy_mode = shipment.target.mode;
    let (adverse, people_treated, people_protected) = match deploy_mode {
        MedicineMode::Vaccine => {
            let susceptible = (pop - exposed - infected - dead - immune).max(0.0);
            // Cap at effective doses (after infrastructure losses)
            let actual = state.medicines[med_idx]
                .estimate_vaccination(susceptible, efficacy, vax_mult)
                .min(effective_doses);
            if actual <= 0.0 { return; }
            let (adverse, adverse_deaths) = adverse_check(rng_misc, actual, is_tested, susceptible);
            let inf = state.regions[reg_idx].get_or_create_infection(disease_idx);
            apply_immune_and_deaths(inf, actual, adverse_deaths);
            state.regions[reg_idx].dead += adverse_deaths;
            build_resistance(state, med_idx, disease_idx, false);
            let net_protected = (actual - adverse_deaths).max(0.0);
            state.medicines[med_idx].total_protected += net_protected;
            (adverse, 0.0, net_protected)
        }
        MedicineMode::Therapeutic => {
            let actual = state.medicines[med_idx]
                .estimate_treatment(infected, efficacy)
                .min(effective_doses);
            if actual <= 0.0 { return; }
            let (adverse, adverse_deaths) = adverse_check(rng_misc, actual, is_tested, infected);
            let inf = state.regions[reg_idx].get_or_create_infection(disease_idx);
            inf.infected -= actual;
            apply_immune_and_deaths(inf, actual, adverse_deaths);
            state.regions[reg_idx].dead += adverse_deaths;
            build_resistance(state, med_idx, disease_idx, true);
            let net_treated = (actual - adverse_deaths).max(0.0);
            state.medicines[med_idx].total_treated += net_treated;
            (adverse, net_treated, 0.0)
        }
    };

    state.events.push(GameEvent::ShipmentDelivered {
        medicine_idx: med_idx,
        region_idx: reg_idx,
        doses: shipment.doses,
        adverse,
        efficiency,
        doses_wasted,
        people_treated,
        people_protected,
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
    let base = if is_treatment { 0.006 } else { 0.001 };
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

/// Send experimental medicine samples to a region's governor. This is a
/// political action, not a mass deployment. Consumes a small number of doses
/// and ties up personnel for delivery. Boosts governor cooperation, but
/// untested medicines risk adverse reactions that backfire politically.
///
/// Returns (success, message).
pub(super) fn emergency_sample_delivery(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    rng: &mut impl Rng,
) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    let Some(region) = state.regions.get(region_idx) else {
        return (false, Some("Invalid region.".into()));
    };
    if region.collapsed {
        return (false, Some(format!("{} has collapsed.", region.name)));
    }
    let Some(med) = state.medicines.get(medicine_idx) else {
        return (false, Some("Invalid medicine.".into()));
    };
    if !med.unlocked {
        return (false, Some("Medicine not yet developed.".into()));
    }
    if med.doses <= 0.0 {
        return (false, Some(format!("No doses remaining for {}.", med.name)));
    }

    let dose_cost = state.emergency_delivery_dose_cost(medicine_idx);

    let delivery_personnel: u32 = 2;
    if state.personnel_available() < delivery_personnel {
        return (false, Some("Not enough available personnel for a delivery team.".into()));
    }

    let med_name = med.name.clone();
    let region_name = region.name.clone();
    let gov_name = region.governor.name.clone();

    // Check if the medicine is tested against any disease active in this region
    let active_diseases: Vec<usize> = state.regions[region_idx].infections.iter()
        .map(|inf| inf.disease_idx)
        .collect();
    let has_tested_match = active_diseases.iter()
        .any(|d_idx| state.medicines[medicine_idx].tested_against.contains(d_idx));
    let has_untested_match = active_diseases.iter()
        .any(|d_idx| !state.medicines[medicine_idx].tested_against.contains(d_idx));

    // Deduct doses
    state.medicines[medicine_idx].doses -= dose_cost;

    // Tie up personnel for 1 day
    state.crisis_operations.push(CrisisOperation {
        label: format!("Sample Delivery to {}", region_name),
        personnel: delivery_personnel,
        ticks_remaining: TICKS_PER_DAY,
    });

    // Determine outcome
    let cooperation_change: f64;
    let mut adverse = false;

    if has_untested_match {
        // Untested medicine: 25% chance of adverse reaction
        let roll: f64 = rng.r#gen();
        if roll < 0.25 {
            // Adverse reaction: governor loses trust
            adverse = true;
            cooperation_change = -10.0;
        } else {
            // Untested but no reaction: moderate cooperation boost
            cooperation_change = 10.0;
        }

    } else if has_tested_match {
        // Tested medicine for an active disease: strong cooperation boost
        cooperation_change = 20.0;
    } else {
        // No active diseases in region or medicine doesn't target them: small boost
        cooperation_change = 8.0;
    }

    // Apply cooperation change
    state.regions[region_idx].governor.cooperation =
        (state.regions[region_idx].governor.cooperation + cooperation_change).clamp(0.0, 100.0);

    state.events.push(GameEvent::EmergencySampleDelivered {
        medicine_idx,
        region_idx,
        cooperation_change,
        adverse,
    });

    let msg = if adverse {
        format!(
            "Adverse reaction to {} samples in {}. {} cooperation fell.",
            med_name, region_name, gov_name,
        )
    } else {
        format!(
            "Delivered {} samples to {} in {} ({:.0} doses). Cooperation improved.",
            med_name, gov_name, region_name, dose_cost,
        )
    };

    (true, Some(msg))
}

pub(super) fn insufficient_funds_message(cost: f64, have: f64) -> String {
    format!("Insufficient funds! Need ¥{cost:.0}, have ¥{have:.0}")
}

/// Deploy medicines to the worst-affected regions. Called once per tick.
/// For each medicine with deployment enabled:
/// - Must be unlocked, tested, and have doses
/// - Always deploys as therapeutic (targets most infected region)
/// - Respects per-medicine region filter (empty = all regions)
/// - Cooldown must be clear for the chosen region
pub(super) fn try_auto_deploy(state: &mut GameState) {
    // Grow deploy vecs if new medicines were created
    while state.deploy_enabled.len() < state.medicines.len() {
        state.deploy_enabled.push(false);
    }
    while state.deploy_regions.len() < state.medicines.len() {
        state.deploy_regions.push(std::collections::BTreeSet::new());
    }

    for med_idx in 0..state.medicines.len() {
        if !state.deploy_enabled.get(med_idx).copied().unwrap_or(false) {
            continue;
        }
        let med = &state.medicines[med_idx];
        if !med.unlocked || med.doses <= 0.0 {
            continue;
        }

        // Only deploy against tested diseases (avoid adverse reactions)
        let deployable = med.deployable_diseases(&state.diseases);
        let tested: Vec<usize> = deployable.iter()
            .copied()
            .filter(|d_idx| med.tested_against.contains(d_idx))
            .collect();
        if tested.is_empty() {
            continue;
        }

        let region_filter = &state.deploy_regions[med_idx];

        // Find the best region to deploy to — targets the region with the
        // most infected population, filtered by the player's region selection.
        let mut best_region: Option<usize> = None;
        let mut best_score: f64 = 0.0;
        let mut best_disease_idx: usize = 0;

        for (r_idx, region) in state.regions.iter().enumerate() {
            if region.collapsed {
                continue;
            }
            // Empty filter = all regions; non-empty = only listed regions
            if !region_filter.is_empty() && !region_filter.contains(&r_idx) {
                continue;
            }
            for &d_idx in &tested {
                if region.deploy_cooldown_remaining(state.tick, med_idx) > 0 {
                    continue;
                }
                // Skip diseases where resistance has made the medicine too weak.
                if state.medicines[med_idx].effective_efficacy(d_idx, &state.diseases)
                    < crate::state::DEPLOY_MIN_EFFICACY
                {
                    continue;
                }
                // Target regions with the most infected
                let score = region.disease_state(d_idx)
                    .map(|inf| inf.infected)
                    .unwrap_or(0.0);
                if score > best_score {
                    best_score = score;
                    best_region = Some(r_idx);
                    best_disease_idx = d_idx;
                }
            }
        }

        if let Some(region_idx) = best_region {
            if best_score <= 0.0 {
                continue;
            }

            let target = DeployTarget {
                disease_idx: best_disease_idx,
                mode: MedicineMode::Therapeutic,
            };

            // deploy_medicine() fires MedicineShipped on success
            deploy_medicine(state, med_idx, region_idx, target);
        } else {
            // No valid target found — check if ALL tested diseases are below efficacy
            // threshold and notify the player once.
            if !state.deploy_blocked_notified.contains(&med_idx) {
                let all_blocked = tested.iter().all(|&d_idx| {
                    state.medicines[med_idx].effective_efficacy(d_idx, &state.diseases) < crate::state::DEPLOY_MIN_EFFICACY
                });
                if all_blocked {
                    state.deploy_blocked_notified.insert(med_idx);
                    state.events.push(crate::state::GameEvent::DeployBlocked { medicine_idx: med_idx });
                }
            }
        }
    }
}

