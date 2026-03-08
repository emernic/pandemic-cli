use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::GameState;
use super::format_number;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();

    // Header row
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:<14}", "Region"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(format!("{:>8}", "Pop"), Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format!("{:>8}", "Alive"), Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format!("{:>8}", "Infected"), Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format!("{:>8}", "Dead"), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(format!("{:>4}", "Risk"), Style::default().fg(Color::DarkGray)),
    ]));

    for region in &state.regions {
        let infected = region.total_infected();
        let dead = region.total_dead();
        let alive = region.alive();

        let threat_level = if infected > 100_000.0 {
            ("CRIT", Color::Red)
        } else if infected > 10_000.0 {
            ("HIGH", Color::LightRed)
        } else if infected > 1_000.0 {
            (" MOD", Color::Yellow)
        } else if infected > 0.0 {
            (" LOW", Color::Green)
        } else {
            ("  OK", Color::DarkGray)
        };

        let line = Line::from(vec![
            Span::styled(
                format!("{:<14}", region.name),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>8}", format_number(region.population as f64)),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>8}", format_number(alive)),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>8}", format_number(infected)),
                Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>8}", format_number(dead)),
                Style::default().fg(if dead > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:>4}", threat_level.0),
                Style::default().fg(threat_level.1).add_modifier(Modifier::BOLD),
            ),
        ]);

        lines.push(line);
    }

    let block = Block::default()
        .title(" World Status ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
