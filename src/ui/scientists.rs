use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, ScientistStatus, ScientistTrait, TICKS_PER_DAY,
};

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let selected = state.ui.panel_selection;

    let alive_scientists: Vec<_> = state.scientists.iter()
        .filter(|s| s.is_alive())
        .collect();

    if alive_scientists.is_empty() {
        lines.push(Line::from(Span::styled(
            "No scientists available.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    for (i, s) in alive_scientists.iter().enumerate() {
        let is_selected = i == selected;
        let highlight = if is_selected { Color::Yellow } else { Color::White };

        // Name and specialty
        let trait_color = match s.scientist_trait {
            ScientistTrait::Brilliant => Color::Cyan,
            ScientistTrait::Reckless => Color::Red,
            ScientistTrait::Cautious => Color::Green,
            ScientistTrait::Versatile => Color::Magenta,
        };

        let marker = if is_selected { "▸ " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(
                &s.name,
                Style::default().fg(highlight).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(s.specialty.label(), Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(s.scientist_trait.label(), Style::default().fg(trait_color)),
        ]));

        // Status line
        let status_spans = match &s.status {
            ScientistStatus::Available => {
                // Check if assigned to a project
                let project = find_assignment(state, s.id);
                match project {
                    Some(desc) => vec![
                        Span::styled("    Assigned: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(desc, Style::default().fg(Color::Blue)),
                    ],
                    None => vec![
                        Span::styled("    Available", Style::default().fg(Color::Green)),
                    ],
                }
            }
            ScientistStatus::BurnedOut { until_tick } => {
                let days_left = (*until_tick as f64 - state.tick as f64) / TICKS_PER_DAY;
                vec![
                    Span::styled(
                        format!("    Burned out ({:.1}d remaining)", days_left.max(0.0)),
                        Style::default().fg(Color::Red),
                    ),
                ]
            }
            ScientistStatus::Infected { until_tick } => {
                let days_left = (*until_tick as f64 - state.tick as f64) / TICKS_PER_DAY;
                vec![
                    Span::styled(
                        format!("    Infected ({:.1}d remaining)", days_left.max(0.0)),
                        Style::default().fg(Color::Red),
                    ),
                ]
            }
            ScientistStatus::Dead => unreachable!("filtered by is_alive()"),
        };
        lines.push(Line::from(status_spans));
    }

    // Summary line at bottom
    let total = alive_scientists.len();
    let assigned_ids = state.assigned_scientist_ids();
    let available = alive_scientists.iter()
        .filter(|s| s.is_available() && !assigned_ids.contains(&s.id))
        .count();
    let assigned = alive_scientists.iter()
        .filter(|s| s.is_available() && assigned_ids.contains(&s.id))
        .count();
    let unavailable = total - available - assigned;

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{total} scientists: "),
            Style::default().fg(Color::White),
        ),
        Span::styled(format!("{available} idle"), Style::default().fg(Color::Green)),
        Span::raw(", "),
        Span::styled(format!("{assigned} assigned"), Style::default().fg(Color::Blue)),
        Span::raw(", "),
        Span::styled(format!("{unavailable} unavailable"), Style::default().fg(
            if unavailable > 0 { Color::Red } else { Color::DarkGray }
        )),
    ]));

    // Scroll to keep selection visible (each scientist = 2 lines)
    let visible_height = area.height.saturating_sub(2) as usize; // minus top+bottom border
    let line_offset = selected * 2;
    let scroll = if line_offset + 2 > visible_height {
        (line_offset + 2 - visible_height) as u16
    } else {
        0
    };

    let block = Block::default()
        .title(" SCIENTISTS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block).scroll((scroll, 0));
    f.render_widget(widget, area);
}

/// Find which research project a scientist is assigned to, if any.
fn find_assignment(state: &GameState, scientist_id: u64) -> Option<String> {
    for p in &state.field_research {
        if p.scientist_ids.contains(&scientist_id) {
            return Some(p.kind.display_label(&state.diseases, &state.medicines));
        }
    }
    if let Some(p) = &state.applied_research {
        if p.scientist_ids.contains(&scientist_id) {
            return Some(p.kind.display_label(&state.diseases, &state.medicines));
        }
    }
    if let Some(p) = &state.basic_research {
        if p.scientist_ids.contains(&scientist_id) {
            return Some(p.kind.display_label(&state.diseases, &state.medicines));
        }
    }
    None
}
