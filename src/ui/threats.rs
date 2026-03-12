use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{BasicTech, GameState, KNOWLEDGE_NAME, KNOWLEDGE_PARTIAL_STATS, grid_reading_order};
use crate::format_number;

/// Maximum selection index for the threats panel.
pub fn selection_max(state: &GameState) -> usize {
    state.diseases.len().saturating_sub(1)
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;

    if state.diseases.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active threats.",
            Style::default().fg(Color::Green),
        )));
    } else {
        // Pre-calculate per-disease total deaths for ranking
        let disease_deaths: Vec<f64> = (0..state.diseases.len())
            .map(|i| state.regions.iter()
                .filter_map(|r| r.disease_state(i))
                .map(|inf| inf.dead)
                .sum())
            .collect();
        let grand_total_deaths: f64 = disease_deaths.iter().sum();

        // Sort display order: detected diseases by deaths desc, undetected last
        let mut display_order: Vec<usize> = (0..state.diseases.len()).collect();
        display_order.sort_by(|&a, &b| {
            let a_detected = state.diseases[a].detected;
            let b_detected = state.diseases[b].detected;
            match (a_detected, b_detected) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => disease_deaths[b].partial_cmp(&disease_deaths[a]).unwrap_or(std::cmp::Ordering::Equal),
            }
        });

        for (display_pos, &i) in display_order.iter().enumerate() {
            let disease = &state.diseases[i];
            let selected = state.ui.panel_selection == display_pos;
            if selected {
                selected_line = Some(lines.len());
            }
            let marker = if selected { "▶ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            // Undetected diseases: show placeholder with deaths and a hint to upgrade screening
            if !disease.detected {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}Unknown pathogen (undetected)", marker),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                let d = disease_deaths[i];
                if d > 0.0 {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", format_deaths_line(d, grand_total_deaths)),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
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
                // Detected but unidentified — show deaths if any
                lines.push(Line::from(Span::styled(
                    "    ???",
                    Style::default().fg(Color::DarkGray),
                )));
                let d = disease_deaths[i];
                if d > 0.0 {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", format_deaths_line(d, grand_total_deaths)),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            } else if disease.knowledge < KNOWLEDGE_PARTIAL_STATS {
                // Name known, partial stats + pathogen type (vector not yet known)
                let mut type_spans = vec![
                    Span::styled(
                        format!("    Type: {}", disease.pathogen_type.label()),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("  Strain: Gen {}", disease.strain_generation),
                        Style::default().fg(Color::DarkGray),
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
                    Span::styled(
                        format!("  Strain: Gen {}", disease.strain_generation),
                        Style::default().fg(Color::DarkGray),
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

            // Comparative triage data — deaths ranking, CFR, region spread, medicine status
            if disease.knowledge >= KNOWLEDGE_NAME {
                let total_dead = disease_deaths[i];

                // Deaths + share of total deaths — primary triage signal
                let death_color = if grand_total_deaths > 0.0 {
                    let share = total_dead / grand_total_deaths;
                    if share >= 0.5 { Color::Red }
                    else if share >= 0.25 { Color::Yellow }
                    else { Color::White }
                } else { Color::DarkGray };
                let death_text = format_deaths_line(total_dead, grand_total_deaths);
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(death_text, Style::default().fg(death_color)),
                ]));

                // Case fatality rate (resolved cases that died)
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

            // Sequence homology: Rapid Sequencing reveals shared genetic markers between
            // wave-coordinated diseases. Shows when knowledge >= 0.66 on both diseases.
            // Factual data only — no commentary. Player connects the dots.
            if state.unlocked_techs.contains(&BasicTech::RapidSequencing)
                && disease.knowledge >= KNOWLEDGE_PARTIAL_STATS
            {
                if let Some(group) = disease.sequence_group {
                    for (other_idx, other) in state.diseases.iter().enumerate() {
                        if other_idx != i
                            && other.sequence_group == Some(group)
                            && other.knowledge >= KNOWLEDGE_PARTIAL_STATS
                        {
                            lines.push(Line::from(Span::styled(
                                format!("    Shares sequences with {}", other.display_name(other_idx)),
                                Style::default().fg(Color::Magenta),
                            )));
                        }
                    }
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

/// Format deaths with share-of-total: "Deaths: 1.2M  (34% of total)" or "Deaths: 230  (<1% of total)".
/// When grand_total is 0 or deaths is 0, omits the percentage.
fn format_deaths_line(deaths: f64, grand_total: f64) -> String {
    if grand_total > 0.0 && deaths > 0.0 {
        let share = deaths / grand_total;
        let pct_str = if share < 0.01 { "<1%".to_string() } else { format!("{:.0}%", share * 100.0) };
        format!("Deaths: {}  ({pct_str} of total)", format_number(deaths))
    } else {
        format!("Deaths: {}", format_number(deaths))
    }
}

/// Push disease warning indicators: outdated medicines, resistance, containment
/// adaptation, and sequence homology. Only shows warnings that require player action
/// or provide actionable intelligence.
fn push_disease_indicators(
    spans: &mut Vec<Span<'static>>,
    state: &GameState,
    disease_idx: usize,
) {
    let disease = &state.diseases[disease_idx];
    if !disease.pathogen_type.is_treatable() {
        spans.push(Span::styled(
            "  UNTREATABLE".to_string(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        return; // No medicine indicators relevant for untreatable diseases
    }
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
            if inf.exposed + inf.infected <= 0.0 && inf.immune <= 0.0 && inf.dead <= 0.0 {
                continue;
            }
            // Distribute region's total estimate proportionally across diseases.
            // Without antigen screening, exposed (incubating) people are invisible.
            let shows_exposed = state.screening_shows_exposed(region_idx);
            let total_real = if shows_exposed {
                region.detected_infected(&state.diseases)
            } else {
                region.detected_symptomatic(&state.diseases)
            };
            let this_disease_total = if shows_exposed { inf.exposed + inf.infected } else { inf.infected };
            let proportion = if total_real > 0.0 { this_disease_total / total_real } else { 0.0 };
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
