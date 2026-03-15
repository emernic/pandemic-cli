use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameOutcome, GameState, SimState, TICKS_PER_DAY, ticks_to_days};
use crate::format_number;

/// Returns the height this bar needs: 2 rows (stats + border).
pub fn height(_state: &GameState) -> u16 {
    2
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let pause_indicator = match &state.outcome {
        GameOutcome::Lost => Span::styled(" DEFEAT ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        GameOutcome::Playing => if state.active_crisis.is_some() {
            Span::styled(" EVENT ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        } else {
            match &state.sim_state {
                SimState::Running => {
                    let speed = state.session.speed_multiplier.max(1);
                    if speed > 1 {
                        Span::styled(format!(" ▶▶ {}x ", speed), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                    } else {
                        Span::styled(" RUNNING ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                    }
                }
                SimState::Paused => Span::styled(" PAUSED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            }
        },
    };

    let line1 = Line::from(vec![
        pause_indicator,
        Span::raw("  "),
        Span::styled(
            format!("Day: {:.1}", ticks_to_days(state.tick as f64)),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        {
            let auth = state.resources.authority;
            let auth_color = match auth {
                crate::state::Authority::Maximum | crate::state::Authority::High => Color::Green,
                crate::state::Authority::Medium | crate::state::Authority::Low => Color::Yellow,
                _ => Color::Red,
            };
            Span::styled(
                format!("Authority: {}", auth.label()),
                Style::default().fg(auth_color),
            )
        },
        Span::raw("  "),
        Span::styled(
            format!("Funds: ¥{:.0}", state.resources.funding),
            Style::default().fg(Color::Green),
        ),
        {
            let gross = state.funding_income_rate() * TICKS_PER_DAY;
            let upkeep = state.personnel_upkeep_rate() * TICKS_PER_DAY;
            let policy = state.total_policy_funding_cost() * TICKS_PER_DAY;
            let net = gross - upkeep - policy;
            let (sign, color) = if net >= 0.0 {
                ("+", Color::DarkGray)
            } else {
                ("", Color::Red)
            };
            Span::styled(
                format!(" ({sign}¥{net:.0}/day)"),
                Style::default().fg(color),
            )
        },
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
        {
            let screened = state.total_visible_infected_screened();
            let any_estimated = state.regions.iter().enumerate()
                .any(|(i, _)| state.screening_visibility(i) < 1.0);
            let prefix = if any_estimated { "Infected: ~" } else { "Infected: " };
            Span::styled(
                format!("{}{}", prefix, format_number(screened)),
                Style::default().fg(Color::Red),
            )
        },
        // Infection trend arrow (compared to ~1 day ago)
        // When infections drop because people are dying (not recovering),
        // show a red ▼ instead of green — declining infected is bad news
        // when the death rate is accelerating.
        match state.infection_trend() {
            Some(ratio) if ratio > 1.05 => Span::styled(" \u{25b2}", Style::default().fg(Color::Red)),
            Some(ratio) if ratio < 0.95 => {
                let deaths_rising = state.death_trend().is_some_and(|d| d > 1.05);
                if deaths_rising {
                    Span::styled(" \u{25bc}", Style::default().fg(Color::Red))
                } else {
                    Span::styled(" \u{25bc}", Style::default().fg(Color::Green))
                }
            }
            Some(_) => Span::styled(" \u{2014}", Style::default().fg(Color::DarkGray)),
            None => Span::raw(""),
        },
        Span::raw("  "),
        Span::styled(
            format!("Dead: {}", format_number(state.total_visible_dead())),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let lines = vec![line1];

    if let Some(notif) = &state.ui.event_notification {
        // Split: stats + research on left, event notification on right
        let notif_width = (area.width / 3).clamp(40, 70);
        let layout = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(notif_width),
        ]).split(area);

        let left_widget = Paragraph::new(lines).block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(left_widget, layout[0]);

        let notif_lines = vec![
            Line::from(Span::styled(
                format!("⚠ {}", notif),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        let notif_widget = Paragraph::new(notif_lines)
            .block(Block::default().borders(Borders::LEFT | Borders::BOTTOM));
        f.render_widget(notif_widget, layout[1]);
    } else {
        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(widget, area);
    }
}

