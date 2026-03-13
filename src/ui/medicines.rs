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

            let auto_on = state.auto_deploy.get(med_idx).copied().unwrap_or(false);
            let auto_tag = if auto_on { " AUTO" } else { "" };
            let type_info = if let Some(mech) = med.mechanism {
                format!("  ({}, {})", med.therapy_type.label(), mech.label())
            } else {
                format!("  ({})", med.therapy_type.label())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", marker, med.name), style),
                Span::styled(type_info, Style::default().fg(Color::Cyan)),
                Span::styled(auto_tag, Style::default().fg(Color::Green)),
            ]));

            let dc = dose_color(med);
            let dose_text = if med.doses <= 0.0 {
                "EMPTY".to_string()
            } else if med.doses < med.max_doses {
                format!("{}/{} doses", format_number(med.doses), format_number(med.max_doses))
            } else {
                format!("{} doses", format_number(med.doses))
            };

            let mut detail_spans = vec![
                Span::raw("    "),
                Span::styled(
                    format!("¥{:.0}+", med.cost),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("  "),
                Span::styled(
                    dose_text,
                    Style::default().fg(dc),
                ),
                Span::raw("  "),
            ];

            // Per-disease name with strain efficacy
            for (j, &d_idx) in med.target_diseases.iter().enumerate() {
                if j > 0 {
                    detail_spans.push(Span::raw(", "));
                }
                let name = state.diseases.get(d_idx)
                    .map(|d| d.display_name(d_idx))
                    .unwrap_or_else(|| format!("#{}", d_idx + 1));
                detail_spans.push(Span::styled(name, Style::default().fg(Color::Red)));

                if med.tested_against.contains(&d_idx) {
                    let efficacy = med.effective_efficacy(d_idx, &state.diseases);
                    let pct = (efficacy * 100.0).round() as u32;
                    let color = if pct >= 85 {
                        Color::Green
                    } else if pct >= 50 {
                        Color::Yellow
                    } else if pct >= 10 {
                        Color::Red
                    } else {
                        Color::DarkGray
                    };
                    // Show ▼ when strain drift has reduced calibration
                    let strain_eff = med.strain_efficacy(d_idx, &state.diseases);
                    let trend = if strain_eff < 1.0 { "\u{25bc}" } else { "" };
                    detail_spans.push(Span::styled(
                        format!(" ({}%{})", pct, trend),
                        Style::default().fg(color),
                    ));
                    // Show resistance level if surveillance unlocked
                    if state.has_resistance_surveillance() {
                        let res_factor = med.resistance_factor(d_idx, &state.diseases);
                        let res_pct = ((1.0 - res_factor) * 100.0).round() as u32;
                        if res_pct > 0 {
                            let res_color = if res_pct >= 30 { Color::Red } else { Color::Yellow };
                            detail_spans.push(Span::styled(
                                format!(" Res:{}%", res_pct),
                                Style::default().fg(res_color),
                            ));
                        }
                    }
                } else {
                    detail_spans.push(Span::styled(
                        " [UNTESTED]",
                        Style::default().fg(Color::Red),
                    ));
                }
            }

            lines.push(Line::from(detail_spans));

            // Show cumulative impact if any deployments have happened
            let total_impact = med.total_treated + med.total_protected;
            if total_impact > 0.0 {
                let mut impact_spans = vec![Span::raw("    ")];
                if med.total_treated > 0.0 {
                    impact_spans.push(Span::styled(
                        format!("{} treated", format_number(med.total_treated)),
                        Style::default().fg(Color::Green),
                    ));
                }
                if med.total_treated > 0.0 && med.total_protected > 0.0 {
                    impact_spans.push(Span::raw("  "));
                }
                if med.total_protected > 0.0 {
                    impact_spans.push(Span::styled(
                        format!("{} protected", format_number(med.total_protected)),
                        Style::default().fg(Color::Green),
                    ));
                }
                lines.push(Line::from(impact_spans));
            }

            // Show manufacture hint when doses are depleted
            if med.doses <= 0.0 {
                let is_manufacturing = state.research_slot(crate::state::ResearchCategory::Applied)
                    .is_some_and(|p| matches!(&p.kind, ResearchKind::ManufactureDoses { medicine_idx: mi } if *mi == med_idx));
                if is_manufacturing {
                    lines.push(Line::from(Span::styled(
                        "    ↻ Restocking in progress (Applied Research)",
                        Style::default().fg(Color::Yellow),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "    → Restock via Research [R] > Applied Research",
                        Style::default().fg(Color::Red),
                    )));
                }
            }

            // Show re-trial hint when any disease has drifted past the medicine's calibration
            let any_strain_outdated = med.target_diseases.iter().any(|&d_idx| {
                med.strain_efficacy(d_idx, &state.diseases) < 1.0
            });
            if any_strain_outdated {
                let retrial_in_progress = state.active_research.iter().filter(|p| p.kind.category() == crate::state::ResearchCategory::Field).any(|p| {
                    matches!(&p.kind, ResearchKind::ClinicalTrial { medicine_idx: mi, .. } if *mi == med_idx)
                });
                if retrial_in_progress {
                    lines.push(Line::from(Span::styled(
                        "    ↻ Re-trial in progress, efficacy will be restored",
                        Style::default().fg(Color::Yellow),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "    → Strain drifted. Re-trial via Research [R] > Field Research",
                        Style::default().fg(Color::Red),
                    )));
                }
            }

            // Show pending shipments for this medicine
            let shipments: Vec<_> = state.pending_shipments.iter()
                .filter(|s| s.medicine_idx == med_idx)
                .collect();
            for s in &shipments {
                let region_name = state.regions.get(s.region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let doses_str = format_number(s.doses);
                let ticks_left = s.arrive_tick.saturating_sub(state.tick);
                let days_left = ticks_left as f64 / TICKS_PER_DAY;
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("→ {doses_str} en route to {region_name} ({days_left:.1}d)"),
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
        // Show auto-deploy status for selected medicine
        let auto_status = unlocked.get(state.ui.panel_selection)
            .and_then(|&(med_idx, _)| state.auto_deploy.get(med_idx).copied())
            .unwrap_or(false);
        let auto_label = if auto_status { " ON" } else { " OFF" };
        lines.push(Line::from(Span::styled(
            format!("  [↑/↓] Select  [Enter] Deploy  [X] Auto-deploy{}  [Esc] Close", auto_label),
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

        let on_cooldown = region.any_deploy_cooldown(state.tick);

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
        if on_cooldown {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "[partial cooldown]".to_string(),
                Style::default().fg(Color::Yellow),
            ));
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

    // Compute efficacy (shared formula in Medicine::effective_efficacy)
    let efficacy = med.effective_efficacy(disease_idx, &state.diseases);
    // Individual factors for display hints
    let strain_eff = med.strain_efficacy(disease_idx, &state.diseases);
    let resistance = med.resistance_factor(disease_idx, &state.diseases);
    let eff_color = if efficacy >= 0.8 {
        Color::Green
    } else if efficacy >= 0.5 {
        Color::Yellow
    } else {
        Color::Red
    };
    let strain_outdated = strain_eff < 1.0;
    let res_pct = ((1.0 - resistance) * 100.0).round() as u32;

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
            format!("{}Protect susceptible (preventive)", marker),
            style,
        )));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{} susceptible", format_number(susceptible)),
                Style::default().fg(if empty { Color::DarkGray } else { Color::Cyan }),
            ),
            Span::raw(" → will protect "),
            Span::styled(
                format_number(will_vaccinate),
                Style::default().fg(eff_color),
            ),
        ]));
        if !empty {
            let pct = if susceptible > 0.0 { will_vaccinate / susceptible * 100.0 } else { 0.0 };
            lines.push(Line::from(Span::styled(
                format!("    {:.1}% of susceptible", pct),
                Style::default().fg(Color::DarkGray),
            )));
        }
        if state.has_resistance_surveillance() {
            lines.push(Line::from(Span::styled(
                "    Resistance pressure: Low",
                Style::default().fg(Color::Green),
            )));
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
            format!("{}Treat infected (therapeutic)", marker),
            style,
        )));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{} infected", format_number(infected)),
                Style::default().fg(if empty { Color::DarkGray } else { Color::Red }),
            ),
            Span::raw(" → will treat "),
            Span::styled(
                format_number(will_treat),
                Style::default().fg(eff_color),
            ),
        ]));
        if !empty {
            let pct = if infected > 0.0 { will_treat / infected * 100.0 } else { 0.0 };
            lines.push(Line::from(Span::styled(
                format!("    {:.0}% of infected", pct),
                Style::default().fg(Color::DarkGray),
            )));
        }
        if state.has_resistance_surveillance() {
            lines.push(Line::from(Span::styled(
                "    Resistance pressure: High (6x vs. preventive)",
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    // Efficacy info
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Efficacy: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:.0}%", efficacy * 100.0),
            Style::default().fg(eff_color),
        ),
    ]));
    if strain_outdated {
        let behind = med.mutations_behind(disease_idx, &state.diseases);
        let behind_str = if behind > 0 {
            format!(", {} mutation{} behind", behind, if behind == 1 { "" } else { "s" })
        } else {
            String::new()
        };
        let retrial_in_progress = state.active_research.iter().filter(|p| p.kind.category() == crate::state::ResearchCategory::Field).any(|p| {
            matches!(&p.kind, ResearchKind::ClinicalTrial { medicine_idx: mi, disease_idx: di }
                if *mi == medicine_idx && *di == disease_idx)
        });
        let action = if retrial_in_progress {
            ": re-trial in progress"
        } else {
            ": re-trial in Research [R] to restore"
        };
        lines.push(Line::from(Span::styled(
            format!("  Strain drift{}{}", behind_str, action),
            Style::default().fg(Color::Yellow),
        )));
    }
    if state.has_resistance_surveillance() && res_pct > 0 {
        let res_color = if res_pct >= 30 { Color::Red } else { Color::Yellow };
        let warning = if res_pct >= 50 { ", consider switching drugs" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("  Resistance: {}%{}", res_pct, warning),
            Style::default().fg(res_color),
        )));
    }

    let deploy_cost = state.medicine_deploy_cost(medicine_idx, region_idx);
    lines.push(Line::from(vec![
        Span::styled("  Cost: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("¥{:.0}", deploy_cost),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("    "),
        Span::styled("Funding: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("¥{:.0}", state.resources.funding),
            Style::default().fg(if state.resources.funding >= deploy_cost {
                Color::Green
            } else {
                Color::Red
            }),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Doses: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}/{}", format_number(med.doses), format_number(med.max_doses)),
            Style::default().fg(dose_color(med)),
        ),
    ]));

    // Show targeting efficiency (screening-dependent dose waste)
    let targeting_eff = state.targeting_efficiency(region_idx);
    if targeting_eff < 0.99 {
        let waste_pct = ((1.0 - targeting_eff) * 100.0) as u32;
        let color = if targeting_eff < 0.60 { Color::Red } else if targeting_eff < 0.80 { Color::Yellow } else { Color::DarkGray };
        lines.push(Line::from(vec![
            Span::styled("  Targeting: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.0}%", targeting_eff * 100.0),
                Style::default().fg(color),
            ),
            Span::styled(
                format!(" ({waste_pct}% dose waste — improve screening to reduce)"),
                Style::default().fg(color),
            ),
        ]));
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
