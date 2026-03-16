use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{AppState, Medicine, MedicineUiState, ResearchKind, grid_reading_order, TICKS_PER_DAY, DEPLOY_MIN_EFFICACY};
use crate::format_number;

/// Maximum selection index for the medicines panel in its current sub-state.
pub fn selection_max(ui_state: &MedicineUiState, state: &AppState) -> usize {
    match ui_state {
        MedicineUiState::BrowseMedicines => {
            state.unlocked_medicine_indices().len().saturating_sub(1)
        }
        MedicineUiState::RegionFilter { .. } => {
            state.regions.len().saturating_sub(1)
        }
    }
}

fn dose_color(med: &Medicine) -> Color {
    if med.doses <= 0.0 {
        Color::Red
    } else if med.doses < med.max_doses * 0.5 {
        Color::Yellow
    } else {
        Color::Cyan
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState) {
    let (title, lines, selected_line) = match &state.ui.medicine_ui {
        Some(MedicineUiState::RegionFilter { medicine_idx }) => {
            render_region_filter(state, *medicine_idx)
        }
        _ => render_browse(state),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

fn render_browse(state: &AppState) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let unlocked_indices = state.unlocked_medicine_indices();
    let unlocked: Vec<(usize, &Medicine)> = unlocked_indices.iter()
        .map(|&i| (i, &state.medicines[i]))
        .collect();

    if unlocked.is_empty() {
        lines.push(Line::from(Span::styled(
            "No medicines available.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, &(med_idx, med)) in unlocked.iter().enumerate() {
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

            // Line 1: Name + deploying status
            let deploy_on = state.deploy_enabled.get(med_idx).copied().unwrap_or(false);
            let shipment_doses: f64 = state.pending_shipments.iter()
                .filter(|s| s.medicine_idx == med_idx)
                .map(|s| s.doses)
                .sum();
            let has_shipments = shipment_doses > 0.0;
            // Check if deploy is ON but blocked because all tested diseases
            // have efficacy below the deployment threshold.
            let deploy_blocked = deploy_on && {
                let tested: Vec<usize> = med.deployable_diseases(&state.diseases)
                    .into_iter()
                    .filter(|d_idx| med.tested_against.contains(d_idx))
                    .collect();
                !tested.is_empty() && tested.iter().all(|&d_idx| {
                    med.effective_efficacy(d_idx, &state.diseases) < DEPLOY_MIN_EFFICACY
                })
            };

            // Region filter summary
            let region_filter = state.deploy_regions.get(med_idx);
            let filter_note = if deploy_on {
                if let Some(regions) = region_filter {
                    if regions.is_empty() {
                        String::new() // all regions
                    } else {
                        let names: Vec<&str> = regions.iter()
                            .filter_map(|&r| state.regions.get(r).map(|reg| reg.name.as_str()))
                            .collect();
                        format!(" ({})", names.join(", "))
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let (status_tag, status_color) = if deploy_on && deploy_blocked {
                (" [INEFFECTIVE]".to_string(), Color::Red)
            } else if deploy_on && med.doses <= 0.0 {
                (" [AWAITING DOSES]".to_string(), Color::Yellow)
            } else if deploy_on {
                (format!(" [DEPLOYING]{}", filter_note), Color::Green)
            } else if has_shipments {
                (format!(" [IN TRANSIT: {} doses]", shipment_doses.round() as u64), Color::Cyan)
            } else {
                (String::new(), Color::Cyan)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", marker, med.name), style),
                Span::styled(status_tag, Style::default().fg(status_color)),
            ]));

            // Line 2: What it fights and how well — THE most important info
            for &d_idx in med.target_diseases.iter() {
                let name = state.diseases.get(d_idx)
                    .map(|d| d.display_name(d_idx))
                    .unwrap_or_else(|| format!("#{}", d_idx + 1));

                if med.tested_against.contains(&d_idx) {
                    let efficacy = med.effective_efficacy(d_idx, &state.diseases);
                    let pct = (efficacy * 100.0).round() as u32;
                    let eff_color = if pct >= 85 {
                        Color::Green
                    } else if pct >= 50 {
                        Color::Yellow
                    } else if pct >= 10 {
                        Color::Red
                    } else {
                        Color::DarkGray
                    };
                    let cross_reactive = med.is_cross_reactive(d_idx);

                    let mut spans = vec![
                        Span::raw("    "),
                        Span::styled(
                            format!("{}% effective", pct),
                            Style::default().fg(eff_color),
                        ),
                        Span::styled(
                            format!(" vs {}", name),
                            Style::default().fg(Color::White),
                        ),
                    ];
                    if cross_reactive {
                        spans.push(Span::styled(
                            " (cross-reactive)",
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                    if state.has_resistance_surveillance() {
                        let res_factor = med.resistance_factor(d_idx, &state.diseases);
                        let res_pct = ((1.0 - res_factor) * 100.0).round() as u32;
                        if res_pct > 0 {
                            let res_color = if res_pct >= 30 { Color::Red } else { Color::Yellow };
                            spans.push(Span::styled(
                                format!("  {}% resistant", res_pct),
                                Style::default().fg(res_color),
                            ));
                        }
                    }
                    lines.push(Line::from(spans));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("UNTESTED", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" vs {}", name), Style::default().fg(Color::White)),
                    ]));
                }
            }

            // Line 3: Doses remaining
            let dc = dose_color(med);
            if med.doses <= 0.0 {
                let is_manufacturing = state.active_research.iter()
                    .any(|p| matches!(&p.kind, ResearchKind::ManufactureDoses { medicine_idx: mi } if *mi == med_idx));
                if is_manufacturing {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("NO DOSES", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::styled(" — restocking in progress", Style::default().fg(Color::Yellow)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("NO DOSES", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::styled(" — restock via Research [R] > Applied", Style::default().fg(Color::Red)),
                    ]));
                }
            } else {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("{} doses remaining", format_number(med.doses)),
                        Style::default().fg(dc),
                    ),
                ]));
            }

            // Impact so far (only if medicine has been used)
            if med.total_treated > 0.0 {
                let impact_parts = vec![format!("{} treated", format_number(med.total_treated))];
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        impact_parts.join(", "),
                        Style::default().fg(Color::Green),
                    ),
                ]));
            }

            // Pending shipments
            let shipments: Vec<_> = state.pending_shipments.iter()
                .filter(|s| s.medicine_idx == med_idx)
                .collect();
            for s in &shipments {
                let region_name = state.regions.get(s.region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let ticks_left = s.arrive_tick.saturating_sub(state.tick);
                let days_left = ticks_left as f64 / TICKS_PER_DAY;
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("Shipping {} doses to {} ({:.1}d)", s.doses.round() as u64, region_name, days_left),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
            }

            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(""));
    if unlocked.is_empty() {
        lines.push(Line::from(Span::styled(
            "  [Esc] Close",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let deploy_status = unlocked.get(state.ui.panel_selection)
            .and_then(|&(med_idx, _)| state.deploy_enabled.get(med_idx).copied())
            .unwrap_or(false);
        let deploy_label = if deploy_status { "Stop" } else { "Deploy" };
        lines.push(Line::from(Span::styled(
            format!("  [Enter] {}  [X] Region filter  [Esc] Close", deploy_label),
            Style::default().fg(Color::DarkGray),
        )));
    }

    (" Medicines ".to_string(), lines, selected_line)
}

fn render_region_filter(state: &AppState, medicine_idx: usize) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let med = &state.medicines[medicine_idx];

    let region_filter = state.deploy_regions.get(medicine_idx);
    let filter_is_all = region_filter.map(|s| s.is_empty()).unwrap_or(true);

    let order = grid_reading_order(state.regions.len());
    for (display_pos, &region_idx) in order.iter().enumerate() {
        let region = &state.regions[region_idx];
        let selected = state.ui.panel_selection == display_pos;
        if selected {
            selected_line = Some(lines.len());
        }

        // Determine if this region is enabled
        let region_enabled = if filter_is_all {
            true
        } else {
            region_filter.map(|s| s.contains(&region_idx)).unwrap_or(true)
        };

        let toggle = if region_enabled { "[X]" } else { "[ ]" };
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if region_enabled {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let infected = region.screened_infected();

        let mut spans = vec![
            Span::styled(format!("{}{} {:<14}", marker, toggle, region.name), style),
            Span::styled(
                format!("{:>6} pop", format_number(region.population as f64)),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:>6} inf", format_number(infected)),
                Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
        ];

        if region.collapsed {
            spans.push(Span::raw("  "));
            spans.push(Span::styled("COLLAPSED", Style::default().fg(Color::Red)));
        }

        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Enter] Toggle region  [Esc] Back",
        Style::default().fg(Color::DarkGray),
    )));

    (format!(" {} — Region Filter ", med.name), lines, selected_line)
}
