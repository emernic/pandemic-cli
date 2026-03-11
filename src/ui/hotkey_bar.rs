use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameOutcome, GameState, Panel};

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let hotkeys = vec![
        ("T", "Threats", Panel::Threats),
        ("R", "Research", Panel::Research),
        ("M", "Medicines", Panel::Medicines),
        ("P", "Policy", Panel::Policy),
        ("O", "Operations", Panel::Operations),
    ];

    let mut spans: Vec<Span> = Vec::new();

    for (key, label, panel) in &hotkeys {
        let active = state.ui.open_panel == *panel;
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("[{}]", key),
            Style::default()
                .fg(if active { Color::Yellow } else { Color::Cyan })
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {}", label),
            Style::default().fg(if active { Color::Yellow } else { Color::White }),
        ));
    }

    // Only show pause/resume when game is still playing
    if state.outcome == GameOutcome::Playing {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "[Space]",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            if state.sim_state.is_running() { " Pause" } else { " Resume" },
            Style::default().fg(Color::White),
        ));
        if state.sim_state.is_running() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "[Z]",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" Speed", Style::default().fg(Color::White)));
        }
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        "[?]",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" Help", Style::default().fg(Color::White)));

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        "[Q]",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" Save & Quit", Style::default().fg(Color::White)));

    let mut lines = Vec::new();
    match &state.outcome {
        GameOutcome::Lost => {
            let collapsed = state.regions.iter().filter(|r| r.collapsed).count();
            let msg = format!("All {collapsed} regions collapsed.");
            lines.push(Line::from(Span::styled(
                msg,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        }
        GameOutcome::Playing => {
            if let Some(msg) = &state.ui.status_message {
                lines.push(Line::from(Span::styled(
                    msg.as_str(),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
    }
    lines.push(Line::from(spans));
    let widget = Paragraph::new(lines).block(Block::default().borders(Borders::TOP));
    f.render_widget(widget, area);
}
