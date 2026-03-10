use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    GameEvent, GameState, ResearchKind, Scientist, ScientistStatus, ScientistTrait,
    BURNOUT_CHANCE_PER_TICK, BURNOUT_RECOVERY_TICKS, BURNOUT_THRESHOLD_RECKLESS,
    BURNOUT_THRESHOLD_TICKS, FIELD_INFECTION_CHANCE_PER_TICK, FIELD_INFECTION_RECOVERY_TICKS,
};

/// Pick the best available scientists for a research project.
/// Prefers specialty matches, then trait bonuses. Returns IDs.
pub(super) fn pick_scientists(
    state: &GameState,
    count: u32,
    kind: &ResearchKind,
) -> Vec<u64> {
    let assigned_ids = state.assigned_scientist_ids();
    let mut candidates: Vec<&Scientist> = state.scientists.iter()
        .filter(|s| s.is_available() && !assigned_ids.contains(&s.id))
        .collect();

    // Sort: specialty match first, then by trait speed multiplier (descending)
    candidates.sort_by(|a, b| {
        let a_match = a.scientist_trait == ScientistTrait::Versatile
            || a.specialty.matches_research(kind, &state.diseases);
        let b_match = b.scientist_trait == ScientistTrait::Versatile
            || b.specialty.matches_research(kind, &state.diseases);
        b_match.cmp(&a_match)
            .then(b.scientist_trait.speed_multiplier().partial_cmp(&a.scientist_trait.speed_multiplier()).unwrap_or(std::cmp::Ordering::Equal))
    });

    candidates.iter().take(count as usize).map(|s| s.id).collect()
}

/// Pick one more scientist to add to an existing project.
pub(super) fn pick_one_scientist(state: &GameState, kind: &ResearchKind, already_assigned: &[u64]) -> Option<u64> {
    let assigned_ids = state.assigned_scientist_ids();
    let mut candidates: Vec<&Scientist> = state.scientists.iter()
        .filter(|s| s.is_available() && !assigned_ids.contains(&s.id) && !already_assigned.contains(&s.id))
        .collect();

    candidates.sort_by(|a, b| {
        let a_match = a.scientist_trait == ScientistTrait::Versatile
            || a.specialty.matches_research(kind, &state.diseases);
        let b_match = b.scientist_trait == ScientistTrait::Versatile
            || b.specialty.matches_research(kind, &state.diseases);
        b_match.cmp(&a_match)
            .then(b.scientist_trait.speed_multiplier().partial_cmp(&a.scientist_trait.speed_multiplier()).unwrap_or(std::cmp::Ordering::Equal))
    });

    candidates.first().map(|s| s.id)
}

/// Release scientists from a completed/cancelled project.
pub(super) fn release_scientists(state: &mut GameState, ids: &[u64]) {
    for s in &mut state.scientists {
        if ids.contains(&s.id) {
            s.assigned_since = None;
        }
    }
}

/// Tick: check for burnout on long-assigned scientists, and recover burned-out ones.
pub(super) fn tick_personnel(state: &mut GameState, rng: &mut ChaCha8Rng) {
    let tick = state.tick;

    // Recover burned-out and infected scientists
    for s in &mut state.scientists {
        match s.status {
            ScientistStatus::BurnedOut { until_tick } | ScientistStatus::Infected { until_tick } => {
                if tick >= until_tick {
                    s.status = ScientistStatus::Available;
                    s.assigned_since = None;
                }
            }
            _ => {}
        }
    }

    // Check for burnout on assigned scientists
    let assigned_ids = state.assigned_scientist_ids();
    let mut burnout_ids = Vec::new();

    for s in &state.scientists {
        if !assigned_ids.contains(&s.id) || !s.is_available() {
            continue;
        }
        // Cautious scientists never burn out
        if s.scientist_trait == ScientistTrait::Cautious {
            continue;
        }
        if let Some(since) = s.assigned_since {
            let threshold = if s.scientist_trait == ScientistTrait::Reckless {
                BURNOUT_THRESHOLD_RECKLESS
            } else {
                BURNOUT_THRESHOLD_TICKS
            };
            if tick.saturating_sub(since) >= threshold {
                let roll: f64 = rng.r#gen::<f64>();
                if roll < BURNOUT_CHANCE_PER_TICK {
                    burnout_ids.push(s.id);
                }
            }
        }
    }

    // Apply burnout
    for id in &burnout_ids {
        if let Some(s) = state.scientists.iter_mut().find(|s| s.id == *id) {
            let name = s.name.clone();
            s.status = ScientistStatus::BurnedOut { until_tick: tick + BURNOUT_RECOVERY_TICKS };
            s.assigned_since = None;
            state.events.push(GameEvent::ScientistBurnout { scientist_name: name });
        }

        // Remove burned-out scientist from their research project
        remove_from_projects(state, *id);
    }

    // Field infection risk: scientists on field research can contract diseases.
    // Chance scales with global infection severity.
    let field_scientist_ids: Vec<u64> = state.field_research.iter()
        .flat_map(|p| p.scientist_ids.iter().copied())
        .collect();

    if !field_scientist_ids.is_empty() {
        let total_infected: f64 = state.regions.iter()
            .flat_map(|r| &r.infections)
            .map(|inf| inf.infected)
            .sum();
        // Scale: at 100K infected full base rate, caps at 3x
        let severity_mult = (total_infected / 100_000.0).clamp(0.1, 3.0);

        let mut infected_ids = Vec::new();
        for &sid in &field_scientist_ids {
            let s = match state.scientists.iter().find(|s| s.id == sid) {
                Some(s) if s.is_available() => s,
                _ => continue,
            };
            // Trait modifier: Reckless 2x risk, Cautious 0.25x risk
            let trait_mult = match s.scientist_trait {
                ScientistTrait::Reckless => 2.0,
                ScientistTrait::Cautious => 0.25,
                _ => 1.0,
            };
            let chance = FIELD_INFECTION_CHANCE_PER_TICK * severity_mult * trait_mult;
            if rng.r#gen::<f64>() < chance {
                infected_ids.push(sid);
            }
        }

        for id in &infected_ids {
            if let Some(s) = state.scientists.iter_mut().find(|s| s.id == *id) {
                let name = s.name.clone();
                s.status = ScientistStatus::Infected { until_tick: tick + FIELD_INFECTION_RECOVERY_TICKS };
                s.assigned_since = None;
                state.events.push(GameEvent::ScientistInfected { scientist_name: name });
            }
            remove_from_projects(state, *id);
        }
    }
}

/// Remove a scientist from whichever research project they're assigned to.
fn remove_from_projects(state: &mut GameState, id: u64) {
    for p in &mut state.field_research {
        if let Some(pos) = p.scientist_ids.iter().position(|&sid| sid == id) {
            p.scientist_ids.remove(pos);
            p.personnel_assigned = p.scientist_ids.len() as u32;
            return;
        }
    }
    if let Some(p) = &mut state.applied_research {
        if let Some(pos) = p.scientist_ids.iter().position(|&sid| sid == id) {
            p.scientist_ids.remove(pos);
            p.personnel_assigned = p.scientist_ids.len() as u32;
            return;
        }
    }
    if let Some(p) = &mut state.basic_research {
        if let Some(pos) = p.scientist_ids.iter().position(|&sid| sid == id) {
            p.scientist_ids.remove(pos);
            p.personnel_assigned = p.scientist_ids.len() as u32;
        }
    }
}
