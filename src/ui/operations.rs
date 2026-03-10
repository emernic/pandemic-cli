use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, KNOWLEDGE_NAME, OpsUiState, TICKS_PER_DAY,
    OP_RECON_PERSONNEL, OP_RECON_TICKS,
    OP_EMERGENCY_PERSONNEL, OP_EMERGENCY_TICKS,
    OP_SURVEY_PERSONNEL, OP_SURVEY_TICKS,
};
use super::hint_line;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    match &state.ui.operations_ui {
        Some(OpsUiState::BrowseOps) | None => render_browse(f, area, state),
        Some(OpsUiState::SelectReconTarget) => render_select_recon(f, area, state),
        Some(OpsUiState::SelectEmergencyTarget) => render_select_region(f, area, state, "EMERGENCY RESPONSE"),
        Some(OpsUiState::SelectSurveyTarget) => render_select_region(f, area, state, "INFRA SURVEY"),
    }
}

fn render_browse(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let selected = state.ui.panel_selection;
    let mut row = 0;

    // Active operations
    if !state.field_operations.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Active Operations",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        for op in &state.field_operations {
            let is_selected = row == selected;
            let marker = if is_selected { "▸ " } else { "  " };
            let days_left = op.ticks_remaining / TICKS_PER_DAY;
            let progress = 1.0 - (op.ticks_remaining / op.total_ticks);

            let target_desc = match &op.kind {
                crate::state::FieldOpKind::Recon { disease_idx } => {
                    state.diseases.get(*disease_idx)
                        .map(|d| d.display_name(*disease_idx))
                        .unwrap_or_else(|| "?".to_string())
                }
                crate::state::FieldOpKind::EmergencyResponse { region_idx } => {
                    state.regions.get(*region_idx)
                        .map(|r| r.name.clone())
                        .unwrap_or_else(|| "?".to_string())
                }
                crate::state::FieldOpKind::InfraSurvey { region_idx } => {
                    state.regions.get(*region_idx)
                        .map(|r| r.name.clone())
                        .unwrap_or_else(|| "?".to_string())
                }
            };

            let highlight = if is_selected { Color::Yellow } else { Color::White };
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Yellow)),
                Span::styled(op.kind.label(), Style::default().fg(highlight).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" → {} ({:.0}%, {:.1}d left, {} personnel)",
                        target_desc, progress * 100.0, days_left, op.personnel),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            row += 1;
        }
        lines.push(Line::raw(""));
    }

    // Available operations
    lines.push(Line::from(Span::styled(
        "  Deploy Operations",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));

    let ops: [(&str, &str, u32, f64); 3] = [
        ("Recon Mission", "Identify unknown pathogen", OP_RECON_PERSONNEL, OP_RECON_TICKS),
        ("Emergency Response", "Reduce lethality in a region", OP_EMERGENCY_PERSONNEL, OP_EMERGENCY_TICKS),
        ("Infra Survey", "Repair worst infrastructure", OP_SURVEY_PERSONNEL, OP_SURVEY_TICKS),
    ];

    let available = state.personnel_available();

    for (name, desc, personnel, ticks) in &ops {
        let is_selected = row == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let highlight = if is_selected { Color::Yellow } else { Color::White };
        let days = *ticks / TICKS_PER_DAY;
        let cost = format!("{} personnel, {:.1} days", personnel, days);

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(*name, Style::default().fg(highlight).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(*desc, Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(cost, Style::default().fg(Color::DarkGray)),
        ]));
        row += 1;
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("  Personnel: {} available / {} total",
                available, state.resources.personnel),
            Style::default().fg(if available > 0 { Color::Green } else { Color::Red }),
        ),
    ]));

    lines.push(hint_line(state, "Select", "Close"));

    let block = Block::default()
        .title(" FIELD OPERATIONS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_select_recon(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let selected = state.ui.panel_selection;

    lines.push(Line::from(Span::styled(
        "  Select target pathogen",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::raw(""));

    let targets: Vec<(usize, &crate::state::Disease)> = state.diseases.iter().enumerate()
        .filter(|(_, d)| d.detected && d.knowledge < KNOWLEDGE_NAME)
        .collect();

    for (i, (d_idx, disease)) in targets.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let highlight = if is_selected { Color::Yellow } else { Color::White };
        let knowledge_pct = (disease.knowledge * 100.0) as u32;

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(
                disease.display_name(*d_idx),
                Style::default().fg(highlight).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({}% identified)", knowledge_pct),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(hint_line(state, "Deploy", "Back"));

    let block = Block::default()
        .title(" RECON MISSION ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_select_region(f: &mut Frame, area: Rect, state: &GameState, title: &str) {
    let mut lines: Vec<Line> = Vec::new();
    let selected = state.ui.panel_selection;

    lines.push(Line::from(Span::styled(
        "  Select target region",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::raw(""));

    let non_collapsed: Vec<(usize, &crate::state::Region)> = state.regions.iter().enumerate()
        .filter(|(_, r)| !r.collapsed)
        .collect();

    for (i, (_, region)) in non_collapsed.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let highlight = if is_selected { Color::Yellow } else { Color::White };

        let total_infected: f64 = region.infections.iter().map(|inf| inf.infected).sum();
        let infected_str = crate::format_number(total_infected);

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(
                &region.name,
                Style::default().fg(highlight).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({} infected)", infected_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(hint_line(state, "Deploy", "Back"));

    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
