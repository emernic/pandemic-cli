use crate::state::{
    FIELD_OPS_RESTORE, GameEvent, GameOutcome, GameState, InfraSystem, ResearchCategory,
    ResearchKind, ResearchProject, KNOWLEDGE_FULL, KNOWLEDGE_NAME,
    TRAIN_PERSONNEL_BATCH,
    LAB_LEVEL_1_COST, LAB_LEVEL_2_COST,
};

/// Start a research project. Pure game logic — does NOT modify UI state.
/// `project_idx` indexes into `state.all_available_projects()`.
///
/// Returns (success, message).
pub(super) fn start_research(state: &mut GameState, project_idx: usize, double_personnel: bool) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }

    let projects = state.all_available_projects();

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
        // Clinical trial speed modifier: Human Trials decree
        let effective_duration = if matches!(kind, ResearchKind::ClinicalTrial { .. }) {
            let mut d = duration;
            if state.enacted_decrees.authorize_human_trials {
                d *= crate::state::HUMAN_TRIALS_SPEED;
            }
            d
        } else {
            duration
        };
        let project = ResearchProject {
            kind: kind.clone(),
            progress: 0.0,
            required_ticks: effective_duration,
            personnel_assigned: personnel,
        };

        state.active_research.push(project);
        return (true, None);
    }
    (false, None)
}


/// Advance research projects by one tick and handle completions.
/// Progress scales with diminishing returns: 2x personnel = 1.5x speed (peak),
/// beyond 2x personnel = negative returns (too many cooks).
/// Returns the number of research completions that should trigger board notifications
/// (DevelopMedicine and BasicResearch completions boost Technocrat satisfaction).
pub(super) fn tick_research(state: &mut GameState, rng: &mut impl rand::Rng) -> u32 {
    // Proactively auto-repeat on idle categories
    try_auto_repeat(state);

    let mut board_notify_count: u32 = 0;

    let lab_mult = state.lab_speed_multiplier();
    // Biotech sector bonus: best regional bonus boosts research speed
    let biotech_bonus = (0..state.regions.len())
        .map(|r| state.sector_bonus(r, crate::state::CorporationSector::Biotech))
        .fold(0.0_f64, f64::max);
    let biotech_mult = 1.0 + crate::state::CorporationSector::Biotech.max_bonus_pct() / 100.0 * biotech_bonus;

    // Advance all research projects
    for project in &mut state.active_research {
        let speed = project.speed(&state.medicines);
        project.progress += speed * lab_mult * biotech_mult;
    }

    // Collect completed projects (drain_filter pattern via retain)
    let mut completed: Vec<ResearchProject> = Vec::new();
    state.active_research.retain(|p| {
        if p.is_complete() {
            completed.push(p.clone());
            false
        } else {
            true
        }
    });
    // Track which categories had completions for post-processing
    let had_field_completion = completed.iter().any(|p| p.kind.category() == ResearchCategory::Field);

    for project in &completed {
        match &project.kind {
            ResearchKind::IdentifyThreat { disease_idx } => {
                let d_idx = *disease_idx;
                let was_unknown = state.diseases.get(d_idx)
                    .is_some_and(|d| d.knowledge < KNOWLEDGE_NAME);
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.knowledge = (disease.knowledge + 0.50).min(KNOWLEDGE_FULL);
                }
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
                    if !medicine.target_diseases.contains(&d_idx) {
                        medicine.target_diseases.push(d_idx);
                    }
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
                while state.auto_deploy.len() <= m_idx {
                    state.auto_deploy.push(false);
                }
                state.auto_deploy[m_idx] = true;
                if state.enacted_decrees.authorize_human_trials {
                    let roll: f64 = rng.r#gen();
                    if roll < crate::state::HUMAN_TRIALS_ADVERSE_CHANCE {
                        let kill_frac = crate::state::HUMAN_TRIALS_KILL_FRACTION;
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
                    disease.within_region_spread *= 0.80;
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
            ResearchKind::FieldOperations { region_idx, system } => {
                let r_idx = *region_idx;
                let sys = *system;
                if let Some(region) = state.regions.get_mut(r_idx) {
                    if !region.collapsed {
                        let target = match sys {
                            InfraSystem::Healthcare => &mut region.healthcare_capacity,
                            InfraSystem::SupplyLines => &mut region.supply_lines,
                            InfraSystem::CivilOrder => &mut region.civil_order,
                        };
                        *target = (*target + FIELD_OPS_RESTORE).min(1.0);
                        state.events.push(GameEvent::InfrastructureStabilized {
                            region_idx: r_idx,
                            system: sys,
                        });
                    }
                }
            }
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let m_idx = *medicine_idx;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.unlocked = true;
                    medicine.strain_generations = medicine.target_diseases.iter()
                        .map(|&d_idx| state.diseases.get(d_idx)
                            .map_or(0, |d| d.strain_generation as i32))
                        .collect();
                }
                state.events.push(GameEvent::MedicineDeveloped { medicine_idx: m_idx });
                board_notify_count += 1;

                if let Some(corp_idx) = state.medicines.get(m_idx).and_then(|m| m.manufacturer_corp_idx) {
                    if let Some(corp) = state.corporations.get_mut(corp_idx) {
                        if corp.board_seat && !corp.bankrupt {
                            let boost = corp.max_reserves * 0.25;
                            corp.reserves = (corp.reserves + boost).min(corp.max_reserves);
                        }
                    }
                }

                let has_trial_available = state.all_available_projects().iter()
                    .any(|p| matches!(p, ResearchKind::ClinicalTrial { medicine_idx: mi, .. } if *mi == m_idx));
                if has_trial_available {
                    let name = state.medicines.get(m_idx)
                        .map(|m| m.name.as_str()).unwrap_or("medicine");
                    state.events.push(GameEvent::ResearchHandoff {
                        message: format!("{} needs clinical trial — open Field Research [R]", name),
                    });
                }
            }
            ResearchKind::ManufactureDoses { medicine_idx } => {
                let m_idx = *medicine_idx;
                let mfg_bonus = state.manufacturing_yield_bonus();
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.doses = medicine.max_doses * mfg_bonus;
                }
            }
            ResearchKind::TrainPersonnel => {
                state.resources.personnel += TRAIN_PERSONNEL_BATCH;
            }
            ResearchKind::BasicResearch { tech } => {
                let tech = *tech;
                if !state.unlocked_techs.contains(&tech) {
                    state.unlocked_techs.push(tech);
                    state.events.push(GameEvent::TechUnlocked { tech });
                    board_notify_count += 1;
                }
            }
        }
    }

    // Notify player if field completions unlocked Applied research options
    if had_field_completion {
        if let Some(kind) = state.all_available_projects().iter()
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

    // Auto-repeat completed repeatable projects
    for project in &completed {
        if state.auto_repeat_research.contains(&project.kind) {
            let projects = state.all_available_projects();
            if let Some(idx) = projects.iter().position(|k| k == &project.kind) {
                let (ok, _) = start_research(state, idx, false);
                if ok {
                    state.events.push(GameEvent::ResearchAutoRestarted { kind: project.kind.clone() });
                }
            }
        }
    }

    board_notify_count
}

/// Try to auto-repeat any repeatable research that has auto-repeat enabled.
/// Called at the start of each tick.
fn try_auto_repeat(state: &mut GameState) {
    let kinds_to_repeat: Vec<ResearchKind> = state.auto_repeat_research.clone();
    for kind in &kinds_to_repeat {
        let projects = state.all_available_projects();
        if let Some(idx) = projects.iter().position(|k| k == kind) {
            let (_, _, cost) = state.effective_costs(&projects[idx]);
            if state.resources.funding < cost {
                continue;
            }
            let (ok, _) = start_research(state, idx, false);
            if ok {
                state.events.push(GameEvent::ResearchAutoRestarted { kind: kind.clone() });
            }
        }
    }
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
        GameOutcome, GameState, ResearchCategory, ResearchFlatItem, ResearchKind, ResearchProject,
    };

    /// Helper: open research panel, navigate to first available item matching `kind_pred`, and confirm through.
    fn start_research_matching(state: &GameState, kind_pred: impl Fn(&ResearchKind) -> bool) -> GameState {
        // Ensure panel is closed first, then open fresh
        let mut s = if state.ui.open_panel == crate::state::Panel::Research {
            apply_action(state, &Action::ClosePanel)
        } else {
            state.clone()
        };
        s = apply_action(&s, &Action::OpenResearch);
        let items = s.research_flat_items();
        let available = s.all_available_projects();
        let idx = items.iter().position(|item| {
            if let ResearchFlatItem::Available(proj_idx) = item {
                available.get(*proj_idx).map_or(false, &kind_pred)
            } else {
                false
            }
        }).expect("expected matching research item in flat list");
        s.ui.panel_selection = idx;
        s = apply_action(&s, &Action::Confirm); // ConfirmProject
        s = apply_action(&s, &Action::Confirm); // Start
        s
    }

    #[test]
    fn research_identify_increases_knowledge() {
        let mut state = GameState::new_default(42);
        // Start identify project on disease 0 (first item in flat list)
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Start
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (160 ticks at 1x speed)
        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Field).is_empty()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0; // Fully identified
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        assert!(!state.medicines[0].unlocked);

        // Start applied research: Develop Antiviral-A
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Applied && !matches!(k, ResearchKind::TrainPersonnel));

        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some());

        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Applied).is_empty());
        assert!(state.medicines[0].unlocked);
    }

    #[test]
    fn research_clinical_trial_marks_tested() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true; // Pre-unlock for testing

        assert!(state.medicines[0].tested_against.is_empty());

        // Start field research: Clinical Trial
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::ClinicalTrial { .. }));

        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());

        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Field).is_empty());
        assert!(state.medicines[0].tested_against.contains(&0));
    }

    #[test]
    fn research_insufficient_personnel_blocks_start() {
        let mut state = GameState::new_default(42);
        state.resources.personnel = 0; // No personnel

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start

        // Should not have started
        assert!(state.active_in_category(ResearchCategory::Field).is_empty());
    }

    #[test]
    fn more_personnel_means_faster_progress() {
        let mut state = GameState::new_default(42);

        // Create a project with base 5 personnel, assign 10 (2x base)
        // With diminishing returns: speed = 1 + (2-1)*(3-2)/2 = 1.5x
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 10, // 2x base (5) — peak of diminishing returns
        }];

        state = tick(&state);
        // At 2x ratio, diminishing returns gives 1.5x speed
        let expected = 1.5;
        assert!(
            (state.active_in_category(ResearchCategory::Field).first().unwrap().progress - expected).abs() < 0.01,
            "2x personnel should give 1.5x speed, got {}",
            state.active_in_category(ResearchCategory::Field).first().unwrap().progress
        );
    }

    #[test]
    fn diminishing_returns_beyond_double() {
        let mut state = GameState::new_default(42);

        // Assign 3x base personnel — should be back to 1.0x speed
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 15, // 3x base (5)
        }];

        state = tick(&state);
        let expected = 1.0;
        assert!(
            (state.active_in_category(ResearchCategory::Field).first().unwrap().progress - expected).abs() < 0.01,
            "3x personnel should give 1.0x speed, got {}",
            state.active_in_category(ResearchCategory::Field).first().unwrap().progress
        );
    }

    #[test]
    fn concurrent_field_and_applied_research() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 1000.0; // enough for both projects
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        // Start field research (first item in flat list)
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Field);
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());

        // Start applied research
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Applied && !matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some());

        // Both running simultaneously
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());
        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some());
    }

    #[test]
    fn research_requires_funding() {
        let mut state = GameState::new_default(42);
        // Identify costs $350; set funding to $100 so it fails
        state.resources.funding = 100.0;

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start
        assert!(state.active_in_category(ResearchCategory::Field).is_empty(), "should not start without funding");
        assert!(state.ui.status_message.as_ref().unwrap().contains("Insufficient funds"));

        // Give enough funding, should succeed (still on ConfirmProject screen)
        state.resources.funding = 500.0;
        state = apply_action(&state, &Action::Confirm); // Try again
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty(), "should start with sufficient funding");
        assert!(state.resources.funding < 500.0, "funding should be deducted");
    }

    #[test]
    fn develop_medicine_sets_strain_generation() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 2;
        state.diseases[0].knowledge = 1.0;

        // Start and complete DevelopMedicine for medicine 0 (targets disease 0)
        state.active_research.push(ResearchProject {
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

        state.active_research = vec![ResearchProject {
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

        state.active_research = vec![ResearchProject {
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
            &mut state.rng_emergence.clone(), crate::state::PathogenType::Bacterium, &[], true,
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

        state.active_research.push(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
        });
        state = tick(&state);

        assert!(state.active_in_category(ResearchCategory::Applied).is_empty(), "project should be complete");
        let expected_doses = state.medicines[0].max_doses * state.manufacturing_yield_bonus();
        assert_eq!(
            state.medicines[0].doses, expected_doses,
            "doses should be restored to max * manufacturing bonus"
        );
    }

    #[test]
    fn genomic_sequencing_reduces_mutation_rate() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        let original_rate = state.diseases[0].pathogen_type.mutation_rate();

        state = start_research_matching(&state, |k| matches!(k, ResearchKind::GenomicSequencing { .. }));
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());

        for _ in 0..200 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Field).is_empty());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = GameState::new_default(42);
        let initial_personnel = state.resources.personnel;

        state = start_research_matching(&state, |k| matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some());

        for _ in 0..160 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Applied).is_empty());
        assert_eq!(state.resources.personnel, initial_personnel + 5);
    }

    #[test]
    fn basic_research_unlocks_tech() {
        let mut state = GameState::new_default(42);
        // Prereq for TargetedDrugDesign: identify any pathogen
        state.diseases[0].knowledge = 0.5;
        state.resources.funding = 1000.0;
        assert!(state.unlocked_techs.is_empty());

        // Navigate: Research → find Basic → Confirm → Confirm
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Basic);
        assert!(state.active_in_category(ResearchCategory::Basic).first().is_some(), "basic research should have started");

        // Advance to completion (240 ticks at 1x speed)
        for _ in 0..240 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Basic).is_empty(), "project should be complete");
        assert!(
            state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign),
            "TargetedDrugDesign should be unlocked"
        );
    }

    #[test]
    fn three_concurrent_research_projects() {

        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 2000.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);
        state.resources.personnel = 30;

        // Use the shared helper that handles panel toggle correctly
        // (it closes the panel first if already open)

        // Start field research
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Field);
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());

        // Start applied research
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Applied && !matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some());

        // Start basic research
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Basic);
        assert!(state.active_in_category(ResearchCategory::Basic).first().is_some());

        // All three running simultaneously
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());
        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some());
        assert!(state.active_in_category(ResearchCategory::Basic).first().is_some());
    }

    #[test]
    fn no_research_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        // Try to start research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start
        assert!(state.active_in_category(ResearchCategory::Field).is_empty(), "should not start research after game over");
    }

    #[test]
    fn parallel_field_research_runs_and_completes_independently() {
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true;
        state.resources.funding = 3000.0;
        state.resources.personnel = 30;

        // Start first field project (Identify will target an unknown disease)
        state.active_research = vec![
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

        assert_eq!(state.active_in_category(ResearchCategory::Field).len(), 2, "should have 2 parallel field projects");
        assert_eq!(state.personnel_busy(), 10, "10 personnel busy across 2 projects");

        // Advance until first project completes but second hasn't
        for _ in 0..55 {
            state = tick(&state);
        }
        assert_eq!(state.active_in_category(ResearchCategory::Field).len(), 1, "first project should have completed");
        assert!(matches!(&state.active_in_category(ResearchCategory::Field)[0].kind, ResearchKind::ClinicalTrial { .. }),
            "remaining project should be the clinical trial");

        // Advance until second completes
        for _ in 0..50 {
            state = tick(&state);
        }
        assert!(state.active_in_category(ResearchCategory::Field).is_empty(), "both projects should have completed");
    }

    #[test]
    fn research_only_gated_by_personnel_and_funding() {
        let mut state = GameState::new_default(42);
        state.resources.personnel = 50;
        state.resources.funding = 5000.0;

        // Fill 3 field projects (the old MAX_FIELD_RESEARCH limit)
        state.active_research.push(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });
        state.active_research.push(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 1 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });
        state.active_research.push(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 2 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });
        assert_eq!(state.active_in_category(ResearchCategory::Field).len(), 3);

        // With no capacity limits, a 4th project should start if we have resources
        let available = state.all_available_projects();
        assert!(!available.is_empty(), "should still have available projects with 3 active");
        let (ok, _msg) = super::start_research(&mut state, 0, false);
        assert!(ok, "should start a 4th project — no capacity limit, only personnel/funding");
        assert!(state.active_research.len() >= 4, "should have 4+ active projects");
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
    fn genomic_sequencing_unavailable_after_effective_rate_drops() {
        use crate::state::PathogenType;

        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.diseases[0].pathogen_type = PathogenType::RnaVirus; // base rate 0.0004
        // Ensure disease has infected population so sequencing can be considered
        state.regions[0].get_or_create_infection(0).infected = 1000.0;
        state.active_research.clear();

        // After 4 sequencings: 0.0004 * 0.5^4 = 0.000025 < 0.00005 threshold
        state.diseases[0].sequencing_count = 4;
        let field_projects = state.available_field_projects();
        assert!(
            !field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should not be available when effective rate ({}) is below threshold",
            state.diseases[0].effective_mutation_rate()
        );

        // After 2 sequencings: 0.0004 * 0.5^2 = 0.0001 > 0.00005 — still available
        state.diseases[0].sequencing_count = 2;
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
        state.resources.authority = crate::state::Authority::Maximum;
        // Develop a medicine first
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![];

        // Start a clinical trial WITHOUT human trials decree
        let projects = state.all_available_projects();
        let trial_idx = projects.iter().position(|k| matches!(k, ResearchKind::ClinicalTrial { .. }));
        assert!(trial_idx.is_some(), "clinical trial should be available");
        let (ok, _) = super::start_research(&mut state, trial_idx.unwrap(), false);
        assert!(ok);
        let normal_duration = state.active_in_category(ResearchCategory::Field).last().unwrap().required_ticks;
        state.active_research.retain(|p| p.kind.category() != ResearchCategory::Field);

        // Now enact human trials and start the same trial
        state.enacted_decrees.authorize_human_trials = true;
        let projects = state.all_available_projects();
        let trial_idx = projects.iter().position(|k| matches!(k, ResearchKind::ClinicalTrial { .. }));
        let (ok, _) = super::start_research(&mut state, trial_idx.unwrap(), false);
        assert!(ok);
        let fast_duration = state.active_in_category(ResearchCategory::Field).last().unwrap().required_ticks;

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
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::CompetitiveDisplacement }
        )), "CompetitiveDisplacement should not be available without VaccinePlatform + CombinationTherapy");

        // Only VaccinePlatform — still not available
        state.unlocked_techs.push(crate::state::BasicTech::VaccinePlatform);
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::CompetitiveDisplacement }
        )), "CompetitiveDisplacement requires both techs, not just VaccinePlatform");

        // Both prereqs — available
        state.unlocked_techs.push(crate::state::BasicTech::CombinationTherapy);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::CompetitiveDisplacement }
        )), "CompetitiveDisplacement should be available with VaccinePlatform + CombinationTherapy");
    }

    #[test]
    fn suppress_pathogen_reduces_within_region_spread_20_percent() {
        use crate::state::KNOWLEDGE_FULL;
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = KNOWLEDGE_FULL;
        state.resources.funding = 5000.0;
        state.resources.personnel = 20;
        // Ensure disease is infecting somewhere
        state.regions[0].get_or_create_infection(0).infected = 1000.0;

        let original_spread = state.diseases[0].within_region_spread;

        // Run suppression to near-completion and tick it over
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::SuppressPathogen { disease_idx: 0 },
            progress: 599.0,
            required_ticks: 600.0,
            personnel_assigned: 8,
        }];

        // Tick to complete
        for _ in 0..5 {
            state = tick(&state);
        }

        assert!(state.active_in_category(ResearchCategory::Field).is_empty(), "suppression project should have completed");
        let reduced = state.diseases[0].within_region_spread;
        let expected = original_spread * 0.80;
        assert!(
            (reduced - expected).abs() < 0.001,
            "within-region spread should drop by 20%: original={original_spread:.4}, expected={expected:.4}, got={reduced:.4}"
        );
        assert_eq!(state.pathogens_suppressed, 1, "suppression counter should increment");
    }

    #[test]
    fn directed_attenuation_prereqs() {
        let mut state = GameState::new_default(42);

        // Without CompetitiveDisplacement — not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::DirectedAttenuation }
        )), "DirectedAttenuation should not be available without CompetitiveDisplacement");

        // With CompetitiveDisplacement — available
        state.unlocked_techs.push(crate::state::BasicTech::CompetitiveDisplacement);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::DirectedAttenuation }
        )), "DirectedAttenuation should be available with CompetitiveDisplacement");
    }

    #[test]
    fn genomic_interdiction_prereqs() {
        let mut state = GameState::new_default(42);

        // Without DirectedAttenuation — not available
        state.unlocked_techs.push(crate::state::BasicTech::CompetitiveDisplacement);
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::GeneDriveContainment }
        )), "GeneDriveContainment should not be available without DirectedAttenuation");

        // With DirectedAttenuation — available
        state.unlocked_techs.push(crate::state::BasicTech::DirectedAttenuation);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::GeneDriveContainment }
        )), "GeneDriveContainment should be available with DirectedAttenuation");
    }

    #[test]
    fn attenuate_pathogen_reduces_lethality_30_percent() {
        use crate::state::KNOWLEDGE_FULL;
        let mut state = GameState::new_default(42);
        state.diseases[0].knowledge = KNOWLEDGE_FULL;
        state.resources.funding = 5000.0;
        state.resources.personnel = 20;
        state.regions[0].get_or_create_infection(0).infected = 1000.0;

        let original_lethality = state.diseases[0].lethality;

        state.active_research = vec![ResearchProject {
            kind: ResearchKind::AttenuatePathogen { disease_idx: 0 },
            progress: 599.0,
            required_ticks: 600.0,
            personnel_assigned: 8,
        }];

        for _ in 0..5 {
            state = tick(&state);
        }

        assert!(state.active_in_category(ResearchCategory::Field).is_empty(), "attenuation project should have completed");
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
        state.regions[0].get_or_create_infection(0).infected = 1000.0;

        assert!(state.diseases[0].cross_region_spread > 0.0, "disease should have cross-region spread initially");

        state.active_research = vec![ResearchProject {
            kind: ResearchKind::InterdictPathogen { disease_idx: 0 },
            progress: 799.0,
            required_ticks: 800.0,
            personnel_assigned: 10,
        }];

        for _ in 0..5 {
            state = tick(&state);
        }

        assert!(state.active_in_category(ResearchCategory::Field).is_empty(), "interdiction project should have completed");
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
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5, // base personnel, 1.0x speed
        }];

        // Baseline: one tick at standard lab
        let base_state = tick(&state);
        let base_progress = base_state.active_in_category(ResearchCategory::Field)[0].progress;

        // Upgrade to level 1 (1.3x multiplier)
        state.lab_level = 1;
        let upgraded_state = tick(&state);
        let upgraded_progress = upgraded_state.active_in_category(ResearchCategory::Field)[0].progress;

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
        // Reset broad-spectrum to locked so identification can trigger the handoff
        for med in &mut state.medicines {
            if med.therapy_type == crate::state::TherapyType::BroadSpectrum {
                med.unlocked = false;
                med.doses = 0.0;
            }
        }
        // Start identify on disease 0 (first item in flat list)
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Start
        assert!(!state.active_in_category(ResearchCategory::Field).is_empty());

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

        // Start develop medicine (applied research)
        state = start_research_matching(&state, |k| k.category() == ResearchCategory::Applied && !matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_in_category(ResearchCategory::Applied).first().is_some(),
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

    #[test]
    fn manufacturing_yield_bonus_from_tech() {
        // StabilizedFormulation tech gives +25% manufacturing yield.
        let mut state = GameState::new_default(42);
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
        let max_doses = state.medicines[0].max_doses;
        state.medicines[0].doses = 0.0;

        // Without tech: get exactly max_doses
        state.active_research.push(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
        });
        let after = tick(&state);
        assert_eq!(
            after.medicines[0].doses,
            max_doses,
            "without tech, doses should equal max_doses"
        );

        // With StabilizedFormulation: get 125% of max_doses
        state.unlocked_techs.push(crate::state::BasicTech::StabilizedFormulation);
        state.medicines[0].doses = 0.0;
        state.active_research.retain(|p| p.kind.category() != ResearchCategory::Applied);
        state.active_research.push(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
        });
        let after_tech = tick(&state);
        assert_eq!(
            after_tech.medicines[0].doses,
            max_doses * 1.25,
            "StabilizedFormulation should give 125% of max doses"
        );
    }

    #[test]
    fn blocked_medicine_developments_shows_identified_but_unresearched() {
        use crate::state::BasicTech;

        let mut state = GameState::new_default(42);

        // Nothing identified — blocked list should be empty
        assert!(
            state.blocked_medicine_developments().is_empty(),
            "no blocked entries before identification"
        );

        // Partially identify disease 0 (knowledge > 0 but < 1.0, no TargetedDrugDesign)
        state.diseases[0].knowledge = 0.6;
        let blocked = state.blocked_medicine_developments();
        assert!(
            !blocked.is_empty(),
            "should show blocked entries once a disease is partially identified"
        );
        assert!(
            blocked.iter().all(|(d_idx, _)| *d_idx == 0),
            "blocked entry should reference disease 0"
        );
        assert!(
            blocked.iter().any(|(_, reason)| reason.contains("Targeted Drug Design")),
            "reason should mention Targeted Drug Design when tech is not unlocked"
        );

        // Unlock TargetedDrugDesign but disease still only 60% studied
        state.unlocked_techs.push(BasicTech::TargetedDrugDesign);
        let blocked_with_tech = state.blocked_medicine_developments();
        assert!(
            !blocked_with_tech.is_empty(),
            "should still show blocked when knowledge < 1.0"
        );
        assert!(
            blocked_with_tech.iter().any(|(_, reason)| reason.contains("Field Research")),
            "reason should reference Field Research when study is incomplete"
        );

        // Fully identify disease 0 (knowledge 1.0) — now it should be available, not blocked
        state.diseases[0].knowledge = 1.0;
        let blocked_full = state.blocked_medicine_developments();
        assert!(
            blocked_full.iter().all(|(d_idx, _)| *d_idx != 0),
            "disease 0 should not be blocked once fully identified with tech"
        );
    }

    #[test]
    fn blocked_medicine_developments_not_duplicated_when_already_available() {
        use crate::state::BasicTech;

        let mut state = GameState::new_default(42);

        // Disease 0 fully identified with tech: targeted medicine should be in available, not blocked.
        state.diseases[0].knowledge = 1.0;
        state.unlocked_techs.push(BasicTech::TargetedDrugDesign);

        let available = state.available_applied_projects();
        let disease0_available = available.iter().any(|k| {
            if let crate::state::ResearchKind::DevelopMedicine { medicine_idx } = k {
                state.medicines[*medicine_idx].target_diseases.contains(&0)
                    && state.medicines[*medicine_idx].therapy_type != crate::state::TherapyType::BroadSpectrum
            } else {
                false
            }
        });
        assert!(disease0_available, "disease 0 targeted medicine should be available");

        let blocked = state.blocked_medicine_developments();
        assert!(
            blocked.iter().all(|(d_idx, _)| *d_idx != 0),
            "disease 0 should not appear in blocked when already available"
        );
    }
}
