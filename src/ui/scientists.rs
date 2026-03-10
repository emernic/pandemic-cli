use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, ResearchKind, ScientistStatus, ScientistTrait, KNOWLEDGE_NAME, TICKS_PER_DAY,
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

    // Scroll to keep selection visible
    let scroll = if selected > 0 {
        // Each scientist takes 2 lines; scroll if needed
        let visible_height = area.height.saturating_sub(4) as usize; // borders + summary
        let line_offset = selected * 2;
        if line_offset >= visible_height {
            (line_offset - visible_height + 2) as u16
        } else {
            0
        }
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
            return Some(format_kind(&p.kind, state));
        }
    }
    if let Some(p) = &state.applied_research {
        if p.scientist_ids.contains(&scientist_id) {
            return Some(format_kind(&p.kind, state));
        }
    }
    if let Some(p) = &state.basic_research {
        if p.scientist_ids.contains(&scientist_id) {
            return Some(format_kind(&p.kind, state));
        }
    }
    None
}

/// Format a ResearchKind for display.
fn format_kind(kind: &ResearchKind, state: &GameState) -> String {
    match kind {
        ResearchKind::IdentifyThreat { disease_idx } => {
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            let verb = if state.diseases.get(*disease_idx)
                .is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME) { "Study" } else { "Identify" };
            format!("{}: {}", verb, name)
        }
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str()).unwrap_or("Unknown");
            format!("Develop: {}", name)
        }
        ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
            let med = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str()).unwrap_or("Unknown");
            let dis = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Trial: {} vs {}", med, dis)
        }
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str()).unwrap_or("Unknown");
            format!("Manufacture: {}", name)
        }
        ResearchKind::GenomicSequencing { disease_idx } => {
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Sequencing: {}", name)
        }
        ResearchKind::TrainPersonnel => "Train Personnel".into(),
        ResearchKind::BasicResearch { tech } => format!("Research: {}", tech.name()),
    }
}
