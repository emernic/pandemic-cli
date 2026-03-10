use std::collections::HashMap;

use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{BasicTech, GameState, MechanismOfAction, TransmissionVector, TICKS_PER_DAY};

/// Spawn a disease with stats scaled up based on current game day.
/// Later diseases are tougher — simulating evolved superbugs.
/// Scaling: +5% per day elapsed, capped at 3x base stats.
pub(super) fn spawn_disease_scaled(state: &mut GameState, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
    let day = state.tick as f64 / TICKS_PER_DAY;
    let scale = (1.0 + day * 0.05).min(3.0);

    let result = state.spawn_disease(rng)?;
    let (disease_idx, _) = result;

    // Boost the newly spawned disease's stats
    let d = &mut state.diseases[disease_idx];
    d.infectivity *= scale;
    d.lethality *= scale;
    d.cross_region_spread *= scale;
    // Don't scale recovery — harder diseases should be harder to recover from

    // Late-game optimization: diseases shift toward Contact transmission
    // (hardest to contain with travel bans, 95% blocked vs 90% airborne)
    // and concentrate their spread within regions rather than across them.
    let optimization = ((day - 15.0) / 15.0).clamp(0.0, 1.0); // 0 at day 15, 1 at day 30
    if optimization > 0.0 {
        let d = &mut state.diseases[disease_idx];
        // Chance to override transmission to Contact
        if rng.r#gen::<f64>() < optimization * 0.5 {
            d.transmission = TransmissionVector::Contact;
        }
        // Concentrate: reduce cross-region spread, boost lethality
        d.cross_region_spread *= 1.0 - optimization * 0.3; // up to 30% less spread
        d.lethality *= 1.0 + optimization * 0.2; // up to 20% more lethal
    }

    // Pre-existing resistance: new diseases emerge partially resistant to
    // mechanisms the player has deployed heavily. Invisible to the player —
    // they just notice their old drugs don't work as well on new threats.
    if day >= 20.0 {
        seed_preexisting_resistance(state, disease_idx);
    }

    // Tech-aware adaptations: diseases that emerge against a capable player
    // are designed to exploit gaps in their toolkit. The arms race is bidirectional.
    adapt_disease_to_player_tech(state, disease_idx, rng);

    Some(result)
}

/// Pre-seed a newly spawned disease with resistance to mechanisms the player
/// has deployed most. Simulates evolutionary pressure from the player's
/// medicine usage without announcing it.
fn seed_preexisting_resistance(state: &mut GameState, disease_idx: usize) {
    let day = state.tick as f64 / TICKS_PER_DAY;
    // Intensity ramps from 0 at day 20 to full at day 40
    let intensity = ((day - 20.0) / 20.0).clamp(0.0, 1.0);

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
    // Cap at 0.3 (30%) — enough to be noticeable but not game-breaking.
    for (&mechanism, &count) in &mech_deployments {
        let deploy_fraction = count as f64 / max_deploys as f64;
        // Only seed resistance for heavily-used mechanisms (>30% of max)
        if deploy_fraction < 0.3 {
            continue;
        }
        let resistance = deploy_fraction * intensity * 0.3; // max 0.3 at full intensity
        if resistance > 0.01 {
            state.diseases[disease_idx].add_resistance(mechanism, resistance);
        }
    }
}

/// Adapt a newly spawned disease to the player's current technological
/// capabilities. Diseases evolve to exploit the specific gaps in advanced
/// toolkits. This is the bidirectional arms race: better tools attract
/// harder threats.
fn adapt_disease_to_player_tech(state: &mut GameState, disease_idx: usize, rng: &mut ChaCha8Rng) {
    let techs = state.unlocked_techs.clone();

    // Tech-specific adaptations only apply when the player has unlocked techs
    if !techs.is_empty() {
        let d = &mut state.diseases[disease_idx];

        // VaccinePlatform unlocked → diseases emerge pre-mutated.
        // The player's vaccines target strain gen 0, but this disease starts ahead.
        // Forces immediate re-sequencing and re-trialing.
        if techs.contains(&BasicTech::VaccinePlatform) {
            d.strain_generation += 1 + (rng.r#gen::<usize>() % 2) as u32; // +1 or +2
        }

        // RapidSequencing unlocked → diseases spread more aggressively across regions.
        // The player can track mutations fast, so diseases compensate by spreading
        // to more regions before detection, making containment harder.
        if techs.contains(&BasicTech::RapidSequencing) {
            d.cross_region_spread *= 1.4;
        }

        // CombinationTherapy unlocked → diseases have broader mechanism resistance.
        // The player can hit with multiple mechanisms, so diseases pre-adapt to more.
        // (Amplifies the existing resistance seeding from seed_preexisting_resistance)
        if techs.contains(&BasicTech::CombinationTherapy) {
            for entry in &mut d.mechanism_resistance {
                if entry.level > 0.01 {
                    entry.level = (entry.level * 1.5).min(0.5);
                }
            }
        }

        // PathogenSuppression unlocked → diseases emerge with higher base lethality.
        // The player can suppress infectivity, so diseases shift toward killing fast
        // rather than spreading wide.
        if techs.contains(&BasicTech::PathogenSuppression) {
            d.lethality *= 1.3;
            d.recovery_rate *= 0.8; // harder to recover from
        }

        // DirectedAttenuation unlocked → diseases emerge with even higher lethality.
        // The player can reduce lethality, so diseases compensate with more virulence.
        if techs.contains(&BasicTech::DirectedAttenuation) {
            d.lethality *= 1.4;
        }

        // GenomicInterdiction unlocked → diseases emerge with much higher cross-region
        // spread. The player can eliminate transmission, so diseases spread aggressively
        // before the player can interdict them.
        if techs.contains(&BasicTech::GenomicInterdiction) {
            d.cross_region_spread *= 1.6;
        }
    }

    // Active quarantines → new diseases emerge with partial containment adaptation.
    // This runs regardless of tech state — quarantine pressure is about active
    // policy measures, not research capabilities.
    let quarantine_count = state.policies.iter().filter(|p| p.quarantine).count();
    if quarantine_count >= 2 {
        let d = &mut state.diseases[disease_idx];
        d.containment_adaptation = 0.2 + (quarantine_count as f64 * 0.05).min(0.3);
    }
}
