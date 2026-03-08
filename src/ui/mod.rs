pub mod hotkey_bar;
pub mod medicines;
pub mod research;
pub mod resources;
pub mod threats;
pub mod region_list;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, Panel};

/// Format a number with human-readable suffix (K, M, B).
pub fn format_number(n: f64) -> String {
    let abs = n.abs();
    if abs < 0.5 {
        return "0".to_string();
    }
    if abs >= 999_999_500.0 {
        format!("{:.1}B", n / 1_000_000_000.0)
    } else if abs >= 999_950.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if abs >= 999.5 {
        format!("{:.1}K", n / 1_000.0)
    } else {
        format!("{:.0}", n)
    }
}

pub fn render(f: &mut Frame, state: &GameState) {
    let has_status = state.ui.status_message.is_some();
    let hotkey_height = if has_status { 3 } else { 2 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),            // resources bar
            Constraint::Min(8),              // main area
            Constraint::Length(hotkey_height), // hotkey bar (+ status line)
        ])
        .split(f.area());

    resources::render(f, chunks[0], state);
    hotkey_bar::render(f, chunks[2], state);

    // Main area: region list, optionally split with a panel
    match &state.ui.open_panel {
        Panel::None => {
            region_list::render(f, chunks[1], state);
        }
        Panel::Threats => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            region_list::render(f, split[0], state);
            threats::render(f, split[1], state);
        }
        Panel::Medicines => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            region_list::render(f, split[0], state);
            medicines::render(f, split[1], state);
        }
        Panel::Research => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            region_list::render(f, split[0], state);
            research::render(f, split[1], state);
        }
        panel => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            region_list::render(f, split[0], state);
            render_placeholder_panel(f, split[1], panel);
        }
    }
}

fn render_placeholder_panel(f: &mut Frame, area: Rect, panel: &Panel) {
    let title = match panel {
        Panel::Research => " Research ",
        Panel::Policy => " Policy ",
        Panel::Help => " Help ",
        _ => " Panel ",
    };

    let content = match panel {
        Panel::Help => vec![
            Line::from(""),
            Line::from(Span::styled("Pandemic Defense", Style::default().fg(Color::Cyan))),
            Line::from(""),
            Line::from("Defend humanity against disease outbreaks."),
            Line::from(""),
            Line::from(Span::styled("Controls:", Style::default().fg(Color::Yellow))),
            Line::from("  [T] View active threats"),
            Line::from("  [R] Research panel"),
            Line::from("  [M] Medicines panel"),
            Line::from("  [P] Policy panel"),
            Line::from("  [Space] Pause/Resume"),
            Line::from("  [↑/↓/←/→] Navigate map & panels"),
            Line::from("  [Esc] Close panel"),
            Line::from("  [Q] Quit"),
        ],
        _ => vec![
            Line::from(""),
            Line::from(Span::styled(
                "Coming soon...",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from("This panel will be implemented"),
            Line::from("as game mechanics are designed."),
        ],
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let widget = Paragraph::new(content).block(block);
    f.render_widget(widget, area);
}
