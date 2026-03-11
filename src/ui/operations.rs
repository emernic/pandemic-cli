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
    FIELD_OP_TYPE_COUNT, DECREE_COUNT,
    decree_display_name,
};
use super::hint_line;
use super::policy::{decree_description, render_confirm_decree, render_region_select};

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    match &state.ui.operations_ui {
        Some(OpsUiState::BrowseOps) | None => render_browse(f, area, state),
        Some(OpsUiState::SelectReconTarget) => render_select_recon(f, area, state),
        Some(OpsUiState::SelectEmergencyTarget) => render_select_region(f, area, state, "EMERGENCY RESPONSE", None),
        Some(OpsUiState::SelectSurveyTarget) => render_select_region(f, area, state, "INFRA SURVEY", None),
        Some(OpsUiState::SelectSupplyTarget) => render_select_region(f, area, state, "SUPPLY REINFORCEMENT", Some(InfraDetail::SupplyLines)),
        Some(OpsUiState::SelectCivilOrderTarget) => render_select_region(f, area, state, "CIVIL STABILIZATION", Some(InfraDetail::CivilOrder)),
        Some(OpsUiState::ConfirmDecree { decree_idx }) => {
            let (title, lines, _) = render_confirm_decree(state, *decree_idx);
            render_panel(f, area, &title, lines);
        }
        Some(OpsUiState::SelectSacrificeRegion) => {
            let (title, lines, _) = render_region_select(
                state,
                "SACRIFICE REGION",
                "Sacrifice",
                "Choose a region to abandon. Its population is written off; income from remaining regions increases permanently.",
            );
            render_panel(f, area, &title, lines);
        }
        Some(OpsUiState::SelectFortifyRegion) => {
            let (title, lines, _) = render_region_select(
                state,
                "FORTIFY REGION",
                "Fortify",
                "Choose a region to restore to full infrastructure. All other regions suffer a permanent infrastructure penalty.",
            );
            render_panel(f, area, &title, lines);
        }
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
    let has_active = !state.field_operations.is_empty() || !state.crisis_operations.is_empty();
    if has_active {
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
        for op in &state.crisis_operations {
            let days_left = op.ticks_remaining / TICKS_PER_DAY;
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(&op.label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" ({:.1}d left, {} personnel tied up)", days_left, op.personnel),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        lines.push(Line::raw(""));
    }

    // Available operations
    lines.push(Line::from(Span::styled(
        "  Deploy Operations",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: array size must equal FIELD_OP_TYPE_COUNT, which bounds panel navigation
    // and matches the 0..=4 arms in handle_operations_confirm() (state.rs).
    let ops: [(&str, &str, u32, f64, Option<f64>); FIELD_OP_TYPE_COUNT] = [
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

    // Emergency Decrees
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  Emergency Decrees",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: loop count must equal DECREE_COUNT
    for decree_idx in 0..DECREE_COUNT {
        let is_selected = row == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let name = decree_display_name(decree_idx);
        let enacted = state.enacted_decrees.is_enacted(decree_idx);
        let unlocked = state.decree_unlocked(decree_idx);

        let (name_color, desc_color) = if enacted {
            (Color::DarkGray, Color::DarkGray)
        } else if is_selected {
            (Color::Yellow, Color::DarkGray)
        } else if unlocked {
            (Color::Red, Color::DarkGray)
        } else {
            (Color::DarkGray, Color::DarkGray)
        };

        let suffix = if enacted {
            " [ENACTED]".to_string()
        } else if !unlocked {
            format!(" [{}]", GameState::decree_unlock_hint(decree_idx))
        } else {
            String::new()
        };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(name, Style::default().fg(name_color).add_modifier(Modifier::BOLD)),
            Span::styled(suffix, Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(decree_description(decree_idx), Style::default().fg(desc_color)),
        ]));
        row += 1;
    }

    // Standing Orders
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  Standing Orders",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: must equal STANDING_ORDER_COUNT (= 2)
    let standing_orders = [
        (
            "Auto-Quarantine at HIGH",
            "Automatically enable Quarantine when infections exceed HIGH threshold (10K).",
            state.standing_orders.auto_quarantine_at_high,
        ),
        (
            "Auto-Travel Ban at CRIT",
            "Automatically enable Travel Ban when infections exceed CRIT threshold (100K).",
            state.standing_orders.auto_travel_ban_at_crit,
        ),
    ];

    for (name, desc, enabled) in &standing_orders {
        let is_selected = row == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let name_color = if is_selected { Color::Yellow } else { Color::White };
        let status = if *enabled { "[ON] " } else { "[OFF]" };
        let status_color = if *enabled { Color::Green } else { Color::DarkGray };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(status, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::styled(" ", Style::default()),
            Span::styled(*name, Style::default().fg(name_color).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(*desc, Style::default().fg(Color::DarkGray)),
        ]));
        row += 1;
    }

    // Outstanding Loans
    if !state.loans.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  Outstanding Loans",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));

        for loan in &state.loans {
            let is_selected = row == selected;
            let marker = if is_selected { "▸ " } else { "  " };
            let highlight = if is_selected { Color::Yellow } else { Color::White };
            let interest_per_day = loan.interest_per_tick() * TICKS_PER_DAY;
            let days_left = (loan.due_day - state.tick as f64 / TICKS_PER_DAY).max(0.0);

            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Yellow)),
                Span::styled(&loan.lender_name, Style::default().fg(highlight).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" — ¥{:.0} outstanding (+¥{:.1}/day, {:.1}d left)",
                        loan.outstanding, interest_per_day, days_left),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            row += 1;
        }
    }

    let _ = row; // suppress unused variable warning

    lines.push(hint_line(state, "Select", "Close"));

    let block = Block::default()
        .title(" ORDERS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_panel(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>) {
    let block = Block::default()
        .title(title.to_string())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

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
