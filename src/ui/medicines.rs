use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{DeployTarget, GameOutcome, GameState, Medicine, MedicineUiState, ResearchKind, grid_reading_order, KNOWLEDGE_NAME, TICKS_PER_DAY};
use crate::ui::hint_line;
use crate::format_number;

/// Maximum selection index for the medicines panel in its current sub-state.
pub fn selection_max(ui_state: &MedicineUiState, state: &GameState) -> usize {
    match ui_state {
        MedicineUiState::BrowseMedicines => {
            state.unlocked_medicine_indices().len().saturating_sub(1)
        }
        MedicineUiState::SelectRegion { .. } => {
            state.regions.len().saturating_sub(1)
        }
        MedicineUiState::SelectDisease { medicine_idx, .. } => {
            state.medicines[*medicine_idx]
                .deployable_diseases(&state.diseases).len()
                .saturating_sub(1)
        }
        MedicineUiState::SelectTarget { .. } => {
            1 // vaccinate (0) or treat (1)
        }
        MedicineUiState::ConfirmDeploy { .. }
        | MedicineUiState::DeployResult { .. } => 0,
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

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines, selected_line) = match &state.ui.medicine_ui {
        Some(MedicineUiState::SelectRegion { medicine_idx }) => {
            render_select_region(state, *medicine_idx)
        }
        Some(MedicineUiState::SelectDisease { medicine_idx, region_idx }) => {
            let (t, l) = render_select_disease(state, *medicine_idx, *region_idx);
            (t, l, None)
        }
        Some(MedicineUiState::SelectTarget { medicine_idx, region_idx, disease_idx }) => {
            let (t, l) = render_select_target(state, *medicine_idx, *region_idx, *disease_idx);
            (t, l, None)
        }
        Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, target }) => {
            let (t, l) = render_confirm_deploy(state, *medicine_idx, *region_idx, target);
            (t, l, None)
        }
        Some(MedicineUiState::DeployResult { medicine_idx, message }) => {
            let (t, l) = render_deploy_result(state, *medicine_idx, message);
            (t, l, None)
        }
        _ => render_browse(state),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

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

fn render_browse(state: &GameState) -> (String, Vec<Line<'static>>, Option<usize>) {
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
            let auto_on = state.auto_deploy.get(med_idx).copied().unwrap_or(false);
            let has_shipments = state.pending_shipments.iter().any(|s| s.medicine_idx == med_idx);
            // Check if auto-deploy is ON but blocked because all tested diseases
            // have efficacy below the deployment threshold (0.04).
            let auto_blocked = auto_on && {
                let tested: Vec<usize> = med.deployable_diseases(&state.diseases)
                    .into_iter()
                    .filter(|d_idx| med.tested_against.contains(d_idx))
                    .collect();
                !tested.is_empty() && tested.iter().all(|&d_idx| {
                    med.effective_efficacy(d_idx, &state.diseases) < 0.04
                })
            };
            let (status_tag, status_color) = if auto_on && auto_blocked {
                (" [INEFFECTIVE]", Color::Red)
            } else if auto_on {
                (" [DEPLOYING]", Color::Green)
            } else if has_shipments {
                (" [IN TRANSIT]", Color::Cyan)
            } else {
                ("", Color::Cyan)
            };
            let type_label = med.therapy_type.label();
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", marker, med.name), style),
                Span::styled(format!("  {}", type_label), Style::default().fg(Color::Cyan)),
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
                    let strain_eff = med.strain_efficacy(d_idx, &state.diseases);
                    let drift_note = if strain_eff < 1.0 { " (outdated)" } else { "" };

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
                    if !drift_note.is_empty() {
                        spans.push(Span::styled(
                            drift_note,
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

            // Line 3: Doses remaining + cost per deploy
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
                    Span::styled(
                        format!("  ¥{:.0} per deploy", med.cost),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }

            // Impact so far (only if medicine has been used)
            let total_impact = med.total_treated + med.total_protected;
            if total_impact > 0.0 {
                let mut impact_parts: Vec<String> = Vec::new();
                if med.total_treated > 0.0 {
                    impact_parts.push(format!("{} treated", format_number(med.total_treated)));
                }
                if med.total_protected > 0.0 {
                    impact_parts.push(format!("{} vaccinated", format_number(med.total_protected)));
                }
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        impact_parts.join(", "),
                        Style::default().fg(Color::Green),
                    ),
                ]));
            }

            // Warnings: strain drift needing action
            let any_strain_outdated = med.target_diseases.iter().any(|&d_idx| {
                med.strain_efficacy(d_idx, &state.diseases) < 1.0
            });
            if any_strain_outdated {
                let retrial_in_progress = state.active_research.iter().filter(|p| p.kind.category() == crate::state::ResearchCategory::Field).any(|p| {
                    matches!(&p.kind, ResearchKind::ClinicalTrial { medicine_idx: mi, .. } if *mi == med_idx)
                });
                if retrial_in_progress {
                    lines.push(Line::from(Span::styled(
                        "    Re-trial in progress",
                        Style::default().fg(Color::Yellow),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "    Strain drifted — re-trial needed (Research [R] > Field)",
                        Style::default().fg(Color::Red),
                    )));
                }
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
                        format!("Shipping to {} ({:.1}d)", region_name, days_left),
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
        let auto_status = unlocked.get(state.ui.panel_selection)
            .and_then(|&(med_idx, _)| state.auto_deploy.get(med_idx).copied())
            .unwrap_or(false);
        let auto_label = if auto_status { "ON" } else { "OFF" };
        lines.push(Line::from(Span::styled(
            format!("  [Enter] Manual deploy  [X] Auto-deploy {}  [Esc] Close", auto_label),
            Style::default().fg(Color::DarkGray),
        )));
    }

    (" Medicines ".to_string(), lines, selected_line)
}

fn render_select_region(state: &GameState, medicine_idx: usize) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let med = &state.medicines[medicine_idx];

    let order = grid_reading_order(state.regions.len());
    for (display_pos, &region_idx) in order.iter().enumerate() {
        let region = &state.regions[region_idx];
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

        // Show region stats: population, screened infected, total dead (detected only)
        let infected = region.screened_infected();
        let dead = region.detected_dead(&state.diseases);

        // Show per-disease cooldown for diseases this medicine targets in this region
        let deployable = med.deployable_diseases(&state.diseases);
        let cooldowns: Vec<(usize, u64)> = deployable.iter()
            .filter_map(|&d_idx| {
                let remaining = region.deploy_cooldown_remaining(state.tick, d_idx);
                if remaining > 0 { Some((d_idx, remaining)) } else { None }
            })
            .collect();

        let mut spans = vec![
            Span::styled(format!("{}{:<14}", marker, region.name), style),
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
        if dead > 0.0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("{:>6} dead", format_number(dead)),
                Style::default().fg(Color::DarkGray),
            ));
        }
        if !cooldowns.is_empty() {
            let all_on_cooldown = cooldowns.len() == deployable.len();
            if all_on_cooldown {
                // Every targetable disease is on cooldown — show simple message
                let max_ticks = cooldowns.iter().map(|(_, t)| *t).max().unwrap_or(0);
                let hours = ((max_ticks as f64 / TICKS_PER_DAY) * 24.0).ceil() as u64;
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    format!("[cooldown {}h]", hours),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                // Some diseases on cooldown, others ready — show which
                let names: Vec<&str> = cooldowns.iter()
                    .filter_map(|(d_idx, _)| state.diseases.get(*d_idx).map(|d| d.name.as_str()))
                    .collect();
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    format!("[cooldown: {}]", names.join(", ")),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
        let eff = region.delivery_efficiency();
        if eff < 0.90 {
            spans.push(Span::raw("  "));
            let eff_color = if eff < 0.50 { Color::Red } else { Color::Yellow };
            spans.push(Span::styled(
                format!("{:.0}% eff", eff * 100.0),
                Style::default().fg(eff_color),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    let hint = if state.outcome == GameOutcome::Playing {
        "  [↑/↓ ←/→] Navigate  [Enter] Select  [Esc] Back"
    } else {
        "  [Esc] Back"
    };
    lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))));

    (format!(" Deploy: {} ", med.name), lines, selected_line)
}

fn render_select_disease(
    state: &GameState,
    medicine_idx: usize,
    region_idx: usize,
) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med = &state.medicines[medicine_idx];
    let region = &state.regions[region_idx];

    let deployable = med.deployable_diseases(&state.diseases);
    for (i, &disease_idx) in deployable.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };
        let disease_name = state.diseases.get(disease_idx)
            .map(|d| d.display_name(disease_idx))
            .unwrap_or_else(|| "Unknown".to_string());
        let cross_reactive = med.is_cross_reactive(disease_idx);

        let inf = region.infections.iter().find(|inf| inf.disease_idx == disease_idx);
        let infected = inf.map(|i| i.infected).unwrap_or(0.0);

        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let mut spans = vec![Span::styled(format!("{}{}", marker, disease_name), style)];
        if cross_reactive {
            spans.push(Span::styled(
                " (cross-reactive, 50% eff)",
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{} infected", format_number(infected)),
                Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
        ]));
    }

    // Show incompatible diseases grayed out so the player understands why they can't be targeted.
    let incompatible: Vec<usize> = state.diseases.iter().enumerate()
        .filter(|(i, d)| d.detected && d.knowledge >= KNOWLEDGE_NAME && !deployable.contains(i))
        .map(|(i, _)| i)
        .collect();
    if !incompatible.is_empty() {
        lines.push(Line::from(""));
        for &disease_idx in &incompatible {
            let name = state.diseases[disease_idx].display_name(disease_idx);
            let reason = if !state.diseases[disease_idx].pathogen_type.is_treatable() {
                "prion, untreatable".to_string()
            } else {
                format!("{}, incompatible", med.therapy_type.label())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", name), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!(" ({})", reason),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    let hint = if state.outcome == GameOutcome::Playing {
        "  [←/→] Change region  [Enter] Select  [Esc] Back"
    } else {
        "  [Esc] Back"
    };
    lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))));

    (format!(" {} → {} ", med.name, region.name), lines)
}

fn render_select_target(
    state: &GameState,
    medicine_idx: usize,
    region_idx: usize,
    disease_idx: usize,
) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med = &state.medicines[medicine_idx];
    let region = &state.regions[region_idx];
    let pop = region.population as f64;
    let disease_name = state.diseases.get(disease_idx)
        .map(|d| d.display_name(disease_idx))
        .unwrap_or_else(|| "Unknown".to_string());

    let inf = region.infections.iter().find(|i| i.disease_idx == disease_idx);
    let efficacy = med.effective_efficacy(disease_idx, &state.diseases);
    let deploy_cost = state.medicine_deploy_cost(medicine_idx, region_idx);

    // Targeting efficiency — shown up top if doses are being wasted
    let targeting_eff = state.targeting_efficiency(region_idx);
    if targeting_eff < 0.99 {
        let waste_pct = ((1.0 - targeting_eff) * 100.0) as u32;
        let color = if targeting_eff < 0.60 { Color::Red } else { Color::Yellow };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}% of doses wasted", waste_pct),
                Style::default().fg(color),
            ),
            Span::styled(
                " — improve screening to reduce",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Option 0: Vaccinate
    {
        let exposed = inf.map(|i| i.exposed).unwrap_or(0.0);
        let infected = inf.map(|i| i.infected).unwrap_or(0.0);
        let shows_immune = state.policies.get(region_idx)
            .map(|p| p.screening.shows_immune())
            .unwrap_or(false);
        let immune = if shows_immune { inf.map(|i| i.immune).unwrap_or(0.0) } else { 0.0 };
        let susceptible = (pop - exposed - infected - region.dead - immune).max(0.0);
        let empty = susceptible == 0.0;
        let will_vaccinate = med.estimate_vaccination(susceptible, efficacy, state.vaccination_multiplier());
        let selected = state.ui.panel_selection == 0;

        let marker = if selected { "▶ " } else { "  " };
        let style = if empty {
            Style::default().fg(Color::DarkGray)
        } else if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(Span::styled(
            format!("{}Vaccinate", marker),
            style,
        )));
        if empty {
            lines.push(Line::from(Span::styled(
                "    No susceptible population",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::raw("    Will protect "),
                Span::styled(
                    format_number(will_vaccinate),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" of {} susceptible", format_number(susceptible)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    // Option 1: Treat
    {
        let infected = inf.map(|i| i.infected).unwrap_or(0.0);
        let empty = infected == 0.0;
        let will_treat = med.estimate_treatment(infected, efficacy);
        let selected = state.ui.panel_selection == 1;

        let marker = if selected { "▶ " } else { "  " };
        let style = if empty {
            Style::default().fg(Color::DarkGray)
        } else if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(Span::styled(
            format!("{}Treat", marker),
            style,
        )));
        if empty {
            lines.push(Line::from(Span::styled(
                "    No infected population",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::raw("    Will treat "),
                Span::styled(
                    format_number(will_treat),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" of {} infected", format_number(infected)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    // Cost — simple and clear
    lines.push(Line::from(""));
    let can_afford = state.resources.funding >= deploy_cost;
    lines.push(Line::from(vec![
        Span::styled("  Cost: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("¥{:.0}", deploy_cost),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!("  (have ¥{:.0})", state.resources.funding),
            Style::default().fg(if can_afford { Color::DarkGray } else { Color::Red }),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Doses: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", format_number(med.doses)),
            Style::default().fg(dose_color(med)),
        ),
    ]));

    // Warnings — only if something is actually wrong
    let strain_eff = med.strain_efficacy(disease_idx, &state.diseases);
    if strain_eff < 1.0 {
        let behind = med.mutations_behind(disease_idx, &state.diseases);
        lines.push(Line::from(Span::styled(
            format!("  Strain drifted {} mutation{} — efficacy reduced",
                behind, if behind == 1 { "" } else { "s" }),
            Style::default().fg(Color::Yellow),
        )));
    }
    if state.has_resistance_surveillance() {
        let resistance = med.resistance_factor(disease_idx, &state.diseases);
        let res_pct = ((1.0 - resistance) * 100.0).round() as u32;
        if res_pct >= 30 {
            lines.push(Line::from(Span::styled(
                format!("  {}% drug resistance — consider switching", res_pct),
                Style::default().fg(Color::Red),
            )));
        } else if res_pct > 0 {
            lines.push(Line::from(Span::styled(
                format!("  {}% drug resistance", res_pct),
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    lines.push(Line::from(""));
    let hint = if state.outcome == GameOutcome::Playing {
        "  [←/→] Change region  [Enter] Deploy  [Esc] Back"
    } else {
        "  [Esc] Back"
    };
    lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))));

    (format!(" {} → {} → {} ", med.name, region.name, disease_name), lines)
}

fn render_confirm_deploy(
    state: &GameState,
    medicine_idx: usize,
    region_idx: usize,
    target: &DeployTarget,
) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med = &state.medicines[medicine_idx];
    let region = &state.regions[region_idx];

    let (action_desc, disease_name) = match target {
        DeployTarget::Vaccinate { disease_idx } => {
            let name = state.diseases[*disease_idx].display_name(*disease_idx);
            (format!("Protect {} against {}", region.name, name), name)
        }
        DeployTarget::Treat { disease_idx } => {
            let name = state.diseases[*disease_idx].display_name(*disease_idx);
            (format!("Treat {} in {}", name, region.name), name)
        }
    };

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {}", action_desc),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ⚠ WARNING: UNTESTED MEDICINE",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Not tested against {}.", disease_name),
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "  25% chance of adverse effects (20% of doses)",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "  will KILL instead of help.",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Deploy anyway?",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(hint_line(state, "Confirm", "Cancel"));

    (format!(" ⚠ {} ", med.name), lines)
}

fn render_deploy_result(
    state: &GameState,
    medicine_idx: usize,
    message: &str,
) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med_name = state.medicines.get(medicine_idx)
        .map(|m| m.name.as_str())
        .unwrap_or("Unknown");

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {}", message),
        Style::default().fg(Color::Green),
    )));
    lines.push(Line::from(""));

    // Show updated medicine state
    if let Some(med) = state.medicines.get(medicine_idx) {
        let dc = dose_color(med);
        let dose_text = if med.doses <= 0.0 {
            "EMPTY".to_string()
        } else {
            format!("{}/{}", format_number(med.doses), format_number(med.max_doses))
        };
        lines.push(Line::from(vec![
            Span::styled("  Doses remaining: ", Style::default().fg(Color::DarkGray)),
            Span::styled(dose_text, Style::default().fg(dc)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Funding: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("¥{:.0}", state.resources.funding),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Continue", "Back"));

    (format!(" ✓ {} [Dispatched] ", med_name), lines)
}
