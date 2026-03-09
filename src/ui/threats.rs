use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, KNOWLEDGE_NAME, KNOWLEDGE_PARTIAL_STATS, grid_reading_order};
use crate::format_number;

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

            // Undetected diseases: show only a subtle "?" indicator
            if !disease.detected {
                lines.push(Line::from(Span::styled(
                    format!("{}?", marker),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
                continue;
            }

            let display_name = disease.display_name(i);
            lines.push(Line::from(Span::styled(
                format!("{}{}", marker, display_name),
                style,
            )));

            if disease.knowledge < KNOWLEDGE_NAME {
                // Completely unknown — show nothing
                lines.push(Line::from(Span::styled(
                    "    ???",
                    Style::default().fg(Color::DarkGray),
                )));
            } else if disease.knowledge < KNOWLEDGE_PARTIAL_STATS {
                // Name known, partial stats + pathogen type (vector not yet known)
                let mut type_spans = vec![
                    Span::styled(
                        format!("    Type: {}", disease.pathogen_type.label()),
                        Style::default().fg(Color::Cyan),
                    ),
                ];
                push_mutation_indicator(&mut type_spans, state, i);
                lines.push(Line::from(type_spans));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("Infect: {:.0}%", disease.infectivity * 100.0),
                        Style::default().fg(Color::Red),
                    ),
                    Span::raw("  "),
                    Span::styled("Lethal: ?", Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled("Recov: ?", Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                // Full stats visible + pathogen type + transmission vector
                let mut type_spans = vec![
                    Span::styled(
                        format!("    Type: {}", disease.pathogen_type.label()),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("  Vector: {}", disease.transmission.label()),
                        Style::default().fg(Color::Yellow),
                    ),
                ];
                push_mutation_indicator(&mut type_spans, state, i);
                lines.push(Line::from(type_spans));
                // Full stats visible
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("Infect: {:.0}%", disease.infectivity * 100.0),
                        Style::default().fg(Color::Red),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("Lethal: {:.1}%", disease.lethality * 100.0),
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("Recov: {:.0}%", disease.recovery_rate * 100.0),
                        Style::default().fg(Color::Green),
                    ),
                ]));
            }

            // Show affected regions once the disease is identified
            if disease.knowledge >= KNOWLEDGE_NAME {
                let order = grid_reading_order(state.regions.len());
                let regions: Vec<&str> = order.iter()
                    .filter_map(|&idx| state.regions.get(idx))
                    .filter(|r| r.disease_state(i).is_some_and(|inf| inf.infected > 0.0))
                    .map(|r| r.name.as_str())
                    .collect();
                if !regions.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("    Present in: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            regions.join(", "),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                }
            }

            // Show knowledge bar
            if disease.knowledge > 0.0 {
                let pct = (disease.knowledge * 100.0).min(100.0);
                let color = if disease.knowledge >= 1.0 {
                    Color::Green
                } else {
                    Color::Blue
                };
                lines.push(Line::from(Span::styled(
                    format!("    Knowledge: {:.0}%", pct),
                    Style::default().fg(color),
                )));
            }

            if selected && disease.knowledge >= KNOWLEDGE_NAME {
                render_disease_detail(&mut lines, state, i);
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

/// Push a mutation indicator span only when the player's medicines are affected.
/// Bare "Mutated" with no actionable context is noise — only surface mutations
/// that require the player to act (re-trial to recalibrate).
fn push_mutation_indicator(
    spans: &mut Vec<Span<'static>>,
    state: &GameState,
    disease_idx: usize,
) {
    if state.has_outdated_medicine(disease_idx) {
        spans.push(Span::styled(
            "  Medicines outdated!".to_string(),
            Style::default().fg(Color::Red),
        ));
    }
}

fn render_disease_detail(lines: &mut Vec<Line>, state: &GameState, disease_idx: usize) {
    let hdr = Style::default().fg(Color::DarkGray);
    // Check if any region has sub-100% visibility (screening not maxed)
    let any_estimated = state.regions.iter().enumerate().any(|(i, _)| {
        state.screening_visibility(i) < 1.0
    });
    let infected_label = if any_estimated { "Infected~" } else { "Infected" };
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("{:<16}", "Region"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", infected_label), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Immune"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Dead"), hdr),
    ]));

    let mut total_infected = 0.0;
    let mut total_immune = 0.0;
    let mut total_dead = 0.0;

    let order = grid_reading_order(state.regions.len());
    for &region_idx in &order {
        let region = &state.regions[region_idx];
        if let Some(inf) = region.disease_state(disease_idx) {
            if inf.infected <= 0.0 && inf.immune <= 0.0 && inf.dead <= 0.0 {
                continue;
            }
            let visibility = state.screening_visibility(region_idx);
            let screened = inf.infected * visibility;
            total_infected += screened;
            total_immune += inf.immune;
            total_dead += inf.dead;

            let name = format!("{:<16}", region.name);
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(
                    format!("{:>8}", format_number(screened)),
                    Style::default().fg(Color::LightRed),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:>8}", format_number(inf.immune)),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:>8}", format_number(inf.dead)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    // Totals
    lines.push(Line::from(vec![
        Span::styled("    ────────────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("{:<16}", "TOTAL"),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_infected)),
            Style::default().fg(Color::LightRed),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_immune)),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_dead)),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
}
