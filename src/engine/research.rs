use crate::state::{
    GameOutcome, GameState, ResearchKind, ResearchProject,
    KNOWLEDGE_FULL,
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
        let (personnel, duration) = kind.costs(&state.medicines);

        if state.personnel_available() >= personnel {
            let project = ResearchProject {
                kind: kind.clone(),
                progress: 0.0,
                required_ticks: duration,
                personnel_assigned: personnel,
            };

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

/// Add personnel to an active research project. More personnel = faster progress.
///
/// Returns an optional status message.
pub(super) fn add_personnel(state: &mut GameState, bench: bool) -> Option<String> {
    let available = state.personnel_available();
    let project = if bench { &mut state.bench_research } else { &mut state.field_research };
    if let Some(project) = project {
        if project.is_complete() {
            return None;
        }
        if available >= 1 {
            project.personnel_assigned += 1;
            Some(format!("Assigned +1 personnel ({} total on project)", project.personnel_assigned))
        } else {
            Some("No available personnel to assign".to_string())
        }
    } else {
        None
    }
}

/// Remove personnel from an active research project.
///
/// Returns an optional status message.
pub(super) fn remove_personnel(state: &mut GameState, bench: bool) -> Option<String> {
    let project = if bench { &mut state.bench_research } else { &mut state.field_research };
    if let Some(project) = project {
        if project.personnel_assigned <= 1 {
            Some("Cannot remove — at least 1 person required".to_string())
        } else {
            project.personnel_assigned -= 1;
            Some(format!("Removed 1 personnel ({} remaining on project)", project.personnel_assigned))
        }
    } else {
        None
    }
}

/// Advance research projects by one tick and handle completions.
/// Progress scales linearly with personnel: base personnel = 1x speed,
/// double personnel = 2x speed.
pub(super) fn tick_research(state: &mut GameState) {
    if let Some(ref mut project) = state.field_research {
        let speed = project.speed(&state.medicines);
        project.progress += speed;
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
        let speed = project.speed(&state.medicines);
        project.progress += speed;
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
    };

    #[test]
    fn research_identify_increases_knowledge() {
        let mut state = GameState::new_default(42);
        // Start identify project on disease 0
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #1
        state = apply_action(&state, &Action::Confirm); // Confirm start
        assert!(state.field_research.is_some());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (160 ticks at 1x speed)
        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = GameState::new_default(42);
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
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true; // Pre-unlock for testing

        assert!(state.medicines[0].tested_against.is_empty());

        // Start field research: Clinical Trial
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
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
    fn research_insufficient_personnel_blocks_start() {
        let mut state = GameState::new_default(42);
        state.resources.personnel = 0; // No personnel

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify
        state = apply_action(&state, &Action::Confirm); // Try to confirm

        // Should not have started
        assert!(state.field_research.is_none());
    }

    #[test]
    fn add_personnel_speeds_up_research() {
        let mut state = GameState::new_default(42);

        // Start a field research project
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts project

        assert!(state.field_research.is_some());
        let initial_personnel = state.field_research.as_ref().unwrap().personnel_assigned;

        // Navigate to ViewActive and add personnel (SelectNext = add in ViewActive)
        state = apply_action(&state, &Action::Confirm); // → ViewActive
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ViewActive { bench: false })));

        state = apply_action(&state, &Action::SelectNext); // Add personnel
        assert_eq!(
            state.field_research.as_ref().unwrap().personnel_assigned,
            initial_personnel + 1,
            "should add 1 personnel"
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Assigned"));
    }

    #[test]
    fn remove_personnel_from_research() {
        let mut state = GameState::new_default(42);

        // Start a field research project (needs 5 personnel)
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts

        assert!(state.field_research.is_some());
        let initial_personnel = state.field_research.as_ref().unwrap().personnel_assigned;
        assert!(initial_personnel > 1, "need >1 to test removal");

        // Navigate to ViewActive and remove personnel
        state = apply_action(&state, &Action::Confirm); // → ViewActive

        state = apply_action(&state, &Action::SelectPrev); // Remove personnel
        assert_eq!(
            state.field_research.as_ref().unwrap().personnel_assigned,
            initial_personnel - 1,
            "should remove 1 personnel"
        );

        // Remove down to 1 — should stop
        for _ in 0..20 {
            state = apply_action(&state, &Action::SelectPrev);
        }
        assert_eq!(
            state.field_research.as_ref().unwrap().personnel_assigned,
            1,
            "should not go below 1"
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("at least 1"));
    }

    #[test]
    fn more_personnel_means_faster_progress() {
        let mut state = GameState::new_default(42);

        // Create a project with base 5 personnel, assign 10 for 2x speed
        state.field_research = Some(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 10, // 2x base (5)
        });

        state = tick(&state);
        // At 2x speed, 1 tick should yield 2.0 progress
        assert!(
            (state.field_research.as_ref().unwrap().progress - 2.0).abs() < 0.01,
            "double personnel should give double speed, got {}",
            state.field_research.as_ref().unwrap().progress
        );
    }

    #[test]
    fn concurrent_field_and_bench_research() {
        let mut state = GameState::new_default(42);
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
        state.diseases[0].strain_generation = 2;
        state.diseases[0].knowledge = 1.0;

        // Start and complete DevelopMedicine for medicine 0 (targets disease 0)
        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 24.0, // will complete on next tick
            required_ticks: 25.0,
            personnel_assigned: 5,
        });

        state = tick(&state);
        assert!(state.medicines[0].unlocked);
        assert_eq!(
            state.medicines[0].strain_generations,
            vec![2],
            "medicine should be calibrated to disease generation at completion"
        );
    }

    #[test]
    fn clinical_trial_updates_strain_generation() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 3;
        state.medicines[0].unlocked = true;
        state.medicines[0].strain_generations = vec![0]; // outdated

        state.field_research = Some(ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        });

        state = tick(&state);
        assert!(state.medicines[0].tested_against.contains(&0));
        assert!(
            state.medicines[0].strain_generations[0] >= 3,
            "clinical trial should update strain calibration"
        );
    }

    #[test]
    fn narrow_medicine_cheaper_to_develop_than_broad() {
        let mut state = GameState::new_default(1);
        let disease2 = crate::state::Disease::generate(
            &mut state.rng.clone(), crate::state::PathogenType::Bacterium, &[], true,
        );
        state.diseases.push(disease2);
        let broad_idx = state.medicines.len() - 1;
        state.medicines[broad_idx].target_diseases.push(1);

        let narrow = ResearchKind::DevelopMedicine { medicine_idx: 0 };
        let broad = ResearchKind::DevelopMedicine { medicine_idx: broad_idx };
        let (narrow_pers, narrow_ticks) = narrow.costs(&state.medicines);
        let (broad_pers, broad_ticks) = broad.costs(&state.medicines);
        assert!(narrow_pers <= broad_pers, "narrow should need fewer personnel");
        assert!(narrow_ticks < broad_ticks, "narrow should be faster");
    }

    #[test]
    fn outdated_strain_shows_retrial_available() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 2;
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![0];
        state.medicines[0].strain_generations = vec![0]; // outdated

        let field_projects = state.available_field_projects();
        let has_retrial = field_projects.iter().any(|k| matches!(k,
            ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 }
        ));
        assert!(has_retrial, "should offer clinical trial for strain-outdated medicine");
    }

    #[test]
    fn manufacture_doses_restores_supply() {
        let mut state = GameState::new_default(42);
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
        state.medicines[0].doses = 0.0;

        let bench = state.available_bench_projects();
        assert!(
            bench.iter().any(|k| matches!(k, ResearchKind::ManufactureDoses { medicine_idx: 0 })),
            "manufacture should be available for depleted medicine"
        );

        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
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
        state.diseases[0].knowledge = 1.0;
        let original_rate = state.diseases[0].pathogen_type.mutation_rate();

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
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

        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = GameState::new_default(42);
        let initial_personnel = state.resources.personnel;

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
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
        // Try to start research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select project
        state = apply_action(&state, &Action::Confirm); // Try to confirm
        assert!(state.field_research.is_none(), "should not start research after game over");
    }
}
