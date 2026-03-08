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
    let pause_indicator = if state.paused {
        Span::styled(" PAUSED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" RUNNING ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    };

    let line = Line::from(vec![
        Span::styled("PANDEMIC DEFENSE", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        pause_indicator,
        Span::raw("  "),
        Span::styled(
            format!("Tick: {}", state.tick),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Funds: ${:.0}", state.resources.funding),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled(
            format!("RP: {:.0}", state.resources.research_points),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("  "),
        Span::styled(
            {
                let avail = state.personnel_available();
                let total = state.resources.personnel;
                if avail < total {
                    format!("Personnel: {}/{}", avail, total)
                } else {
                    format!("Personnel: {}", total)
                }
            },
            Style::default().fg(Color::White),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Infected: {}", format_number(state.total_infected())),
            Style::default().fg(Color::Red),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Dead: {}", format_number(state.total_dead())),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let widget = Paragraph::new(line).block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(widget, area);
}
