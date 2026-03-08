use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::GameState;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();

    if state.diseases.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active threats.",
            Style::default().fg(Color::Green),
        )));
    } else {
        for (i, disease) in state.diseases.iter().enumerate() {
            let selected = state.ui.panel_selection == i;
            let marker = if selected { "▶ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            lines.push(Line::from(Span::styled(
                format!("{}{}", marker, disease.name),
                style,
            )));
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("Infectivity: {:.0}%", disease.infectivity * 100.0),
                    Style::default().fg(Color::Red),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("Lethality: {:.0}%", disease.lethality * 100.0),
                    Style::default().fg(Color::Magenta),
                ),
            ]));

            // Show which regions are infected
            let affected: Vec<&str> = state
                .regions
                .iter()
                .filter(|r| r.infections.iter().any(|inf| inf.disease_idx == i))
                .map(|r| r.name.as_str())
                .collect();

            if !affected.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw("    Regions: "),
                    Span::styled(
                        affected.join(", "),
                        Style::default().fg(Color::LightRed),
                    ),
                ]));
            }

            lines.push(Line::from(""));
        }
    }

    let block = Block::default()
        .title(" Threats ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
