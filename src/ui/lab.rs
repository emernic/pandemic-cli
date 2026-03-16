use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{AppState, LabTab, LAB_LEVEL_1_COST, LAB_LEVEL_2_COST, PERSONNEL_UPKEEP_COST, ResearchKind, LabUiState, TherapyType, KNOWLEDGE_FOR_MEDICINE, KNOWLEDGE_FULL, KNOWLEDGE_NAME, TICKS_PER_DAY, TRAIN_PERSONNEL_BATCH, format_days, personnel_speed, ScreeningModality, ScreeningRunSize, WELLS_PER_PLATE};
use crate::ui::hint_line;

/// Maximum selection index for the lab panel in its current sub-state.
pub fn selection_max(ui_state: &LabUiState, state: &AppState) -> usize {
    match ui_state {
        LabUiState::Browse { tab } => {
            if *tab == LabTab::Reactors {
                // Reactors tab: one item per reactor + buy button (if under max)
                let buy_slot = if state.reactors.len() < crate::state::MAX_REACTORS { 1 } else { 0 };
                (state.reactors.len() + buy_slot).saturating_sub(1)
            } else {
                state.lab_tab_items(*tab).len().saturating_sub(1)
            }
        }
        LabUiState::ConfirmProject { .. } | LabUiState::ConfirmLabUpgrade { .. } => 0,
        LabUiState::ScreeningSelectDisease => {
            state.screening_eligible_diseases().len().saturating_sub(1)
        }
        LabUiState::ScreeningSelectModality { .. } => {
            ScreeningModality::ALL.iter()
                .filter(|m| m.is_unlocked(&state.unlocked_techs))
                .count()
                .saturating_sub(1)
        }
        LabUiState::ScreeningSelectSize { .. } => {
            ScreeningRunSize::ALL.iter()
                .filter(|s| s.is_unlocked())
                .count()
                .saturating_sub(1)
        }
        LabUiState::ReactorSelectMedicine { reactor_idx } => {
            let eligible_count = state.reactor_eligible_medicines().len();
            let has_medicine = state.reactors.get(*reactor_idx)
                .map(|r| r.medicine_idx.is_some())
                .unwrap_or(false);
            // +1 for "Clear assignment" option if reactor has a medicine assigned
            let clear_slot = if has_medicine && eligible_count > 0 { 1 } else { 0 };
            (eligible_count + clear_slot).saturating_sub(1)
        }
        LabUiState::TrialSelectHit => {
            state.screening_hits.len().saturating_sub(1)
        }
        LabUiState::TrialSelectRigor { .. } => {
            crate::state::TrialRigor::ALL.len().saturating_sub(1)
        }
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState) {
    let lab_ui = match &state.ui.lab_ui {
        Some(ui) => ui.clone(),
        None => return,
    };

    let tab = lab_ui.tab();

    // Split area: tab bar (4 lines: border + empty + tabs + underline) + content below
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // tab bar
            Constraint::Min(1),   // tab content
        ])
        .split(area);

    render_tab_bar(f, chunks[0], tab);

    match &lab_ui {
        LabUiState::Browse { tab } => {
            if *tab == LabTab::Reactors {
                render_reactors_tab(f, chunks[1], state);
            } else {
                render_tab_content(f, chunks[1], state, *tab);
            }
        }
        LabUiState::ConfirmProject { project_idx, double_personnel, .. } => {
            render_confirm(f, chunks[1], state, *project_idx, *double_personnel);
        }
        LabUiState::ConfirmLabUpgrade { .. } => {
            render_confirm_lab_upgrade(f, chunks[1], state);
        }
        LabUiState::ScreeningSelectDisease
        | LabUiState::ScreeningSelectModality { .. }
        | LabUiState::ScreeningSelectSize { .. } => {
            render_screening_wizard(f, chunks[1], state, &lab_ui);
        }
        LabUiState::ReactorSelectMedicine { reactor_idx } => {
            render_reactor_select_medicine(f, chunks[1], state, *reactor_idx);
        }
        LabUiState::TrialSelectHit
        | LabUiState::TrialSelectRigor { .. } => {
            render_trial_wizard(f, chunks[1], state, &lab_ui);
        }
    }
}

/// Render the tab bar with navigation arrows.
fn render_tab_bar(f: &mut Frame, area: Rect, active_tab: LabTab) {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ◄  ", Style::default().fg(Color::DarkGray)));

    for (i, tab) in LabTab::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        if *tab == active_tab {
            spans.push(Span::styled(
                tab.label(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                tab.label(),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    spans.push(Span::styled("  ► ", Style::default().fg(Color::DarkGray)));

    let tab_line = Line::from(spans);
    // Underline beneath the active tab
    let mut underline_spans: Vec<Span> = Vec::new();
    underline_spans.push(Span::raw("    ")); // match ◄ spacing
    for (i, tab) in LabTab::ALL.iter().enumerate() {
        if i > 0 {
            underline_spans.push(Span::raw("   "));
        }
        if *tab == active_tab {
            underline_spans.push(Span::styled(
                "▔".repeat(tab.label().len()),
                Style::default().fg(Color::White),
            ));
        } else {
            underline_spans.push(Span::raw(" ".repeat(tab.label().len())));
        }
    }

    let lines = vec![
        Line::from(""),
        tab_line,
        Line::from(underline_spans),
    ];

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::TOP)
        .border_style(Style::default().fg(Color::Blue))
        .title(" Lab ");

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

/// Render the content area for the currently active tab.
fn render_tab_content(f: &mut Frame, area: Rect, state: &AppState, tab: LabTab) {
    let items = state.lab_tab_items(tab);
    let available = state.all_available_projects();
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;

    if items.is_empty() {
        lines.push(Line::from(""));
        let msg = match tab {
            LabTab::Sequencing => "  No pathogens to sequence.",
            LabTab::Screening => "  Identify a pathogen first to start screening.",
            LabTab::Trials => "  No trials available.",
            LabTab::Reactors => "  No medicines to manufacture.",
        };
        lines.push(Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray))));
    }

    for (item_idx, item) in items.iter().enumerate() {
        let selected = state.ui.panel_selection == item_idx;
        match item {
            crate::state::ResearchFlatItem::Active(ai) => {
                if let Some(project) = state.active_research.get(*ai) {
                    if selected { selected_line = Some(lines.len()); }
                    render_active_project(&mut lines, project, selected, state);
                }
            }
            crate::state::ResearchFlatItem::Available(avail_idx) => {
                if let Some(kind) = available.get(*avail_idx) {
                    if selected { selected_line = Some(lines.len()); }
                    render_available_project(&mut lines, kind, selected, state);
                }
            }
            crate::state::ResearchFlatItem::FullStockpile(kind) => {
                if selected { selected_line = Some(lines.len()); }
                let marker = if selected { "▶ " } else { "  " };
                let auto_tag = if state.auto_repeat_research.contains(kind) { " AUTO" } else { "" };
                lines.push(Line::from(Span::styled(
                    format!("{}{}{} [FULL]", marker, kind.display_label(&state.diseases, &state.medicines), auto_tag),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }
            crate::state::ResearchFlatItem::ActiveScreening(si) => {
                if let Some(run) = state.screening_runs.get(*si) {
                    if selected { selected_line = Some(lines.len()); }
                    render_active_screening(&mut lines, run, selected, state);
                }
            }
            crate::state::ResearchFlatItem::StartNewScreening => {
                lines.push(Line::from(""));
                if selected { selected_line = Some(lines.len()); }
                let marker = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}[NEW] Start Screening Run", marker),
                    style,
                )));
                lines.push(Line::from(Span::styled(
                    "    Screen compound libraries against identified pathogens",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }
            crate::state::ResearchFlatItem::StartNewTrial => {
                lines.push(Line::from(""));
                if selected { selected_line = Some(lines.len()); }
                let marker = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };
                let hit_count = state.screening_hits.len();
                lines.push(Line::from(Span::styled(
                    format!("{}[NEW] Start Clinical Trial ({} hit{} available)",
                        marker, hit_count, if hit_count == 1 { "" } else { "s" }),
                    style,
                )));
                lines.push(Line::from(Span::styled(
                    "    Select a screening hit and trial rigor level",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }
            crate::state::ResearchFlatItem::UpgradeLab => {
                lines.push(Line::from(""));
                if selected { selected_line = Some(lines.len()); }
                let marker = if selected { "▶ " } else { "  " };
                let upgrade_style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Magenta)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}[PURCHASE] Upgrade Lab", marker),
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
            }
        }
    }

    // Max lab info when fully upgraded (only in Sequencing tab where upgrade lives)
    if tab == LabTab::Sequencing && state.lab_level >= 2 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {} (max) — all research 60% faster", state.lab_level_name()),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Trials tab: show screening hits awaiting trials
    if tab == LabTab::Trials && !state.screening_hits.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── Screening Hits ──",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )));
        // Group hits by disease
        let mut hits_by_disease: std::collections::BTreeMap<usize, Vec<&crate::state::ScreeningHit>> = std::collections::BTreeMap::new();
        for hit in &state.screening_hits {
            hits_by_disease.entry(hit.disease_idx).or_default().push(hit);
        }
        for (d_idx, hits) in &hits_by_disease {
            let disease_name = state.diseases.get(*d_idx)
                .map(|d| d.display_name(*d_idx))
                .unwrap_or_else(|| "?".to_string());
            lines.push(Line::from(Span::styled(
                format!("  {} — {} hit{} awaiting trial", disease_name, hits.len(), if hits.len() == 1 { "" } else { "s" }),
                Style::default().fg(Color::Green),
            )));
            for hit in hits {
                lines.push(Line::from(Span::styled(
                    format!("    {}  {}  Kd: {:.1} nM ({})",
                        hit.compound_id, hit.modality.label(), hit.kd_nm, hit.affinity_label()),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    let hint = if tab == LabTab::Trials {
        "  [↑/↓] Select  [Enter] Start Trial  [D] Discard  [←/→] Tab  [Esc] Close"
    } else {
        "  [↑/↓] Select  [Enter] Confirm  [←/→] Tab  [X] Auto  [Esc] Close"
    };
    lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))));

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

/// Render an active research project (shows progress).
fn render_active_project(lines: &mut Vec<Line<'static>>, project: &crate::state::ResearchProject, selected: bool, state: &AppState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
    let remaining = (project.required_ticks - project.progress).max(0.0);
    let personnel_speed = project.speed(&state.medicines);
    let infra_mult = state.research_infra_multiplier();
    let speed = personnel_speed * infra_mult;
    let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
    let auto_tag = if state.auto_repeat_research.contains(&project.kind) { " AUTO" } else { "" };
    lines.push(Line::from(Span::styled(
        format!("{}[ACTIVE]{} {}", marker, auto_tag, project.kind.display_label(&state.diseases, &state.medicines)),
        style,
    )));
    if let Some(detail) = format_detail(&project.kind, state) {
        lines.push(Line::from(Span::styled(
            format!("    {}", detail),
            Style::default().fg(Color::DarkGray),
        )));
    }
    // Progress bar
    let bar_width = 20;
    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width - filled;
    let bar = format!("{}{}", "▓".repeat(filled), "░".repeat(empty));
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(bar, Style::default().fg(Color::Green)),
        Span::styled(format!("  {:.0}%  {}", pct, format_days(effective_remaining)), Style::default().fg(Color::Green)),
    ]));
    let speed_tag = if (speed - 1.0).abs() < 0.01 {
        String::new()
    } else {
        format!("  {:.1}x speed", speed)
    };
    lines.push(Line::from(Span::styled(
        format!("    {} personnel{}", project.personnel_assigned, speed_tag),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
}

/// Render an available (startable) research project with cost details.
fn render_available_project(lines: &mut Vec<Line<'static>>, kind: &ResearchKind, selected: bool, state: &AppState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let auto_tag = if state.auto_repeat_research.contains(kind) { " AUTO" } else { "" };
    lines.push(Line::from(Span::styled(
        format!("{}{}{}", marker, kind.display_label(&state.diseases, &state.medicines), auto_tag),
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

fn render_confirm(f: &mut Frame, area: Rect, state: &AppState, project_idx: usize, double_personnel: bool) {
    let mut lines: Vec<Line> = Vec::new();
    let projects = state.all_available_projects();

    if let Some(kind) = projects.get(project_idx) {
        let (base_personnel, ticks, funding) = state.effective_costs(kind);
        let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };
        let has_personnel = state.personnel_available() >= personnel;
        let has_funding = state.resources.funding >= funding;

        lines.push(Line::from(Span::styled(
            "  Confirm",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            format!("  Start: {}", kind.display_label(&state.diseases, &state.medicines)),
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

        let checkbox = if double_personnel { "[X]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", checkbox)),
            Span::styled("Assign 2x personnel", Style::default().fg(
                if double_personnel { Color::Yellow } else { Color::DarkGray }
            )),
            Span::styled("  [X] toggle", Style::default().fg(Color::DarkGray)),
        ]));

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

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_confirm_lab_upgrade(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    let (cost, next_name, pct) = if state.lab_level == 0 {
        (LAB_LEVEL_1_COST, "Enhanced Sequencing", 30)
    } else {
        (LAB_LEVEL_2_COST, "Advanced Genomics Center", 60)
    };
    let can_afford = state.resources.funding >= cost;

    lines.push(Line::from(Span::styled(
        "  Lab Upgrade > Confirm",
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

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

/// Render an active screening run with well-plate visualization.
fn render_active_screening(lines: &mut Vec<Line<'static>>, run: &crate::state::ScreeningRun, selected: bool, state: &AppState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let disease_name = state.diseases.get(run.disease_idx)
        .map(|d| d.display_name(run.disease_idx))
        .unwrap_or_else(|| "?".to_string());
    lines.push(Line::from(Span::styled(
        format!("{}[ACTIVE] {} vs {}", marker, run.modality.label(), disease_name),
        style,
    )));

    // Well plate visualization
    let wells_tested = run.wells_tested();
    let total = run.total_wells();
    let hits_found = run.hits.len();

    // Show compact plate grid: 12 symbols per plate (each represents 8 wells)
    let plates = run.plates();
    let mut plate_spans: Vec<Span> = vec![Span::raw("    ")];
    for plate in 0..plates {
        if plate > 0 {
            plate_spans.push(Span::raw("  "));
        }
        let plate_start = plate * WELLS_PER_PLATE;
        let plate_end = plate_start + WELLS_PER_PLATE;
        // 12 symbols per plate, each represents 8 wells
        let mut plate_str = String::new();
        for sym_idx in 0..12 {
            let well_start = plate_start + sym_idx * 8;
            let well_end = (well_start + 8).min(plate_end);
            if well_end <= wells_tested {
                // Show ◉ if any stored hit falls within this symbol's well range
                let has_hit = run.hits.iter().any(|h| h.well_index >= well_start && h.well_index < well_end);
                if has_hit {
                    plate_str.push('◉');
                } else {
                    plate_str.push('●');
                }
            } else if well_start < wells_tested {
                plate_str.push('●'); // partially tested
            } else {
                plate_str.push('○');
            }
        }
        let plate_color = if plate_end <= wells_tested {
            Color::Green
        } else if plate_start < wells_tested {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        plate_spans.push(Span::styled(plate_str, Style::default().fg(plate_color)));
    }
    lines.push(Line::from(plate_spans));

    // Plate labels
    let mut label_spans: Vec<Span> = vec![Span::raw("    ")];
    for plate in 0..plates {
        if plate > 0 {
            label_spans.push(Span::raw("  "));
        }
        label_spans.push(Span::styled(
            format!("plate {:>2}    ", plate + 1),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(label_spans));

    // Status line
    let pct = (run.progress / run.required_ticks * 100.0).min(100.0);
    let remaining = (run.required_ticks - run.progress).max(0.0);
    let speed = personnel_speed(run.personnel_assigned, run.run_size.personnel());
    let infra_mult = state.research_infra_multiplier();
    let effective_remaining = if speed * infra_mult > 0.0 {
        remaining / (speed * infra_mult)
    } else {
        remaining
    };
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("Screened: {}/{}", wells_tested, total),
            Style::default().fg(Color::White),
        ),
        Span::raw("   "),
        Span::styled(
            format!("Hits: ◉ {}", hits_found),
            Style::default().fg(if hits_found > 0 { Color::Green } else { Color::DarkGray }),
        ),
        Span::raw("   "),
        Span::styled(
            format!("{} staff", run.personnel_assigned),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("   "),
        Span::styled(
            format!("{:.0}%  {}", pct, format_days(effective_remaining)),
            Style::default().fg(Color::Green),
        ),
    ]));
    lines.push(Line::from(""));
}

/// Render the screening wizard (disease → modality → run size).
fn render_screening_wizard(f: &mut Frame, area: Rect, state: &AppState, lab_ui: &LabUiState) {
    let mut lines: Vec<Line> = Vec::new();

    match lab_ui {
        LabUiState::ScreeningSelectDisease => {
            lines.push(Line::from(Span::styled(
                "  New Screening Run > Select Target",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── Target Disease ──",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            let eligible = state.screening_eligible_diseases();
            if eligible.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No identified diseases to screen against.",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            for (i, &d_idx) in eligible.iter().enumerate() {
                let selected = state.ui.panel_selection == i;
                let marker = if selected { "▶ " } else { "  " };
                let disease = &state.diseases[d_idx];
                let name = disease.display_name(d_idx);
                let knowledge_pct = (disease.knowledge * 100.0) as u32;
                let status = if knowledge_pct >= 100 {
                    "fully sequenced".to_string()
                } else {
                    format!("{}% knowledge", knowledge_pct)
                };
                let style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}{}", marker, name), style),
                    Span::styled(format!("  ({})", status), Style::default().fg(Color::DarkGray)),
                ]));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  [Enter] Select  [Esc] Cancel",
                Style::default().fg(Color::DarkGray),
            )));
        }
        LabUiState::ScreeningSelectModality { disease_idx } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "?".to_string());
            lines.push(Line::from(Span::styled(
                format!("  New Screening Run > {} > Modality", disease_name),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── Modality ──",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            let mut selectable_idx = 0;
            for modality in ScreeningModality::ALL.iter() {
                let unlocked = modality.is_unlocked(&state.unlocked_techs);
                if unlocked {
                    let selected = state.ui.panel_selection == selectable_idx;
                    let marker = if selected { "▶ " } else { "  " };
                    let style = if selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}{}", marker, modality.label()), style),
                        Span::styled(
                            format!("  {}", modality.description()),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    selectable_idx += 1;
                } else {
                    let lock_reason = modality.required_tech()
                        .map(|t| format!("need {}", t.name()))
                        .unwrap_or_else(|| "coming soon".to_string());
                    lines.push(Line::from(Span::styled(
                        format!("    {} [LOCKED — {}]", modality.label(), lock_reason),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  [Enter] Select  [Esc] Back",
                Style::default().fg(Color::DarkGray),
            )));
        }
        LabUiState::ScreeningSelectSize { disease_idx, modality } => {
            let disease_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "?".to_string());
            lines.push(Line::from(Span::styled(
                format!("  New Screening Run > {} > {} > Run Size", disease_name, modality.label()),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── Run Size ──",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            let mut selectable_idx = 0;
            for size in ScreeningRunSize::ALL.iter() {
                if size.is_unlocked() {
                    let selected = state.ui.panel_selection == selectable_idx;
                    let marker = if selected { "▶ " } else { "  " };
                    let style = if selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let can_afford = state.resources.funding >= size.funding_cost();
                    let has_personnel = state.personnel_available() >= size.personnel();
                    let cost_color = if can_afford { Color::Yellow } else { Color::Red };
                    let pers_color = if has_personnel { Color::Cyan } else { Color::Red };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {}◆ {:<8}", marker, size.label()),
                            style,
                        ),
                        Span::styled(
                            format!("{} plates ({} compounds)", size.plates(), size.total_wells()),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    lines.push(Line::from(vec![
                        Span::raw("          "),
                        Span::styled(
                            format!("~{}  ", format_days(size.base_ticks())),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("{} staff  ", size.personnel()),
                            Style::default().fg(pers_color),
                        ),
                        Span::styled(
                            format!("¥{:.0}", size.funding_cost()),
                            Style::default().fg(cost_color),
                        ),
                    ]));
                    selectable_idx += 1;
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("    ◇ {} [LOCKED — need HTS tech]", size.label()),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  [Enter] Begin Run  [Esc] Back",
                Style::default().fg(Color::DarkGray),
            )));
        }
        _ => {} // unreachable for other states
    }

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

/// Render the trial wizard (hit selection → rigor selection).
fn render_trial_wizard(f: &mut Frame, area: Rect, state: &AppState, lab_ui: &LabUiState) {
    let selected_idx = state.ui.panel_selection;
    let mut lines: Vec<Line<'static>> = Vec::new();

    match lab_ui {
        LabUiState::TrialSelectHit => {
            lines.push(Line::from(Span::styled(
                "  Clinical Trial > Select Screening Hit",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            if state.screening_hits.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No screening hits available. Run screening first.",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                for (i, hit) in state.screening_hits.iter().enumerate() {
                    let selected = i == selected_idx;
                    let marker = if selected { "▶ " } else { "  " };
                    let style = if selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let disease_name = state.diseases.get(hit.disease_idx)
                        .map(|d| d.display_name(hit.disease_idx))
                        .unwrap_or_else(|| "?".to_string());
                    lines.push(Line::from(Span::styled(
                        format!("{}{}  {}  Kd: {:.1} nM ({})",
                            marker, hit.compound_id, hit.modality.label(),
                            hit.kd_nm, hit.affinity_label()),
                        style,
                    )));
                    lines.push(Line::from(Span::styled(
                        format!("    vs {}",  disease_name),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  [Enter] Select  [D] Discard  [Esc] Back",
                Style::default().fg(Color::DarkGray),
            )));
        }
        LabUiState::TrialSelectRigor { hit_index } => {
            let hit = state.screening_hits.get(*hit_index);
            let compound_id = hit.map(|h| h.compound_id.as_str()).unwrap_or("?");

            lines.push(Line::from(Span::styled(
                format!("  Clinical Trial > {} > Select Rigor", compound_id),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Higher rigor = accurate stats but slower and more expensive.",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "  Lower rigor = fast and cheap but reported stats may be wrong.",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));

            for (i, rigor) in crate::state::TrialRigor::ALL.iter().enumerate() {
                let selected = i == selected_idx;
                let marker = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let (personnel, duration, funding) = rigor.costs();
                let days = duration / crate::state::TICKS_PER_DAY;
                lines.push(Line::from(Span::styled(
                    format!("{}{}", marker, rigor.label()),
                    style,
                )));
                lines.push(Line::from(Span::styled(
                    format!("    {} — ¥{:.0}, {} personnel, {:.1} days",
                        rigor.description(), funding, personnel, days),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  [Enter] Begin Trial  [Esc] Back",
                Style::default().fg(Color::DarkGray),
            )));
        }
        _ => {}
    }

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

/// Supplementary detail line for a research project.
fn format_detail(kind: &ResearchKind, state: &AppState) -> Option<String> {
    match kind {
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let current = crate::format_number(med.doses);
            let target = crate::format_number(med.max_doses);
            Some(format!("{} → {} doses", current, target))
        }
        ResearchKind::GenomicSequencing { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            let current_rate = disease.effective_variant_rate();
            let new_rate = current_rate * 0.5;
            Some(format!("Variant spawn rate: {:.4} → {:.4}", current_rate, new_rate))
        }
        ResearchKind::TrainPersonnel => {
            let added_upkeep = TRAIN_PERSONNEL_BATCH as f64 * PERSONNEL_UPKEEP_COST * TICKS_PER_DAY;
            Some(format!("Current: {} personnel (+¥{:.0}/day upkeep after)",
                state.resources.personnel, added_upkeep))
        }
        ResearchKind::IdentifyThreat { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            if disease.knowledge >= KNOWLEDGE_NAME {
                let has_targeted_tech = state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign);
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
        ResearchKind::ClinicalTrial { medicine_idx, rigor, .. } => {
            let med = state.medicines.get(*medicine_idx)?;
            let eff_label = med.reported_efficacy.as_deref().unwrap_or("???");
            let se_label = med.reported_side_effects.as_deref().unwrap_or("???");
            Some(format!("{} | Eff: {}  Side Fx: {}", rigor.label(), eff_label, se_label))
        }
    }
}

/// Render the Reactors tab with ASCII art reactor vessels.
fn render_reactors_tab(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;

    let reactor_count = state.reactors.len();
    let can_buy = reactor_count < crate::state::MAX_REACTORS;

    // Header line
    let header_text = format!("  {} production reactor{}", reactor_count,
        if reactor_count == 1 { "" } else { "s" });
    let mut header_spans: Vec<Span> = vec![
        Span::styled(header_text, Style::default().fg(Color::White)),
    ];
    if can_buy {
        let can_afford = state.resources.funding >= crate::state::REACTOR_COST;
        header_spans.push(Span::raw("    "));
        header_spans.push(Span::styled(
            format!("Buy reactor (¥{:.0})", crate::state::REACTOR_COST),
            Style::default().fg(if can_afford { Color::Green } else { Color::DarkGray }),
        ));
    }
    lines.push(Line::from(header_spans));
    lines.push(Line::from(""));

    // Render each reactor as ASCII art
    for (i, reactor) in state.reactors.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        if selected { selected_line = Some(lines.len()); }
        render_reactor_vessel(&mut lines, reactor, selected, state);
    }

    // Buy reactor button (at index == reactor_count)
    if can_buy {
        let buy_selected = state.ui.panel_selection == reactor_count;
        if buy_selected { selected_line = Some(lines.len()); }
        let marker = if buy_selected { "▶ " } else { "  " };
        let can_afford = state.resources.funding >= crate::state::REACTOR_COST;
        let style = if buy_selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if can_afford {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(
            format!("{}[+] Buy Reactor (¥{:.0})", marker, crate::state::REACTOR_COST),
            style,
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "  [↑/↓] Select  [Enter] Start  [C] Change Medicine  [X] Cycle Auto  [Esc] Close",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

/// Render a single reactor vessel as ASCII art.
fn render_reactor_vessel(lines: &mut Vec<Line<'static>>, reactor: &crate::state::Reactor, selected: bool, state: &AppState) {
    let marker = if selected { "▶ " } else { "  " };

    let (med_name, batch_progress_pct, status_line) = if let Some(med_idx) = reactor.medicine_idx {
        let med = state.medicines.get(med_idx);
        let name = med.map(|m| m.name.as_str()).unwrap_or("Unknown");

        if reactor.active {
            // Active batch — show progress
            let pct = if reactor.batch_required > 0.0 {
                (reactor.batch_progress / reactor.batch_required * 100.0).min(100.0)
            } else { 0.0 };
            let remaining = (reactor.batch_required - reactor.batch_progress).max(0.0);
            let base_personnel = {
                let kind = ResearchKind::ManufactureDoses { medicine_idx: med_idx };
                let (p, _, _) = kind.costs(&state.medicines);
                p
            };
            let speed = personnel_speed(reactor.personnel_assigned, base_personnel)
                * state.research_infra_multiplier();
            let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
            let target = med.map(|m| m.max_doses).unwrap_or(0.0);
            (name.to_string(), Some(pct / 100.0),
             format!("batch {:.0}%  ▣ {} doses  {}", pct, crate::format_number(target),
                 format_days(effective_remaining)))
        } else {
            // Idle — reactor is empty, show stockpile info in status text only
            let current = med.map(|m| m.doses).unwrap_or(0.0);
            let max = med.map(|m| m.max_doses).unwrap_or(1.0);
            let status = if current >= max { "FULL" } else { "idle" };
            let hint = if current >= max { "[C] change medicine" } else { "[Enter] start  [C] change" };
            (name.to_string(), None,
             format!("{} doses ({})  {}", crate::format_number(current), status, hint))
        }
    } else {
        ("empty".to_string(), None, "[Enter] assign medicine".to_string())
    };

    // ASCII art reactor vessel (5 rows tall, 11 chars wide)
    // Top ~1/3 is always headspace; material fills the bottom ~2/3
    let vessel_height = 5;
    let max_fill_rows = (vessel_height * 2 + 2) / 3; // 4 out of 5 rows max (headspace = 1 row)

    let is_active = reactor.active;
    let (top_border, bottom_border, side_l, side_r) = if is_active {
        ("┏━━━━━━━━━┓", "┗━━━━┯━━━━┛", "┃", "┃")
    } else {
        ("┌─────────┐", "└─────────┘", "│", "│")
    };

    // Vessel top
    let top_style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if is_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    lines.push(Line::from(vec![
        Span::raw(format!("{}   ", marker)),
        Span::styled(top_border.to_string(), top_style),
    ]));

    // Vessel body rows (bottom-up fill)
    // Active batch: raw material (░/▒) fills bottom 2/3, solid (▓/█) replaces from bottom as batch progresses
    // Idle/empty: vessel is empty
    for row in 0..vessel_height {
        let row_from_bottom = vessel_height - 1 - row;
        let (fill_char, fill_color) = if let Some(progress) = batch_progress_pct {
            // Active batch: material rows fill the bottom max_fill_rows
            if row_from_bottom < max_fill_rows {
                // How many rows are "reacted" (solid) — progress converts fuzzy to solid from bottom
                let solid_rows = (progress * max_fill_rows as f64).round() as usize;
                if row_from_bottom < solid_rows {
                    // Solid fill (reacted product)
                    ('█', Color::Green)
                } else {
                    // Fuzzy fill (raw material)
                    ('░', Color::Yellow)
                }
            } else {
                (' ', Color::DarkGray)
            }
        } else {
            // Idle or no medicine — empty vessel
            (' ', Color::DarkGray)
        };
        let fill_str: String = std::iter::repeat(fill_char).take(9).collect();
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(side_l.to_string(), top_style),
            Span::styled(fill_str, Style::default().fg(fill_color)),
            Span::styled(side_r.to_string(), top_style),
        ]));
    }

    // Vessel bottom
    lines.push(Line::from(vec![
        Span::raw("     "),
        Span::styled(bottom_border.to_string(), top_style),
    ]));

    // Pipe connector for active
    if is_active {
        lines.push(Line::from(vec![
            Span::raw("          "),
            Span::styled("│", top_style),
        ]));
    }

    // Medicine name
    let name_style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if reactor.medicine_idx.is_some() {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(Span::styled(
        format!("     {}", med_name),
        name_style,
    )));

    // Status line
    lines.push(Line::from(Span::styled(
        format!("     {}", status_line),
        Style::default().fg(Color::DarkGray),
    )));

    // Auto-deploy and repeat toggles
    if reactor.medicine_idx.is_some() {
        let auto_tag = if reactor.auto_deploy { "[X] auto-deploy" } else { "[ ] auto-deploy" };
        let repeat_tag = if reactor.repeat { "[X] repeat" } else { "[ ] repeat" };
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(auto_tag, Style::default().fg(
                if reactor.auto_deploy { Color::Green } else { Color::DarkGray }
            )),
            Span::raw("  "),
            Span::styled(repeat_tag, Style::default().fg(
                if reactor.repeat { Color::Green } else { Color::DarkGray }
            )),
        ]));
    }

    lines.push(Line::from(""));
}

/// Render the medicine selection wizard for reactor configuration.
fn render_reactor_select_medicine(f: &mut Frame, area: Rect, state: &AppState, reactor_idx: usize) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "  Configure Reactor > Select Medicine",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let eligible = state.reactor_eligible_medicines();
    if eligible.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No medicines available. Develop a medicine first.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    for (i, &med_idx) in eligible.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };
        let med = &state.medicines[med_idx];
        let current = crate::format_number(med.doses);
        let target = crate::format_number(med.max_doses);
        let full = med.doses >= med.max_doses;

        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {}{}", marker, med.name), style),
            Span::styled(
                format!("  ({}/{} doses{})", current, target, if full { " FULL" } else { "" }),
                Style::default().fg(if full { Color::DarkGray } else { Color::Cyan }),
            ),
        ]));

        // Second line: therapy type, target diseases, reported stats
        let therapy_label = med.therapy_type.label();
        let targets: String = if med.target_diseases.is_empty() {
            "All".to_string()
        } else {
            med.target_diseases.iter()
                .filter_map(|&d| state.diseases.get(d).map(|dis| dis.name.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let eff_label = med.reported_efficacy.as_deref().unwrap_or("???");
        let se_label = med.reported_side_effects.as_deref().unwrap_or("—");
        let detail_style = Style::default().fg(Color::DarkGray);
        lines.push(Line::from(Span::styled(
            format!("      {} | {} | Eff: {} | Side Fx: {}", therapy_label, targets, eff_label, se_label),
            detail_style,
        )));
    }

    // Show "Clear assignment" option if reactor currently has a medicine
    let has_medicine = state.reactors.get(reactor_idx)
        .map(|r| r.medicine_idx.is_some())
        .unwrap_or(false);
    if has_medicine && !eligible.is_empty() {
        let clear_idx = eligible.len();
        let selected = state.ui.panel_selection == clear_idx;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(
            format!("  {}Clear assignment", marker),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Enter] Select  [Esc] Cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Blue));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
