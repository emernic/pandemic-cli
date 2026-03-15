use rand::Rng;

use crate::state::{
    Disease, GameEvent, GameState, PathogenType, RegionTrait,
    COINFECTION_LETHALITY_PER_DISEASE, COINFECTION_THRESHOLD,
    HOSPITAL_EXPOSURE_FACTOR, TICKS_PER_DAY,
};

/// Scale a policy reduction factor by governor effectiveness.
/// For a policy that multiplies by `factor` (e.g., 0.3 = 70% reduction),
/// hostility (effectiveness < 1.0) weakens the reduction:
///   effective = 1.0 - (1.0 - factor) * effectiveness
/// At effectiveness=1.0: returns factor unchanged.
/// At effectiveness=0.7: a 0.3 factor becomes 0.51 (49% reduction instead of 70%).
fn scale_policy_factor(factor: f64, effectiveness: f64) -> f64 {
    1.0 - (1.0 - factor) * effectiveness
}

/// Per-disease outflows computed in phase 1, applied in phase 2.
struct DiseaseOutflows {
    new_exposed: f64,
    exposed_to_infected: f64,
    new_deaths: f64,
    new_recoveries: f64,
}

/// Spread diseases within each region. Uses `diseases` (the original tick's
/// disease parameters) to avoid borrow conflicts — the caller passes
/// `&state.diseases` from the immutable input while `new` is the mutable clone.
///
/// Uses a shared death model: `region.dead` is the single authoritative death
/// counter. When people die from any disease, they are proportionally removed
/// from all other diseases' infected/immune pools (dead people can't be sick
/// with or immune to anything).
pub(super) fn tick_spread_within(
    new: &mut GameState,
    diseases: &[Disease],
    rng: &mut impl Rng,
) {
    for (region_idx, region) in new.regions.iter_mut().enumerate() {
        let pop = region.population as f64;
        let policy = new.policies.get(region_idx);
        let quarantine_active = policy.is_some_and(|p| p.quarantine);
        let discourage_hosp = policy.is_some_and(|p| p.discourage_hosp);
        let sanitation_active = policy.is_some_and(|p| p.water_sanitation);
        let gov_eff = region.policy_effectiveness();
        let alive = (pop - region.dead).max(0.0);

        // Co-infection: collect infected counts for qualifying diseases.
        // Used to estimate the fraction of each disease's infected who are also infected
        // with other diseases (independence assumption: P(co-infected with B) = I_B / alive).
        let coinfecting_infected: Vec<(usize, f64)> = region.infections.iter()
            .enumerate()
            .filter(|(_, inf)| inf.infected >= COINFECTION_THRESHOLD)
            .map(|(i, inf)| (i, inf.infected))
            .collect();

        // Phase 1: compute outflows for each disease without mutating yet.
        let mut outflows: Vec<DiseaseOutflows> = Vec::with_capacity(region.infections.len());
        for (inf_idx, inf) in region.infections.iter().enumerate() {
            if let Some(disease) = diseases.get(inf.disease_idx) {
                let susceptible = alive - inf.exposed - inf.infected - inf.immune;
                if susceptible <= 0.0 {
                    outflows.push(DiseaseOutflows { new_exposed: 0.0, exposed_to_infected: 0.0, new_deaths: 0.0, new_recoveries: 0.0 });
                    continue;
                }

                let noise: f64 = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.1;
                let mut spread_rate = if quarantine_active {
                    let base_f = disease.transmission.quarantine_factor();
                    disease.within_region_spread * scale_policy_factor(base_f, gov_eff)
                } else {
                    disease.within_region_spread
                };
                // Baseline: hospitals increase spread (+25% from hospital exposure).
                // Discourage Hospitalization removes this penalty.
                if !discourage_hosp {
                    spread_rate *= HOSPITAL_EXPOSURE_FACTOR;
                } else {
                    // Gov effectiveness: partial exposure remains with weak governors
                    let effective = 1.0 + (HOSPITAL_EXPOSURE_FACTOR - 1.0) * (1.0 - gov_eff);
                    spread_rate *= effective;
                }
                if sanitation_active {
                    let f = disease.transmission.water_sanitation_factor();
                    spread_rate *= scale_policy_factor(f, gov_eff);
                }
                // Screening identifies and isolates cases, reducing spread
                let screening_factor = policy
                    .map(|p| scale_policy_factor(p.screening.spread_factor(), gov_eff))
                    .unwrap_or(1.0);
                spread_rate *= screening_factor;
                // Dense Urban trait: +30% within-region spread
                if region.has_trait(RegionTrait::DenseUrban) {
                    spread_rate *= 1.3;
                }
                // Infrastructure: civil order anarchy increases spread
                if region.civil_order <= 0.0 {
                    spread_rate *= crate::state::CIVIL_ORDER_ANARCHY_SPREAD;
                }
                // SEIR: only infectious (not exposed) individuals transmit.
                // New transmissions enter the exposed compartment.
                let new_exposed =
                    (spread_rate * inf.infected * (susceptible / pop) * noise)
                        .max(0.0).min(susceptible);

                // Drain exposed → infected at rate 1/incubation_ticks per tick.
                let incubation_rate = if disease.incubation_ticks > 0.0 {
                    1.0 / disease.incubation_ticks
                } else {
                    1.0 // instant transition if no incubation
                };
                let exposed_to_infected = (inf.exposed * incubation_rate).min(inf.exposed);

                let mut lethality = disease.lethality * region.healthcare_modifier;
                // StrongPublicHealth: better hospitals reduce baseline lethality by 15%.
                // Discourage Hospitalization cancels this bonus (can't benefit from
                // hospitals you told people not to use) and adds a universal penalty.
                if region.has_trait(RegionTrait::StrongPublicHealth) && !discourage_hosp {
                    lethality *= 0.85; // 15% lethality reduction from superior hospitals
                }
                if discourage_hosp {
                    lethality *= scale_policy_factor(1.50, gov_eff); // +50% lethality for all regions
                }
                if region.hospital_level >= 2 {
                    lethality *= 0.60; // Medical Center: 40% total lethality reduction
                } else if region.hospital_level >= 1 {
                    lethality *= 0.75; // Field Hospital: 25% lethality reduction
                }
                // Co-infection amplifies lethality only for estimated co-infected individuals.
                // Under independence: fraction of this disease's infected who also have disease B
                // ≈ I_B / alive. Sum contributions from all other qualifying diseases.
                let coinfection_boost: f64 = coinfecting_infected.iter()
                    .filter(|(idx, _)| *idx != inf_idx)
                    .map(|(_, other_infected)| {
                        COINFECTION_LETHALITY_PER_DISEASE * (other_infected / alive.max(1.0))
                    })
                    .sum();
                lethality *= 1.0 + coinfection_boost;
                // Infrastructure: healthcare capacity degradation increases lethality
                if region.healthcare_capacity < crate::state::INFRA_CRITICAL {
                    lethality *= crate::state::HEALTHCARE_CRITICAL_LETHALITY;
                } else if region.healthcare_capacity < crate::state::INFRA_STRESSED {
                    lethality *= crate::state::HEALTHCARE_STRESSED_LETHALITY;
                }
                let mut new_deaths = (lethality * inf.infected * noise).max(0.0);
                let mut new_recoveries = (disease.recovery_rate * inf.infected * noise).max(0.0);
                let total_outflow = new_deaths + new_recoveries;
                if total_outflow > inf.infected {
                    let scale = inf.infected / total_outflow;
                    new_deaths *= scale;
                    new_recoveries *= scale;
                }

                outflows.push(DiseaseOutflows { new_exposed, exposed_to_infected, new_deaths, new_recoveries });
            } else {
                outflows.push(DiseaseOutflows { new_exposed: 0.0, exposed_to_infected: 0.0, new_deaths: 0.0, new_recoveries: 0.0 });
            }
        }

        // Phase 2: apply outflows and accumulate total deaths.
        // Cap total deaths at alive population, then scale per-disease attribution
        // proportionally so sum(inf.dead) stays consistent with region.dead.
        let raw_total_deaths: f64 = outflows.iter().map(|o| o.new_deaths).sum();
        let total_new_deaths = raw_total_deaths.min(alive);
        let death_scale = if raw_total_deaths > 0.0 { total_new_deaths / raw_total_deaths } else { 1.0 };

        for (i, outflow) in outflows.iter().enumerate() {
            let actual_deaths = outflow.new_deaths * death_scale;
            let inf = &mut region.infections[i];
            // SEIR: new transmissions enter exposed, exposed drains into infected.
            inf.exposed = inf.exposed + outflow.new_exposed - outflow.exposed_to_infected;
            if inf.exposed < 1.0 {
                inf.exposed = 0.0;
            }
            inf.infected = inf.infected + outflow.exposed_to_infected - actual_deaths - outflow.new_recoveries;
            if inf.infected < 1.0 {
                inf.infected = 0.0;
            }
            inf.immune += outflow.new_recoveries;
            inf.dead += actual_deaths; // attribution counter for display
        }
        region.dead += total_new_deaths;

        // Phase 3: cross-disease culling. Dead people are removed from ALL
        // diseases' pools proportionally. A person who died from Disease A
        // might have been infected with or immune to Disease B — reduce B's
        // pools by the fraction of the alive population that just died.
        if total_new_deaths > 0.0 && alive > 0.0 {
            // Each disease's pools are reduced proportionally by OTHER diseases' deaths.
            for (i, outflow) in outflows.iter().enumerate() {
                let inf = &mut region.infections[i];
                // This disease's own deaths already reduced inf.infected above.
                // Only cull for OTHER diseases' deaths.
                let other_deaths = total_new_deaths - outflow.new_deaths * death_scale;
                if other_deaths > 0.0 {
                    let other_survive = 1.0 - (other_deaths / alive);
                    inf.exposed *= other_survive;
                    inf.infected *= other_survive;
                    inf.immune *= other_survive;
                    if inf.exposed < 1.0 {
                        inf.exposed = 0.0;
                    }
                    if inf.infected < 1.0 {
                        inf.infected = 0.0;
                    }
                }
            }
        }
    }
}

/// Spread diseases between connected regions. Clones regions internally for
/// snapshot-based diffusion. Uses `diseases` from the original tick state.
pub(super) fn tick_spread_cross_region(
    new: &mut GameState,
    diseases: &[Disease],
    rng: &mut impl Rng,
) {
    let regions_snapshot: Vec<_> = new.regions.clone();
    for (i, region) in new.regions.iter_mut().enumerate() {
        // No spread into collapsed regions
        if regions_snapshot[i].collapsed {
            continue;
        }
        let dest_has_travel_ban = new.policies.get(i).is_some_and(|p| p.travel_ban);
        let dest_has_border_controls = new.policies.get(i).is_some_and(|p| p.border_controls);
        let dest_gov_eff = regions_snapshot[i].policy_effectiveness();
        let dest_screening_factor = new.policies.get(i)
            .map(|p| p.screening.spread_factor())
            .unwrap_or(1.0);

        for (d_idx, disease) in diseases.iter().enumerate() {
            let connected_infected: f64 = regions_snapshot[i]
                .connections
                .iter()
                .filter_map(|&conn_idx| {
                    // Annihilated regions emit zero spread (population eliminated)
                    if new.policies.get(conn_idx).is_some_and(|p| p.nuclear_state.is_dropped()) {
                        return None;
                    }
                    // Collapsed regions still emit spread, but at reduced rate
                    // (broken infrastructure, but no containment either)
                    let collapse_factor = if regions_snapshot[conn_idx].collapsed {
                        0.3
                    } else {
                        1.0
                    };
                    let source_has_travel_ban =
                        new.policies.get(conn_idx).is_some_and(|p| p.travel_ban);
                    let source_has_border_controls =
                        new.policies.get(conn_idx).is_some_and(|p| p.border_controls);
                    let source_gov_eff = regions_snapshot.get(conn_idx)
                        .map(|r| r.policy_effectiveness())
                        .unwrap_or(1.0);
                    let source_screening_factor = new.policies.get(conn_idx)
                        .map(|p| p.screening.spread_factor())
                        .unwrap_or(1.0);
                    // Travel ban supersedes border controls
                    // Governor hostility weakens enforcement (use min effectiveness of both endpoints)
                    let eff = dest_gov_eff.min(source_gov_eff);
                    let ban_factor = if source_has_travel_ban || dest_has_travel_ban {
                        let base_f = disease.transmission.travel_ban_factor();
                        scale_policy_factor(base_f, eff)
                    } else if source_has_border_controls || dest_has_border_controls {
                        scale_policy_factor(0.5, eff)
                    } else {
                        1.0
                    };
                    // Mass Rapid screening reduces cross-region spread at both ends
                    let screening = scale_policy_factor(
                        dest_screening_factor.min(source_screening_factor), eff
                    );
                    // Island Geography: 50% less inbound spread
                    let island_factor = if regions_snapshot[i].has_trait(RegionTrait::IslandGeography) {
                        0.5
                    } else {
                        1.0
                    };
                    // Include both exposed and infected in cross-region spread:
                    // exposed travelers are pre-symptomatic but still carry the disease.
                    regions_snapshot[conn_idx]
                        .disease_state(d_idx)
                        .map(|inf| (inf.exposed + inf.infected) * ban_factor * collapse_factor * screening * island_factor)
                })
                .sum();

            if connected_infected <= 0.0 {
                continue;
            }

            let has_active_infection = region
                .infections
                .iter()
                .any(|inf| inf.disease_idx == d_idx && (inf.exposed + inf.infected) > 0.0);

            if !has_active_infection {
                let roll: f64 = rng.r#gen();
                let chance = disease.cross_region_spread
                    * disease.transmission.cross_region_modifier()
                    * (connected_infected / 10_000.0);
                if roll < chance.min(0.5) {
                    // Seed proportional to connected infected — a larger outbreak
                    // next door means more travelers carrying the disease.
                    // Split across infected + exposed so the SEIR pipeline has
                    // enough fuel to sustain early exponential growth without
                    // needing a continuous importation trickle.
                    let seed_total = (connected_infected * 0.005).clamp(50.0, 5000.0);
                    let inf_entry = region.get_or_create_infection(d_idx);
                    inf_entry.infected = seed_total * 0.4;
                    inf_entry.exposed = seed_total * 0.6;
                    // Only notify the player about detected diseases spreading
                    if new.diseases[d_idx].detected {
                        new.events.push(GameEvent::DiseaseSpreadToRegion {
                            disease_idx: d_idx,
                            region_idx: i,
                        });
                    }
                }
            }
        }
    }
}

/// Bacterial horizontal gene transfer: broad-spectrum resistance (mechanism=None)
/// can spread between co-located Bacterium diseases. This makes bacteria
/// fundamentally scarier — one resistance event cascades across all bacterial
/// threats. Targeted antibiotic resistance does NOT transfer.
///
/// Rate: ~10% of the resistance gap per day. A donor at 0.4 resistance gives
/// recipients ~0.26 over 10 days — enough to noticeably degrade efficacy
/// within a typical game's timeframe.
/// Only fires when both diseases have active infections in at least one
/// shared region (conjugation requires physical proximity).
pub(super) fn tick_horizontal_gene_transfer(new: &mut GameState) {
    // Collect indices of all Bacterium diseases
    let bacteria: Vec<usize> = new.diseases.iter().enumerate()
        .filter(|(_, d)| d.pathogen_type == PathogenType::Bacterium)
        .map(|(i, _)| i)
        .collect();
    if bacteria.len() < 2 {
        return;
    }

    // For each pair, check if they co-exist in any region
    let transfer_rate = 0.10 / TICKS_PER_DAY;
    let mut transfers: Vec<(usize, usize, f64)> = Vec::new(); // (from, to, amount)

    for i in 0..bacteria.len() {
        for j in (i + 1)..bacteria.len() {
            let di = bacteria[i];
            let dj = bacteria[j];

            // Check co-location: both must have infected > 0 in at least one region
            let coexist = new.regions.iter().any(|r| {
                r.disease_state(di).is_some_and(|inf| inf.infected > 0.0)
                    && r.disease_state(dj).is_some_and(|inf| inf.infected > 0.0)
            });
            if !coexist {
                continue;
            }

            // Transfer mechanism=None resistance from higher to lower
            let ri = new.diseases[di].get_resistance(None);
            let rj = new.diseases[dj].get_resistance(None);
            let gap = ri - rj;
            if gap > 0.01 {
                let amount = gap * transfer_rate;
                transfers.push((di, dj, amount));
            } else if gap < -0.01 {
                let amount = (-gap) * transfer_rate;
                transfers.push((dj, di, amount));
            }
        }
    }

    // Apply transfers and emit events for significant ones
    for (from, to, amount) in transfers {
        new.diseases[to].add_resistance(None, amount);
        // Emit event when resistance crosses 0.05 threshold (first noticeable)
        let new_level = new.diseases[to].get_resistance(None);
        if new_level >= 0.05 && new_level - amount < 0.05 {
            new.events.push(GameEvent::ResistanceTransferred {
                from_disease_idx: from,
                to_disease_idx: to,
            });
        }
    }
}


