use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{BasicTech, AppState, KNOWLEDGE_NAME, KNOWLEDGE_PARTIAL_STATS, PathogenType, TICKS_PER_DAY, grid_reading_order};
use crate::format_number;

/// Maximum selection index for the threats panel.
pub fn selection_max(state: &AppState) -> usize {
    state.diseases.len().saturating_sub(1)
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;

    if state.diseases.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active threats.",
            Style::default().fg(Color::Green),
        )));
    } else {
        let disease_deaths: Vec<f64> = (0..state.diseases.len())
            .map(|i| state.regions.iter()
                .filter_map(|r| r.disease_state(i))
                .map(|inf| inf.dead)
                .sum())
            .collect();
        let grand_total_deaths: f64 = disease_deaths.iter().sum();

        let display_order = state.threats_display_order();

        for (display_pos, &i) in display_order.iter().enumerate() {
            let disease = &state.diseases[i];
            let selected = state.ui.panel_selection == display_pos;
            if selected {
                selected_line = Some(lines.len());
            }
            let marker = if selected { "▶ " } else { "  " };
            let base_style = if disease.hidden {
                Style::default().fg(Color::DarkGray)
            } else if selected {
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
            let hidden_tag = if disease.hidden { " [HIDDEN]" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", marker, display_name), base_style),
                Span::styled(hidden_tag.to_string(), Style::default().fg(Color::DarkGray)),
            ]));

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
            } else {
                // ── Identity & biology ──
                let has_research_vector = disease.knowledge >= KNOWLEDGE_PARTIAL_STATS;
                let mut id_spans: Vec<Span> = vec![
                    Span::styled(
                        format!("    {}", disease.pathogen_type.label()),
                        Style::default().fg(Color::Cyan),
                    ),
                ];
                if has_research_vector {
                    id_spans.push(Span::styled(
                        format!(" · {}", disease.transmission.label()),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if let Some(ref lineage) = disease.parent_lineage {
                    id_spans.push(Span::styled(
                        format!(" · Variant of {}", lineage),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(id_spans));

                // Special trait warnings on their own line
                let warnings = disease_warnings(state, i);
                for (text, color, bold) in &warnings {
                    let style = if *bold {
                        Style::default().fg(*color).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(*color)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("    {text}"),
                        style,
                    )));
                }

                lines.push(Line::from(""));

                // ── Impact ──
                let total_dead = disease_deaths[i];

                // Observed CFR: only count deaths AND immune from regions with
                // screening that reveals immune counts, so both halves of the
                // ratio come from the same population. Without this, CFR is
                // biased high (global deaths / partial immune = ~100%).
                let (screened_dead, screened_immune) = state.regions.iter().enumerate()
                    .filter_map(|(region_idx, region)| {
                        if !state.screening_shows_immune(region_idx) { return None; }
                        let inf = region.disease_state(i)?;
                        Some((inf.dead, inf.immune))
                    })
                    .fold((0.0, 0.0), |(d, im), (dd, ii)| (d + dd, im + ii));
                let has_any_immune_screening = state.regions.iter().enumerate()
                    .any(|(idx, _)| state.screening_shows_immune(idx));
                let resolved = screened_dead + screened_immune;

                let lethal_span = if disease.knowledge < KNOWLEDGE_PARTIAL_STATS {
                    Span::styled("Lethality: ?", Style::default().fg(Color::DarkGray))
                } else if !has_any_immune_screening || screened_immune <= 0.0 {
                    // Without immune data, CFR = deaths/deaths = 100% always — useless.
                    // Show "?" and hint that screening is needed.
                    Span::styled("Lethality: ? (need screening)", Style::default().fg(Color::DarkGray))
                } else if resolved > 0.0 {
                    let cfr = (screened_dead / resolved) * 100.0;
                    let color = if cfr > 30.0 { Color::Red }
                        else if cfr > 10.0 { Color::Yellow }
                        else { Color::Green };
                    Span::styled(
                        format!("Lethality: {cfr:.0}%"),
                        Style::default().fg(color),
                    )
                } else {
                    Span::styled("Lethality: ?", Style::default().fg(Color::DarkGray))
                };

                let rt_span = if disease.knowledge >= KNOWLEDGE_PARTIAL_STATS {
                    if let Some(rt) = disease.observed_rt() {
                        let (symbol, color) = if rt > 1.5 { ("▲", Color::Red) }
                            else if rt > 1.05 { ("▲", Color::Yellow) }
                            else if rt > 0.95 { ("─", Color::White) }
                            else { ("▼", Color::Green) };
                        let avg_vis: f64 = state.regions.iter().enumerate()
                            .map(|(idx, _)| state.screening_visibility(idx))
                            .sum::<f64>() / state.regions.len() as f64;
                        let confidence = if avg_vis < 0.3 { "~" } else { "" };
                        Span::styled(
                            format!("Rt: {confidence}{rt:.1} {symbol}"),
                            Style::default().fg(color),
                        )
                    } else {
                        Span::styled("Rt: ?", Style::default().fg(Color::DarkGray))
                    }
                } else {
                    Span::styled("Rt: ?", Style::default().fg(Color::DarkGray))
                };

                // Deaths + share of total
                let death_color = if grand_total_deaths > 0.0 {
                    let share = total_dead / grand_total_deaths;
                    if share >= 0.5 { Color::Red }
                    else if share >= 0.25 { Color::Yellow }
                    else { Color::White }
                } else { Color::DarkGray };

                // Current observed infected from the snapshot system
                let total_infected = disease.current_day_observed_infected;

                lines.push(Line::from(vec![
                    Span::raw("    "),
                    lethal_span,
                    Span::raw("    "),
                    rt_span,
                    Span::raw("    "),
                    Span::styled(
                        format!("Infected~{}", format_number(total_infected)),
                        Style::default().fg(Color::LightRed),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format_deaths_line(total_dead, grand_total_deaths),
                        Style::default().fg(death_color),
                    ),
                ]));

                // 20-day death projection (requires EpidemiologicalForecasting tech)
                if state.has_forecasting() {
                    let projected = state.projected_deaths(i, 20.0);
                    if projected >= 1.0 {
                        let proj_color = if projected >= 10_000.0 { Color::Red }
                            else if projected >= 1_000.0 { Color::Yellow }
                            else { Color::White };
                        lines.push(Line::from(Span::styled(
                            format!("    Projected +{} deaths / 20 days", format_number(projected)),
                            Style::default().fg(proj_color),
                        )));
                    }
                }

                // Region spread — only count regions where screening has detected
                // meaningful infections, to avoid leaking disease presence.
                let order = grid_reading_order(state.regions.len());
                let affected: Vec<&str> = order.iter()
                    .filter_map(|&idx| state.regions.get(idx).map(|r| (idx, r)))
                    .filter(|&(ri, r)| {
                        let dead = r.disease_state(i).map_or(0.0, |inf| inf.dead);
                        if dead >= 1.0 { return true; }
                        let shows_exposed = state.screening_shows_exposed(ri);
                        let screened = r.screened_infected_for_disease(i, &state.diseases, shows_exposed);
                        screened >= 1.0
                    })
                    .map(|(_, r)| r.name.as_str())
                    .collect();
                let spread_color = if affected.len() >= 4 { Color::Red }
                    else if affected.len() >= 2 { Color::Yellow }
                    else { Color::White };
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("Spread: {}/{} regions", affected.len(), state.regions.len()),
                        Style::default().fg(spread_color),
                    ),
                    Span::styled(
                        format!("  ({})", affected.join(", ")),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));

                // First-detection info: show where the disease was first observed
                if !disease.first_detected_regions.is_empty() {
                    let det_names: Vec<&str> = disease.first_detected_regions.iter()
                        .filter_map(|&idx| state.regions.get(idx).map(|r| r.name.as_str()))
                        .collect();
                    let day_str = format!("{:.0}", disease.detected_day);
                    lines.push(Line::from(Span::styled(
                        format!("    First detected: {} (Day {})", det_names.join(", "), day_str),
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                lines.push(Line::from(""));

                // ── Response status ──
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

                // Knowledge bar
                if disease.knowledge > 0.0 && disease.knowledge < 1.0 {
                    let pct = (disease.knowledge * 100.0).min(100.0);
                    lines.push(Line::from(Span::styled(
                        format!("    Knowledge: {:.0}%", pct),
                        Style::default().fg(Color::Blue),
                    )));
                }

                // Sequence homology
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
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

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

/// Collect disease warning indicators as (text, color, bold) tuples.
/// Callers decide how to display them (inline, stacked, etc.).
fn disease_warnings(
    state: &AppState,
    disease_idx: usize,
) -> Vec<(String, Color, bool)> {
    let disease = &state.diseases[disease_idx];
    let mut warnings = Vec::new();
    if disease.pathogen_type == PathogenType::RnaVirus {
        warnings.push(("Causes social disruption".to_string(), Color::Yellow, false));
    }
    if disease.pathogen_type == PathogenType::Fungus {
        warnings.push(("Degrades infrastructure".to_string(), Color::Yellow, false));
    }
    if !disease.pathogen_type.is_treatable() {
        warnings.push(("UNTREATABLE".to_string(), Color::Red, true));
        return warnings; // No medicine indicators relevant for untreatable diseases
    }
    if state.has_resistance_surveillance() && state.has_resistant_medicine(disease_idx) {
        warnings.push(("Resistance building!".to_string(), Color::Yellow, false));
    }
    warnings
}

enum MedStatus {
    Deployed,   // has been deployed at least once
    Available,  // unlocked, has doses, but never deployed
    Tested,     // tested but no doses
    InDevelopment, // research in progress
    None,       // nothing
}

fn medicine_status_for_disease(state: &AppState, disease_idx: usize) -> MedStatus {
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
    let researching = state.active_research.iter().any(|r| r.references_disease(disease_idx));
    if researching {
        return MedStatus::InDevelopment;
    }
    MedStatus::None
}

fn render_disease_detail(lines: &mut Vec<Line>, state: &AppState, disease_idx: usize) {
    let hdr = Style::default().fg(Color::DarkGray);
    let has_forecast = state.has_forecasting();
    // Check if any region has sub-100% visibility (screening not maxed)
    let any_estimated = state.regions.iter().enumerate().any(|(i, _)| {
        state.screening_visibility(i) < 1.0
    });
    let infected_label = if any_estimated { "Infected~" } else { "Infected" };
    // Check if any region has Antigen+ screening (shows immune counts)
    let any_shows_immune = state.policies.iter().any(|p| p.screening.shows_immune());
    let mut header_spans = vec![
        Span::raw("    "),
        Span::styled(format!("{:<16}", "Region"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", infected_label), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Immune"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Dead"), hdr),
    ];
    if has_forecast {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(format!("{:>8}", "Proj20d"), hdr));
    }
    lines.push(Line::from(header_spans));

    let disease = &state.diseases[disease_idx];
    let mut total_infected = 0.0;
    let mut total_immune = 0.0;
    let mut total_dead = 0.0;
    let mut total_projected = 0.0;

    let order = grid_reading_order(state.regions.len());
    for &region_idx in &order {
        let region = &state.regions[region_idx];
        if let Some(inf) = region.disease_state(disease_idx) {
            let shows_exposed = state.screening_shows_exposed(region_idx);
            let screened = region.screened_infected_for_disease(disease_idx, &state.diseases, shows_exposed);
            // Hide regions where screening hasn't detected meaningful infections,
            // unless deaths reveal the disease's presence there.
            if screened < 1.0 && inf.dead < 1.0 && inf.immune <= 0.0 {
                continue;
            }
            // Collapsed regions: only count dead toward totals (infected/immune
            // are no longer actionable). Show a dimmed row with just the dead count.
            if region.collapsed {
                total_dead += inf.dead;
                let label = format!("{} [X]", region.name);
                let name = format!("{:<16}", &label[..label.len().min(16)]);
                let mut row = vec![
                    Span::raw("    "),
                    Span::styled(name, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(format!("{:>8}", "—"), Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(format!("{:>8}", "—"), Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(
                        format!("{:>8}", format_number(inf.dead)),
                        Style::default().fg(Color::DarkGray),
                    ),
                ];
                if has_forecast {
                    row.push(Span::raw("  "));
                    row.push(Span::styled(format!("{:>8}", "—"), Style::default().fg(Color::DarkGray)));
                }
                lines.push(Line::from(row));
                continue;
            }
            let shows_immune = state.policies.get(region_idx)
                .map(|p| p.screening.shows_immune())
                .unwrap_or(false);
            let shown_immune = if shows_immune { inf.immune } else { 0.0 };
            total_infected += screened;
            total_immune += shown_immune;
            total_dead += inf.dead;

            let name = format!("{:<16}", region.name);
            let mut row = vec![
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
            ];
            if has_forecast {
                let proj = inf.infected * disease.lethality * TICKS_PER_DAY * 20.0;
                total_projected += proj;
                let proj_color = if proj >= 10_000.0 { Color::Red }
                    else if proj >= 1_000.0 { Color::Yellow }
                    else { Color::White };
                row.push(Span::raw("  "));
                row.push(Span::styled(
                    format!("{:>8}", if proj >= 1.0 { format!("+{}", format_number(proj)) } else { "—".to_string() }),
                    Style::default().fg(proj_color),
                ));
            }
            lines.push(Line::from(row));
        }
    }

    // Totals
    let mut sep = vec![
        Span::styled("    ────────────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
    ];
    if has_forecast {
        sep.push(Span::styled("──────────", Style::default().fg(Color::DarkGray)));
    }
    lines.push(Line::from(sep));
    let mut totals = vec![
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
    ];
    if has_forecast {
        let proj_color = if total_projected >= 10_000.0 { Color::Red }
            else if total_projected >= 1_000.0 { Color::Yellow }
            else { Color::White };
        totals.push(Span::raw("  "));
        totals.push(Span::styled(
            format!("{:>8}", if total_projected >= 1.0 { format!("+{}", format_number(total_projected)) } else { "—".to_string() }),
            Style::default().fg(proj_color),
        ));
    }
    lines.push(Line::from(totals));
}
