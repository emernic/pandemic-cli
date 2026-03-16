use crate::state::{
    AUTO_MANUFACTURE_THRESHOLD, BasicTech, GameEvent, GameOutcome, WorldState,
    ResearchKind, ResearchProject, KNOWLEDGE_FULL, KNOWLEDGE_NAME,
    TRAIN_PERSONNEL_BATCH,
    LAB_LEVEL_1_COST, LAB_LEVEL_2_COST,
};

/// Start a research project. Pure game logic — does NOT modify UI state.
/// `project_idx` indexes into `state.all_available_projects()`.
///
/// Returns (success, message).
pub(super) fn start_research(state: &mut WorldState, project_idx: usize, double_personnel: bool) -> (bool, Option<String>) {
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
pub(super) fn tick_research(state: &mut WorldState, rng: &mut impl rand::Rng, events: &mut Vec<GameEvent>) -> u32 {
    // Proactively auto-repeat on idle categories
    try_auto_repeat(state, events);
    // Auto-start cued techs when prerequisites and resources become available
    try_cued_starts(state, events);

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
    let had_field_completion = completed.iter().any(|p| p.kind.is_field_work());

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
                    events.push(GameEvent::PathogenIdentified { disease_idx: d_idx });
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
                }
                events.push(GameEvent::TrialCompleted {
                    medicine_idx: m_idx,
                    disease_idx: d_idx,
                });
                while state.deploy_enabled.len() <= m_idx {
                    state.deploy_enabled.push(false);
                }
                state.deploy_enabled[m_idx] = true;
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
                            events.push(GameEvent::HumanTrialAdverseEvent {
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
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let m_idx = *medicine_idx;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.unlocked = true;
                }
                events.push(GameEvent::MedicineDeveloped { medicine_idx: m_idx });
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
                    events.push(GameEvent::ResearchHandoff {
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
                    events.push(GameEvent::TechUnlocked { tech });
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
                events.push(GameEvent::ResearchHandoff {
                    message: format!("{} development available — open Applied Research [R]", name),
                });
            }
        }
    }

    // Auto-repeat completed repeatable projects
    for project in &completed {
        if state.auto_repeat_research.contains(&project.kind) {
            // Manufacturing only auto-repeats when doses drop to threshold
            if let ResearchKind::ManufactureDoses { medicine_idx } = &project.kind {
                let dose_frac = state.medicines.get(*medicine_idx)
                    .map(|m| if m.max_doses > 0.0 { m.doses / m.max_doses } else { 1.0 })
                    .unwrap_or(1.0);
                if dose_frac > AUTO_MANUFACTURE_THRESHOLD {
                    continue;
                }
            }
            let projects = state.all_available_projects();
            if let Some(idx) = projects.iter().position(|k| k == &project.kind) {
                let (ok, _) = start_research(state, idx, false);
                if ok {
                    events.push(GameEvent::ResearchAutoRestarted { kind: project.kind.clone() });
                }
            }
        }
    }

    board_notify_count
}

/// Try to auto-repeat any repeatable research that has auto-repeat enabled.
/// Called at the start of each tick.
fn try_auto_repeat(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let kinds_to_repeat: Vec<ResearchKind> = state.auto_repeat_research.clone();
    for kind in &kinds_to_repeat {
        // Manufacturing only auto-repeats when doses drop to threshold
        if let ResearchKind::ManufactureDoses { medicine_idx } = kind {
            let dose_frac = state.medicines.get(*medicine_idx)
                .map(|m| if m.max_doses > 0.0 { m.doses / m.max_doses } else { 1.0 })
                .unwrap_or(1.0);
            if dose_frac > AUTO_MANUFACTURE_THRESHOLD {
                continue;
            }
        }
        let projects = state.all_available_projects();
        if let Some(idx) = projects.iter().position(|k| k == kind) {
            let (_, _, cost) = state.effective_costs(&projects[idx]);
            if state.resources.funding < cost {
                continue;
            }
            let (ok, _) = start_research(state, idx, false);
            if ok {
                events.push(GameEvent::ResearchAutoRestarted { kind: kind.clone() });
            }
        }
    }
}

/// Try to auto-start cued techs whose prerequisites and resources are now available.
/// Removes techs from the cue once started (or if already unlocked/researching).
fn try_cued_starts(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let cued: Vec<BasicTech> = state.cued_techs.clone();
    for tech in &cued {
        // Already unlocked or researching — silently remove from cue
        if state.unlocked_techs.contains(tech) {
            state.cued_techs.retain(|t| t != tech);
            continue;
        }
        let already_researching = state.active_research.iter().any(|r| {
            matches!(r.kind, ResearchKind::BasicResearch { tech: t } if t == *tech)
        });
        if already_researching {
            state.cued_techs.retain(|t| t != tech);
            continue;
        }

        // Check prerequisites
        if !tech.prerequisites_met(state) {
            continue;
        }

        // Try to start
        let projects = state.all_available_projects();
        let target_kind = ResearchKind::BasicResearch { tech: *tech };
        if let Some(idx) = projects.iter().position(|k| *k == target_kind) {
            let (ok, _) = start_research(state, idx, false);
            if ok {
                state.cued_techs.retain(|t| t != tech);
                events.push(GameEvent::CuedResearchStarted { tech: *tech });
            }
        }
    }
}

/// Upgrade the global research lab (level 0→1 or 1→2). One-time funding cost.
/// Returns (success, message).
pub(super) fn upgrade_lab(state: &mut WorldState) -> (bool, Option<String>) {
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
        GameOutcome, AppState, ResearchFlatItem, ResearchKind, ResearchProject,
    };

    /// Helper: open Lab panel, navigate to first available item matching `kind_pred`, and confirm through.
    /// For BasicResearch, use `start_basic_research` instead (those are in the Research panel now).
    fn start_research_matching(state: &AppState, kind_pred: impl Fn(&ResearchKind) -> bool) -> AppState {
        use crate::state::LabTab;
        // Ensure panel is closed first, then open fresh
        let mut s = if state.ui.open_panel == crate::state::Panel::Lab {
            apply_action(state, &Action::ClosePanel)
        } else {
            state.clone()
        };
        s = apply_action(&s, &Action::OpenLab);
        let available = s.all_available_projects();
        // Find which tab contains the matching item and its index within that tab
        let mut found = None;
        for tab in LabTab::ALL {
            let items = s.lab_tab_items(tab);
            if let Some(idx) = items.iter().position(|item| {
                if let ResearchFlatItem::Available(proj_idx) = item {
                    available.get(*proj_idx).map_or(false, &kind_pred)
                } else {
                    false
                }
            }) {
                found = Some((tab, idx));
                break;
            }
        }
        let (tab, idx) = found.expect("expected matching research item in some tab");
        // Navigate to the correct tab
        s.ui.lab_ui = Some(crate::state::LabUiState::Browse { tab });
        s.ui.panel_selection = idx;
        s = apply_action(&s, &Action::Confirm); // ConfirmProject
        s = apply_action(&s, &Action::Confirm); // Start
        s
    }

    /// Helper: open Research (tech tree) panel, find first available BasicResearch, and confirm.
    fn start_basic_research(state: &AppState) -> AppState {
        let mut s = apply_action(state, &Action::ClosePanel);
        s = apply_action(&s, &Action::OpenResearch);
        let techs = crate::ui::tech_tree::layout_techs();
        let idx = techs.iter().position(|tech| {
            !s.unlocked_techs.contains(tech)
                && tech.prerequisites_met(&s.world)
                && !s.active_research.iter().any(|r| {
                    matches!(r.kind, ResearchKind::BasicResearch { tech: t } if t == *tech)
                })
        }).expect("expected available BasicResearch in tech tree");
        s.ui.panel_selection = idx;
        s = apply_action(&s, &Action::Confirm);
        s
    }

    #[test]
    fn research_identify_increases_knowledge() {
        let mut state = AppState::new_default(42);
        // Start identify project on disease 0 (first item in flat list)
        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Start
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (160 ticks at 1x speed)
        for _ in 0..160 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0; // Fully identified
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        assert!(!state.medicines[0].unlocked);

        // Start applied research: Develop Antiviral-A
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. }));

        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        for _ in 0..200 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().is_empty());
        assert!(state.medicines[0].unlocked);
    }

    #[test]
    fn research_clinical_trial_marks_tested() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true; // Pre-unlock for testing

        assert!(state.medicines[0].tested_against.is_empty());

        // Start field research: Clinical Trial
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::ClinicalTrial { .. }));

        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        for _ in 0..160 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert!(state.medicines[0].tested_against.contains(&0));
    }

    #[test]
    fn research_insufficient_personnel_blocks_start() {
        let mut state = AppState::new_default(42);
        state.resources.personnel = 0; // No personnel

        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start

        // Should not have started
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
    }

    #[test]
    fn more_personnel_means_faster_progress() {
        let mut state = AppState::new_default(42);

        // Create a project with base 5 personnel, assign 10 (2x base)
        // With diminishing returns: speed = 1 + (2-1)*(3-2)/2 = 1.5x
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 10, // 2x base (5) — peak of diminishing returns
        }];

        state = state.with_world(tick(&state).0);
        // At 2x ratio, diminishing returns gives 1.5x speed
        let expected = 1.5;
        assert!(
            (state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress - expected).abs() < 0.01,
            "2x personnel should give 1.5x speed, got {}",
            state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress
        );
    }

    #[test]
    fn diminishing_returns_beyond_double() {
        let mut state = AppState::new_default(42);

        // Assign 3x base personnel — should be back to 1.0x speed
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 15, // 3x base (5)
        }];

        state = state.with_world(tick(&state).0);
        let expected = 1.0;
        assert!(
            (state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress - expected).abs() < 0.01,
            "3x personnel should give 1.0x speed, got {}",
            state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress
        );
    }

    #[test]
    fn concurrent_field_and_applied_research() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 1000.0; // enough for both projects
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        // Start field research (first item in flat list)
        state = start_research_matching(&state, |k| k.is_field_work());
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        // Start applied research
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. }));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        // Both running simultaneously
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());
    }

    #[test]
    fn research_requires_funding() {
        let mut state = AppState::new_default(42);
        // Identify costs $350; set funding to $100 so it fails
        state.resources.funding = 100.0;

        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "should not start without funding");
        assert!(state.session.status_message.as_ref().unwrap().contains("Insufficient funds"));

        // Give enough funding, should succeed (still on ConfirmProject screen)
        state.resources.funding = 500.0;
        state = apply_action(&state, &Action::Confirm); // Try again
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "should start with sufficient funding");
        assert!(state.resources.funding < 500.0, "funding should be deducted");
    }

    #[test]
    fn develop_medicine_unlocks() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;

        state.active_research.push(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        });

        state = state.with_world(tick(&state).0);
        assert!(state.medicines[0].unlocked);
    }

    #[test]
    fn clinical_trial_adds_target_and_tested() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = true;

        state.active_research = vec![ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        }];

        state = state.with_world(tick(&state).0);
        assert!(state.medicines[0].tested_against.contains(&0));
        assert!(state.medicines[0].target_diseases.contains(&0));
    }

    #[test]
    fn clinical_trial_enables_deploy() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = true;

        state.active_research = vec![ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        }];

        // deploy should be disabled before trial completes
        assert!(!state.deploy_enabled.get(0).copied().unwrap_or(false),
            "deploy should be off before trial");

        state = state.with_world(tick(&state).0);

        assert!(state.medicines[0].tested_against.contains(&0),
            "medicine should be tested after trial");
        assert!(state.deploy_enabled.get(0).copied().unwrap_or(false),
            "deploy should be enabled automatically after trial completes");
    }

    #[test]
    fn narrow_medicine_cheaper_to_develop_than_broad() {
        let mut state = AppState::new_default(1);
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
    fn manufacture_doses_restores_supply() {
        use crate::engine::execute_command;
        use crate::state::GameCommand;

        let mut state = AppState::new_default(42);
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
        state.medicines[0].doses = 0.0;
        state.resources.funding = 10000.0;

        // Configure reactor to produce medicine 0
        execute_command(&mut state, &GameCommand::ConfigureReactor { reactor_idx: 0, medicine_idx: Some(0) });
        assert_eq!(state.reactors[0].medicine_idx, Some(0));

        // Start a batch
        execute_command(&mut state, &GameCommand::StartReactorBatch { reactor_idx: 0 });
        assert!(state.reactors[0].active, "reactor should be running");

        // Fast-forward batch to near completion
        state.reactors[0].batch_progress = state.reactors[0].batch_required - 1.0;

        state = state.with_world(tick(&state).0);

        assert!(!state.reactors[0].active, "reactor batch should be complete");
        let expected_doses = state.medicines[0].max_doses * state.manufacturing_yield_bonus();
        assert_eq!(
            state.medicines[0].doses, expected_doses,
            "doses should be restored to max * manufacturing bonus"
        );
    }

    #[test]
    fn genomic_sequencing_reduces_mutation_rate() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        let original_rate = state.diseases[0].pathogen_type.mutation_rate();

        state = start_research_matching(&state, |k| matches!(k, ResearchKind::GenomicSequencing { .. }));
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        for _ in 0..200 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = AppState::new_default(42);
        let initial_personnel = state.resources.personnel;

        state = start_research_matching(&state, |k| matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        for _ in 0..160 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().is_empty());
        assert_eq!(state.resources.personnel, initial_personnel + 5);
    }

    #[test]
    fn basic_research_unlocks_tech() {
        let mut state = AppState::new_default(42);
        // Prereq for TargetedDrugDesign: identify any pathogen
        state.diseases[0].knowledge = 0.5;
        state.resources.funding = 1000.0;
        assert!(state.unlocked_techs.is_empty());

        // Navigate: Research panel → find first available BasicResearch → Confirm
        state = start_basic_research(&state);
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().first().is_some(), "basic research should have started");

        // Advance to completion (240 ticks at 1x speed)
        for _ in 0..240 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().is_empty(), "project should be complete");
        assert!(
            state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign),
            "TargetedDrugDesign should be unlocked"
        );
    }

    #[test]
    fn three_concurrent_research_projects() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 2000.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);
        state.resources.personnel = 30;

        // Use the shared helper that handles panel toggle correctly
        // (it closes the panel first if already open)

        // Start field research
        state = start_research_matching(&state, |k| k.is_field_work());
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        // Start applied research
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. }));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        // Start basic research (via Research/tech tree panel)
        state = start_basic_research(&state);
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().first().is_some());

        // All three running simultaneously
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().first().is_some());
    }

    #[test]
    fn no_research_after_game_over() {
        let mut state = AppState::new_default(42);
        state.outcome = GameOutcome::Lost;
        // Try to start research
        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "should not start research after game over");
    }

    #[test]
    fn parallel_field_research_runs_and_completes_independently() {
        let mut state = AppState::new_default(42);
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

        assert_eq!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().len(), 2, "should have 2 parallel field projects");
        assert_eq!(state.personnel_busy(), 10, "10 personnel busy across 2 projects");

        // Advance until first project completes but second hasn't
        for _ in 0..55 {
            state = state.with_world(tick(&state).0);
        }
        assert_eq!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().len(), 1, "first project should have completed");
        assert!(matches!(&state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>()[0].kind, ResearchKind::ClinicalTrial { .. }),
            "remaining project should be the clinical trial");

        // Advance until second completes
        for _ in 0..50 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "both projects should have completed");
    }

    #[test]
    fn research_only_gated_by_personnel_and_funding() {
        let mut state = AppState::new_default(42);
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
        assert_eq!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().len(), 3);

        // With no capacity limits, a 4th project should start if we have resources
        let available = state.all_available_projects();
        assert!(!available.is_empty(), "should still have available projects with 3 active");
        let (ok, _msg) = super::start_research(&mut state, 0, false);
        assert!(ok, "should start a 4th project — no capacity limit, only personnel/funding");
        assert!(state.active_research.len() >= 4, "should have 4+ active projects");
    }

    #[test]
    fn rapid_sequencing_unlocks_after_sequencing() {
        let mut state = AppState::new_default(42);
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
        let mut state = AppState::new_default(42);
        let kind = ResearchKind::GenomicSequencing { disease_idx: 0 };

        let (_, base_dur, _) = state.effective_costs(&kind);
        assert_eq!(base_dur, 200.0, "base genomic sequencing should be 200 ticks");

        state.unlocked_techs.push(crate::state::BasicTech::RapidSequencing);
        let (_, rapid_dur, _) = state.effective_costs(&kind);
        assert_eq!(rapid_dur, 100.0, "with RapidSequencing, should be 100 ticks");
    }

    #[test]
    fn vaccine_platform_prereqs() {
        use crate::state::BasicTech;
        let mut state = AppState::new_default(42);
        // No chain prereqs → not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::VaccinePlatform }
        )), "VaccinePlatform should not be available without prereqs");

        // Unlock column 0 chain up to PhageTherapy only → still not available
        state.unlocked_techs.push(BasicTech::TargetedDrugDesign);
        state.unlocked_techs.push(BasicTech::MonoclonalAntibodies);
        state.unlocked_techs.push(BasicTech::PhageTherapy);
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::VaccinePlatform }
        )), "VaccinePlatform should not be available with only PhageTherapy");

        // Also unlock column 1 chain up to MetagenomicSurveillance → now available
        state.unlocked_techs.push(BasicTech::RapidSequencing);
        state.unlocked_techs.push(BasicTech::ResistanceSurveillance);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::VaccinePlatform }
        )), "VaccinePlatform should be available after PhageTherapy + MetagenomicSurveillance");
    }

    #[test]
    fn vaccine_platform_unlocks_vaccination() {
        let mut state = AppState::new_default(42);
        assert!(!state.can_vaccinate(), "vaccination should be locked without VaccinePlatform");
        assert_eq!(state.vaccination_multiplier(), 1.0);

        state.unlocked_techs.push(crate::state::BasicTech::VaccinePlatform);
        assert!(state.can_vaccinate(), "vaccination should be unlocked with VaccinePlatform");
        assert_eq!(state.vaccination_multiplier(), 3.0);

        // Verify estimate_vaccination uses the multiplier
        let med = &state.medicines[0];
        let base = med.estimate_vaccination(1_000_000.0, 1.0, 1.0);
        let boosted = med.estimate_vaccination(1_000_000.0, 1.0, 3.0);
        assert!(
            (boosted - base * 3.0).abs() < 1.0,
            "VaccinePlatform 3x multiplier: base={base}, boosted={boosted}"
        );
    }

    #[test]
    fn combination_therapy_prereqs() {
        use crate::state::BasicTech;
        let mut state = AppState::new_default(42);
        // No chain prereqs or deployed medicines → not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::CombinationTherapy }
        )), "CombinationTherapy should not be available without prereqs");

        // Unlock column 1 chain (RapidSeq → ResSurv → MetaSurv → EpiForecasting)
        state.unlocked_techs.push(BasicTech::RapidSequencing);
        state.unlocked_techs.push(BasicTech::ResistanceSurveillance);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        state.unlocked_techs.push(BasicTech::EpidemiologicalForecasting);

        // Chain prereq met but no deployed medicines → still not available
        state.medicines[0].deployed_count = 1;
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::CombinationTherapy }
        )), "CombinationTherapy should not be available with only 1 deployed medicine");

        // Deploy a second medicine → available
        state.medicines[1].deployed_count = 1;
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::CombinationTherapy }
        )), "CombinationTherapy should be available with EpiForecasting + 2+ deployed medicines");
    }

    #[test]
    fn combination_therapy_halves_resistance() {
        let mut state = AppState::new_default(42);
        assert_eq!(state.resistance_multiplier(), 1.0);

        state.unlocked_techs.push(crate::state::BasicTech::CombinationTherapy);
        assert_eq!(state.resistance_multiplier(), 0.5);
    }


    #[test]
    fn genomic_sequencing_unavailable_after_effective_rate_drops() {
        use crate::state::PathogenType;

        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.diseases[0].pathogen_type = PathogenType::RnaVirus; // base rate 0.0002
        // Ensure disease has infected population so sequencing can be considered
        state.regions[0].get_or_create_infection(0).infected = 1000.0;
        state.active_research.clear();

        // After 6 sequencings: 0.0002 * 0.5^6 = 0.000003125 < 0.000005 threshold
        state.diseases[0].sequencing_count = 6;
        let field_projects = state.available_field_projects();
        assert!(
            !field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should not be available when effective rate ({}) is below threshold",
            state.diseases[0].effective_variant_rate()
        );

        // After 2 sequencings: 0.0002 * 0.5^2 = 0.00005 > 0.000005 — still available
        state.diseases[0].sequencing_count = 2;
        let field_projects = state.available_field_projects();
        assert!(
            field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should still be available when effective rate ({}) is above threshold",
            state.diseases[0].effective_variant_rate()
        );
    }

    #[test]
    fn human_trials_halves_clinical_trial_duration() {
        let mut state = AppState::new_default(42);
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
        let normal_duration = state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().last().unwrap().required_ticks;
        state.active_research.retain(|p| !p.kind.is_field_work());

        // Now enact human trials and start the same trial
        state.enacted_decrees.authorize_human_trials = true;
        let projects = state.all_available_projects();
        let trial_idx = projects.iter().position(|k| matches!(k, ResearchKind::ClinicalTrial { .. }));
        let (ok, _) = super::start_research(&mut state, trial_idx.unwrap(), false);
        assert!(ok);
        let fast_duration = state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().last().unwrap().required_ticks;

        // Duration should be halved
        assert!(
            (fast_duration - normal_duration * crate::state::HUMAN_TRIALS_SPEED).abs() < 1.0,
            "human trials should halve duration: normal={normal_duration}, fast={fast_duration}"
        );
    }


    #[test]
    fn lab_upgrade_increases_research_speed() {
        use crate::state::LAB_LEVEL_1_COST;

        let mut state = AppState::new_default(42);
        state.resources.funding = 1000.0;
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5, // base personnel, 1.0x speed
        }];

        // Baseline: one tick at standard lab
        let (base_state, _) = tick(&state);
        let base_progress = base_state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>()[0].progress;

        // Upgrade to level 1 (1.3x multiplier)
        state.lab_level = 1;
        let (upgraded_state, _) = tick(&state);
        let upgraded_progress = upgraded_state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>()[0].progress;

        assert!(
            (upgraded_progress / base_progress - 1.3).abs() < 0.01,
            "Lab level 1 should give 1.3x speed, got {}x",
            upgraded_progress / base_progress
        );

        // Verify upgrade_lab deducts cost and increments level
        let mut s = AppState::new_default(42);
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
        let mut state = AppState::new_default(42);
        // Reset broad-spectrum to locked so identification can trigger the handoff
        for med in &mut state.medicines {
            if med.therapy_type == crate::state::TherapyType::BroadSpectrum {
                med.unlocked = false;
                med.doses = 0.0;
            }
        }
        // Start identify on disease 0 (first item in flat list)
        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Start
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        // Advance to completion, checking events each tick (clone-and-mutate means
        // events are only on the state returned by the tick that generated them)
        let mut found_handoff = false;
        for _ in 0..200 {
            let tick_result = tick(&state);
            state = state.with_world(tick_result.0);
            let tick_events = tick_result.1;
            if tick_events.iter().any(|e|
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
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);

        // Start develop medicine (applied research)
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. }));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some(),
            "Applied research should start. UI state: {:?}", state.ui.lab_ui);

        // Advance to completion, checking events each tick
        let mut found_handoff = false;
        for _ in 0..600 {
            let tick_result = tick(&state);
            state = state.with_world(tick_result.0);
            let tick_events = tick_result.1;
            if tick_events.iter().any(|e|
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
        let mut state = AppState::new_default(42);
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
        let (after, _) = tick(&state);
        assert_eq!(
            after.medicines[0].doses,
            max_doses,
            "without tech, doses should equal max_doses"
        );

        // With StabilizedFormulation: get 125% of max_doses
        state.unlocked_techs.push(crate::state::BasicTech::StabilizedFormulation);
        state.medicines[0].doses = 0.0;
        state.active_research.retain(|p| !matches!(p.kind, ResearchKind::DevelopMedicine { .. } | ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel));
        state.active_research.push(ResearchProject {
            kind: ResearchKind::ManufactureDoses { medicine_idx: 0 },
            progress: 14.0,
            required_ticks: 15.0,
            personnel_assigned: 3,
        });
        let (after_tech, _) = tick(&state);
        assert_eq!(
            after_tech.medicines[0].doses,
            max_doses * 1.25,
            "StabilizedFormulation should give 125% of max doses"
        );
    }

    #[test]
    fn blocked_medicine_developments_shows_identified_but_unresearched() {
        use crate::state::BasicTech;

        let mut state = AppState::new_default(42);

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

        let mut state = AppState::new_default(42);

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
