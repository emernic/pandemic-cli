use crate::state::{
    GameEvent, GameOutcome, GameState, ResearchKind, ResearchProject,
    ResearchTrack, KNOWLEDGE_FULL, KNOWLEDGE_NAME,
    TRAIN_PERSONNEL_BATCH,
    LAB_LEVEL_1_COST, LAB_LEVEL_2_COST,
};

/// Start a research project. Pure game logic — does NOT modify UI state.
///
/// Returns (success, message).
pub(super) fn start_research(state: &mut GameState, track: ResearchTrack, project_idx: usize, double_personnel: bool) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    match track {
        ResearchTrack::Field => {
            if !state.field_research_has_capacity() {
                return (false, None);
            }
        }
        _ => {
            if state.research_slot(track).is_some() {
                return (false, None);
            }
        }
    }

    let projects = state.available_projects(track);

    if let Some(kind) = projects.get(project_idx) {
        let (base_personnel, duration, funding_cost) = state.effective_costs(kind);
        let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };

        if state.resources.funding < funding_cost {
            return (false, Some(super::medicine::insufficient_funds_message(funding_cost, state.resources.funding)));
        }
        if state.personnel_available() < personnel {
            return (false, Some(format!(
                "Need {} personnel, only {} available",
                personnel, state.personnel_available(),
            )));
        }
        state.resources.funding -= funding_cost;
        // Human Trials decree: clinical trials complete faster
        let effective_duration = if matches!(kind, ResearchKind::ClinicalTrial { .. })
            && state.enacted_decrees.authorize_human_trials
        {
            duration * crate::state::HUMAN_TRIALS_SPEED
        } else {
            duration
        };
        let project = ResearchProject {
            kind: kind.clone(),
            progress: 0.0,
            required_ticks: effective_duration,
            personnel_assigned: personnel,
        };

        match track {
            ResearchTrack::Field => state.field_research.push(project),
            ResearchTrack::Applied => state.applied_research = Some(project),
            ResearchTrack::Basic => state.basic_research = Some(project),
        }
        return (true, None);
    }
    (false, None)
}


/// Advance research projects by one tick and handle completions.
/// Progress scales with diminishing returns: 2x personnel = 1.5x speed (peak),
/// beyond 2x personnel = negative returns (too many cooks).
pub(super) fn tick_research(state: &mut GameState, rng: &mut impl rand::Rng) {
    // Proactively auto-start research on idle tracks (e.g., new unknown pathogen
    // detected while auto-research is enabled and a field slot is free).
    for track in [ResearchTrack::Field, ResearchTrack::Applied, ResearchTrack::Basic] {
        try_auto_start(state, track);
    }

    let lab_mult = state.lab_speed_multiplier();

    // Advance all field research projects and collect completion effects
    for project in &mut state.field_research {
        let speed = project.speed(&state.medicines);
        project.progress += speed * lab_mult;
    }
    // Process completions (drain_filter pattern via retain)
    let mut completed_fields: Vec<ResearchProject> = Vec::new();
    state.field_research.retain(|p| {
        if p.is_complete() {
            completed_fields.push(p.clone());
            false
        } else {
            true
        }
    });
    for project in &completed_fields {
        match &project.kind {
            ResearchKind::IdentifyThreat { disease_idx } => {
                let d_idx = *disease_idx;
                let was_unknown = state.diseases.get(d_idx)
                    .is_some_and(|d| d.knowledge < KNOWLEDGE_NAME);
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.knowledge = (disease.knowledge + 0.50).min(KNOWLEDGE_FULL);
                }
                // Fire identification event when crossing the name threshold
                if was_unknown && state.diseases.get(d_idx)
                    .is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME)
                {
                    state.events.push(GameEvent::PathogenIdentified { disease_idx: d_idx });
                }
            }
            ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
                let m_idx = *medicine_idx;
                let d_idx = *disease_idx;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    if !medicine.tested_against.contains(&d_idx) {
                        medicine.tested_against.push(d_idx);
                    }
                    // Promote cross-reactive targets to primary targets.
                    // A successful trial proves the medicine works against this disease.
                    if !medicine.target_diseases.contains(&d_idx) {
                        medicine.target_diseases.push(d_idx);
                    }
                    // Update strain calibration to current disease generation
                    let pos = medicine.target_diseases.iter().position(|&d| d == d_idx).unwrap();
                    let current_gen = state.diseases.get(d_idx)
                        .map_or(0, |d| d.strain_generation) as i32;
                    while medicine.strain_generations.len() <= pos {
                        medicine.strain_generations.push(0);
                    }
                    medicine.strain_generations[pos] = current_gen;
                }
                state.events.push(GameEvent::TrialCompleted {
                    medicine_idx: m_idx,
                    disease_idx: d_idx,
                });
                // Enable auto-deploy when a trial succeeds so players discover the feature.
                // Idempotent — safe to set on re-trials (mutation catch-up). Players can
                // disable it in the Medicines panel [X] if they prefer manual control.
                while state.auto_deploy.len() <= m_idx {
                    state.auto_deploy.push(false);
                }
                state.auto_deploy[m_idx] = true;
                // Human Trials decree: chance of adverse event killing infected
                if state.enacted_decrees.authorize_human_trials {
                    let roll: f64 = rng.r#gen();
                    if roll < crate::state::HUMAN_TRIALS_ADVERSE_CHANCE {
                        let kill_frac = crate::state::HUMAN_TRIALS_KILL_FRACTION;
                        // Kill fraction of infected across all regions for this disease
                        let mut total_killed = 0.0;
                        for region in &mut state.regions {
                            if let Some(inf) = region.infections.iter_mut()
                                .find(|i| i.disease_idx == d_idx)
                            {
                                let killed = inf.infected * kill_frac;
                                inf.infected -= killed;
                                inf.dead += killed;
                                region.dead += killed;
                                total_killed += killed;
                            }
                        }
                        if total_killed > 0.0 {
                            state.events.push(GameEvent::HumanTrialAdverseEvent {
                                disease_idx: d_idx,
                                deaths: total_killed,
                            });
                        }
                    }
                }
            }
            ResearchKind::GenomicSequencing { disease_idx } => {
                let d_idx = *disease_idx;
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.sequencing_count += 1;
                }
            }
            ResearchKind::SuppressPathogen { disease_idx } => {
                let d_idx = *disease_idx;
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.infectivity *= 0.80;
                    state.pathogens_suppressed += 1;
                    state.events.push(GameEvent::PathogenSuppressed { disease_idx: d_idx });
                }
            }
            ResearchKind::AttenuatePathogen { disease_idx } => {
                let d_idx = *disease_idx;
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.lethality *= 0.70;
                    state.pathogens_attenuated += 1;
                    state.events.push(GameEvent::PathogenAttenuated { disease_idx: d_idx });
                }
            }
            ResearchKind::InterdictPathogen { disease_idx } => {
                let d_idx = *disease_idx;
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.cross_region_spread = 0.0;
                    state.pathogens_interdicted += 1;
                    state.events.push(GameEvent::PathogenInterdicted { disease_idx: d_idx });
                }
            }
            _ => {}
        }
    }
    // Auto-start next field project if any completed and auto is on
    if !completed_fields.is_empty() {
        try_auto_start(state, ResearchTrack::Field);
        // Notify player if field completions unlocked Applied research options
        if state.applied_research.is_none() {
            if let Some(kind) = state.available_applied_projects().iter()
                .find(|p| matches!(p, ResearchKind::DevelopMedicine { .. }))
            {
                if let ResearchKind::DevelopMedicine { medicine_idx } = kind {
                    let name = state.medicines.get(*medicine_idx)
                        .map(|m| m.name.as_str()).unwrap_or("medicine");
                    state.events.push(GameEvent::ResearchHandoff {
                        message: format!("{} development available — open Applied Research [R]", name),
                    });
                }
            }
        }
    }
    if let Some(ref mut project) = state.applied_research {
        let speed = project.speed(&state.medicines);
        project.progress += speed * lab_mult;
    }
    if state.applied_research.as_ref().is_some_and(|p| p.is_complete()) {
        let project = state.applied_research.take().unwrap();
        match &project.kind {
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let m_idx = *medicine_idx;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.unlocked = true;
                    // Calibrate to current strain generations of all target diseases
                    medicine.strain_generations = medicine.target_diseases.iter()
                        .map(|&d_idx| state.diseases.get(d_idx)
                            .map_or(0, |d| d.strain_generation as i32))
                        .collect();
                }
                state.events.push(GameEvent::MedicineDeveloped { medicine_idx: m_idx });
                // Notify about clinical trial availability on Field track
                let has_trial_available = state.available_field_projects().iter()
                    .any(|p| matches!(p, ResearchKind::ClinicalTrial { medicine_idx: mi, .. } if *mi == m_idx));
                if has_trial_available && state.field_research_has_capacity() {
                    let name = state.medicines.get(m_idx)
                        .map(|m| m.name.as_str()).unwrap_or("medicine");
                    state.events.push(GameEvent::ResearchHandoff {
                        message: format!("{} needs clinical trial — open Field Research [R]", name),
                    });
                }
            }
            ResearchKind::ManufactureDoses { medicine_idx } => {
                let m_idx = *medicine_idx;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.doses = medicine.max_doses;
                }
            }
            ResearchKind::TrainPersonnel => {
                state.resources.personnel += TRAIN_PERSONNEL_BATCH;
            }
            _ => {}
        }
        try_auto_start(state, ResearchTrack::Applied);
    }
    if let Some(ref mut project) = state.basic_research {
        let speed = project.speed(&state.medicines);
        project.progress += speed * lab_mult;
    }
    if state.basic_research.as_ref().is_some_and(|p| p.is_complete()) {
        let project = state.basic_research.take().unwrap();
        if let ResearchKind::BasicResearch { tech } = &project.kind {
            let tech = *tech;
            if !state.unlocked_techs.contains(&tech) {
                state.unlocked_techs.push(tech);
                state.events.push(GameEvent::TechUnlocked { tech });
            }
        }
        try_auto_start(state, ResearchTrack::Basic);
    }
}

/// Try to auto-start the next research project on a track (if auto-research is enabled).
///
/// To avoid starving other tracks of funding, auto-start will skip if the project
/// cost would leave less than the cheapest available project on any other track.
fn try_auto_start(state: &mut GameState, track: ResearchTrack) {
    if !state.auto_research[track.index()] {
        return;
    }
    // Check if there's room to start a new project
    match track {
        ResearchTrack::Field => {
            if !state.field_research_has_capacity() {
                return;
            }
        }
        _ => {
            if state.research_slot(track).is_some() {
                return;
            }
        }
    }
    let projects = state.available_projects(track);
    if projects.is_empty() {
        return;
    }
    // Check if we can afford this project without starving other tracks
    let (_, _, cost) = state.effective_costs(&projects[0]);
    let reserve = min_progression_cost(state, track);
    if state.resources.funding - cost < reserve {
        return;
    }
    // Try to start the first (highest-priority) project with default personnel
    let (ok, _msg) = start_research(state, track, 0, false);
    if ok {
        state.events.push(GameEvent::ResearchAutoStarted { track });
    }
}

/// Minimum funding needed across other tracks for strategic progression.
/// Checks available DevelopMedicine and BasicResearch projects on other tracks.
/// Returns 0.0 if no progression projects are available.
fn min_progression_cost(state: &GameState, exclude: ResearchTrack) -> f64 {
    let mut min_cost = f64::MAX;
    for track in [ResearchTrack::Field, ResearchTrack::Applied, ResearchTrack::Basic] {
        if track == exclude {
            continue;
        }
        let has_capacity = match track {
            ResearchTrack::Field => state.field_research_has_capacity(),
            _ => state.research_slot(track).is_none(),
        };
        if !has_capacity {
            continue;
        }
        for kind in state.available_projects(track) {
            // Only reserve for progression projects, not routine utility
            let is_progression = matches!(
                kind,
                ResearchKind::DevelopMedicine { .. }
                    | ResearchKind::BasicResearch { .. }
                    | ResearchKind::ClinicalTrial { .. }
            );
            if !is_progression {
                continue;
            }
            let (_, _, cost) = state.effective_costs(&kind);
            if cost < min_cost {
                min_cost = cost;
            }
        }
    }
    if min_cost == f64::MAX { 0.0 } else { min_cost }
}

/// Upgrade the global research lab (level 0→1 or 1→2). One-time funding cost.
/// Returns (success, message).
pub(super) fn upgrade_lab(state: &mut GameState) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    let (cost, next_name) = match state.lab_level {
        0 => (LAB_LEVEL_1_COST, "Enhanced Sequencing Lab"),
        1 => (LAB_LEVEL_2_COST, "Advanced Genomics Center"),
        _ => return (false, Some("Research lab is already at maximum level.".into())),
    };
    if state.resources.funding < cost {
        return (false, Some(format!("Not enough funding (need ¥{:.0})", cost)));
    }
    state.resources.funding -= cost;
    state.lab_level += 1;
    (true, Some(format!("Lab upgraded to {}. Research speed +{}%.", next_name,
        if state.lab_level == 1 { 30 } else { 60 })))
}

#[cfg(test)]
mod tests {
    use crate::action::Action;
    use crate::apply_action;
    use crate::engine::tick;
    use crate::state::{
        GameOutcome, GameState, ResearchKind, ResearchProject, ResearchTrack,
    };

    #[test]
    fn research_identify_increases_knowledge() {
        let mut state = GameState::new_default(42);
        // Start identify project on disease 0
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #1
        state = apply_action(&state, &Action::Confirm); // Confirm start
        assert!(!state.field_research.is_empty());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (160 ticks at 1x speed)
        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.field_research.is_empty()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0; // Fully identified
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        assert!(!state.medicines[0].unlocked);

        // Start applied research: Develop Antiviral-A
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Applied Research
        state = apply_action(&state, &Action::Confirm);     // Enter Applied
        state = apply_action(&state, &Action::Confirm);     // Select Develop Antiviral-A
        state = apply_action(&state, &Action::Confirm);     // Confirm

        assert!(state.applied_research.is_some());

        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.applied_research.is_none());
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

        assert!(!state.field_research.is_empty());

        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.field_research.is_empty());
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
        assert!(state.field_research.is_empty());
    }

    #[test]
    fn more_personnel_means_faster_progress() {
        let mut state = GameState::new_default(42);

        // Create a project with base 5 personnel, assign 10 (2x base)
        // With diminishing returns: speed = 1 + (2-1)*(3-2)/2 = 1.5x
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 10, // 2x base (5) — peak of diminishing returns
        }];

        state = tick(&state);
        // At 2x ratio, diminishing returns gives 1.5x speed
        assert!(
            (state.field_research.first().unwrap().progress - 1.5).abs() < 0.01,
            "2x personnel should give 1.5x speed (diminishing returns), got {}",
            state.field_research.first().unwrap().progress
        );
    }

    #[test]
    fn diminishing_returns_beyond_double() {
        let mut state = GameState::new_default(42);

        // Assign 3x base personnel — should be back to 1.0x speed
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 15, // 3x base (5)
        }];

        state = tick(&state);
        assert!(
            (state.field_research.first().unwrap().progress - 1.0).abs() < 0.01,
            "3x personnel should give 1.0x speed (negative returns), got {}",
            state.field_research.first().unwrap().progress
        );
    }

    #[test]
    fn concurrent_field_and_applied_research() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 1000.0; // enough for both projects
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        // Start field research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #2
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(!state.field_research.is_empty());

        // Start applied research
        state = apply_action(&state, &Action::ClosePanel); // Back to categories
        state = apply_action(&state, &Action::SelectNext);  // Applied Research
        state = apply_action(&state, &Action::Confirm);     // Enter Applied
        state = apply_action(&state, &Action::Confirm);     // Select Develop
        state = apply_action(&state, &Action::Confirm);     // Confirm
        assert!(state.applied_research.is_some());

        // Both running simultaneously
        assert!(!state.field_research.is_empty());
        assert!(state.applied_research.is_some());
    }

    #[test]
    fn research_requires_funding() {
        let mut state = GameState::new_default(42);
        // Identify costs $350; set funding to $100 so it fails
        state.resources.funding = 100.0;

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify
        state = apply_action(&state, &Action::Confirm); // Try to confirm
        assert!(state.field_research.is_empty(), "should not start without funding");
        assert!(state.ui.status_message.as_ref().unwrap().contains("Insufficient funds"));

        // Give enough funding, should succeed
        state.resources.funding = 500.0;
        state = apply_action(&state, &Action::Confirm); // Try again
        assert!(!state.field_research.is_empty(), "should start with sufficient funding");
        assert!(state.resources.funding < 500.0, "funding should be deducted");
    }

    #[test]
    fn develop_medicine_sets_strain_generation() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 2;
        state.diseases[0].knowledge = 1.0;

        // Start and complete DevelopMedicine for medicine 0 (targets disease 0)
        state.applied_research = Some(ResearchProject {
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

        state.field_research = vec![ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        }];

        state = tick(&state);
        assert!(state.medicines[0].tested_against.contains(&0));
        assert!(
            state.medicines[0].strain_generations[0] >= 3,
            "clinical trial should update strain calibration"
        );
    }

    #[test]
    fn clinical_trial_enables_auto_deploy() {
        let mut state = GameState::new_default(42);
        state.medicines[0].unlocked = true;

        state.field_research = vec![ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        }];

        // auto_deploy should be false/absent before trial completes
        assert!(!state.auto_deploy.get(0).copied().unwrap_or(false),
            "auto_deploy should be off before trial");

        state = tick(&state);

        assert!(state.medicines[0].tested_against.contains(&0),
            "medicine should be tested after trial");
        assert!(state.auto_deploy.get(0).copied().unwrap_or(false),
            "auto_deploy should be enabled automatically after trial completes");
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
        let (narrow_pers, narrow_ticks, narrow_funding) = narrow.costs(&state.medicines);
        let (broad_pers, broad_ticks, broad_funding) = broad.costs(&state.medicines);
        assert!(narrow_pers <= broad_pers, "narrow should need fewer personnel");
        assert!(narrow_ticks < broad_ticks, "narrow should be faster");
        assert!(narrow_funding < broad_funding, "narrow should cost less funding");
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

        let applied = state.available_applied_projects();
        assert!(
            applied.iter().any(|k| matches!(k, ResearchKind::ManufactureDoses { medicine_idx: 0 })),
            "manufacture should be available for depleted medicine"
        );

        state.applied_research = Some(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
        });
        state = tick(&state);

        assert!(state.applied_research.is_none(), "project should be complete");
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
        assert!(!state.field_research.is_empty());

        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.field_research.is_empty());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = GameState::new_default(42);
        let initial_personnel = state.resources.personnel;

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Applied Research
        state = apply_action(&state, &Action::Confirm);     // Enter Applied
        let applied_projects = state.available_applied_projects();
        let train_idx = applied_projects.iter().position(|k| matches!(k,
            ResearchKind::TrainPersonnel
        )).expect("should have train personnel available");
        for _ in 0..train_idx {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // Select
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.applied_research.is_some());

        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.applied_research.is_none());
        assert_eq!(state.resources.personnel, initial_personnel + 5);
    }

    #[test]
    fn basic_research_unlocks_tech() {
        let mut state = GameState::new_default(42);
        // Prereq for TargetedDrugDesign: identify any pathogen
        state.diseases[0].knowledge = 0.5;
        state.resources.funding = 1000.0;
        assert!(state.unlocked_techs.is_empty());

        // Navigate: Research → Basic → first project → Confirm
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Applied
        state = apply_action(&state, &Action::SelectNext); // Basic
        state = apply_action(&state, &Action::Confirm);     // Enter Basic
        state = apply_action(&state, &Action::Confirm);     // Select TargetedDrugDesign
        state = apply_action(&state, &Action::Confirm);     // Confirm start
        assert!(state.basic_research.is_some(), "basic research should have started");

        // Advance to completion (240 ticks at 1x speed)
        for _ in 0..240 {
            state = tick(&state);
        }
        assert!(state.basic_research.is_none(), "project should be complete");
        assert!(
            state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign),
            "TargetedDrugDesign should be unlocked"
        );
    }

    #[test]
    fn three_concurrent_research_tracks() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 2000.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);
        state.resources.personnel = 30;

        // Start field research directly
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field
        state = apply_action(&state, &Action::Confirm); // Select first
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(!state.field_research.is_empty());

        // Start applied research
        state = apply_action(&state, &Action::ClosePanel);
        state = apply_action(&state, &Action::SelectNext); // Applied
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        assert!(state.applied_research.is_some());

        // Start basic research
        state = apply_action(&state, &Action::ClosePanel);
        state = apply_action(&state, &Action::SelectNext); // Applied
        state = apply_action(&state, &Action::SelectNext); // Basic
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        assert!(state.basic_research.is_some());

        // All three running simultaneously
        assert!(!state.field_research.is_empty());
        assert!(state.applied_research.is_some());
        assert!(state.basic_research.is_some());
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
        assert!(state.field_research.is_empty(), "should not start research after game over");
    }

    #[test]
    fn parallel_field_research_runs_and_completes_independently() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true;
        state.resources.funding = 3000.0;
        state.resources.personnel = 30;

        // Start first field project (Identify will target an unknown disease)
        state.field_research = vec![
            ResearchProject {
                kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
                progress: 0.0,
                required_ticks: 50.0,
                personnel_assigned: 5,
            },
            ResearchProject {
                kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
                progress: 0.0,
                required_ticks: 100.0,
                personnel_assigned: 5,
            },
        ];

        assert_eq!(state.field_research.len(), 2, "should have 2 parallel field projects");
        assert_eq!(state.personnel_busy(), 10, "10 personnel busy across 2 projects");

        // Advance until first project completes but second hasn't
        for _ in 0..55 {
            state = tick(&state);
        }
        assert_eq!(state.field_research.len(), 1, "first project should have completed");
        assert!(matches!(&state.field_research[0].kind, ResearchKind::ClinicalTrial { .. }),
            "remaining project should be the clinical trial");

        // Advance until second completes
        for _ in 0..50 {
            state = tick(&state);
        }
        assert!(state.field_research.is_empty(), "both projects should have completed");
    }

    #[test]
    fn field_research_capped_at_max() {
        use crate::state::MAX_FIELD_RESEARCH;
        let mut state = GameState::new_default(42);
        state.resources.personnel = 50;

        // Fill all field slots
        for i in 0..MAX_FIELD_RESEARCH {
            state.field_research.push(ResearchProject {
                kind: ResearchKind::IdentifyThreat { disease_idx: i },
                progress: 0.0,
                required_ticks: 160.0,
                personnel_assigned: 5,
            });
        }
        assert!(!state.field_research_has_capacity(), "should be at capacity");
        assert_eq!(state.field_research.len(), MAX_FIELD_RESEARCH);

        // Try to start another — should fail
        let (ok, _msg) = super::start_research(
            &mut state, ResearchTrack::Field, 0, false,
        );
        assert!(!ok, "should not start field research when at capacity");
        assert_eq!(state.field_research.len(), MAX_FIELD_RESEARCH);
    }

    #[test]
    fn rapid_sequencing_unlocks_after_sequencing() {
        let mut state = GameState::new_default(42);
        // No sequencing done yet — RapidSequencing should not be available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::RapidSequencing }
        )), "RapidSequencing should not be available without sequencing");

        // Complete one sequencing
        state.diseases[0].sequencing_count = 1;
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::RapidSequencing }
        )), "RapidSequencing should be available after sequencing");
    }

    #[test]
    fn rapid_sequencing_halves_genomic_sequencing_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::GenomicSequencing { disease_idx: 0 };

        let (_, base_dur, _) = state.effective_costs(&kind);
        assert_eq!(base_dur, 200.0, "base genomic sequencing should be 200 ticks");

        state.unlocked_techs.push(crate::state::BasicTech::RapidSequencing);
        let (_, rapid_dur, _) = state.effective_costs(&kind);
        assert_eq!(rapid_dur, 100.0, "with RapidSequencing, should be 100 ticks");
    }

    #[test]
    fn vaccine_platform_prereqs() {
        let mut state = GameState::new_default(42);
        // No advanced drug tech → not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::VaccinePlatform }
        )), "VaccinePlatform should not be available without mAb or Phage");

        // Unlock MonoclonalAntibodies → available
        state.unlocked_techs.push(crate::state::BasicTech::MonoclonalAntibodies);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::VaccinePlatform }
        )), "VaccinePlatform should be available after MonoclonalAntibodies");
    }

    #[test]
    fn vaccine_platform_triples_vaccination() {
        let mut state = GameState::new_default(42);
        assert_eq!(state.vaccination_multiplier(), 1.0);

        state.unlocked_techs.push(crate::state::BasicTech::VaccinePlatform);
        assert_eq!(state.vaccination_multiplier(), 3.0);

        // Verify estimate_vaccination uses the multiplier
        let med = &state.medicines[0];
        let base = med.estimate_vaccination(1_000_000.0, 1.0, 1.0);
        let boosted = med.estimate_vaccination(1_000_000.0, 1.0, 3.0);
        assert!(
            (boosted - base * 3.0).abs() < 1.0,
            "VaccinePlatform should triple vaccination: base={base}, boosted={boosted}"
        );
    }

    #[test]
    fn combination_therapy_prereqs() {
        let mut state = GameState::new_default(42);
        // No deployed medicines → not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::CombinationTherapy }
        )), "CombinationTherapy should not be available without 2+ deployed medicines");

        // Deploy only 1 medicine → still not available
        state.medicines[0].deployed_count = 1;
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::CombinationTherapy }
        )), "CombinationTherapy should not be available with only 1 deployed medicine");

        // Deploy a second medicine → available
        state.medicines[1].deployed_count = 1;
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::CombinationTherapy }
        )), "CombinationTherapy should be available after deploying 2+ different medicines");
    }

    #[test]
    fn combination_therapy_halves_resistance() {
        let mut state = GameState::new_default(42);
        assert_eq!(state.resistance_multiplier(), 1.0);

        state.unlocked_techs.push(crate::state::BasicTech::CombinationTherapy);
        assert_eq!(state.resistance_multiplier(), 0.5);
    }

    #[test]
    fn auto_research_starts_next_project_on_completion() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5000.0;
        state.resources.personnel = 30;

        // Enable auto-research for field track
        state.auto_research[ResearchTrack::Field.index()] = true;

        // Manually start an identify project that's almost done
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 149.0,
            required_ticks: 150.0,
            personnel_assigned: 5,
        }];

        // Tick to complete it
        for _ in 0..5 {
            state = tick(&state);
        }

        // The identify should have completed, and auto-research should have started the next project
        assert!(state.diseases[0].knowledge >= 0.49, "identification should have completed");
        assert!(
            !state.field_research.is_empty(),
            "auto-research should have started a new field project"
        );
    }

    #[test]
    fn auto_research_does_not_start_when_disabled() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5000.0;
        state.resources.personnel = 30;

        // Auto-research OFF (default)
        assert!(!state.auto_research[ResearchTrack::Field.index()]);

        // Manually start an identify project that's almost done
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 149.0,
            required_ticks: 150.0,
            personnel_assigned: 5,
        }];

        // Tick to complete it
        for _ in 0..5 {
            state = tick(&state);
        }

        // Identification complete but no auto-start
        assert!(state.diseases[0].knowledge >= 0.49);
        assert!(
            state.field_research.is_empty(),
            "no auto-start when auto-research is disabled"
        );
    }

    #[test]
    fn auto_research_identifies_new_unknown_pathogen() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5000.0;
        state.resources.personnel = 30;

        // Enable auto-research for field track
        state.auto_research[ResearchTrack::Field.index()] = true;

        // No field research running, disease 0 is unknown and detected
        assert!(state.field_research.is_empty());
        assert!(state.diseases[0].knowledge < 1.0);
        assert!(state.diseases[0].detected);

        // A single tick should auto-start identification
        state = tick(&state);
        assert!(
            !state.field_research.is_empty(),
            "auto-research should proactively start identification of unknown pathogen"
        );
        assert!(matches!(
            &state.field_research[0].kind,
            ResearchKind::IdentifyThreat { disease_idx: 0 }
        ));
    }

    #[test]
    fn auto_research_reserves_funds_for_other_tracks() {
        let mut state = GameState::new_default(42);
        state.resources.personnel = 30;

        // Set up: disease identified (knowledge 0.5), broad-spectrum available ($700)
        // No basic techs unlocked, so TargetedDrugDesign ($600) is also available
        state.diseases[0].knowledge = 0.5;

        // Enable auto-research for field track
        state.auto_research[ResearchTrack::Field.index()] = true;

        // Put a study project that's about to complete (costs $350)
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 159.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        }];

        // $900 funds. After study completes, the next field project would be
        // sequencing ($500). Reserve = min($700 broad-spectrum, $600 TargetedDrugDesign) = $600.
        // $900 - $500 = $400 < $600 → sequencing should be BLOCKED.
        state.resources.funding = 900.0;

        // Tick to complete the study
        for _ in 0..5 {
            state = tick(&state);
        }

        // Funds should be preserved (no expensive auto-start ate them)
        assert!(
            state.resources.funding >= 600.0,
            "auto-research should reserve at least $600 for progression, got ${:.0}",
            state.resources.funding
        );
    }

    #[test]
    fn genomic_sequencing_unavailable_after_effective_rate_drops() {
        use crate::state::PathogenType;

        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.diseases[0].pathogen_type = PathogenType::RnaVirus; // base rate 0.001
        // Ensure disease has infected population so sequencing can be considered
        state.regions[0].get_or_create_infection(0).infected = 1000.0;
        state.field_research.clear();

        // After 4 sequencings: 0.001 * 0.5^4 = 0.0000625 < 0.0001 threshold
        state.diseases[0].sequencing_count = 4;
        let field_projects = state.available_field_projects();
        assert!(
            !field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should not be available when effective rate ({}) is below threshold",
            state.diseases[0].effective_mutation_rate()
        );

        // After 3 sequencings: 0.001 * 0.5^3 = 0.000125 > 0.0001 — still available
        state.diseases[0].sequencing_count = 3;
        let field_projects = state.available_field_projects();
        assert!(
            field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should still be available when effective rate ({}) is above threshold",
            state.diseases[0].effective_mutation_rate()
        );
    }

    #[test]
    fn human_trials_halves_clinical_trial_duration() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);
        state.resources.funding = 10_000.0;
        state.resources.political_power = 1.0;
        // Develop a medicine first
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![];

        // Start a clinical trial WITHOUT human trials decree
        let projects = state.available_projects(ResearchTrack::Field);
        let trial_idx = projects.iter().position(|k| matches!(k, ResearchKind::ClinicalTrial { .. }));
        assert!(trial_idx.is_some(), "clinical trial should be available");
        let (ok, _) = super::start_research(&mut state, ResearchTrack::Field, trial_idx.unwrap(), false);
        assert!(ok);
        let normal_duration = state.field_research.last().unwrap().required_ticks;
        state.field_research.clear();

        // Now enact human trials and start the same trial
        state.enacted_decrees.authorize_human_trials = true;
        let projects = state.available_projects(ResearchTrack::Field);
        let trial_idx = projects.iter().position(|k| matches!(k, ResearchKind::ClinicalTrial { .. }));
        let (ok, _) = super::start_research(&mut state, ResearchTrack::Field, trial_idx.unwrap(), false);
        assert!(ok);
        let fast_duration = state.field_research.last().unwrap().required_ticks;

        // Duration should be halved
        assert!(
            (fast_duration - normal_duration * crate::state::HUMAN_TRIALS_SPEED).abs() < 1.0,
            "human trials should halve duration: normal={normal_duration}, fast={fast_duration}"
        );
    }

    #[test]
    fn pathogen_suppression_prereqs() {
        let mut state = GameState::new_default(42);

        // No prereqs — not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::PathogenSuppression }
        )), "PathogenSuppression should not be available without VaccinePlatform + CombinationTherapy");

        // Only VaccinePlatform — still not available
        state.unlocked_techs.push(crate::state::BasicTech::VaccinePlatform);
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::PathogenSuppression }
        )), "PathogenSuppression requires both techs, not just VaccinePlatform");

        // Both prereqs — available
        state.unlocked_techs.push(crate::state::BasicTech::CombinationTherapy);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::PathogenSuppression }
        )), "PathogenSuppression should be available with VaccinePlatform + CombinationTherapy");
    }

    #[test]
    fn suppress_pathogen_reduces_infectivity_20_percent() {
        use crate::state::KNOWLEDGE_FULL;
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = KNOWLEDGE_FULL;
        state.resources.funding = 5000.0;
        state.resources.personnel = 20;
        // Ensure disease is infecting somewhere
        state.regions[0].infections[0].infected = 1000.0;

        let original_infectivity = state.diseases[0].infectivity;

        // Run suppression to near-completion and tick it over
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::SuppressPathogen { disease_idx: 0 },
            progress: 599.0,
            required_ticks: 600.0,
            personnel_assigned: 8,
        }];

        // Tick to complete
        for _ in 0..5 {
            state = tick(&state);
        }

        assert!(state.field_research.is_empty(), "suppression project should have completed");
        let reduced = state.diseases[0].infectivity;
        let expected = original_infectivity * 0.80;
        assert!(
            (reduced - expected).abs() < 0.001,
            "infectivity should drop by 20%: original={original_infectivity:.4}, expected={expected:.4}, got={reduced:.4}"
        );
        assert_eq!(state.pathogens_suppressed, 1, "suppression counter should increment");
    }

    #[test]
    fn directed_attenuation_prereqs() {
        let mut state = GameState::new_default(42);

        // Without PathogenSuppression — not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::DirectedAttenuation }
        )), "DirectedAttenuation should not be available without PathogenSuppression");

        // With PathogenSuppression — available
        state.unlocked_techs.push(crate::state::BasicTech::PathogenSuppression);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::DirectedAttenuation }
        )), "DirectedAttenuation should be available with PathogenSuppression");
    }

    #[test]
    fn genomic_interdiction_prereqs() {
        let mut state = GameState::new_default(42);

        // Without DirectedAttenuation — not available
        state.unlocked_techs.push(crate::state::BasicTech::PathogenSuppression);
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::GenomicInterdiction }
        )), "GenomicInterdiction should not be available without DirectedAttenuation");

        // With DirectedAttenuation — available
        state.unlocked_techs.push(crate::state::BasicTech::DirectedAttenuation);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::GenomicInterdiction }
        )), "GenomicInterdiction should be available with DirectedAttenuation");
    }

    #[test]
    fn attenuate_pathogen_reduces_lethality_30_percent() {
        use crate::state::KNOWLEDGE_FULL;
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = KNOWLEDGE_FULL;
        state.resources.funding = 5000.0;
        state.resources.personnel = 20;
        state.regions[0].infections[0].infected = 1000.0;

        let original_lethality = state.diseases[0].lethality;

        state.field_research = vec![ResearchProject {
            kind: ResearchKind::AttenuatePathogen { disease_idx: 0 },
            progress: 599.0,
            required_ticks: 600.0,
            personnel_assigned: 8,
        }];

        for _ in 0..5 {
            state = tick(&state);
        }

        assert!(state.field_research.is_empty(), "attenuation project should have completed");
        let reduced = state.diseases[0].lethality;
        let expected = original_lethality * 0.70;
        assert!(
            (reduced - expected).abs() < 0.001,
            "lethality should drop by 30%: original={original_lethality:.4}, expected={expected:.4}, got={reduced:.4}"
        );
        assert_eq!(state.pathogens_attenuated, 1, "attenuation counter should increment");
    }

    #[test]
    fn interdict_pathogen_eliminates_cross_region_spread() {
        use crate::state::KNOWLEDGE_FULL;
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = KNOWLEDGE_FULL;
        state.resources.funding = 5000.0;
        state.resources.personnel = 20;
        state.regions[0].infections[0].infected = 1000.0;

        assert!(state.diseases[0].cross_region_spread > 0.0, "disease should have cross-region spread initially");

        state.field_research = vec![ResearchProject {
            kind: ResearchKind::InterdictPathogen { disease_idx: 0 },
            progress: 799.0,
            required_ticks: 800.0,
            personnel_assigned: 10,
        }];

        for _ in 0..5 {
            state = tick(&state);
        }

        assert!(state.field_research.is_empty(), "interdiction project should have completed");
        assert!(
            state.diseases[0].cross_region_spread == 0.0,
            "cross-region spread should be eliminated, got {}",
            state.diseases[0].cross_region_spread
        );
        assert_eq!(state.pathogens_interdicted, 1, "interdiction counter should increment");
    }

    #[test]
    fn lab_upgrade_increases_research_speed() {
        use crate::state::LAB_LEVEL_1_COST;

        let mut state = GameState::new_default(42);
        state.resources.funding = 1000.0;
        state.field_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5, // base personnel, 1.0x speed
        }];

        // Baseline: one tick at standard lab
        let base_state = tick(&state);
        let base_progress = base_state.field_research[0].progress;

        // Upgrade to level 1 (1.3x multiplier)
        state.lab_level = 1;
        let upgraded_state = tick(&state);
        let upgraded_progress = upgraded_state.field_research[0].progress;

        assert!(
            (upgraded_progress / base_progress - 1.3).abs() < 0.01,
            "Lab level 1 should give 1.3x speed, got {}x",
            upgraded_progress / base_progress
        );

        // Verify upgrade_lab deducts cost and increments level
        let mut s = GameState::new_default(42);
        s.resources.funding = 1000.0;
        let (ok, msg) = super::upgrade_lab(&mut s);
        assert!(ok);
        assert!(msg.is_some(), "upgrade should return a message");
        assert_eq!(s.lab_level, 1);
        assert!((s.resources.funding - (1000.0 - LAB_LEVEL_1_COST)).abs() < 0.01);
    }

    #[test]
    fn handoff_notification_after_identification() {
        use crate::state::GameEvent;
        let mut state = GameState::new_default(42);
        // Start identify on disease 0
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify
        state = apply_action(&state, &Action::Confirm); // Confirm start
        assert!(!state.field_research.is_empty());

        // Advance to completion, checking events each tick (clone-and-mutate means
        // events are only on the state returned by the tick that generated them)
        let mut found_handoff = false;
        for _ in 0..200 {
            state = tick(&state);
            if state.events.iter().any(|e|
                matches!(e, GameEvent::ResearchHandoff { message } if message.contains("development available"))
            ) {
                found_handoff = true;
            }
        }
        assert!(state.diseases[0].knowledge >= 0.5, "Disease should be identified");
        assert!(found_handoff, "Should notify about medicine development after identification");
    }

    #[test]
    fn handoff_notification_after_medicine_developed() {
        use crate::state::GameEvent;
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        // Start develop medicine on applied track
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Applied
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.applied_research.is_some(),
            "Applied research should start. UI state: {:?}", state.ui.research_ui);

        // Advance to completion, checking events each tick
        let mut found_handoff = false;
        for _ in 0..600 {
            state = tick(&state);
            if state.events.iter().any(|e|
                matches!(e, GameEvent::ResearchHandoff { message } if message.contains("clinical trial"))
            ) {
                found_handoff = true;
            }
        }
        assert!(state.medicines.iter().any(|m| m.unlocked), "Medicine should be developed");
        assert!(found_handoff, "Should notify about clinical trial after medicine development");
    }
}
