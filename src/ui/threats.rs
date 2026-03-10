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
    let mut selected_line: Option<usize> = None;

    if state.diseases.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active threats.",
            Style::default().fg(Color::Green),
        )));
    } else {
        for (i, disease) in state.diseases.iter().enumerate() {
            let selected = state.ui.panel_selection == i;
            if selected {
                selected_line = Some(lines.len());
            }
            let marker = if selected { "▶ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            // Undetected diseases: show a placeholder with a hint to upgrade screening
            if !disease.detected {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}Unknown pathogen (undetected)", marker),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    "    Upgrade [P] screening to detect sooner",
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
                push_disease_indicators(&mut type_spans, state, i);
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
                push_disease_indicators(&mut type_spans, state, i);
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

            // Comparative triage data — CFR, region spread, medicine status
            if disease.knowledge >= KNOWLEDGE_NAME {
                // Case fatality rate (resolved cases that died)
                let total_dead: f64 = state.regions.iter()
                    .filter_map(|r| r.disease_state(i))
                    .map(|inf| inf.dead)
                    .sum();
                let total_immune: f64 = state.regions.iter()
                    .filter_map(|r| r.disease_state(i))
                    .map(|inf| inf.immune)
                    .sum();
                let resolved = total_dead + total_immune;
                let cfr_span = if resolved > 0.0 {
                    let cfr = (total_dead / resolved) * 100.0;
                    let color = if cfr > 30.0 { Color::Red }
                        else if cfr > 10.0 { Color::Yellow }
                        else { Color::Green };
                    Span::styled(format!("CFR: {cfr:.0}%"), Style::default().fg(color))
                } else {
                    Span::styled("CFR: ?", Style::default().fg(Color::DarkGray))
                };

                // Region spread count
                let order = grid_reading_order(state.regions.len());
                let affected: Vec<&str> = order.iter()
                    .filter_map(|&idx| state.regions.get(idx))
                    .filter(|r| r.disease_state(i).is_some_and(|inf| inf.infected > 0.0))
                    .map(|r| r.name.as_str())
                    .collect();
                let spread_color = if affected.len() >= 4 { Color::Red }
                    else if affected.len() >= 2 { Color::Yellow }
                    else { Color::White };

                lines.push(Line::from(vec![
                    Span::raw("    "),
                    cfr_span,
                    Span::raw("  "),
                    Span::styled(
                        format!("Spread: {}/{}", affected.len(), state.regions.len()),
                        Style::default().fg(spread_color),
                    ),
                    Span::styled(
                        format!("  ({})", affected.join(", ")),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));

                // Medicine status
                let med_status = medicine_status_for_disease(state, i);
                let (med_text, med_color) = match med_status {
                    MedStatus::Deployed => ("Medicine: deployed", Color::Green),
                    MedStatus::Available => ("Medicine: available (not deployed)", Color::Cyan),
                    MedStatus::Tested => ("Medicine: tested, needs doses", Color::Blue),
                    MedStatus::InDevelopment => ("Medicine: in development", Color::Yellow),
                    MedStatus::None => ("Medicine: none", Color::Red),
                };
                lines.push(Line::from(Span::styled(
                    format!("    {med_text}"),
                    Style::default().fg(med_color),
                )));
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

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = selected_line.map(|line| {
        if line as u16 >= inner_height {
            (line as u16).saturating_sub(inner_height * 2 / 3)
        } else {
            0
        }
    }).unwrap_or(0);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

/// Push disease warning indicators: outdated medicines, resistance, containment
/// adaptation. Only shows warnings that require player action.
fn push_disease_indicators(
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
    if state.has_resistance_surveillance() && state.has_resistant_medicine(disease_idx) {
        spans.push(Span::styled(
            "  Resistance building!".to_string(),
            Style::default().fg(Color::Yellow),
        ));
    }
    // Show containment adaptation when significant
    let adaptation = state.diseases.get(disease_idx)
        .map(|d| d.containment_adaptation)
        .unwrap_or(0.0);
    if adaptation >= 0.10 {
        let pct = (adaptation * 100.0).round() as u32;
        let color = if pct >= 50 { Color::Red } else { Color::Yellow };
        spans.push(Span::styled(
            format!("  Containment adapt: {}%", pct),
            Style::default().fg(color),
        ));
    }
}

enum MedStatus {
    Deployed,   // has been deployed at least once
    Available,  // unlocked, has doses, but never deployed
    Tested,     // tested but no doses
    InDevelopment, // research in progress
    None,       // nothing
}

fn medicine_status_for_disease(state: &GameState, disease_idx: usize) -> MedStatus {
    // Check medicines targeting this disease
    for med in &state.medicines {
        if med.target_diseases.contains(&disease_idx) && med.unlocked {
            if med.deployed_count > 0 {
                return MedStatus::Deployed;
            }
            if med.doses > 0.0 {
                return MedStatus::Available;
            }
            if med.tested_against.contains(&disease_idx) {
                return MedStatus::Tested;
            }
        }
    }
    // Check if research is targeting this disease
    let researching = state.applied_research.as_ref().is_some_and(|r| r.references_disease(disease_idx))
        || state.field_research.iter().any(|r| r.references_disease(disease_idx));
    if researching {
        return MedStatus::InDevelopment;
    }
    MedStatus::None
}

fn render_disease_detail(lines: &mut Vec<Line>, state: &GameState, disease_idx: usize) {
    let hdr = Style::default().fg(Color::DarkGray);
    // Check if any region has sub-100% visibility (screening not maxed)
    let any_estimated = state.regions.iter().enumerate().any(|(i, _)| {
        state.screening_visibility(i) < 1.0
    });
    let infected_label = if any_estimated { "Infected~" } else { "Infected" };
    // Check if any region has Antigen+ screening (shows immune counts)
    let any_shows_immune = state.policies.iter().any(|p| p.screening.shows_immune());
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
            // Distribute region's total estimate proportionally across diseases
            let total_real = region.detected_infected(&state.diseases);
            let proportion = if total_real > 0.0 { inf.infected / total_real } else { 0.0 };
            let screened = region.estimated_infected * proportion;
            let shows_immune = state.policies.get(region_idx)
                .map(|p| p.screening.shows_immune())
                .unwrap_or(false);
            let shown_immune = if shows_immune { inf.immune } else { 0.0 };
            total_infected += screened;
            total_immune += shown_immune;
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
                    format!("{:>8}", if shows_immune { format_number(shown_immune) } else { "?".to_string() }),
                    Style::default().fg(if shows_immune { Color::Green } else { Color::DarkGray }),
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
            format!("{:>8}", if any_shows_immune { format_number(total_immune) } else { "?".to_string() }),
            Style::default().fg(if any_shows_immune { Color::Green } else { Color::DarkGray }),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_dead)),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
}
