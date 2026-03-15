use std::collections::HashMap;

use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    Disease, GameEvent, GameState, MAX_DISEASES, Medicine,
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
            let player_has_counter = t.matched_therapy()
                .is_some_and(|therapy| deployed_therapies.contains(&therapy));
            // Types the player CAN treat get lower weight; types they CAN'T get higher.
            // Untreatable types (prions) always get the "no counter" weight.
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

        // Cancel any active research targeting the recycled disease.
        state.active_research.retain(|r| !r.references_disease(idx));

        // Remove old medicines targeting the recycled disease (excluding broad-spectrum).
        state.medicines.retain(|m| {
            m.therapy_type == TherapyType::BroadSpectrum
                || !(m.target_diseases.len() == 1 && m.target_diseases[0] == idx)
        });
        // Add new medicines for the replacement disease.
        state.medicines.extend(Medicine::targeted_medicines(idx, pathogen_type));

        idx
    } else {
        // Normal path: append new disease.
        let idx = state.diseases.len();
        let mut disease = Disease::generate(rng, pathogen_type, &used_names, true);
        disease.detected = false;
        disease.spawned_at_tick = state.tick;
        state.diseases.push(disease);
        state.medicines.extend(Medicine::targeted_medicines(idx, pathogen_type));

        // Update broad-spectrum medicine to also target new disease.
        // BS medicines work against any pathogen type without per-disease
        // clinical trials, so we register both target and tested status.
        for med in &mut state.medicines {
            if med.therapy_type == TherapyType::BroadSpectrum {
                if !med.target_diseases.contains(&idx) {
                    med.target_diseases.push(idx);
                }
                if !med.tested_against.contains(&idx) {
                    med.tested_against.push(idx);
                }
            }
        }

        idx
    };

    // If Emergency Countermeasure has been enacted, new diseases are also affected.
    if state.enacted_decrees.emergency_countermeasure {
        use crate::state::{COUNTERMEASURE_SPREAD_WITHIN_MULT, COUNTERMEASURE_SPREAD_MULT};
        state.diseases[disease_idx].within_region_spread *= COUNTERMEASURE_SPREAD_WITHIN_MULT;
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
/// Within-region infectivity scales at +10%/day (uncapped), lethality at +3.5%/day.
/// Cross-region spread scales gently at +2%/day to preserve regional containment.
/// Each new disease spawns in exactly one region — no multi-region seeding.
pub(super) fn spawn_disease_scaled(state: &mut GameState, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
    let day = state.tick as f64 / TICKS_PER_DAY;
    // Within-region spread outpaces lethality so later diseases sustain
    // growth even under quarantine.
    // Day 20: inf 3.0x/leth 1.7x, Day 40: 5.0x/2.4x, Day 60: 7.0x/3.1x
    let inf_scale = 1.0 + day * 0.10;
    let leth_scale = 1.0 + day * 0.035;

    let result = spawn_disease(state, rng)?;
    let (disease_idx, _) = result;

    // Boost the newly spawned disease's stats
    let d = &mut state.diseases[disease_idx];
    d.within_region_spread *= inf_scale;
    d.lethality *= leth_scale;
    // Cross-region scales much more gently than within-region — regional
    // containment is a core player strategy. A day-60 disease should be
    // harder to contain but not bypass borders entirely.
    let cross_scale = 1.0 + day * 0.02; // +2%/day vs +10%/day for within-region
    d.cross_region_spread *= cross_scale;
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

    Some(result)
}

/// Check each original (non-variant) disease for variant spawning.
/// Each disease rolls against its effective_variant_rate() per tick.
/// Only root diseases (variant_number == 0) can spawn variants.
pub(super) fn tick_variant_spawning(state: &mut GameState, rng: &mut ChaCha8Rng, events: &mut Vec<GameEvent>) {
    // Collect spawn candidates: (parent_idx, effective_rate)
    let candidates: Vec<(usize, f64)> = state.diseases.iter().enumerate()
        .filter(|(_, d)| d.variant_number == 0) // only root diseases spawn variants
        .map(|(i, d)| (i, d.effective_variant_rate()))
        .collect();

    for (parent_idx, rate) in candidates {
        if rng.r#gen::<f64>() >= rate {
            continue;
        }
        // Try to spawn a variant
        if let Some(variant_idx) = spawn_variant(state, parent_idx, rng) {
            let parent_name = state.diseases[parent_idx].name.clone();
            events.push(GameEvent::VariantEmerged {
                disease_idx: variant_idx,
                parent_name,
            });
        }
    }
}

/// Spawn a variant of the given parent disease.
/// Returns the new disease index on success, None if at capacity.
fn spawn_variant(
    state: &mut GameState,
    parent_idx: usize,
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    // Check capacity — try to recycle a burned-out slot
    let recycle_idx = if state.diseases.len() >= MAX_DISEASES {
        find_burned_out_disease(state)
    } else {
        None
    };
    if state.diseases.len() >= MAX_DISEASES && recycle_idx.is_none() {
        return None;
    }

    let parent = &state.diseases[parent_idx];
    let parent_name = parent.name.clone();
    let lineage_name = parent.parent_lineage.clone().unwrap_or_else(|| parent_name.clone());

    // Count existing variants of this lineage to determine variant_number
    let existing_variants = state.diseases.iter()
        .filter(|d| {
            d.parent_lineage.as_deref() == Some(&lineage_name)
        })
        .count() as u32;
    let variant_number = existing_variants + 1;

    // Generate variant name: "Parent II", "Parent III", etc.
    let variant_name = format!("{} {}", lineage_name, roman_numeral(variant_number + 1));

    // Scale stats relative to parent, boosted by time elapsed since parent spawned.
    // This means variants are always tougher than their parents.
    let parent_day = parent.spawned_at_tick as f64 / TICKS_PER_DAY;
    let current_day = state.tick as f64 / TICKS_PER_DAY;
    let days_elapsed = (current_day - parent_day).max(1.0);
    let inf_boost = 1.0 + days_elapsed * 0.10;
    let leth_boost = 1.0 + days_elapsed * 0.035;
    let cross_boost = 1.0 + days_elapsed * 0.02;

    let mut variant = Disease {
        name: variant_name,
        pathogen_type: parent.pathogen_type,
        transmission: parent.transmission,
        within_region_spread: parent.within_region_spread * inf_boost,
        lethality: parent.lethality * leth_boost,
        cross_region_spread: parent.cross_region_spread * cross_boost,
        recovery_rate: parent.recovery_rate,
        knowledge: parent.knowledge * 0.4, // 40% of parent's knowledge
        parent_lineage: Some(lineage_name),
        variant_number,
        sequencing_count: 0, // variants need their own sequencing
        detected: false,
        spawned_at_tick: state.tick,
        mechanism_resistance: vec![],
        sequence_group: None,
        incubation_ticks: parent.incubation_ticks,
        first_detected_regions: vec![],
        detected_day: 0.0,
        prev_day_observed_infected: 0.0,
        current_day_observed_infected: 0.0,
    };

    // Ensure stats don't go below minimum floors
    variant.within_region_spread = variant.within_region_spread.max(0.001);
    variant.lethality = variant.lethality.max(0.0001);

    // Place the variant in a random non-collapsed region
    let valid_regions: Vec<usize> = state.regions.iter().enumerate()
        .filter(|(_, r)| !r.collapsed)
        .map(|(i, _)| i)
        .collect();
    if valid_regions.is_empty() {
        return None;
    }
    let region_idx = valid_regions[rng.r#gen::<usize>() % valid_regions.len()];

    let disease_idx = if let Some(idx) = recycle_idx {
        // Recycle burned-out slot — same cleanup as spawn_disease
        state.diseases[idx] = variant;
        if idx < state.intel_pre_detection_briefed.len() {
            state.intel_pre_detection_briefed[idx] = false;
        }
        for region in &mut state.regions {
            region.infections.retain(|inf| inf.disease_idx != idx);
        }
        state.active_research.retain(|r| !r.references_disease(idx));
        // Remove old targeted medicines (keep broad-spectrum)
        state.medicines.retain(|m| {
            m.therapy_type == TherapyType::BroadSpectrum
                || !(m.target_diseases.len() == 1 && m.target_diseases[0] == idx)
        });
        idx
    } else {
        let idx = state.diseases.len();
        state.diseases.push(variant);
        idx
    };

    // Seed initial infection
    let initial_infected = 10.0;
    state.regions[region_idx].infections.push(RegionDiseaseState {
        disease_idx,
        exposed: 0.0,
        infected: initial_infected,
        dead: 0.0,
        immune: 0.0,
    });

    // Generate targeted medicines for the variant (same as new disease emergence)
    let new_meds = Medicine::targeted_medicines(disease_idx, state.diseases[disease_idx].pathogen_type);
    state.medicines.extend(new_meds);

    // Update broad-spectrum medicine to also target variant.
    // BS medicines work against any pathogen type without per-disease
    // clinical trials, so we register both target and tested status.
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

    // Apply Emergency Countermeasure if enacted
    if state.enacted_decrees.emergency_countermeasure {
        use crate::state::{COUNTERMEASURE_SPREAD_WITHIN_MULT, COUNTERMEASURE_SPREAD_MULT};
        state.diseases[disease_idx].within_region_spread *= COUNTERMEASURE_SPREAD_WITHIN_MULT;
        state.diseases[disease_idx].cross_region_spread *= COUNTERMEASURE_SPREAD_MULT;
    }

    Some(disease_idx)
}

/// Convert a number to a Roman numeral string (for variant naming).
fn roman_numeral(n: u32) -> String {
    match n {
        2 => "II".into(),
        3 => "III".into(),
        4 => "IV".into(),
        5 => "V".into(),
        6 => "VI".into(),
        7 => "VII".into(),
        8 => "VIII".into(),
        9 => "IX".into(),
        10 => "X".into(),
        _ => format!("{}", n),
    }
}