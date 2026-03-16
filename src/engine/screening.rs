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
    if !run_size.is_unlocked() {
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
    let lab_mult = state.lab_speed_multiplier();
    let biotech_bonus = (0..state.regions.len())
        .map(|r| state.sector_bonus(r, crate::state::CorporationSector::Biotech))
        .fold(0.0_f64, f64::max);
    let biotech_mult = 1.0 + crate::state::CorporationSector::Biotech.max_bonus_pct() / 100.0 * biotech_bonus;

    // Advance progress on all runs
    for run in &mut state.screening_runs {
        let speed = crate::state::personnel_speed(run.personnel_assigned, run.run_size.personnel());
        run.progress += speed * lab_mult * biotech_mult;

        // Generate hits incrementally as wells are tested
        let wells_now = run.wells_tested();
        if wells_now > run.wells_checked_for_hits {
            let new_wells = wells_now - run.wells_checked_for_hits;
            let mut hit_rng = ChaCha8Rng::seed_from_u64(
                run.hit_seed.wrapping_add(run.wells_checked_for_hits as u64),
            );
            for _ in 0..new_wells {
                if hit_rng.r#gen::<f64>() < SCREENING_HIT_RATE {
                    let kd_nm = generate_kd(&mut hit_rng);
                    let compound_id = generate_compound_id(run.modality, run.hits.len() + 1);
                    run.hits.push(ScreeningHit {
                        disease_idx: run.disease_idx,
                        modality: run.modality,
                        kd_nm,
                        compound_id,
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
        let disease_name = state.diseases.get(run.disease_idx)
            .map(|d| d.display_name(run.disease_idx))
            .unwrap_or_else(|| format!("Disease #{}", run.disease_idx));

        for hit in run.hits {
            state.screening_hits.push(hit);
        }

        events.push(GameEvent::ScreeningComplete {
            disease_idx: run.disease_idx,
            hit_count,
        });

        let msg = if hit_count > 0 {
            format!(
                "Screening complete: {} {} hit{} found against {}",
                hit_count,
                run.modality.label(),
                if hit_count == 1 { "" } else { "s" },
                disease_name,
            )
        } else {
            format!(
                "Screening complete: no hits found against {} ({})",
                disease_name,
                run.modality.label(),
            )
        };
        state.event_log.push_front((state.tick as f64 / crate::state::TICKS_PER_DAY, msg));
        while state.event_log.len() > 50 {
            state.event_log.pop_back();
        }
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
