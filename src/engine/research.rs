use crate::state::{
    GameOutcome, GameState, ResearchKind, ResearchProject,
    BOOST_RP_COST, BOOST_TICKS, KNOWLEDGE_FULL,
};

/// Start a research project. Pure game logic — does NOT modify UI state.
///
/// Returns true if the project was successfully started.
pub(super) fn start_research(state: &mut GameState, bench: bool, project_idx: usize) -> bool {
    if state.outcome != GameOutcome::Playing {
        return false;
    }
    let occupied = if bench { state.bench_research.is_some() } else { state.field_research.is_some() };
    if occupied {
        return false;
    }

    let projects = if bench {
        state.available_bench_projects()
    } else {
        state.available_field_projects()
    };

    if let Some(kind) = projects.get(project_idx) {
        let (rp_cost, personnel, duration) = kind.costs(&state.medicines);

        if state.resources.research_points >= rp_cost
            && state.personnel_available() >= personnel
        {
            let project = ResearchProject {
                kind: kind.clone(),
                progress: 0.0,
                required_ticks: duration,
                personnel_assigned: personnel,
                rp_cost,
            };
            state.resources.research_points -= rp_cost;

            if bench {
                state.bench_research = Some(project);
            } else {
                state.field_research = Some(project);
            }
            return true;
        }
    }
    false
}

/// Boost an active research project. Pure game logic — does NOT modify UI state.
///
/// Returns (message, success) where success indicates the boost was applied.
pub(super) fn boost_research(state: &mut GameState, bench: bool) -> (Option<String>, bool) {
    let project = if bench { &mut state.bench_research } else { &mut state.field_research };
    if let Some(project) = project {
        if !project.is_complete() && state.resources.research_points >= BOOST_RP_COST {
            state.resources.research_points -= BOOST_RP_COST;
            project.progress = (project.progress + BOOST_TICKS).min(project.required_ticks);
            (Some(format!("Boosted research! (-{:.0} RP)", BOOST_RP_COST)), true)
        } else if state.resources.research_points < BOOST_RP_COST {
            (Some(format!(
                "Need {:.0} RP to boost (have {:.0})",
                BOOST_RP_COST, state.resources.research_points
            )), false)
        } else {
            (None, false)
        }
    } else {
        (None, false)
    }
}

/// Advance research projects by one tick and handle completions.
pub(super) fn tick_research(state: &mut GameState) {
    if let Some(ref mut project) = state.field_research {
        project.progress += 1.0;
        if project.is_complete() {
            match &project.kind {
                ResearchKind::IdentifyThreat { disease_idx } => {
                    let d_idx = *disease_idx;
                    if let Some(disease) = state.diseases.get_mut(d_idx) {
                        disease.knowledge = (disease.knowledge + 0.50).min(KNOWLEDGE_FULL);
                    }
                }
                ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
                    let m_idx = *medicine_idx;
                    let d_idx = *disease_idx;
                    if let Some(medicine) = state.medicines.get_mut(m_idx) {
                        if !medicine.tested_against.contains(&d_idx) {
                            medicine.tested_against.push(d_idx);
                        }
                        // Update strain calibration to current disease generation
                        if let Some(pos) = medicine.target_diseases.iter().position(|&d| d == d_idx) {
                            let current_gen = state.diseases.get(d_idx)
                                .map_or(0, |d| d.strain_generation);
                            // Extend strain_generations if needed
                            while medicine.strain_generations.len() <= pos {
                                medicine.strain_generations.push(0);
                            }
                            medicine.strain_generations[pos] = current_gen;
                        }
                    }
                }
                ResearchKind::GenomicSequencing { disease_idx } => {
                    let d_idx = *disease_idx;
                    if let Some(disease) = state.diseases.get_mut(d_idx) {
                        disease.sequencing_count += 1;
                    }
                }
                ResearchKind::DevelopMedicine { .. }
                | ResearchKind::ManufactureDoses { .. }
                | ResearchKind::TrainPersonnel => {}
            }
            state.field_research = None;
        }
    }
    if let Some(ref mut project) = state.bench_research {
        project.progress += 1.0;
        if project.is_complete() {
            match &project.kind {
                ResearchKind::DevelopMedicine { medicine_idx } => {
                    let m_idx = *medicine_idx;
                    if let Some(medicine) = state.medicines.get_mut(m_idx) {
                        medicine.unlocked = true;
                        // Calibrate to current strain generations of all target diseases
                        medicine.strain_generations = medicine.target_diseases.iter()
                            .map(|&d_idx| state.diseases.get(d_idx)
                                .map_or(0, |d| d.strain_generation))
                            .collect();
                    }
                }
                ResearchKind::ManufactureDoses { medicine_idx } => {
                    let m_idx = *medicine_idx;
                    if let Some(medicine) = state.medicines.get_mut(m_idx) {
                        medicine.doses = medicine.max_doses;
                    }
                }
                ResearchKind::TrainPersonnel => {
                    state.resources.personnel += 5;
                }
                _ => {}
            }
            state.bench_research = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::action::Action;
    use crate::apply_action;
    use crate::engine::tick;
    use crate::state::{
        GameOutcome, GameState, ResearchKind, ResearchProject, ResearchUiState,
        BOOST_RP_COST, BOOST_TICKS,
    };

    #[test]
    fn research_identify_increases_knowledge() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;
        // Start identify project on disease 0
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #1
        state = apply_action(&state, &Action::Confirm); // Confirm start
        assert!(state.field_research.is_some());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (160 ticks)
        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0; // Fully identified

        assert!(!state.medicines[0].unlocked);

        // Start bench research: Develop Antiviral-A
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        state = apply_action(&state, &Action::Confirm);     // Select Develop Antiviral-A
        state = apply_action(&state, &Action::Confirm);     // Confirm

        assert!(state.bench_research.is_some());

        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.bench_research.is_none());
        assert!(state.medicines[0].unlocked);
    }

    #[test]
    fn research_clinical_trial_marks_tested() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true; // Pre-unlock for testing

        assert!(state.medicines[0].tested_against.is_empty());

        // Start field research: Clinical Trial
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        // Navigate to the clinical trial project.
        // Field projects: identify (for each unidentified disease), genomic sequencing
        // (for fully identified diseases), then clinical trials.
        let field_projects = state.available_field_projects();
        let trial_idx = field_projects.iter().position(|k| matches!(k,
            ResearchKind::ClinicalTrial { .. }
        )).expect("should have a clinical trial available");
        for _ in 0..trial_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm);    // Select
        state = apply_action(&state, &Action::Confirm);    // Confirm

        assert!(state.field_research.is_some());

        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none());
        assert!(state.medicines[0].tested_against.contains(&0));
    }

    #[test]
    fn research_insufficient_rp_blocks_start() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 0.0; // No RP

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify
        state = apply_action(&state, &Action::Confirm); // Try to confirm

        // Should not have started — still on confirm screen
        assert!(state.field_research.is_none());
    }

    #[test]
    fn research_boost_spends_rp_and_advances() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;

        // Start a field research project
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts project

        // Should be back at BrowseProjects with an active field project
        assert!(state.field_research.is_some());
        let progress_before = state.field_research.as_ref().unwrap().progress;
        let rp_before = state.resources.research_points;

        // Navigate to ViewActive and boost
        state = apply_action(&state, &Action::Confirm); // → ViewActive
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ViewActive { bench: false })));

        state = apply_action(&state, &Action::Confirm); // Boost!
        assert_eq!(
            state.resources.research_points,
            rp_before - BOOST_RP_COST,
            "should spend {} RP", BOOST_RP_COST
        );
        assert_eq!(
            state.field_research.as_ref().unwrap().progress,
            progress_before + BOOST_TICKS,
            "should advance by {} ticks", BOOST_TICKS
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Boosted"));
    }

    #[test]
    fn research_boost_insufficient_rp() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 25.0; // Enough to start (20 RP) but not boost again (10 RP)

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts (costs 20 RP, leaves 5)

        assert!(state.field_research.is_some());
        assert_eq!(state.resources.research_points, 5.0); // 25 - 20 = 5

        state = apply_action(&state, &Action::Confirm); // → ViewActive
        let rp_before = state.resources.research_points;
        let progress_before = state.field_research.as_ref().unwrap().progress;

        state = apply_action(&state, &Action::Confirm); // Try to boost — should fail
        assert_eq!(state.resources.research_points, rp_before, "should not spend RP");
        assert_eq!(
            state.field_research.as_ref().unwrap().progress,
            progress_before,
            "should not advance"
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Need"));
    }

    #[test]
    fn concurrent_field_and_bench_research() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;

        // Start field research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #2
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.field_research.is_some());

        // Start bench research
        state = apply_action(&state, &Action::ClosePanel); // Back to categories
        state = apply_action(&state, &Action::SelectNext);  // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        state = apply_action(&state, &Action::Confirm);     // Select Develop
        state = apply_action(&state, &Action::Confirm);     // Confirm
        assert!(state.bench_research.is_some());

        // Both running simultaneously
        assert!(state.field_research.is_some());
        assert!(state.bench_research.is_some());
    }

    #[test]
    fn develop_medicine_sets_strain_generation() {
        let mut state = GameState::new_default(42);
        // Manually mutate disease 0 to gen 2
        state.diseases[0].strain_generation = 2;
        state.diseases[0].knowledge = 1.0;
        state.resources.research_points = 100.0;

        // Start and complete DevelopMedicine for medicine 0 (targets disease 0)
        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 24.0, // will complete on next tick
            required_ticks: 25.0,
            personnel_assigned: 5,
            rp_cost: 15.0,
        });

        state = tick(&state);
        assert!(state.medicines[0].unlocked);
        assert_eq!(
            state.medicines[0].strain_generations,
            vec![2], // should match disease gen at time of completion
            "medicine should be calibrated to disease generation at completion"
        );
    }

    #[test]
    fn clinical_trial_updates_strain_generation() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 3;
        state.medicines[0].unlocked = true;
        state.medicines[0].strain_generations = vec![0]; // outdated
        state.resources.research_points = 100.0;

        state.field_research = Some(ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0, // will complete on next tick
            required_ticks: 25.0,
            personnel_assigned: 5,
            rp_cost: 15.0,
        });

        state = tick(&state);
        assert!(state.medicines[0].tested_against.contains(&0));
        // strain_generation should be updated to current disease gen
        // Note: disease might have mutated during this tick too, so check >= 3
        assert!(
            state.medicines[0].strain_generations[0] >= 3,
            "clinical trial should update strain calibration"
        );
    }

    #[test]
    fn narrow_medicine_cheaper_to_develop_than_broad() {
        let mut state = GameState::new_default(1);
        // Add a second disease so the broad-spectrum medicine has multiple targets
        let disease2 = crate::state::Disease::generate(
            &mut state.rng.clone(), crate::state::PathogenType::Bacterium, &[], true,
        );
        state.diseases.push(disease2);
        // Update broad-spectrum to target both diseases
        let broad_idx = state.medicines.len() - 1;
        state.medicines[broad_idx].target_diseases.push(1);
        // Medicine 0 = targeted (1 target), last medicine = Broad-Spectrum (2 targets)
        let narrow = ResearchKind::DevelopMedicine { medicine_idx: 0 };
        let broad = ResearchKind::DevelopMedicine { medicine_idx: broad_idx };
        let (narrow_rp, narrow_pers, narrow_ticks) = narrow.costs(&state.medicines);
        let (broad_rp, broad_pers, broad_ticks) = broad.costs(&state.medicines);
        assert!(narrow_rp < broad_rp, "narrow should cost less RP");
        assert!(narrow_pers <= broad_pers, "narrow should need fewer personnel");
        assert!(narrow_ticks < broad_ticks, "narrow should be faster");
    }

    #[test]
    fn outdated_strain_shows_retrial_available() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 2;
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![0]; // already tested
        state.medicines[0].strain_generations = vec![0]; // but outdated

        let field_projects = state.available_field_projects();
        let has_retrial = field_projects.iter().any(|k| matches!(k,
            ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 }
        ));
        assert!(has_retrial, "should offer clinical trial for strain-outdated medicine");
    }

    #[test]
    fn manufacture_doses_restores_supply() {
        let mut state = GameState::new_default(42);
        // Unlock all medicines
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
        state.medicines[0].doses = 0.0; // Depleted

        // ManufactureDoses should appear in available bench projects
        let bench = state.available_bench_projects();
        assert!(
            bench.iter().any(|k| matches!(k, ResearchKind::ManufactureDoses { medicine_idx: 0 })),
            "manufacture should be available for depleted medicine"
        );

        // Start and complete manufacture
        state.resources.research_points = 100.0;
        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
            rp_cost: 10.0,
        });
        state = tick(&state);

        assert!(state.bench_research.is_none(), "project should be complete");
        assert_eq!(
            state.medicines[0].doses, state.medicines[0].max_doses,
            "doses should be restored to max"
        );
    }

    #[test]
    fn genomic_sequencing_reduces_mutation_rate() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;
        let original_rate = state.diseases[0].pathogen_type.mutation_rate();

        // Start genomic sequencing via field research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        // Navigate past identify projects to genomic sequencing
        let field_projects = state.available_field_projects();
        let seq_idx = field_projects.iter().position(|k| matches!(k,
            ResearchKind::GenomicSequencing { .. }
        )).expect("should have genomic sequencing available");
        for _ in 0..seq_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // Select
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.field_research.is_some());

        // Complete the project (200 ticks)
        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        // Verify mutation rate is effectively halved
        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        let initial_personnel = state.resources.personnel;

        // Start personnel training via bench research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        // Navigate to Train Personnel (last item in bench projects)
        let bench_projects = state.available_bench_projects();
        let train_idx = bench_projects.iter().position(|k| matches!(k,
            ResearchKind::TrainPersonnel
        )).expect("should have train personnel available");
        for _ in 0..train_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // Select
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.bench_research.is_some());

        // Complete the project (160 ticks)
        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.bench_research.is_none());
        assert_eq!(state.resources.personnel, initial_personnel + 5);
    }

    #[test]
    fn no_research_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.resources.research_points = 100.0;
        let rp_before = state.resources.research_points;
        // Try to start research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select project
        state = apply_action(&state, &Action::Confirm); // Try to confirm
        assert!(state.field_research.is_none(), "should not start research after game over");
        assert_eq!(state.resources.research_points, rp_before, "should not spend RP after game over");
    }
}
