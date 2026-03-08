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

    for region in &state.regions {
        let infected: f64 = region.infections.iter().map(|i| i.infected).sum();
        let dead: f64 = region.infections.iter().map(|i| i.dead).sum();
        // Avoid displaying "-0"
        let infected = if infected == 0.0 { 0.0 } else { infected };
        let dead = if dead == 0.0 { 0.0 } else { dead };

        let threat_level = if infected > 100_000.0 {
            ("CRIT", Color::Red)
        } else if infected > 10_000.0 {
            ("HIGH", Color::LightRed)
        } else if infected > 1_000.0 {
            ("MOD", Color::Yellow)
        } else if infected > 0.0 {
            ("LOW", Color::Green)
        } else {
            ("OK", Color::DarkGray)
        };

        let pop_millions = region.population as f64 / 1_000_000.0;

        let line = Line::from(vec![
            Span::styled(
                format!("{:<14}", region.name),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>7.0}M", pop_millions),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>8.0}i", infected),
                Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>7.0}d", dead),
                Style::default().fg(if dead > 0.0 { Color::DarkGray } else { Color::DarkGray }),
            ),
            Span::raw(" ["),
            Span::styled(
                format!("{:>4}", threat_level.0),
                Style::default().fg(threat_level.1).add_modifier(Modifier::BOLD),
            ),
            Span::raw("]"),
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
