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
    OP_SUPPLY_PERSONNEL, OP_SUPPLY_TICKS, OP_SUPPLY_COST,
    OP_CIVIL_PERSONNEL, OP_CIVIL_TICKS, OP_CIVIL_COST,
};
use super::hint_line;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    match &state.ui.operations_ui {
        Some(OpsUiState::BrowseOps) | None => render_browse(f, area, state),
        Some(OpsUiState::SelectReconTarget) => render_select_recon(f, area, state),
        Some(OpsUiState::SelectEmergencyTarget) => render_select_region(f, area, state, "EMERGENCY RESPONSE", None),
        Some(OpsUiState::SelectSurveyTarget) => render_select_region(f, area, state, "INFRA SURVEY", None),
        Some(OpsUiState::SelectSupplyTarget) => render_select_region(f, area, state, "SUPPLY REINFORCEMENT", Some(InfraDetail::SupplyLines)),
        Some(OpsUiState::SelectCivilOrderTarget) => render_select_region(f, area, state, "CIVIL STABILIZATION", Some(InfraDetail::CivilOrder)),
    }
}

/// What infrastructure detail to show next to each region in the selection screen.
enum InfraDetail {
    SupplyLines,
    CivilOrder,
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
                crate::state::FieldOpKind::InfraSurvey { region_idx }
                | crate::state::FieldOpKind::SupplyChainReinforcement { region_idx }
                | crate::state::FieldOpKind::CivilOrderStabilization { region_idx } => {
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

    let ops: [(&str, &str, u32, f64, Option<f64>); 5] = [
        ("Recon Mission", "Identify unknown pathogen", OP_RECON_PERSONNEL, OP_RECON_TICKS, None),
        ("Emergency Response", "Reduce lethality in a region", OP_EMERGENCY_PERSONNEL, OP_EMERGENCY_TICKS, None),
        ("Infra Survey", "Repair worst infrastructure", OP_SURVEY_PERSONNEL, OP_SURVEY_TICKS, None),
        ("Supply Reinforcement", "Restore supply lines + permanent resilience", OP_SUPPLY_PERSONNEL, OP_SUPPLY_TICKS, Some(OP_SUPPLY_COST)),
        ("Civil Stabilization", "Restore civil order + permanent resilience", OP_CIVIL_PERSONNEL, OP_CIVIL_TICKS, Some(OP_CIVIL_COST)),
    ];

    let available = state.personnel_available();

    for (name, desc, personnel, ticks, funding_cost) in &ops {
        let is_selected = row == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let highlight = if is_selected { Color::Yellow } else { Color::White };
        let days = *ticks / TICKS_PER_DAY;
        let cost = if let Some(fc) = funding_cost {
            format!("{} personnel, {:.1} days, ¥{:.0}", personnel, days, fc)
        } else {
            format!("{} personnel, {:.1} days", personnel, days)
        };

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

fn render_select_region(f: &mut Frame, area: Rect, state: &GameState, title: &str, detail: Option<InfraDetail>) {
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

        let detail_str = match &detail {
            Some(InfraDetail::SupplyLines) => {
                let pct = (region.supply_lines * 100.0) as u32;
                let res = (region.supply_resilience * 100.0) as u32;
                if res > 0 {
                    format!("  (supply: {}%, resilience: {}%)", pct, res)
                } else {
                    format!("  (supply lines: {}%)", pct)
                }
            }
            Some(InfraDetail::CivilOrder) => {
                let pct = (region.civil_order * 100.0) as u32;
                let res = (region.civil_resilience * 100.0) as u32;
                if res > 0 {
                    format!("  (civil order: {}%, resilience: {}%)", pct, res)
                } else {
                    format!("  (civil order: {}%)", pct)
                }
            }
            None => {
                let infected_str = crate::format_number(region.estimated_infected);
                format!("  ({} infected)", infected_str)
            }
        };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(
                &region.name,
                Style::default().fg(highlight).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                detail_str,
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
