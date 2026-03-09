pub mod hotkey_bar;
pub mod medicines;
pub mod policy;
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

use crate::state::{GameEvent, GameOutcome, GameState, Panel};
use crate::format_number;

/// Build a hint line like "[Enter] Select  [Esc] Close", omitting the Enter
/// portion when the game is over (Confirm is blocked post-game).
pub fn hint_line(state: &GameState, enter_label: &str, esc_label: &str) -> Line<'static> {
    let hint = if state.outcome == GameOutcome::Playing {
        format!("  [Enter] {enter_label}  [Esc] {esc_label}")
    } else {
        format!("  [Esc] {esc_label}")
    };
    Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)))
}

/// Convert game events from the most recent tick into a status message.
/// Called after each tick by the game loop / snapshot runner. This keeps
/// human-facing strings in the UI layer, not in engine.rs.
pub fn process_events(state: &mut GameState) {
    if state.events.is_empty() {
        return;
    }

    // Handle game-over: pause and close panels (UI concern, not engine's job)
    if state.events.iter().any(|e| matches!(e, GameEvent::GameOver)) {
        state.paused = true;
        state.ui.open_panel = Panel::None;
    }

    // Pick the most important event to display as status message.
    // Priority: PolicySuspended > FundingWarning > DiseaseMutated
    let suspended: Vec<_> = state.events.iter()
        .filter_map(|e| match e {
            GameEvent::PolicySuspended { region_idx, policy_name } => {
                let region = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str())
                    .unwrap_or("Unknown");
                Some(format!("{} in {}", policy_name, region))
            }
            _ => None,
        })
        .collect();

    // Priority: NewDiseaseEmerged > PolicySuspended > FundingWarning > DiseaseMutated
    let msg = if let Some(GameEvent::NewDiseaseEmerged { region_idx, .. }) =
        state.events.iter().find(|e| matches!(e, GameEvent::NewDiseaseEmerged { .. }))
    {
        let region_name = state.regions.get(*region_idx)
            .map(|r| r.name.as_str())
            .unwrap_or("Unknown");
        format!("NEW THREAT detected in {region_name}! Use [T] to view.")
    } else if !suspended.is_empty() {
        format!("Funding crisis: suspended {}", suspended.join(", "))
    } else if state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)) {
        "LOW FUNDS: Policies at risk of suspension!".to_string()
    } else if let Some(GameEvent::DiseaseMutated { disease_idx, new_generation }) =
        state.events.iter().find(|e| matches!(e, GameEvent::DiseaseMutated { .. }))
    {
        let name = state.diseases.get(*disease_idx)
            .map(|d| d.display_name(*disease_idx))
            .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
        format!("{name} has mutated! (Gen {new_generation})")
    } else {
        return;
    };
    state.ui.status_message = Some(msg);
}

pub fn render(f: &mut Frame, state: &GameState) {
    let header_height = resources::height(state);
    let has_extra_line = state.ui.status_message.is_some() || state.outcome != GameOutcome::Playing;
    let hotkey_height = if has_extra_line { 3 } else { 2 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),  // resources bar (expands when research active)
            Constraint::Min(8),              // main area
            Constraint::Length(hotkey_height), // hotkey bar (+ status line)
        ])
        .split(f.area());

    resources::render(f, chunks[0], state);
    hotkey_bar::render(f, chunks[2], state);

    // Main area: region list, optionally split with a panel
    match &state.ui.open_panel {
        Panel::None if state.outcome != GameOutcome::Playing => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            region_list::render(f, split[0], state);
            render_game_over(f, split[1], state);
        }
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
        Panel::Policy => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            region_list::render(f, split[0], state);
            policy::render(f, split[1], state);
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
            Line::from("  [Q] Save & Quit"),
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

fn render_game_over(f: &mut Frame, area: Rect, state: &GameState) {
    let won = state.outcome == GameOutcome::Won;
    let (title, border_color) = if won {
        (" VICTORY ", Color::Green)
    } else {
        (" DEFEAT ", Color::Red)
    };

    let total_dead = state.total_dead();
    let total_immune = state.total_immune();
    let initial_pop = state.initial_population();
    let survivors = initial_pop - total_dead;
    let survival_pct = if initial_pop > 0.0 { (survivors / initial_pop) * 100.0 } else { 0.0 };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    let headline = if won {
        "All diseases eradicated. Humanity is saved."
    } else {
        "Humanity has fallen. Too many lives were lost."
    };
    lines.push(Line::from(Span::styled(
        format!("  {headline}"),
        Style::default().fg(if won { Color::Green } else { Color::Red }),
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ── Summary ──",
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    let stat_label = Style::default().fg(Color::DarkGray);
    let stat_value = Style::default().fg(Color::White);

    lines.push(Line::from(vec![
        Span::styled("  Duration:       ", stat_label),
        Span::styled(format!("{} ticks", state.tick), stat_value),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Total Dead:     ", stat_label),
        Span::styled(
            format_number(total_dead),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!("  ({:.1}% of population)", (total_dead / initial_pop) * 100.0 + 0.0),
            stat_label,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Survivors:      ", stat_label),
        Span::styled(
            format_number(survivors),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!("  ({survival_pct:.1}%)"),
            stat_label,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Immunized:      ", stat_label),
        Span::styled(
            format_number(total_immune),
            Style::default().fg(Color::Cyan),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Still Infected: ", stat_label),
        Span::styled(
            format_number(state.total_infected()),
            Style::default().fg(if state.total_infected() > 0.0 { Color::Yellow } else { Color::DarkGray }),
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ── Regions ──",
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    for region in &state.regions {
        let dead = region.total_dead();
        let alive = region.alive();
        let pop = region.population as f64;
        let dead_pct = if pop > 0.0 { (dead / pop) * 100.0 + 0.0 } else { 0.0 };
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<16}", region.name), stat_value),
            Span::styled(format!("{:>8} alive", format_number(alive)), Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled(
                format!("{:>8} dead ({:.1}%)", format_number(dead), dead_pct),
                Style::default().fg(if dead > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
        ]));
    }

    // Strategic tips (defeat only)
    if !won {
        let tips = state.defeat_tips();
        if !tips.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── What to try next time ──",
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::from(""));
            for tip in &tips {
                lines.push(Line::from(Span::styled(
                    format!("  • {tip}"),
                    Style::default().fg(Color::White),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Q] Save & Quit  [T/R/M] Browse panels",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
