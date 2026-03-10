use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{DeployTarget, GameState, Medicine, MedicineUiState, grid_reading_order, KNOWLEDGE_NAME, TICKS_PER_DAY};
use crate::ui::hint_line;
use crate::format_number;

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
    let unlocked: Vec<(usize, &Medicine)> = state.medicines.iter().enumerate()
        .filter(|(_, m)| m.unlocked).collect();

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
                    let strain_eff = med.strain_efficacy(d_idx, &state.diseases);
                    let res_factor = med.resistance_factor(d_idx, &state.diseases);
                    let combined = strain_eff * res_factor;
                    let pct = (combined * 100.0).round() as u32;
                    let color = if pct >= 85 {
                        Color::Green
                    } else if pct >= 50 {
                        Color::Yellow
                    } else {
                        Color::Red
                    };
                    // Show ▼ when efficacy has degraded below 85%
                    let trend = if pct < 85 { "\u{25bc}" } else { "" };
                    detail_spans.push(Span::styled(
                        format!(" ({}%{})", pct, trend),
                        Style::default().fg(color),
                    ));
                    // Show resistance level if surveillance unlocked
                    if state.has_resistance_surveillance() {
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

            // Show pending shipments for this medicine
            let shipments: Vec<_> = state.pending_shipments.iter()
                .filter(|s| s.medicine_idx == med_idx)
                .collect();
            for s in &shipments {
                let region_name = state.regions.get(s.region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let doses_str = format_number(s.doses);
                if s.blocked_by_travel_ban {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            format!("⚠ {doses_str} → {region_name} BLOCKED (travel ban)"),
                            Style::default().fg(Color::Red),
                        ),
                    ]));
                } else {
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

    lines.push(Line::from(Span::styled(
        format!("  Deploy: {}", med.name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

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
    lines.push(hint_line(state, "Select", "Back"));

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

    lines.push(Line::from(Span::styled(
        format!("  {} → {}", med.name, region.name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

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
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", name), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!(" ({}, incompatible)", med.therapy_type.label()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Select", "Back"));

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

    lines.push(Line::from(Span::styled(
        format!("  {} → {} → {}", med.name, region.name, disease_name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

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
        let infected = inf.map(|i| i.infected).unwrap_or(0.0);
        let shows_immune = state.policies.get(region_idx)
            .map(|p| p.screening.shows_immune())
            .unwrap_or(false);
        let immune = if shows_immune { inf.map(|i| i.immune).unwrap_or(0.0) } else { 0.0 };
        let susceptible = (pop - infected - region.dead - immune).max(0.0);
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
        lines.push(Line::from(Span::styled(
            format!("  Strain outdated ({:.0}% match)", strain_eff * 100.0),
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

    let deploy_cost = med.deploy_cost();
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

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Deploy", "Back"));

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
