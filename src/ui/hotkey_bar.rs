use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, Panel};

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let hotkeys = vec![
        ("T", "Threats", Panel::Threats),
        ("R", "Research", Panel::Research),
        ("M", "Medicines", Panel::Medicines),
        ("P", "Policy", Panel::Policy),
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

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        "[Space]",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        if state.paused { " Resume" } else { " Pause" },
        Style::default().fg(Color::White),
    ));

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
    spans.push(Span::styled(" Quit", Style::default().fg(Color::White)));

    let line = Line::from(spans);
    let widget = Paragraph::new(line).block(Block::default().borders(Borders::TOP));
    f.render_widget(widget, area);
}
