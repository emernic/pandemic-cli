use rand::Rng;

use crate::state::{
    Disease, GameEvent, GameState, RegionDiseaseState, TransmissionVector,
};

/// Spread diseases within each region. Uses `diseases` (the original tick's
/// disease parameters) to avoid borrow conflicts — the caller passes
/// `&state.diseases` from the immutable input while `new` is the mutable clone.
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

        for inf in &mut region.infections {
            if let Some(disease) = diseases.get(inf.disease_idx) {
                let susceptible = pop - inf.infected - inf.dead - inf.immune;
                if susceptible <= 0.0 {
                    continue;
                }

                let noise: f64 = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.1;
                let mut infectivity = if quarantine_active {
                    disease.infectivity * disease.transmission.quarantine_factor()
                } else {
                    disease.infectivity
                };
                // Contact diseases spread faster when hospital surge is active
                // (healthcare workers in close contact with patients)
                if hospital_active {
                    infectivity *= disease.transmission.hospital_infectivity_factor();
                }
                // Water sanitation halves waterborne disease infectivity
                if sanitation_active && disease.transmission == TransmissionVector::Waterborne {
                    infectivity *= 0.5;
                }
                let new_infections =
                    infectivity * inf.infected * (susceptible / pop) * noise;
                let new_infections = new_infections.max(0.0).min(susceptible);

                // Deaths and recoveries are concurrent outflows from the infected pool.
                // Compute both, then scale proportionally if they exceed infected.
                let lethality = if hospital_active {
                    disease.lethality * 0.5
                } else {
                    disease.lethality
                };
                let mut new_deaths = (lethality * inf.infected * noise).max(0.0);
                let mut new_recoveries = (disease.recovery_rate * inf.infected * noise).max(0.0);
                let total_outflow = new_deaths + new_recoveries;
                if total_outflow > inf.infected {
                    let scale = inf.infected / total_outflow;
                    new_deaths *= scale;
                    new_recoveries *= scale;
                }

                inf.infected = inf.infected + new_infections - new_deaths - new_recoveries;
                // Snap to zero when below 1 person — aligns with WIN_INFECTED_THRESHOLD
                if inf.infected < 1.0 {
                    inf.infected = 0.0;
                }
                inf.immune += new_recoveries;
                inf.dead += new_deaths;
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
        let dest_has_screening = new.policies.get(i).is_some_and(|p| p.border_controls);

        for (d_idx, disease) in diseases.iter().enumerate() {
            let connected_infected: f64 = regions_snapshot[i]
                .connections
                .iter()
                .filter_map(|&conn_idx| {
                    // No spread from collapsed regions
                    if regions_snapshot[conn_idx].collapsed {
                        return None;
                    }
                    let source_has_travel_ban =
                        new.policies.get(conn_idx).is_some_and(|p| p.travel_ban);
                    let source_has_screening =
                        new.policies.get(conn_idx).is_some_and(|p| p.border_controls);
                    // Travel ban supersedes border controls
                    let ban_factor = if source_has_travel_ban || dest_has_travel_ban {
                        disease.transmission.travel_ban_factor()
                    } else if source_has_screening || dest_has_screening {
                        0.5
                    } else {
                        1.0
                    };
                    regions_snapshot[conn_idx]
                        .disease_state(d_idx)
                        .map(|inf| inf.infected * ban_factor)
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
            disease.infectivity = (disease.infectivity * inf_factor).clamp(0.010, 0.070);
            let leth_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            disease.lethality = (disease.lethality * leth_factor).clamp(0.001, 0.020);
            new.events.push(GameEvent::DiseaseMutated {
                disease_idx: d_idx,
                new_generation: disease.strain_generation,
            });
        }
    }
}
