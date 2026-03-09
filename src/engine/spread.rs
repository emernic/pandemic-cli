use rand::Rng;

use crate::state::{
    Disease, GameEvent, GameState, PathogenType, RegionDiseaseState, TICKS_PER_DAY,
};

/// Per-disease outflows computed in phase 1, applied in phase 2.
struct DiseaseOutflows {
    new_infections: f64,
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
        let hospital_active = policy.is_some_and(|p| p.hospital_surge);
        let sanitation_active = policy.is_some_and(|p| p.water_sanitation);
        let alive = (pop - region.dead).max(0.0);

        // Phase 1: compute outflows for each disease without mutating yet.
        let mut outflows: Vec<DiseaseOutflows> = Vec::with_capacity(region.infections.len());
        for inf in &region.infections {
            if let Some(disease) = diseases.get(inf.disease_idx) {
                let susceptible = alive - inf.infected - inf.immune;
                if susceptible <= 0.0 {
                    outflows.push(DiseaseOutflows { new_infections: 0.0, new_deaths: 0.0, new_recoveries: 0.0 });
                    continue;
                }

                let noise: f64 = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.1;
                let mut infectivity = if quarantine_active {
                    disease.infectivity * disease.transmission.quarantine_factor()
                } else {
                    disease.infectivity
                };
                if hospital_active {
                    infectivity *= disease.transmission.hospital_infectivity_factor();
                }
                if sanitation_active {
                    infectivity *= disease.transmission.water_sanitation_factor();
                }
                // Mass Rapid screening identifies and isolates cases, reducing spread
                let screening_factor = policy
                    .map(|p| p.screening.spread_factor())
                    .unwrap_or(1.0);
                infectivity *= screening_factor;
                let new_infections =
                    (infectivity * inf.infected * (susceptible / pop) * noise)
                        .max(0.0).min(susceptible);

                let mut lethality = disease.lethality * region.healthcare_modifier;
                if hospital_active {
                    lethality *= 0.5;
                }
                if region.healthcare_invested {
                    lethality *= 0.75;
                }
                let mut new_deaths = (lethality * inf.infected * noise).max(0.0);
                let mut new_recoveries = (disease.recovery_rate * inf.infected * noise).max(0.0);
                let total_outflow = new_deaths + new_recoveries;
                if total_outflow > inf.infected {
                    let scale = inf.infected / total_outflow;
                    new_deaths *= scale;
                    new_recoveries *= scale;
                }

                outflows.push(DiseaseOutflows { new_infections, new_deaths, new_recoveries });
            } else {
                outflows.push(DiseaseOutflows { new_infections: 0.0, new_deaths: 0.0, new_recoveries: 0.0 });
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
            inf.infected = inf.infected + outflow.new_infections - actual_deaths - outflow.new_recoveries;
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
                    inf.infected *= other_survive;
                    inf.immune *= other_survive;
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
        let dest_screening_factor = new.policies.get(i)
            .map(|p| p.screening.spread_factor())
            .unwrap_or(1.0);

        for (d_idx, disease) in diseases.iter().enumerate() {
            let connected_infected: f64 = regions_snapshot[i]
                .connections
                .iter()
                .filter_map(|&conn_idx| {
                    // Annihilated regions emit zero spread (population eliminated)
                    if new.policies.get(conn_idx).is_some_and(|p| p.nuclear_annihilation) {
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
                    let source_screening_factor = new.policies.get(conn_idx)
                        .map(|p| p.screening.spread_factor())
                        .unwrap_or(1.0);
                    // Travel ban supersedes border controls
                    let ban_factor = if source_has_travel_ban || dest_has_travel_ban {
                        disease.transmission.travel_ban_factor()
                    } else if source_has_border_controls || dest_has_border_controls {
                        0.5
                    } else {
                        1.0
                    };
                    // Mass Rapid screening reduces cross-region spread at both ends
                    let screening = dest_screening_factor.min(source_screening_factor);
                    regions_snapshot[conn_idx]
                        .disease_state(d_idx)
                        .map(|inf| inf.infected * ban_factor * collapse_factor * screening)
                })
                .sum();

            if connected_infected <= 0.0 {
                continue;
            }

            let has_active_infection = region
                .infections
                .iter()
                .any(|inf| inf.disease_idx == d_idx && inf.infected > 0.0);

            if !has_active_infection {
                let roll: f64 = rng.r#gen();
                let chance = disease.cross_region_spread
                    * disease.transmission.cross_region_modifier()
                    * (connected_infected / 10_000.0);
                if roll < chance.min(0.5) {
                    // Seed proportional to connected infected — a larger outbreak
                    // next door means more travelers carrying the disease.
                    let seed_count = (connected_infected * 0.001).clamp(1.0, 1000.0);
                    // Check if there's an existing entry (e.g., from vaccination)
                    if let Some(existing) = region
                        .infections
                        .iter_mut()
                        .find(|inf| inf.disease_idx == d_idx)
                    {
                        existing.infected = seed_count;
                    } else {
                        region.infections.push(RegionDiseaseState {
                            disease_idx: d_idx,
                            infected: seed_count,
                            dead: 0.0,
                            immune: 0.0,
                        });
                    }
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

/// Apply disease mutation. Each disease has a chance to mutate per tick,
/// drifting infectivity and lethality parameters slightly.
pub(super) fn tick_mutation(new: &mut GameState, rng: &mut impl Rng) {
    // Disease mutation (sequencing reduces mutation rate by half per level)
    for (d_idx, disease) in new.diseases.iter_mut().enumerate() {
        let mutation_chance = disease.effective_mutation_rate();
        if rng.r#gen::<f64>() < mutation_chance {
            disease.strain_generation += 1;
            // Small random parameter changes (±10% of current value), clamped to
            // prevent runaway drift over many mutations.
            let inf_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            // Clamps prevent runaway drift. Note: prions with max outflow
            // (lethality cap 0.005 + recovery ≤0.0006 = 0.0056) can have
            // R0 < 1 at the infectivity floor (0.003/0.0056 ≈ 0.54) — they
            // burn out and get replaced by the spawn system, which is fine.
            disease.infectivity = (disease.infectivity * inf_factor).clamp(0.003, 0.020);
            let leth_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            disease.lethality = (disease.lethality * leth_factor).clamp(0.0003, 0.005);
            new.events.push(GameEvent::DiseaseMutated {
                disease_idx: d_idx,
                new_generation: disease.strain_generation,
                infectivity_factor: inf_factor,
                lethality_factor: leth_factor,
            });
        }
    }
}
