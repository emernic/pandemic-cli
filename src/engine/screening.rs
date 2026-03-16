use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    GameEvent, GameOutcome, ScreeningHit, ScreeningModality, ScreeningRun,
    ScreeningRunSize, WorldState, SCREENING_HIT_RATE, KNOWLEDGE_NAME,
};

/// Start a new screening run. Validates inputs, deducts costs, creates the run.
/// Returns (success, optional error message).
pub(super) fn start_screening(
    state: &mut WorldState,
    disease_idx: usize,
    modality: ScreeningModality,
    run_size: ScreeningRunSize,
) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }

    // Validate disease is eligible (identified)
    let eligible = match state.diseases.get(disease_idx) {
        Some(d) if d.detected && d.knowledge >= KNOWLEDGE_NAME => true,
        _ => false,
    };
    if !eligible {
        return (false, Some("Disease must be identified before screening.".into()));
    }

    // Validate modality is unlocked
    if !modality.is_unlocked(&state.unlocked_techs) {
        return (false, Some(format!("{} is not yet unlocked.", modality.label())));
    }

    // Validate run size is unlocked
    if !run_size.is_unlocked(&state.unlocked_techs) {
        return (false, Some(format!("{} run size is not yet unlocked.", run_size.label())));
    }

    // Check funding
    let cost = run_size.funding_cost();
    if state.resources.funding < cost {
        return (false, Some(format!(
            "Need ¥{:.0}, only ¥{:.0} available.",
            cost, state.resources.funding,
        )));
    }

    // Check personnel
    let personnel = run_size.personnel();
    if state.personnel_available() < personnel {
        return (false, Some(format!(
            "Need {} personnel, only {} available.",
            personnel, state.personnel_available(),
        )));
    }

    // Deduct costs
    state.resources.funding -= cost;

    // Create the run with a deterministic sub-seed
    let hit_seed = state.rng_research.r#gen::<u64>();
    let run = ScreeningRun {
        disease_idx,
        modality,
        run_size,
        progress: 0.0,
        required_ticks: run_size.base_ticks(),
        personnel_assigned: personnel,
        hits: Vec::new(),
        hit_seed,
        wells_checked_for_hits: 0,
    };

    state.screening_runs.push(run);
    (true, None)
}

/// Advance all active screening runs by one tick. Handle completions.
pub(super) fn tick_screening(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let infra_mult = state.research_infra_multiplier();

    // Advance progress on all runs
    for run in &mut state.screening_runs {
        let speed = crate::state::personnel_speed(run.personnel_assigned, run.run_size.personnel());
        run.progress += speed * infra_mult;

        // Generate hits incrementally as wells are tested
        let wells_now = run.wells_tested();
        if wells_now > run.wells_checked_for_hits {
            let new_wells = wells_now - run.wells_checked_for_hits;
            let mut hit_rng = ChaCha8Rng::seed_from_u64(
                run.hit_seed.wrapping_add(run.wells_checked_for_hits as u64),
            );
            for i in 0..new_wells {
                let well_idx = run.wells_checked_for_hits + i;
                if hit_rng.r#gen::<f64>() < SCREENING_HIT_RATE {
                    let kd_nm = generate_kd(&mut hit_rng);
                    let compound_id = generate_compound_id(run.modality, run.hits.len() + 1);
                    run.hits.push(ScreeningHit {
                        disease_idx: run.disease_idx,
                        modality: run.modality,
                        kd_nm,
                        compound_id,
                        well_index: well_idx,
                    });
                }
            }
            run.wells_checked_for_hits = wells_now;
        }
    }

    // Collect completed runs
    let mut completed = Vec::new();
    state.screening_runs.retain(|run| {
        if run.is_complete() {
            completed.push(run.clone());
            false
        } else {
            true
        }
    });

    // Process completions: move hits to the global hits pool
    for run in completed {
        let hit_count = run.hits.len();

        for hit in run.hits {
            state.screening_hits.push(hit);
        }

        events.push(GameEvent::ScreeningComplete {
            disease_idx: run.disease_idx,
            hit_count,
        });
    }
}

/// Generate a binding affinity (Kd) in nanomolar.
/// Log-normal distribution: most hits are moderate (10-100 nM),
/// occasional strong binders (<10 nM), rare excellent (<1 nM).
fn generate_kd(rng: &mut impl Rng) -> f64 {
    // Log-uniform between 0.1 and 500 nM, skewed toward higher values
    let log_min = 0.1_f64.ln();
    let log_max = 500.0_f64.ln();
    let log_kd = log_min + rng.r#gen::<f64>() * (log_max - log_min);
    log_kd.exp()
}

/// Generate a compound identifier like "SM-001", "mAb-002", "RNA-003".
fn generate_compound_id(modality: ScreeningModality, seq: usize) -> String {
    let prefix = match modality {
        ScreeningModality::SmallMolecule => "SM",
        ScreeningModality::MonoclonalAntibody => "mAb",
        ScreeningModality::RnaTherapeutic => "RNA",
    };
    format!("{}-{:03}", prefix, seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GameEvent, ScreeningModality, ScreeningRunSize, KNOWLEDGE_NAME};

    /// Build a game state with disease 0 identified (knowledge >= KNOWLEDGE_NAME)
    /// so screening can target it.
    fn state_with_identified_disease() -> WorldState {
        let app = crate::engine::new_game(42);
        let mut world = app.world;
        // Mark disease 0 as detected and identified
        world.diseases[0].detected = true;
        world.diseases[0].knowledge = KNOWLEDGE_NAME;
        // Ensure enough funding and personnel
        world.resources.funding = 10_000.0;
        world.resources.personnel = 20;
        world
    }

    #[test]
    fn start_screening_rejects_unidentified_disease() {
        let mut world = state_with_identified_disease();
        world.diseases[0].knowledge = KNOWLEDGE_NAME - 0.01;

        let (ok, msg) = start_screening(
            &mut world,
            0,
            ScreeningModality::SmallMolecule,
            ScreeningRunSize::Small,
        );
        assert!(!ok);
        assert!(msg.unwrap().contains("identified"));
    }

    #[test]
    fn start_screening_rejects_insufficient_funding() {
        let mut world = state_with_identified_disease();
        world.resources.funding = 1.0; // way below Small cost of 50

        let (ok, msg) = start_screening(
            &mut world,
            0,
            ScreeningModality::SmallMolecule,
            ScreeningRunSize::Small,
        );
        assert!(!ok);
        assert!(msg.unwrap().contains("¥"));
    }

    #[test]
    fn start_screening_rejects_insufficient_personnel() {
        let mut world = state_with_identified_disease();
        world.resources.personnel = 0;

        let (ok, msg) = start_screening(
            &mut world,
            0,
            ScreeningModality::SmallMolecule,
            ScreeningRunSize::Small,
        );
        assert!(!ok);
        assert!(msg.unwrap().contains("personnel"));
    }

    #[test]
    fn start_screening_rejects_locked_modality() {
        let mut world = state_with_identified_disease();
        // MonoclonalAntibody requires BasicTech::MonoclonalAntibodies
        let (ok, msg) = start_screening(
            &mut world,
            0,
            ScreeningModality::MonoclonalAntibody,
            ScreeningRunSize::Small,
        );
        assert!(!ok);
        assert!(msg.unwrap().contains("not yet unlocked"));
    }

    #[test]
    fn start_screening_success_deducts_funds_and_creates_run() {
        let mut world = state_with_identified_disease();
        let funding_before = world.resources.funding;
        let cost = ScreeningRunSize::Small.funding_cost();

        let (ok, msg) = start_screening(
            &mut world,
            0,
            ScreeningModality::SmallMolecule,
            ScreeningRunSize::Small,
        );
        assert!(ok);
        assert!(msg.is_none());
        assert_eq!(world.screening_runs.len(), 1);
        assert!((world.resources.funding - (funding_before - cost)).abs() < 0.01);

        let run = &world.screening_runs[0];
        assert_eq!(run.disease_idx, 0);
        assert_eq!(run.modality, ScreeningModality::SmallMolecule);
        assert_eq!(run.run_size, ScreeningRunSize::Small);
        assert_eq!(run.progress, 0.0);
        assert_eq!(run.personnel_assigned, ScreeningRunSize::Small.personnel());
    }

    #[test]
    fn tick_screening_advances_wells_proportionally_and_completes() {
        let mut world = state_with_identified_disease();
        let (ok, _) = start_screening(
            &mut world,
            0,
            ScreeningModality::SmallMolecule,
            ScreeningRunSize::Small,
        );
        assert!(ok);

        let total_wells = world.screening_runs[0].total_wells();
        let required_ticks = world.screening_runs[0].required_ticks;

        // After a few ticks, wells_tested should be proportional to progress
        for _ in 0..3 {
            tick_screening(&mut world, &mut Vec::new());
        }
        let run = &world.screening_runs[0];
        let expected_frac = (run.progress / run.required_ticks).min(1.0);
        let expected_wells = (expected_frac * total_wells as f64).round() as u32;
        assert_eq!(
            run.wells_tested(), expected_wells,
            "wells_tested should track progress proportionally",
        );
        assert!(run.wells_tested() > 0, "should have tested some wells after 3 ticks");
        assert!(run.wells_tested() < total_wells, "should not be done yet");

        // Tick to completion
        let max_ticks = (required_ticks * 2.0) as usize;
        let mut events = Vec::new();
        for _ in 0..max_ticks {
            if world.screening_runs.is_empty() {
                break;
            }
            events.clear();
            tick_screening(&mut world, &mut events);
        }

        assert!(
            world.screening_runs.is_empty(),
            "run should complete within {} ticks (required: {})",
            max_ticks,
            required_ticks,
        );
        assert!(
            events.iter().any(|e| matches!(e, GameEvent::ScreeningComplete { .. })),
            "should emit ScreeningComplete event",
        );
    }

    #[test]
    fn hit_generation_is_deterministic() {
        // Run two identical screenings with the same seed and verify same hits
        let make_world = || {
            let mut world = state_with_identified_disease();
            let (ok, _) = start_screening(
                &mut world,
                0,
                ScreeningModality::SmallMolecule,
                ScreeningRunSize::Small,
            );
            assert!(ok);
            world
        };

        let mut world1 = make_world();
        let mut world2 = make_world();

        // Force identical hit seeds (they come from rng_research which may differ)
        world2.screening_runs[0].hit_seed = world1.screening_runs[0].hit_seed;

        // Tick both to completion
        let max_ticks = (world1.screening_runs[0].required_ticks * 2.0) as usize;
        for _ in 0..max_ticks {
            let mut e1 = Vec::new();
            let mut e2 = Vec::new();
            tick_screening(&mut world1, &mut e1);
            tick_screening(&mut world2, &mut e2);
        }

        // Same number of hits with same kd values
        assert_eq!(world1.screening_hits.len(), world2.screening_hits.len());
        for (h1, h2) in world1.screening_hits.iter().zip(world2.screening_hits.iter()) {
            assert!((h1.kd_nm - h2.kd_nm).abs() < 1e-10, "hits should have identical kd_nm");
            assert_eq!(h1.compound_id, h2.compound_id);
        }
    }

    #[test]
    fn completed_run_hits_move_to_global_pool() {
        let mut world = state_with_identified_disease();
        let (ok, _) = start_screening(
            &mut world,
            0,
            ScreeningModality::SmallMolecule,
            ScreeningRunSize::Medium, // more wells = more likely to get hits
        );
        assert!(ok);
        assert!(world.screening_hits.is_empty());

        // Tick to completion
        let max_ticks = (world.screening_runs[0].required_ticks * 2.0) as usize;
        let mut all_events = Vec::new();
        for _ in 0..max_ticks {
            let mut events = Vec::new();
            tick_screening(&mut world, &mut events);
            all_events.extend(events);
        }

        // Verify completion event reports correct hit count
        let complete_event = all_events.iter().find_map(|e| {
            if let GameEvent::ScreeningComplete { hit_count, .. } = e {
                Some(*hit_count)
            } else {
                None
            }
        });
        assert!(complete_event.is_some(), "should have completion event");
        assert_eq!(
            complete_event.unwrap(),
            world.screening_hits.len(),
            "event hit_count should match global hits pool size",
        );
    }
}
