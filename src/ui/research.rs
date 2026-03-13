use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{FIELD_OPS_RESTORE, GameState, InfraSystem, LAB_LEVEL_1_COST, LAB_LEVEL_2_COST, Medicine, PERSONNEL_UPKEEP_COST, ResearchCategory, ResearchKind, ResearchUiState, TherapyType, KNOWLEDGE_FOR_MEDICINE, KNOWLEDGE_FULL, KNOWLEDGE_NAME, TICKS_PER_DAY, TRAIN_PERSONNEL_BATCH, format_days, personnel_speed};
use crate::ui::hint_line;

/// Maximum selection index for the research panel in its current sub-state.
pub fn selection_max(ui_state: &ResearchUiState, state: &GameState) -> usize {
    match ui_state {
        ResearchUiState::BrowseAll => {
            state.research_flat_items().len().saturating_sub(1)
        }
        ResearchUiState::ConfirmProject { .. } | ResearchUiState::ConfirmLabUpgrade => 0,
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines, selected_line) = match &state.ui.research_ui {
        Some(ResearchUiState::BrowseAll) => render_flat(state),
        Some(ResearchUiState::ConfirmProject { category, project_idx, double_personnel }) => {
            let (t, l) = render_confirm(state, *category, *project_idx, *double_personnel);
            (t, l, None)
        }
        Some(ResearchUiState::ConfirmLabUpgrade) => {
            let (t, l) = render_confirm_lab_upgrade(state);
            (t, l, None)
        }
        None => (" Research ".to_string(), vec![], None),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = selected_line.map(|line| {
        if line as u16 >= inner_height {
            (line as u16).saturating_sub(inner_height * 2 / 3)
        } else {
            0
        }
    }).unwrap_or(0);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

fn category_name(category: ResearchCategory) -> &'static str {
    match category {
        ResearchCategory::Field => "Field",
        ResearchCategory::Applied => "Applied",
        ResearchCategory::Basic => "Basic",
    }
}

/// Render the flat research panel with section headers for each category.
fn render_flat(state: &GameState) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let items = state.research_flat_items();
    let mut item_idx = 0usize; // tracks position in `items` as we render

    // ─── Field Research ───
    render_section_header(&mut lines, "Field Research", ResearchCategory::Field, state);
    let field_active = state.active_in_category(ResearchCategory::Field);
    let n_field_active = field_active.len();
    let has_field_capacity = state.field_research_has_capacity();

    if n_field_active == 0 && (!has_field_capacity || state.available_projects(ResearchCategory::Field).is_empty()) {
        lines.push(Line::from(Span::styled(
            "  No projects available.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Active field projects
    for project in &field_active {
        let selected = state.ui.panel_selection == item_idx;
        if selected { selected_line = Some(lines.len()); }
        render_active_project(&mut lines, project, selected, state);
        item_idx += 1;
    }

    // Available field projects
    if has_field_capacity {
        let field_available = state.available_projects(ResearchCategory::Field);
        if !field_available.is_empty() && n_field_active > 0 {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  ── Start New ({}/{} slots) ──", n_field_active, crate::state::MAX_FIELD_RESEARCH),
                Style::default().fg(Color::DarkGray),
            )));
        }
        for kind in &field_available {
            let selected = state.ui.panel_selection == item_idx;
            if selected { selected_line = Some(lines.len()); }
            render_available_project(&mut lines, kind, selected, state);
            item_idx += 1;
        }
    }

    // ─── Applied Research ───
    lines.push(Line::from(""));
    render_section_header(&mut lines, "Applied Research", ResearchCategory::Applied, state);

    let applied_is_train = state.research_slot(ResearchCategory::Applied)
        .is_some_and(|p| matches!(p.kind, ResearchKind::TrainPersonnel));
    if let Some(project) = state.research_slot(ResearchCategory::Applied) {
        if !applied_is_train {
            let selected = state.ui.panel_selection == item_idx;
            if selected { selected_line = Some(lines.len()); }
            render_active_project(&mut lines, project, selected, state);
            item_idx += 1;
        } else {
            // TrainPersonnel is active but shown in Lab; show slot-busy hint here
            lines.push(Line::from(Span::styled(
                "  (slot used by Train Personnel)",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        let applied_available = state.available_projects(ResearchCategory::Applied);
        // Filter out TrainPersonnel — it's shown in the Lab section instead
        let applied_non_train: Vec<_> = applied_available.iter()
            .filter(|k| !matches!(k, ResearchKind::TrainPersonnel))
            .collect();
        if applied_non_train.is_empty() {
            // Show hints about why nothing is available
            let blocked = state.blocked_medicine_developments();
            if !blocked.is_empty() {
                for (disease_idx, reason) in &blocked {
                    let disease_name = state.diseases.get(*disease_idx)
                        .map(|d| d.display_name(*disease_idx))
                        .unwrap_or_else(|| "Unknown".to_string());
                    lines.push(Line::from(Span::styled(
                        format!("  [PENDING] Develop: {}", disease_name),
                        Style::default().fg(Color::DarkGray),
                    )));
                    lines.push(Line::from(Span::styled(
                        format!("    {}", reason),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            } else {
                let has_any_develop = state.research_slot(ResearchCategory::Applied)
                    .is_some_and(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. }))
                    || state.available_applied_projects()
                        .iter()
                        .any(|k| matches!(k, ResearchKind::DevelopMedicine { .. }));
                let any_identified = state.diseases.iter().any(|d| d.knowledge > 0.0);
                if !has_any_develop && !any_identified {
                    lines.push(Line::from(Span::styled(
                        "  No diseases identified yet.",
                        Style::default().fg(Color::DarkGray),
                    )));
                    lines.push(Line::from(Span::styled(
                        "  (Start an Identify Threat project in Field Research.)",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "  No projects available.",
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        } else {
            for kind in &applied_available {
                if matches!(kind, ResearchKind::TrainPersonnel) {
                    continue;
                }
                let selected = state.ui.panel_selection == item_idx;
                if selected { selected_line = Some(lines.len()); }
                render_available_project(&mut lines, kind, selected, state);
                item_idx += 1;
            }
        }
    }

    // ─── Basic Research ───
    lines.push(Line::from(""));
    render_section_header(&mut lines, "Basic Research", ResearchCategory::Basic, state);

    if let Some(project) = state.research_slot(ResearchCategory::Basic) {
        let selected = state.ui.panel_selection == item_idx;
        if selected { selected_line = Some(lines.len()); }
        render_active_project(&mut lines, project, selected, state);
        item_idx += 1;
    } else {
        let basic_available = state.available_projects(ResearchCategory::Basic);
        if basic_available.is_empty() {
            // Show lock/complete status
            let all_unlocked = crate::state::BasicTech::all()
                .iter()
                .all(|t| state.unlocked_techs.contains(t));
            if all_unlocked {
                lines.push(Line::from(Span::styled(
                    "  All technologies unlocked.",
                    Style::default().fg(Color::Green),
                )));
            } else {
                let hint = crate::state::BasicTech::all()
                    .iter()
                    .find(|t| !state.unlocked_techs.contains(t))
                    .map(|t| format!("  [LOCKED] {} to unlock {}", t.prereq_description(), t.name()))
                    .unwrap_or_else(|| "  [LOCKED]".to_string());
                lines.push(Line::from(Span::styled(
                    hint,
                    Style::default().fg(Color::DarkGray),
                )));
            }
        } else {
            for kind in &basic_available {
                let selected = state.ui.panel_selection == item_idx;
                if selected { selected_line = Some(lines.len()); }
                render_available_project(&mut lines, kind, selected, state);
                item_idx += 1;
            }
        }
    }

    // ─── Lab ───
    lines.push(Line::from(""));
    if state.lab_level >= 2 {
        lines.push(Line::from(Span::styled(
            format!("  ─── {} (max) ───", state.lab_level_name()),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "    All research runs 60% faster",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  ─── Lab ───",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Train Personnel — always shown in Lab section, whether active or available
    if applied_is_train {
        let project = state.research_slot(ResearchCategory::Applied).unwrap();
        let selected = state.ui.panel_selection == item_idx;
        if selected { selected_line = Some(lines.len()); }
        render_active_project(&mut lines, project, selected, state);
        item_idx += 1;
    } else if state.research_slot(ResearchCategory::Applied).is_none() {
        let applied_projects = state.available_projects(ResearchCategory::Applied);
        for kind in &applied_projects {
            if matches!(kind, ResearchKind::TrainPersonnel) {
                let selected = state.ui.panel_selection == item_idx;
                if selected { selected_line = Some(lines.len()); }
                render_available_project(&mut lines, kind, selected, state);
                item_idx += 1;
            }
        }
    }

    // Lab upgrade button
    if state.lab_level < 2 {
        let selected = state.ui.panel_selection == item_idx;
        if selected { selected_line = Some(lines.len()); }
        let marker = if selected { "▶ " } else { "  " };
        let upgrade_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Magenta)
        };
        lines.push(Line::from(Span::styled(
            format!("{}[PURCHASE] Upgrade Research Lab", marker),
            upgrade_style,
        )));
        let (cost, next_name, pct) = if state.lab_level == 0 {
            (LAB_LEVEL_1_COST, "Enhanced Sequencing", 30)
        } else {
            (LAB_LEVEL_2_COST, "Advanced Genomics Center", 60)
        };
        let can_afford = state.resources.funding >= cost;
        let cost_style = if can_afford { Color::Cyan } else { Color::Red };
        lines.push(Line::from(vec![
            Span::styled(
                format!("    {} → {} (+{}% speed)", state.lab_level_name(), next_name, pct),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!(" [¥{:.0}]", cost),
                Style::default().fg(cost_style),
            ),
        ]));
        item_idx += 1;
    }

    let _ = item_idx;
    lines.push(Line::from(""));
    if !items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  [↑/↓] Select  [Enter] Confirm  [X] Auto",
            Style::default().fg(Color::DarkGray),
        )));
    }

    (" Research ".to_string(), lines, selected_line)
}

/// Render a section header like "─── Field Research ───"
fn render_section_header(lines: &mut Vec<Line<'static>>, name: &str, _track: ResearchCategory, _state: &GameState) {
    lines.push(Line::from(Span::styled(
        format!("  ─── {} ───", name),
        Style::default().fg(Color::DarkGray),
    )));
}

/// Render an active research project (shows progress).
fn render_active_project(lines: &mut Vec<Line<'static>>, project: &crate::state::ResearchProject, selected: bool, state: &GameState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
    let remaining = (project.required_ticks - project.progress).max(0.0);
    let speed = project.speed(&state.medicines);
    let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
    let auto_tag = if state.auto_repeat_research.contains(&project.kind) { " AUTO" } else { "" };
    lines.push(Line::from(Span::styled(
        format!("{}[ACTIVE]{} {}", marker, auto_tag, project.kind.display_label(&state.diseases, &state.medicines, &state.regions)),
        style,
    )));
    lines.push(Line::from(Span::styled(
        format!("    Progress: {:.0}%, {} remaining", pct, format_days(effective_remaining)),
        Style::default().fg(Color::Green),
    )));
}

/// Render an available (startable) research project with cost details.
fn render_available_project(lines: &mut Vec<Line<'static>>, kind: &ResearchKind, selected: bool, state: &GameState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let auto_tag = if state.auto_repeat_research.contains(kind) { " AUTO" } else { "" };
    lines.push(Line::from(Span::styled(
        format!("{}{}{}", marker, kind.display_label(&state.diseases, &state.medicines, &state.regions), auto_tag),
        style,
    )));
    if let Some(detail) = format_detail(kind, state) {
        lines.push(Line::from(Span::styled(
            format!("    {}", detail),
            Style::default().fg(Color::DarkGray),
        )));
    }
    let (personnel, ticks, funding) = state.effective_costs(kind);
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("¥{:.0}", funding), Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(format!("{} personnel", personnel), Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(format_days(ticks), Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));
}

fn render_confirm(state: &GameState, category: ResearchCategory, project_idx: usize, double_personnel: bool) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let projects = state.available_projects(category);

    if let Some(kind) = projects.get(project_idx) {
        let (base_personnel, ticks, funding) = state.effective_costs(kind);
        let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };
        let has_personnel = state.personnel_available() >= personnel;
        let has_funding = state.resources.funding >= funding;

        // Breadcrumb
        lines.push(Line::from(Span::styled(
            format!("  Research > {} > Confirm", category_name(category)),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            format!("  Start: {}", kind.display_label(&state.diseases, &state.medicines, &state.regions)),
            Style::default().fg(Color::Cyan),
        )));
        if let Some(detail) = format_detail(kind, state) {
            lines.push(Line::from(Span::styled(
                format!("  {}", detail),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  Cost: "),
            Span::styled(format!("¥{:.0}", funding), Style::default().fg(
                if has_funding { Color::Green } else { Color::Red }
            )),
            Span::styled(
                format!("  (have ¥{:.0})", state.resources.funding),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  Personnel: "),
            Span::styled(format!("{}", personnel), Style::default().fg(
                if has_personnel { Color::Green } else { Color::Red }
            )),
            Span::styled(
                format!("  ({} available)", state.personnel_available()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Toggle checkbox for 2x personnel
        let checkbox = if double_personnel { "[X]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", checkbox)),
            Span::styled("Assign 2x personnel", Style::default().fg(
                if double_personnel { Color::Yellow } else { Color::DarkGray }
            )),
            Span::styled("  [X] toggle", Style::default().fg(Color::DarkGray)),
        ]));

        // Show effective speed based on personnel ratio
        let speed = personnel_speed(personnel, base_personnel);
        let effective_ticks = ticks / speed;
        lines.push(Line::from(vec![
            Span::raw("  Duration: "),
            Span::styled(format_days(effective_ticks), Style::default().fg(Color::White)),
            Span::styled(format!("  ({:.1}x speed)", speed), Style::default().fg(
                if speed > 1.0 { Color::Green } else { Color::DarkGray }
            )),
        ]));

        let can_afford = has_personnel && has_funding;

        lines.push(Line::from(""));
        if can_afford {
            lines.push(hint_line(state, "Confirm", "Back"));
        } else {
            lines.push(Line::from(Span::styled(
                "  Insufficient resources!",
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(Span::styled(
                "  [Esc] Back",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    (" Confirm Research ".to_string(), lines)
}

fn render_confirm_lab_upgrade(state: &GameState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let (cost, next_name, pct) = if state.lab_level == 0 {
        (LAB_LEVEL_1_COST, "Enhanced Sequencing", 30)
    } else {
        (LAB_LEVEL_2_COST, "Advanced Genomics Center", 60)
    };
    let can_afford = state.resources.funding >= cost;

    lines.push(Line::from(Span::styled(
        "  Research > Lab Upgrade > Confirm",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Upgrade: {} → {}", state.lab_level_name(), next_name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(Span::styled(
        format!("  All research runs {}% faster", pct),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  Cost: "),
        Span::styled(format!("¥{:.0}", cost), Style::default().fg(
            if can_afford { Color::Green } else { Color::Red }
        )),
        Span::styled(
            format!("  (have ¥{:.0})", state.resources.funding),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(""));
    if can_afford {
        lines.push(hint_line(state, "Confirm", "Back"));
    } else {
        lines.push(Line::from(Span::styled(
            "  Insufficient funding!",
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(Span::styled(
            "  [Esc] Back",
            Style::default().fg(Color::DarkGray),
        )));
    }

    (" Confirm Lab Upgrade ".to_string(), lines)
}

/// Label showing the manufacturing corporation for a medicine.
/// Returns " | Mfg: CorpName (+Approval)" or " | Mfg: CorpName" or "".
fn manufacturer_label(med: &Medicine, state: &GameState) -> String {
    let corp_idx = match med.manufacturer_corp_idx {
        Some(idx) => idx,
        None => return String::new(),
    };
    let corp = match state.corporations.get(corp_idx) {
        Some(c) => c,
        None => return String::new(),
    };
    if corp.board_seat {
        format!(" | Mfg: {} (+Approval)", corp.name)
    } else {
        format!(" | Mfg: {}", corp.name)
    }
}

/// Supplementary detail line for a research project (targets, knowledge, etc).
fn format_detail(kind: &ResearchKind, state: &GameState) -> Option<String> {
    match kind {
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let names: Vec<String> = med.target_diseases.iter()
                .filter_map(|&d_idx| {
                    state.diseases.get(d_idx)
                        .map(|d| d.display_name(d_idx))
                })
                .collect();
            let mfg = manufacturer_label(med, state);
            if let Some(mech) = med.mechanism {
                let resist_label = if mech.resistance_rate_multiplier() > 1.2 {
                    "High"
                } else if mech.resistance_rate_multiplier() > 0.7 {
                    "Med"
                } else {
                    "Low"
                };
                Some(format!("{}: {} | Eff {:.0}%, Resist: {}{}",
                    mech.tradeoff_label(),
                    names.join(", "),
                    mech.efficacy_modifier() * 100.0,
                    resist_label,
                    mfg))
            } else {
                Some(format!("Targets: {}{}",
                    names.join(", "),
                    mfg))
            }
        }
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let yield_bonus = state.manufacturing_yield_bonus();
            let actual_doses = med.max_doses * yield_bonus;
            if (yield_bonus - 1.0).abs() > 0.01 {
                Some(format!("Produces {} doses (+{:.0}% mfg bonus)",
                    crate::format_number(actual_doses),
                    (yield_bonus - 1.0) * 100.0))
            } else {
                Some(format!("Restores to {} doses", crate::format_number(med.max_doses)))
            }
        }
        ResearchKind::GenomicSequencing { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            let current_rate = disease.effective_mutation_rate();
            let new_rate = current_rate * 0.5;
            Some(format!("Mutation rate: {:.4} → {:.4}", current_rate, new_rate))
        }
        ResearchKind::TrainPersonnel => {
            let added_upkeep = TRAIN_PERSONNEL_BATCH as f64 * PERSONNEL_UPKEEP_COST * TICKS_PER_DAY;
            Some(format!("Current: {} personnel (+¥{:.0}/day upkeep after)",
                state.resources.personnel, added_upkeep))
        }
        ResearchKind::IdentifyThreat { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            if disease.knowledge >= KNOWLEDGE_NAME {
                // Already identified — explain what further study unlocks
                let has_targeted_tech = state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign);
                // Broad-spectrum targets all diseases — it's unlockable once ANY disease
                // reaches KNOWLEDGE_FOR_MEDICINE. Don't mislead player about this disease
                // "unlocking" broad-spectrum if it's already available or developed.
                let broad_already_available = state.medicines.iter().any(|m| {
                    m.therapy_type == TherapyType::BroadSpectrum
                        && (m.unlocked
                            || m.target_diseases.iter().any(|&d_idx| {
                                state.diseases.get(d_idx).map_or(false, |d| {
                                    d.knowledge >= KNOWLEDGE_FOR_MEDICINE
                                })
                            }))
                });
                let next = if disease.knowledge < KNOWLEDGE_FOR_MEDICINE {
                    if broad_already_available {
                        "Targeted medicine requires full study. Keep studying"
                    } else {
                        "Unlocks broad-spectrum medicine development"
                    }
                } else if disease.knowledge < KNOWLEDGE_FULL {
                    if has_targeted_tech {
                        "Unlocks targeted medicine development"
                    } else {
                        "Targeted medicines also need Basic Research: Targeted Drug Design"
                    }
                } else {
                    "Fully studied"
                };
                Some(format!("Knowledge: {:.0}% ({})", disease.knowledge * 100.0, next))
            } else if disease.knowledge > 0.0 {
                Some(format!("Knowledge: {:.0}%", disease.knowledge * 100.0))
            } else {
                None
            }
        }
        ResearchKind::BasicResearch { tech } => {
            Some(tech.description().to_string())
        }
        ResearchKind::SuppressPathogen { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            Some(format!("Current infectivity: {:.4} → {:.4}", disease.infectivity, disease.infectivity * 0.80))
        }
        ResearchKind::AttenuatePathogen { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            Some(format!("Current lethality: {:.4} → {:.4}", disease.lethality, disease.lethality * 0.70))
        }
        ResearchKind::InterdictPathogen { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            Some(format!("Cross-region spread: {:.4} → 0.0000", disease.cross_region_spread))
        }
        ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let is_retrial = med.tested_against.contains(disease_idx);
            if is_retrial {
                let strain_eff = med.strain_efficacy(*disease_idx, &state.diseases);
                let behind = med.mutations_behind(*disease_idx, &state.diseases);
                Some(format!("Recalibrate strain drift ({} mutation{} behind, strain eff {:.0}%)",
                    behind, if behind == 1 { "" } else { "s" }, strain_eff * 100.0))
            } else {
                Some("First trial — tests efficacy and enables deployment".to_string())
            }
        }
        ResearchKind::FieldOperations { region_idx, system } => {
            let region = state.regions.get(*region_idx)?;
            let current = match system {
                InfraSystem::Healthcare => region.healthcare_capacity,
                InfraSystem::SupplyLines => region.supply_lines,
                InfraSystem::CivilOrder => region.civil_order,
            };
            let after = (current + FIELD_OPS_RESTORE).min(1.0);
            Some(format!("{}: {:.0}% → {:.0}%", system.label(), current * 100.0, after * 100.0))
        }
    }
}
