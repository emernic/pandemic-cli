use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{FIELD_OPS_RESTORE, GameState, InfraSystem, LAB_LEVEL_1_COST, LAB_LEVEL_2_COST, PERSONNEL_UPKEEP_COST, ResearchKind, RESEARCH_TRACK_COUNT, ResearchTrack, ResearchUiState, TherapyType, KNOWLEDGE_FOR_MEDICINE, KNOWLEDGE_FULL, KNOWLEDGE_NAME, TICKS_PER_DAY, TRAIN_PERSONNEL_BATCH, format_days, personnel_speed};
use crate::ui::hint_line;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines, selected_line) = match &state.ui.research_ui {
        Some(ResearchUiState::BrowseCategories) => render_categories(state),
        Some(ResearchUiState::BrowseProjects { track }) => render_projects(state, *track),
        Some(ResearchUiState::ConfirmProject { track, project_idx, double_personnel }) => {
            let (t, l) = render_confirm(state, *track, *project_idx, *double_personnel);
            (t, l, None)
        }
        Some(ResearchUiState::ViewActive { track, slot_idx }) => {
            let (t, l) = render_active(state, *track, *slot_idx);
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

fn render_categories(state: &GameState) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;

    let categories = [
        ("Field Research", "Identify threats, run clinical trials", ResearchTrack::Field),
        ("Applied Research", "Develop medicines, manufacture doses", ResearchTrack::Applied),
        ("Basic Research", "Unlock new therapeutic technologies", ResearchTrack::Basic),
    ];
    for (i, (name, desc, track)) in categories.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        if selected {
            selected_line = Some(lines.len());
        }
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(Span::styled(
            format!("{}{}", marker, name),
            style,
        )));

        let (status, status_color) = if *track == ResearchTrack::Field {
            let n = state.field_research.len();
            if n > 0 {
                (format!(" [{}/{}]", n, crate::state::MAX_FIELD_RESEARCH), Color::Cyan)
            } else {
                (" [NONE]".to_string(), Color::DarkGray)
            }
        } else if state.research_slot(*track).is_some() {
            (" [ACTIVE]".to_string(), match track {
                ResearchTrack::Field => Color::Cyan,
                ResearchTrack::Applied => Color::Magenta,
                ResearchTrack::Basic => Color::Green,
            })
        } else if *track == ResearchTrack::Basic && state.available_basic_projects().is_empty() {
            let all_unlocked = crate::state::BasicTech::all()
                .iter()
                .all(|t| state.unlocked_techs.contains(t));
            if all_unlocked {
                (" [COMPLETE]".to_string(), Color::Green)
            } else {
                // Find the first unmet tech and show its prereq as a hint
                let hint = crate::state::BasicTech::all()
                    .iter()
                    .find(|t| !state.unlocked_techs.contains(t))
                    .map(|t| format!(" [LOCKED] {} to unlock {}", t.prereq_description(), t.name()))
                    .unwrap_or_else(|| " [LOCKED]".to_string());
                (hint, Color::DarkGray)
            }
        } else if *track == ResearchTrack::Applied {
            let has_low_doses = state.medicines.iter().any(|m| m.unlocked && m.doses < m.max_doses * 0.5);
            if has_low_doses {
                (" [MANUFACTURE READY]".to_string(), Color::Yellow)
            } else {
                (" [NONE]".to_string(), Color::DarkGray)
            }
        } else {
            (" [NONE]".to_string(), Color::DarkGray)
        };
        let auto_label = if state.auto_research[track.index()] {
            Span::styled(" AUTO", Style::default().fg(Color::Green))
        } else {
            Span::raw("")
        };
        lines.push(Line::from(vec![
            Span::styled(format!("    {}", desc), Style::default().fg(Color::DarkGray)),
            Span::styled(status, Style::default().fg(status_color)),
            auto_label,
        ]));
        lines.push(Line::from(""));
    }

    // Lab upgrade entry (always at index RESEARCH_TRACK_COUNT, after all tracks)
    let lab_selected = state.ui.panel_selection == RESEARCH_TRACK_COUNT;
    if lab_selected {
        selected_line = Some(lines.len());
    }
    let lab_marker = if lab_selected { "▶ " } else { "  " };
    let lab_style = if lab_selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let lab_title = if state.lab_level >= 2 {
        "Research Lab"
    } else {
        "Upgrade Research Lab"
    };
    lines.push(Line::from(Span::styled(
        format!("{}{}", lab_marker, lab_title),
        lab_style,
    )));
    let (lab_desc, lab_status, lab_status_color) = if state.lab_level >= 2 {
        (
            format!("    {}: all research runs 60% faster", state.lab_level_name()),
            String::new(),
            Color::Green,
        )
    } else {
        let (cost, next_name, pct) = if state.lab_level == 0 {
            (LAB_LEVEL_1_COST, "Enhanced Sequencing", 30)
        } else {
            (LAB_LEVEL_2_COST, "Advanced Genomics Center", 60)
        };
        let can_afford = state.resources.funding >= cost;
        let status = if can_afford {
            format!(" [¥{:.0}]", cost)
        } else {
            format!(" [¥{:.0} needed]", cost)
        };
        let status_color = if can_afford { Color::Cyan } else { Color::Red };
        (
            format!("    {} → {} (+{}% speed)", state.lab_level_name(), next_name, pct),
            status,
            status_color,
        )
    };
    lines.push(Line::from(vec![
        Span::styled(lab_desc, Style::default().fg(Color::DarkGray)),
        Span::styled(lab_status, Style::default().fg(lab_status_color)),
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [↑/↓] Select  [Enter] Confirm  [X] Auto",
        Style::default().fg(Color::DarkGray),
    )));

    (" Research ".to_string(), lines, selected_line)
}

fn track_name(track: ResearchTrack) -> &'static str {
    match track {
        ResearchTrack::Field => "Field",
        ResearchTrack::Applied => "Applied",
        ResearchTrack::Basic => "Basic",
    }
}

fn render_projects(state: &GameState, track: ResearchTrack) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let title = match track {
        ResearchTrack::Field => " Field Research ",
        ResearchTrack::Applied => " Applied Research ",
        ResearchTrack::Basic => " Basic Research ",
    };
    let mut has_selectable_items = false;

    // Breadcrumb so the player knows where they are in the hierarchy
    lines.push(Line::from(Span::styled(
        format!("  Research > {}", track_name(track)),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    if track == ResearchTrack::Field {
        // Field track: show active projects first, then available (if capacity remains)
        let n_active = state.field_research.len();

        for (i, project) in state.field_research.iter().enumerate() {
            has_selectable_items = true;
            let selected = state.ui.panel_selection == i;
            if selected {
                selected_line = Some(lines.len());
            }
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
            lines.push(Line::from(Span::styled(
                format!("{}[ACTIVE] {}", marker, project.kind.display_label(&state.diseases, &state.medicines, &state.regions)),
                style,
            )));
            lines.push(Line::from(Span::styled(
                format!("    Progress: {:.0}%, {} remaining", pct, format_days(effective_remaining)),
                Style::default().fg(Color::Green),
            )));
        }

        // Show available projects if capacity remains
        if state.field_research_has_capacity() {
            let projects = state.available_projects(track);
            if !projects.is_empty() {
                if n_active > 0 {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        format!("  ── Start New ({}/{} slots) ──", n_active, crate::state::MAX_FIELD_RESEARCH),
                        Style::default().fg(Color::DarkGray),
                    )));
                    lines.push(Line::from(""));
                }
                has_selectable_items = true;
                for (i, kind) in projects.iter().enumerate() {
                    let sel_idx = n_active + i;
                    let selected = state.ui.panel_selection == sel_idx;
                    if selected {
                        selected_line = Some(lines.len());
                    }
                    let marker = if selected { "▶ " } else { "  " };
                    let style = if selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    lines.push(Line::from(Span::styled(
                        format!("{}{}", marker, kind.display_label(&state.diseases, &state.medicines, &state.regions)),
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
            } else if n_active == 0 {
                lines.push(Line::from(Span::styled(
                    "  No projects available.",
                    Style::default().fg(Color::DarkGray),
                )));
            }
        } else if n_active == 0 {
            lines.push(Line::from(Span::styled(
                "  No projects available.",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        // Applied/Basic: single-slot behavior
        let active = state.research_slot(track);

        if let Some(project) = active {
            has_selectable_items = true;
            let selected = state.ui.panel_selection == 0;
            if selected {
                selected_line = Some(lines.len());
            }
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
            lines.push(Line::from(Span::styled(
                format!("{}[ACTIVE] {}", marker, project.kind.display_label(&state.diseases, &state.medicines, &state.regions)),
                style,
            )));
            lines.push(Line::from(Span::styled(
                format!("    Progress: {:.0}%, {} remaining", pct, format_days(effective_remaining)),
                Style::default().fg(Color::Green),
            )));
        } else {
            let projects = state.available_projects(track);

            if projects.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No projects available.",
                    Style::default().fg(Color::DarkGray),
                )));
                let hint = match track {
                    ResearchTrack::Applied => Some("(Identify diseases to unlock medicine development)"),
                    ResearchTrack::Basic => Some("(Identify a pathogen to unlock basic research)"),
                    ResearchTrack::Field => None,
                };
                if let Some(hint_text) = hint {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", hint_text),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            } else {
                has_selectable_items = true;
                for (i, kind) in projects.iter().enumerate() {
                    let selected = state.ui.panel_selection == i;
                    if selected {
                        selected_line = Some(lines.len());
                    }
                    let marker = if selected { "▶ " } else { "  " };
                    let style = if selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    lines.push(Line::from(Span::styled(
                        format!("{}{}", marker, kind.display_label(&state.diseases, &state.medicines, &state.regions)),
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
            }
        }
    }

    // For Applied track: show diseases that are identified but not yet developable,
    // and show a hint when no diseases have been identified at all.
    if track == ResearchTrack::Applied {
        let blocked = state.blocked_medicine_developments();
        if !blocked.is_empty() {
            lines.push(Line::from(""));
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
            // No blocked diseases: either all are covered, or nothing is identified yet.
            // Show a hint if no medicine development work is available or active.
            let has_any_develop = state.applied_research.as_ref()
                .is_some_and(|p| matches!(p.kind, ResearchKind::DevelopMedicine { .. }))
                || state.available_applied_projects()
                    .iter()
                    .any(|k| matches!(k, ResearchKind::DevelopMedicine { .. }));
            let any_identified = state.diseases.iter().any(|d| d.knowledge > 0.0);
            if !has_any_develop && !any_identified {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  No diseases identified yet.",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(Span::styled(
                    "  (Start an Identify Threat project in Field Research.)",
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    let auto_status = if state.auto_research[track.index()] { " ON" } else { " OFF" };
    if has_selectable_items {
        lines.push(Line::from(Span::styled(
            format!("  [↑/↓] Select  [Enter] Confirm  [X] Auto{}", auto_status),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  [Esc] Back",
            Style::default().fg(Color::DarkGray),
        )));
    }

    (title.to_string(), lines, selected_line)
}

fn render_confirm(state: &GameState, track: ResearchTrack, project_idx: usize, double_personnel: bool) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let projects = state.available_projects(track);

    if let Some(kind) = projects.get(project_idx) {
        let (base_personnel, ticks, funding) = state.effective_costs(kind);
        let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };
        let has_personnel = state.personnel_available() >= personnel;
        let has_funding = state.resources.funding >= funding;

        // Breadcrumb
        lines.push(Line::from(Span::styled(
            format!("  Research > {} > Confirm", track_name(track)),
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

fn render_active(state: &GameState, track: ResearchTrack, slot_idx: usize) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let project = match track {
        ResearchTrack::Field => state.field_research.get(slot_idx),
        _ => state.research_slot(track),
    };

    if let Some(project) = project {
        let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
        let remaining = (project.required_ticks - project.progress).max(0.0);

        // Breadcrumb
        lines.push(Line::from(Span::styled(
            format!("  Research > {} > Active", track_name(track)),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            format!("  {}", project.kind.display_label(&state.diseases, &state.medicines, &state.regions)),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));

        // Progress bar
        let bar_width = 30;
        let filled = (pct / 100.0 * bar_width as f64) as usize;
        let empty = bar_width - filled;
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "█".repeat(filled),
                Style::default().fg(Color::Green),
            ),
            Span::styled(
                "░".repeat(empty),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(format!(" {:.0}%", pct)),
        ]));
        let speed = project.speed(&state.medicines);
        let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {} remaining", format_days(effective_remaining)),
            Style::default().fg(Color::White),
        )));
        let speed_color = if speed >= 1.4 { Color::Green } else if speed >= 1.0 { Color::Cyan } else { Color::Red };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} personnel assigned", project.personnel_assigned),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("  ({:.1}x speed)", speed),
                Style::default().fg(speed_color),
            ),
        ]));

        if !project.is_complete() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  [Enter] Back to projects", Style::default().fg(Color::DarkGray))));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  No active project.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc] Back",
        Style::default().fg(Color::DarkGray),
    )));

    let title = match track {
        ResearchTrack::Field => " Active: Field ",
        ResearchTrack::Applied => " Active: Applied ",
        ResearchTrack::Basic => " Active: Basic ",
    };
    (title.to_string(), lines)
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
            if let Some(mech) = med.mechanism {
                let resist_label = if mech.resistance_rate_multiplier() > 1.2 {
                    "High"
                } else if mech.resistance_rate_multiplier() > 0.7 {
                    "Med"
                } else {
                    "Low"
                };
                Some(format!("{}: {} | Eff {:.0}%, Resist: {}",
                    mech.tradeoff_label(),
                    names.join(", "),
                    mech.efficacy_modifier() * 100.0,
                    resist_label))
            } else {
                Some(format!("Targets: {}", names.join(", ")))
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
        _ => None,
    }
}
