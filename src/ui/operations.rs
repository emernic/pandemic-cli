use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, OpsUiState, TICKS_PER_DAY,
    DECREE_COUNT, DECREE_APPROVAL_COSTS,
    decree_display_name,
};
use super::hint_line;
use super::policy::{decree_description, render_confirm_decree, render_region_select};

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    match &state.ui.operations_ui {
        Some(OpsUiState::BrowseOps) | None => render_browse(f, area, state),
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

fn render_browse(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let selected = state.ui.panel_selection;
    let mut row = 0;

    // Crisis operations (temporary personnel commitments)
    if !state.crisis_operations.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Active Operations",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
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

    // Emergency Decrees
    lines.push(Line::from(Span::styled(
        "  Emergency Decrees",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: loop count must equal DECREE_COUNT
    for decree_idx in 0..DECREE_COUNT {
        let is_selected = row == selected;
        let marker = if is_selected { "▶ " } else { "  " };
        let name = decree_display_name(decree_idx);
        let enacted = state.enacted_decrees.is_enacted(decree_idx);
        let unlocked = state.decree_unlocked(decree_idx);

        if !enacted && !unlocked {
            // Locked — show 🔒 icon with unlock hint, matching Policies panel style
            let name_style = if is_selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let hint = GameState::decree_unlock_hint(decree_idx);
            lines.push(Line::from(vec![
                Span::styled(marker, name_style),
                Span::styled("🔒 ", Style::default().fg(Color::DarkGray)),
                Span::styled(name, name_style),
                Span::styled(
                    format!("  ({})", hint),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(""));
        } else {
            // Enacted or unlocked — show with description
            let cost_pct = (DECREE_APPROVAL_COSTS[decree_idx] * 100.0) as u32;
            let (name_color, desc_color, suffix) = if enacted {
                (Color::DarkGray, Color::DarkGray, " [ENACTED]".to_string())
            } else if is_selected {
                (Color::Yellow, Color::DarkGray, format!(" [-{}% approval]", cost_pct))
            } else {
                (Color::Red, Color::DarkGray, format!(" [-{}% approval]", cost_pct))
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
        }
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
        let marker = if is_selected { "▶ " } else { "  " };
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
            let marker = if is_selected { "▶ " } else { "  " };
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
