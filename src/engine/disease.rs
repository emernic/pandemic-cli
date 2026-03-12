use std::collections::HashMap;

use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    BasicTech, Disease, GameState, MAX_DISEASES, Medicine, MechanismOfAction, MutationMode,
    PathogenType, RegionDiseaseState, ScreeningLevel, TherapyType, TransmissionVector, TICKS_PER_DAY,
};

/// Spawn a new disease: pick a pathogen type, slot the disease, and place the initial outbreak.
/// Returns (disease_idx, region_idx) on success, None if at capacity with no recyclable slots.
pub(super) fn spawn_disease(state: &mut GameState, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
    // If at capacity, try to recycle a burned-out disease slot.
    let recycle_idx = if state.diseases.len() >= MAX_DISEASES {
        find_burned_out_disease(state)
    } else {
        None
    };

    if state.diseases.len() >= MAX_DISEASES && recycle_idx.is_none() {
        return None;
    }

    // Pick a pathogen type. Distribution shifts as the game progresses:
    // Early (day 0-20): mostly RNA viruses and bacteria (natural outbreaks)
    // Mid (day 20-50): balanced across all types
    // Late (day 50+): skews toward fungi and prions (deadlier, harder to treat)
    // Enforce diversity: no type appears more than twice among active diseases.
    let mut type_counts = HashMap::new();
    for (i, d) in state.diseases.iter().enumerate() {
        // Skip the slot being recycled — it's about to be replaced
        if recycle_idx == Some(i) { continue; }
        *type_counts.entry(d.pathogen_type).or_insert(0usize) += 1;
    }
    let day = state.tick as f64 / TICKS_PER_DAY;
    let mut types = pathogen_type_pool(day, rng);
    types.retain(|t| type_counts.get(t).copied().unwrap_or(0) < 2);
    // Fallback: if all types are saturated, allow any type
    if types.is_empty() {
        types = vec![
            PathogenType::RnaVirus,
            PathogenType::DnaVirus,
            PathogenType::Bacterium,
            PathogenType::Fungus,
            PathogenType::Prion,
        ];
    }
    // Counter-capability selection: bias toward types the player can't treat.
    // Ramps smoothly from day 20, fully active by day 50.
    let counter_weight = ((day - 20.0) / 30.0).clamp(0.0, 1.0);
    let pathogen_type = if counter_weight > 0.0 && types.len() > 1 {
        // Check which therapy types the player has deployed
        let deployed_therapies: Vec<TherapyType> = state.medicines.iter()
            .filter(|m| m.deployed_count > 0)
            .map(|m| m.therapy_type)
            .collect();

        let weights: Vec<f64> = types.iter().map(|t| {
            let matched_therapy = t.matched_therapy();
            let player_has_counter = deployed_therapies.contains(&matched_therapy);
            // Types the player CAN treat get lower weight; types they CAN'T get higher
            let counter_bonus = if player_has_counter { 0.3 } else { 2.0 };
            // Blend: uniform early, counter-weighted late
            (1.0 - counter_weight) + counter_bonus * counter_weight
        }).collect();

        let total: f64 = weights.iter().sum();
        let mut roll = rng.r#gen::<f64>() * total;
        let mut chosen = types[0];
        for (j, &w) in weights.iter().enumerate() {
            roll -= w;
            if roll <= 0.0 {
                chosen = types[j];
                break;
            }
        }
        chosen
    } else {
        types[rng.r#gen::<usize>() % types.len()]
    };

    let used_names: Vec<String> = state.diseases.iter().map(|d| d.name.clone()).collect();

    let disease_idx = if let Some(idx) = recycle_idx {
        // Recycle: replace the burned-out disease and clean up its traces.
        let mut disease = Disease::generate(rng, pathogen_type, &used_names, true);
        disease.detected = false;
        disease.spawned_at_tick = state.tick;
        state.diseases[idx] = disease;

        // Reset intel briefing flag so the new disease can trigger pre-detection briefings.
        if idx < state.intel_pre_detection_briefed.len() {
            state.intel_pre_detection_briefed[idx] = false;
        }

        // Remove all infection entries for the old disease in all regions.
        for region in &mut state.regions {
            region.infections.retain(|inf| inf.disease_idx != idx);
        }

        // Cancel any active field research targeting the recycled disease.
        state.field_research.retain(|r| !r.references_disease(idx));
        if state.applied_research.as_ref().is_some_and(|r| r.references_disease(idx)) {
            state.applied_research = None;
        }

        // Remove old medicines targeting the recycled disease (excluding broad-spectrum).
        state.medicines.retain(|m| {
            m.therapy_type == TherapyType::BroadSpectrum
                || !(m.target_diseases.len() == 1 && m.target_diseases[0] == idx)
        });
        // Add new medicines for the replacement disease.
        state.medicines.extend(Medicine::targeted_medicines(idx, pathogen_type));
        super::corporations::assign_manufacturers(state);

        idx
    } else {
        // Normal path: append new disease.
        let idx = state.diseases.len();
        let mut disease = Disease::generate(rng, pathogen_type, &used_names, true);
        disease.detected = false;
        disease.spawned_at_tick = state.tick;
        state.diseases.push(disease);
        state.medicines.extend(Medicine::targeted_medicines(idx, pathogen_type));
        super::corporations::assign_manufacturers(state);

        // Update broad-spectrum medicine to also target new disease
        for med in &mut state.medicines {
            if med.therapy_type == TherapyType::BroadSpectrum
                && !med.target_diseases.contains(&idx)
            {
                med.target_diseases.push(idx);
            }
        }

        idx
    };

    // If Emergency Countermeasure has been enacted, new diseases are also affected.
    if state.enacted_decrees.emergency_countermeasure {
        use crate::state::{COUNTERMEASURE_INFECTIVITY_MULT, COUNTERMEASURE_SPREAD_MULT};
        state.diseases[disease_idx].infectivity *= COUNTERMEASURE_INFECTIVITY_MULT;
        state.diseases[disease_idx].cross_region_spread *= COUNTERMEASURE_SPREAD_MULT;
    }

    // Place initial outbreak. Targeting shifts smoothly:
    // Day 0-20: roughly uniform with vulnerability weighting (weak defenses attractive)
    // Day 24-50: vulnerability blends into strategic importance (high population,
    //   infrastructure, active policies). The player's strongholds become
    //   the most attractive targets. The pattern feels designed, not random.
    let day = state.tick as f64 / TICKS_PER_DAY;
    let targeting = (day / 20.0).min(1.0); // 0→1 over days 0-20
    let strategic = ((day - 24.0) / 26.0).clamp(0.0, 1.0); // 0→1 over days 24-50

    let viable: Vec<usize> = state.regions.iter().enumerate()
        .filter(|(_, r)| !r.collapsed)
        .map(|(i, _)| i)
        .collect();
    let region_idx = if viable.is_empty() {
        rng.r#gen::<usize>() % state.regions.len()
    } else {
        let weights: Vec<f64> = viable.iter().map(|&i| {
            let base = 1.0;

            // Vulnerability score (mid-game): weak defenses are attractive
            let screening_vuln = match state.policies[i].screening {
                ScreeningLevel::None => 3.0,
                ScreeningLevel::Basic => 2.0,
                ScreeningLevel::Antigen => 1.0,
                ScreeningLevel::MassRapid => 0.5,
            };
            let hospital_vuln = match state.regions[i].hospital_level {
                0 => 2.0,
                1 => 1.0,
                _ => 0.5,
            };
            let infection_load = state.regions[i].total_infected().min(100_000.0) / 100_000.0;
            let vuln = screening_vuln + hospital_vuln + infection_load * 2.0;

            // Strategic importance score (late-game): the player's strongholds
            // are attractive — high population, infrastructure, active investment.
            let pop_importance = state.regions[i].population as f64 / 1e9;
            let infrastructure = state.regions[i].hospital_level as f64 * 1.5;
            let active_policies = [
                state.policies[i].travel_ban,
                state.policies[i].quarantine,
                state.policies[i].border_controls,
                state.policies[i].discourage_hosp,
            ].iter().filter(|&&b| b).count() as f64;
            let strategic_value = pop_importance * 2.0 + infrastructure + active_policies;

            // Blend: uniform → vulnerability → strategic importance
            let directional = vuln * (1.0 - strategic) + strategic_value * strategic;
            base + targeting * directional
        }).collect();

        // Weighted random selection
        let total: f64 = weights.iter().sum();
        let mut roll = rng.r#gen::<f64>() * total;
        let mut chosen = viable[0];
        for (j, &w) in weights.iter().enumerate() {
            roll -= w;
            if roll <= 0.0 {
                chosen = viable[j];
                break;
            }
        }
        chosen
    };
    let initial_infected = 500.0 + rng.r#gen::<f64>() * 2_000.0;
    state.regions[region_idx].infections.push(RegionDiseaseState {
        disease_idx,
        exposed: 0.0,
        infected: initial_infected,
        dead: 0.0,
        immune: 0.0,
    });

    Some((disease_idx, region_idx))
}

/// Find a disease with zero infected/exposed across all regions (fully burned out).
fn find_burned_out_disease(state: &GameState) -> Option<usize> {
    for (d_idx, _disease) in state.diseases.iter().enumerate() {
        let total_active: f64 = state.regions.iter()
            .filter_map(|r| r.disease_state(d_idx))
            .map(|inf| inf.exposed + inf.infected)
            .sum();
        if total_active < 1.0 {
            return Some(d_idx);
        }
    }
    None
}

/// Build a weighted pool of pathogen types for disease spawning.
/// Distribution shifts as the game progresses:
/// Early (progression=0): RNA×3, Bact×3, DNA×1, Fungus×1
/// Late  (progression=1): RNA×1, Bact×1, DNA×2, Fungus×3 + prion chance
fn pathogen_type_pool(day: f64, rng: &mut ChaCha8Rng) -> Vec<PathogenType> {
    // Progression factor: 0.0 at day 0, ~1.0 at day 50, capped at 1.0
    let progression = (day / 50.0).min(1.0);

    let rna_weight = lerp_round(3.0, 1.0, progression);
    let bact_weight = lerp_round(3.0, 1.0, progression);
    let dna_weight = lerp_round(1.0, 2.0, progression);
    let fungus_weight = lerp_round(1.0, 3.0, progression);

    let mut types = Vec::new();
    for _ in 0..rna_weight { types.push(PathogenType::RnaVirus); }
    for _ in 0..bact_weight { types.push(PathogenType::Bacterium); }
    for _ in 0..dna_weight { types.push(PathogenType::DnaVirus); }
    for _ in 0..fungus_weight { types.push(PathogenType::Fungus); }

    // Prion chance rises from 5% early to 25% late
    let prion_chance = 0.05 + 0.20 * progression;
    if rng.r#gen::<f64>() < prion_chance {
        types.push(PathogenType::Prion);
    }

    types
}

/// Linear interpolation rounded to nearest integer (at least 1).
fn lerp_round(start: f64, end: f64, t: f64) -> usize {
    (start + (end - start) * t).round().max(1.0) as usize
}

/// Spawn a disease with stats scaled up based on current game day.
/// Later diseases are tougher — simulating evolved superbugs.
/// Infectivity scales at +10%/day (uncapped), lethality at +3.5%/day.
/// The split ensures diseases can sustain growth even under quarantine.
/// After day 20, diseases also seed into multiple regions simultaneously.
pub(super) fn spawn_disease_scaled(state: &mut GameState, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
    let day = state.tick as f64 / TICKS_PER_DAY;
    // Scaling with infectivity outpacing lethality. Infectivity must stay
    // high enough that R > 1 even under quarantine. Halved per-day rates to
    // match TICKS_PER_DAY halving — absolute behavior per tick is unchanged.
    // Day 20: inf 3.0x/leth 1.7x, Day 40: 5.0x/2.4x, Day 60: 7.0x/3.1x
    let inf_scale = 1.0 + day * 0.10;
    let leth_scale = 1.0 + day * 0.035;

    let result = spawn_disease(state, rng)?;
    let (disease_idx, _) = result;

    // Boost the newly spawned disease's stats
    let d = &mut state.diseases[disease_idx];
    d.infectivity *= inf_scale;
    d.lethality *= leth_scale;
    d.cross_region_spread *= inf_scale;
    // Don't scale recovery — harder diseases should be harder to recover from

    // Late-game diseases shift toward Contact transmission (hardest to
    // contain) and get an extra lethality boost on top of the base scaling.
    let optimization = ((day - 30.0) / 30.0).clamp(0.0, 1.0); // 0 at day 30, 1 at day 60
    if optimization > 0.0 {
        let d = &mut state.diseases[disease_idx];
        // Chance to override transmission to Contact
        if rng.r#gen::<f64>() < optimization * 0.5 {
            d.transmission = TransmissionVector::Contact;
        }
        // Extra lethality boost for late-game diseases
        d.lethality *= 1.0 + optimization * 0.5; // up to 50% more lethal on top of scaling
    }

    // Multi-region seeding: after day 30, new diseases emerge simultaneously
    // in additional non-collapsed regions. Pushed from day 20 to day 30 to
    // give containment policies a window to matter before seeding makes them moot.
    // By day 60, every viable region gets seeded.
    let multi_seed = ((day - 30.0) / 30.0).clamp(0.0, 1.0); // 0 at day 30, 1 at day 60
    if multi_seed > 0.0 {
        let (_, primary_region) = result;
        let viable: Vec<usize> = state.regions.iter().enumerate()
            .filter(|(i, r)| !r.collapsed && *i != primary_region)
            .map(|(i, _)| i)
            .collect();
        // Seed count scales with day^2 to ensure late-game diseases hit hard
        // Day 40: ~8.5k, Day 60: ~18.5k, Day 80: ~32.5k
        let base_seed = 500.0 + day * day * 5.0;
        for &region_idx in &viable {
            if rng.r#gen::<f64>() < multi_seed {
                let seed_count = base_seed + rng.r#gen::<f64>() * base_seed;
                state.regions[region_idx].get_or_create_infection(disease_idx).infected += seed_count;
            }
        }
    }

    // Pre-existing resistance: new diseases emerge partially resistant to
    // mechanisms the player has deployed heavily. Invisible to the player —
    // they just notice their old drugs don't work as well on new threats.
    // Kicks in at day 20 to create a noticeable mid-game shift.
    if day >= 20.0 {
        seed_preexisting_resistance(state, disease_idx);
    }

    // Anomalous mutation patterns for late-game diseases (day 25+).
    // Some pathogens exhibit locked or directional mutation — visible only in
    // the data. No UI commentary. A careful player comparing strain generations
    // across diseases can spot the discrepancy.
    if day >= 25.0 {
        // Probability ramps from 0% at day 25 to 50% at day 60.
        let anomaly_prob = ((day - 25.0) / 35.0).clamp(0.0, 1.0) * 0.5;
        if rng.r#gen::<f64>() < anomaly_prob {
            let mode_roll = rng.r#gen::<f64>();
            state.diseases[disease_idx].mutation_mode = if mode_roll < 0.4 {
                MutationMode::Locked
            } else if mode_roll < 0.7 {
                MutationMode::DirectedLethality
            } else {
                MutationMode::DirectedInfectivity
            };
        }
    }

    // Mild tech-aware adaptations: new diseases reflect the player's toolkit
    // but benefits of each tech always outweigh the adaptation penalty.
    adapt_disease_to_player_tech(state, disease_idx);

    // Auto-register new diseases with broad-spectrum medicines. These are
    // known drug classes that work against any pathogen type, so they don't
    // need per-disease clinical trials.
    for med in &mut state.medicines {
        if med.therapy_type == TherapyType::BroadSpectrum {
            if !med.target_diseases.contains(&disease_idx) {
                med.target_diseases.push(disease_idx);
            }
            if !med.tested_against.contains(&disease_idx) {
                med.tested_against.push(disease_idx);
            }
        }
    }

    Some(result)
}

/// Pre-seed a newly spawned disease with resistance to mechanisms the player
/// has deployed most. Simulates evolutionary pressure from the player's
/// medicine usage without announcing it.
fn seed_preexisting_resistance(state: &mut GameState, disease_idx: usize) {
    let day = state.tick as f64 / TICKS_PER_DAY;
    // Intensity ramps from 0 at day 20 to full at day 60
    let intensity = ((day - 20.0) / 40.0).clamp(0.0, 1.0);

    // Aggregate total deployments per mechanism across all medicines
    let mut mech_deployments: HashMap<Option<MechanismOfAction>, u32> = HashMap::new();
    for med in &state.medicines {
        if med.deployed_count > 0 {
            *mech_deployments.entry(med.mechanism).or_insert(0) += med.deployed_count;
        }
    }

    if mech_deployments.is_empty() {
        return;
    }

    let max_deploys = *mech_deployments.values().max().unwrap_or(&0);
    if max_deploys == 0 {
        return;
    }

    // For each mechanism the player has used, add proportional resistance.
    // Broad-spectrum (None mechanism) caps at 50% because it's the universal
    // tool every player deploys first. Targeted mechanisms cap at 30%.
    // The higher BS cap creates the visible "this disease doesn't respond"
    // moment that signals the mid-game shift.
    for (&mechanism, &count) in &mech_deployments {
        let deploy_fraction = count as f64 / max_deploys as f64;
        // Only seed resistance for heavily-used mechanisms (>30% of max)
        if deploy_fraction < 0.3 {
            continue;
        }
        let cap = if mechanism.is_none() { 0.5 } else { 0.3 };
        let resistance = (deploy_fraction * intensity * cap).min(cap);
        if resistance > 0.01 {
            state.diseases[disease_idx].add_resistance(mechanism, resistance);
        }
    }
}

/// Adapt a newly spawned disease to the player's current technological
/// capabilities. New diseases reflect the player's unlocked techs:
/// VaccinePlatform causes +1 strain drift, PathogenSuppression and
/// DirectedAttenuation cause slight lethality increases, etc.
///
/// DESIGN PRINCIPLE: adaptations must be mild enough that the player
/// always feels NET stronger after unlocking a tech. A tech that costs
/// more than it gives is a trap, not an interesting decision. The
/// natural difficulty ramp (more diseases, mutations, infrastructure
/// decay, delivery efficiency loss) provides the real late-game pressure.
/// Adaptations add variety, not punishment.
fn adapt_disease_to_player_tech(state: &mut GameState, disease_idx: usize) {
    let techs = state.unlocked_techs.clone();

    if !techs.is_empty() {
        let d = &mut state.diseases[disease_idx];

        // VaccinePlatform (3x vaccination): diseases emerge +1 strain generation
        // ahead. The player's 3x vaccination far outweighs ~15% efficacy drift.
        if techs.contains(&BasicTech::VaccinePlatform) {
            d.strain_generation += 1;
        }

        // RapidSequencing (2x sequencing speed): no adaptation.
        // Faster identification is a speed benefit, not a combat multiplier.
        // Punishing faster diagnostics with faster spread is illogical.

        // CombinationTherapy (halves resistance buildup): diseases start with
        // slightly elevated mechanism resistance. The 50% reduction in ongoing
        // resistance accumulation dominates this small starting penalty.
        if techs.contains(&BasicTech::CombinationTherapy) {
            for entry in &mut d.mechanism_resistance {
                if entry.level > 0.01 {
                    entry.level = (entry.level * 1.2).min(0.4);
                }
            }
        }

        // PathogenSuppression (unlocks Suppress: -20% infectivity per project):
        // diseases emerge slightly more lethal. One Suppress project more than
        // compensates for a 15% lethality increase.
        if techs.contains(&BasicTech::PathogenSuppression) {
            d.lethality *= 1.15;
        }

        // DirectedAttenuation (unlocks Attenuate: -30% lethality per project):
        // diseases emerge slightly more lethal. One Attenuate project (-30%)
        // more than compensates for a 20% lethality increase.
        if techs.contains(&BasicTech::DirectedAttenuation) {
            d.lethality *= 1.2;
        }

        // GenomicInterdiction (unlocks Interdict: eliminates cross-region spread):
        // diseases spread somewhat faster across regions. Interdict completely
        // eliminates spread for one disease, far outweighing a 30% increase.
        if techs.contains(&BasicTech::GenomicInterdiction) {
            d.cross_region_spread *= 1.3;
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;
    use rand::SeedableRng;

    #[test]
    fn tech_adaptations_are_mild() {
        // Design invariant: with ALL techs unlocked, the combined adaptation
        // multipliers on a disease should be mild enough that benefits outweigh costs.
        // Test by applying adaptations to a known disease and checking multipliers.
        let mut state = GameState::new_default(42);

        // Record disease 0's base stats
        let base_lethality = state.diseases[0].lethality;
        let base_spread = state.diseases[0].cross_region_spread;
        let base_gen = state.diseases[0].strain_generation;

        // Unlock all techs, then apply adaptations to disease 0
        state.unlocked_techs = vec![
            BasicTech::VaccinePlatform,
            BasicTech::RapidSequencing,
            BasicTech::CombinationTherapy,
            BasicTech::PathogenSuppression,
            BasicTech::DirectedAttenuation,
            BasicTech::GenomicInterdiction,
        ];
        adapt_disease_to_player_tech(&mut state, 0);

        // Combined lethality: PathogenSuppression 1.15 × DirectedAttenuation 1.2 = 1.38x
        let lethality_ratio = state.diseases[0].lethality / base_lethality;
        assert!(
            lethality_ratio < 1.5,
            "combined lethality adaptation {:.2}x should be < 1.5x (got {:.4} from {:.4})",
            lethality_ratio, state.diseases[0].lethality, base_lethality
        );

        // Cross-region spread: only GenomicInterdiction 1.3x (RapidSequencing removed)
        let spread_ratio = state.diseases[0].cross_region_spread / base_spread;
        assert!(
            spread_ratio < 1.5,
            "combined spread adaptation {:.2}x should be < 1.5x (got {:.4} from {:.4})",
            spread_ratio, state.diseases[0].cross_region_spread, base_spread
        );

        // VaccinePlatform: exactly +1 strain generation
        assert_eq!(
            state.diseases[0].strain_generation, base_gen + 1,
            "VaccinePlatform should add exactly +1 strain generation"
        );
    }

    #[test]
    fn rapid_sequencing_has_no_adaptation() {
        // RapidSequencing should not penalize new diseases at all.
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(99);

        // Spawn baseline disease with no techs
        let base_result = spawn_disease(&mut state, &mut rng);

        // Reset and try with only RapidSequencing
        let mut state2 = GameState::new_default(42);
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);
        state2.unlocked_techs = vec![BasicTech::RapidSequencing];
        let seq_result = spawn_disease(&mut state2, &mut rng2);

        let (Some((bi, _)), Some((si, _))) = (base_result, seq_result) else {
            panic!("spawn_disease should succeed for both baseline and RapidSequencing");
        };
        let base_d = &state.diseases[bi];
        let seq_d = &state2.diseases[si];
        assert_eq!(
            base_d.cross_region_spread, seq_d.cross_region_spread,
            "RapidSequencing should not affect cross-region spread"
        );
        assert_eq!(
            base_d.lethality, seq_d.lethality,
            "RapidSequencing should not affect lethality"
        );
    }
}
