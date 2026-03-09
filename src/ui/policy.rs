use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, PolicyUiState, ScreeningLevel, TransmissionVector, TICKS_PER_DAY,
    TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL,
    QUARANTINE_COST, QUARANTINE_PERSONNEL,
    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL, HOSPITAL_SURGE_SPREAD_FACTOR,
    BORDER_CONTROLS_COST, BORDER_CONTROLS_PERSONNEL,
    WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
    MARTIAL_LAW_COST, MARTIAL_LAW_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    HEALTHCARE_INVESTMENT_COST,
    SCREENING_BASIC_COST, SCREENING_ANTIGEN_COST, SCREENING_MASS_RAPID_COST,
    grid_reading_order, POLICY_POL_THRESHOLDS,
    DECREE_COUNT, DECREE_POL_THRESHOLDS,
    decree_display_name,
    CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY,
    SACRIFICE_INCOME_BONUS,
};
use crate::ui::hint_line;
use crate::format_number;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines) = match &state.ui.policy_ui {
        Some(PolicyUiState::ManagePolicies { region_idx }) => {
            render_manage(state, *region_idx)
        }
        Some(PolicyUiState::SelectSacrificeRegion) => {
            render_sacrifice_select(state)
        }
        _ => render_browse(state),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_browse(state: &GameState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    let total_cost = state.total_policy_funding_cost();
    if total_cost > 0.0 {
        lines.push(Line::from(vec![
            Span::styled("  Policy cost: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("${:.0}/day", total_cost * TICKS_PER_DAY),
                Style::default().fg(Color::Yellow),
            ),
        ]));
        lines.push(Line::from(""));
    }

    let order = grid_reading_order(state.regions.len());
    for (display_pos, &region_idx) in order.iter().enumerate() {
        let region = &state.regions[region_idx];
        let selected = state.ui.panel_selection == display_pos;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let policy = state.policies.get(region_idx);
        let has_active = policy.is_some_and(|p| p.any_active()) || region.healthcare_invested;

        let mut spans = vec![
            Span::styled(format!("{}{:<16}", marker, region.name), style),
        ];

        if has_active {
            let cost = policy.map(|p| p.funding_cost()).unwrap_or(0.0);
            let mut labels: Vec<&str> = [
                policy.is_some_and(|p| p.travel_ban).then_some("Travel Ban"),
                policy.is_some_and(|p| p.quarantine).then_some("Quarantine"),
                policy.is_some_and(|p| p.hospital_surge).then_some("Hospital"),
                policy.is_some_and(|p| p.border_controls).then_some("Border"),
                policy.is_some_and(|p| p.water_sanitation).then_some("Sanitation"),
                policy.is_some_and(|p| p.martial_law).then_some("Martial Law"),
                policy.is_some_and(|p| p.nuclear_annihilation).then_some("☢ NUKED"),
                region.healthcare_invested.then_some("Healthcare"),
            ].into_iter().flatten().collect();
            if let Some(p) = policy {
                match p.screening {
                    ScreeningLevel::Basic => labels.push("Screen:Basic"),
                    ScreeningLevel::Antigen => labels.push("Screen:Ag"),
                    ScreeningLevel::MassRapid => labels.push("Screen:Rapid"),
                    ScreeningLevel::None => {}
                }
            }

            spans.push(Span::styled(
                labels.join(", "),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::styled(
                format!("  ${:.0}/day", cost * TICKS_PER_DAY),
                Style::default().fg(Color::Yellow),
            ));
        } else {
            spans.push(Span::styled(
                "No active policies",
                Style::default().fg(Color::DarkGray),
            ));
        }

        lines.push(Line::from(spans));
    }

    // Emergency Decrees section
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ─── EMERGENCY DECREES ───",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));

    let num_regions = state.regions.len();
    let decree_descs: [String; DECREE_COUNT] = [
        format!("+{} personnel, -${:.0}/day income (permanent)",
            CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY * TICKS_PER_DAY),
        "Clinical trials 50% faster, risk of adverse events (permanent)".to_string(),
        format!("Abandon a region, +{:.0}% income from the rest (permanent)",
            (SACRIFICE_INCOME_BONUS - 1.0) * 100.0),
    ];

    for decree_idx in 0..DECREE_COUNT {
        let display_pos = num_regions + decree_idx;
        let selected = state.ui.panel_selection == display_pos;
        let marker = if selected { "▶ " } else { "  " };
        let enacted = state.enacted_decrees.is_enacted(decree_idx);
        let name = decree_display_name(decree_idx);

        if enacted {
            let name_style = if selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let mut spans = vec![
                Span::styled(format!("{}", marker), name_style),
                Span::styled("[ENACTED] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(name.to_string(), name_style),
            ];
            // Show sacrifice target
            if decree_idx == 2 {
                if let Some(r_idx) = state.enacted_decrees.sacrificed_region {
                    let r_name = state.regions.get(r_idx)
                        .map(|r| r.name.as_str())
                        .unwrap_or("?");
                    spans.push(Span::styled(
                        format!(" ({})", r_name),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            lines.push(Line::from(spans));
        } else {
            let pol_unlocked = state.resources.political_power >= DECREE_POL_THRESHOLDS[decree_idx];
            if !pol_unlocked {
                let name_style = if selected {
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{}", marker), name_style),
                    Span::styled("🔒 ", Style::default().fg(Color::DarkGray)),
                    Span::styled(name.to_string(), name_style),
                    Span::styled(
                        format!("  (POL {:.0}%)", DECREE_POL_THRESHOLDS[decree_idx] * 100.0),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            } else {
                let name_style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Red)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{}", marker), name_style),
                    Span::styled("⚠ ", Style::default().fg(Color::Red)),
                    Span::styled(name.to_string(), name_style),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(decree_descs[decree_idx].clone(), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Select", "Close"));

    (" Policy ".to_string(), lines)
}

fn render_manage(state: &GameState, region_idx: usize) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let region = &state.regions[region_idx];
    let policy = state.policies.get(region_idx).cloned().unwrap_or_default();

    lines.push(Line::from(Span::styled(
        format!("  {}", region.name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    let visibility = state.screening_visibility(region_idx);
    let infected = region.screened_infected(&state.diseases, visibility);
    let dead = region.detected_dead(&state.diseases);
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("Pop: {}  ", format_number(region.population as f64)),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("Inf: {}  ", format_number(infected)),
            Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
        ),
        Span::styled(
            format!("Dead: {}", format_number(dead)),
            Style::default().fg(if dead > 0.0 { Color::Red } else { Color::DarkGray }),
        ),
    ]));
    lines.push(Line::from(""));

    // Policy toggles — each entry explicitly carries its policy_idx (see POLICY_COUNT
    // doc in state.rs for the index mapping). Display position != policy_idx in general,
    // though currently they happen to match.
    //                   (policy_idx, name, active, cost_str, desc, personnel_needed)
    let policies: Vec<(usize, &str, bool, String, &str, Option<u32>)> = vec![
        (0, "Travel Ban", policy.travel_ban,
         format!("${:.0}/day + {} pers.", TRAVEL_BAN_COST * TICKS_PER_DAY, TRAVEL_BAN_PERSONNEL),
         "Reduces cross-region spread, halves income", Some(TRAVEL_BAN_PERSONNEL)),
        (1, "Quarantine", policy.quarantine,
         format!("${:.0}/day + {} pers.", QUARANTINE_COST * TICKS_PER_DAY, QUARANTINE_PERSONNEL),
         "Reduces infection rate (varies by transmission)", Some(QUARANTINE_PERSONNEL)),
        (2, "Hospital Surge", policy.hospital_surge,
         format!("${:.0}/day + {} pers.", HOSPITAL_SURGE_COST * TICKS_PER_DAY, HOSPITAL_SURGE_PERSONNEL),
         "Halves lethality, +25% spread (hospital exposure)", Some(HOSPITAL_SURGE_PERSONNEL)),
        (3, "Border Controls", policy.border_controls,
         format!("${:.0}/day + {} pers.", BORDER_CONTROLS_COST * TICKS_PER_DAY, BORDER_CONTROLS_PERSONNEL),
         "Blocks 50% spread into/out of region", Some(BORDER_CONTROLS_PERSONNEL)),
        (4, "Water Sanitation", policy.water_sanitation,
         format!("${:.0}/day + {} pers.", WATER_SANITATION_COST * TICKS_PER_DAY, WATER_SANITATION_PERSONNEL),
         "Halves waterborne spread within the region", Some(WATER_SANITATION_PERSONNEL)),
        (5, "Basic Screening", policy.screening == ScreeningLevel::Basic,
         format!("${:.0}/day + 1 pers.", SCREENING_BASIC_COST * TICKS_PER_DAY),
         "Rough infected estimates, faster detection", Some(1)),
        (6, "Antigen Screening", policy.screening == ScreeningLevel::Antigen,
         format!("${:.0}/day + 2 pers.", SCREENING_ANTIGEN_COST * TICKS_PER_DAY),
         "Shows infected + immune counts, good accuracy", Some(2)),
        (7, "Mass Rapid Screen", policy.screening == ScreeningLevel::MassRapid,
         format!("${:.0}/day + 4 pers.", SCREENING_MASS_RAPID_COST * TICKS_PER_DAY),
         "Near-complete data, reduces spread by 25%", Some(4)),
        (8, "Martial Law", policy.martial_law,
         format!("${:.0}/day + {} pers.", MARTIAL_LAW_COST * TICKS_PER_DAY, MARTIAL_LAW_PERSONNEL),
         "+15% collapse resilience (must enact before collapse)", Some(MARTIAL_LAW_PERSONNEL)),
        (9, "☢ Nuclear Option", policy.nuclear_annihilation,
         format!("One-time: ${:.0}", NUCLEAR_ANNIHILATION_COST),
         "Eliminate 99% of population — stops all disease spread", None),
        (10, "Healthcare Investment", region.healthcare_invested,
         format!("One-time: ${:.0}", HEALTHCARE_INVESTMENT_COST),
         "Permanent 25% lethality reduction", None),
    ];

    for (display_pos, (policy_idx, name, active, cost_str, desc, personnel_needed)) in policies.iter().enumerate() {
        let selected = state.ui.panel_selection == display_pos;
        let marker = if selected { "▶ " } else { "  " };

        // Collapsed regions: only nuclear annihilation (idx 9) is available
        // Non-collapsed regions: nuclear annihilation is not available
        // Healthcare investment (idx 10): only available pre-collapse
        let structurally_locked = if region.collapsed {
            *policy_idx != 9 && !*active
        } else {
            *policy_idx == 9
        };

        if structurally_locked {
            let name_style = if selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let reason = if region.collapsed { "collapsed" } else { "not collapsed" };
            lines.push(Line::from(vec![
                Span::styled(format!("{}", marker), name_style),
                Span::styled("— ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", name), name_style),
                Span::styled(
                    format!("  ({})", reason),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        let pol_unlocked = state.policy_unlocked(region_idx, *policy_idx);

        let can_afford_personnel = personnel_needed
            .map(|need| {
                let mut avail = state.personnel_available();
                if *active {
                    // If already active, its personnel would be freed on disable
                    avail += need;
                } else if *policy_idx >= 5 && *policy_idx <= 7 {
                    // Screening upgrade: personnel from current tier would be freed
                    avail += policy.screening.personnel_cost();
                }
                avail >= need
            })
            .unwrap_or(true);

        if !*active && !pol_unlocked {
            // Locked by POL — show as unavailable
            let threshold = POLICY_POL_THRESHOLDS[*policy_idx];
            let name_style = if selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}", marker), name_style),
                Span::styled("🔒 ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", name), name_style),
                Span::styled(
                    format!("  (POL {:.0}%)", threshold * 100.0),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        let status_style = if *active {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else if can_afford_personnel {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Red)
        };

        let name_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let status = if *active { "[ON] " } else { "[OFF]" };

        lines.push(Line::from(vec![
            Span::styled(format!("{}", marker), name_style),
            Span::styled(format!("{} ", status), status_style),
            Span::styled(format!("{}", name), name_style),
        ]));
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(*desc, Style::default().fg(Color::DarkGray)),
        ]));
        // Effectiveness hints for transmission-sensitive policies
        if let Some(hint) = effectiveness_hint(state, region_idx, *policy_idx) {
            lines.push(hint);
        }
        // Estimated daily impact for active policies
        if *active {
            if let Some(impact) = impact_estimate(state, region_idx, *policy_idx) {
                lines.push(impact);
            }
        }
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("Cost: {cost_str}"),
                Style::default().fg(Color::Yellow),
            ),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(hint_line(state, "Toggle", "Back"));

    (format!(" Policy: {} ", region.name), lines)
}

fn render_sacrifice_select(state: &GameState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "  ⚠ SACRIFICE REGION — THIS CANNOT BE UNDONE ⚠",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  The chosen region will be abandoned. Remaining regions",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        format!("  gain +{:.0}% income. Select a region to sacrifice:",
            (SACRIFICE_INCOME_BONUS - 1.0) * 100.0),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let non_collapsed: Vec<(usize, &crate::state::Region)> = state.regions.iter()
        .enumerate()
        .filter(|(_, r)| !r.collapsed)
        .collect();

    for (display_pos, (_, region)) in non_collapsed.iter().enumerate() {
        let selected = state.ui.panel_selection == display_pos;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let pop_str = format_number(region.population as f64);
        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<16}", marker, region.name), style),
            Span::styled(
                format!("Pop: {}", pop_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Sacrifice", "Cancel"));

    (" ⚠ Sacrifice Region ".to_string(), lines)
}

/// Generate an effectiveness hint line for transmission-sensitive policies.
/// Shows per-disease reduction percentages based on transmission vector.
fn effectiveness_hint(state: &GameState, region_idx: usize, policy_idx: usize) -> Option<Line<'static>> {
    // Only transmission-sensitive policies get hints
    // 0=Travel Ban, 1=Quarantine, 2=Hospital Surge, 4=Water Sanitation
    if !matches!(policy_idx, 0 | 1 | 2 | 4) {
        return None;
    }

    let region = &state.regions[region_idx];

    // Collect detected diseases with active infections in this region
    let active_diseases: Vec<(String, TransmissionVector)> = region
        .infections
        .iter()
        .filter(|inf| inf.infected > 0.0)
        .filter_map(|inf| {
            let disease = state.diseases.get(inf.disease_idx)?;
            if disease.detected {
                Some((disease.display_name(inf.disease_idx), disease.transmission))
            } else {
                None
            }
        })
        .collect();

    if active_diseases.is_empty() {
        return None;
    }

    let mut spans: Vec<Span<'static>> = vec![Span::raw("      → ")];

    for (j, (name, vector)) in active_diseases.iter().enumerate() {
        if j > 0 {
            spans.push(Span::styled(", ", Style::default().fg(Color::DarkGray)));
        }

        let (label, color) = match policy_idx {
            0 => { // Travel Ban
                let reduction = (1.0 - vector.travel_ban_factor()) * 100.0;
                let color = if reduction >= 80.0 { Color::Green } else { Color::Yellow };
                (format!("{name} ({}, -{reduction:.0}%)", vector.label()), color)
            }
            1 => { // Quarantine
                let reduction = (1.0 - vector.quarantine_factor()) * 100.0;
                let color = if reduction >= 50.0 { Color::Green }
                    else if reduction >= 30.0 { Color::Yellow }
                    else { Color::Red };
                (format!("{name} ({}, -{reduction:.0}%)", vector.label()), color)
            }
            2 => { // Hospital Surge — universal +25% spread
                let increase = (HOSPITAL_SURGE_SPREAD_FACTOR - 1.0) * 100.0;
                (format!("{name} ({}, +{increase:.0}% spread!)", vector.label()), Color::Red)
            }
            4 => { // Water Sanitation
                match vector {
                    TransmissionVector::Waterborne => {
                        (format!("{name} (waterborne, -50%)"), Color::Green)
                    }
                    _ => {
                        (format!("{name} ({}, no effect)", vector.label()), Color::DarkGray)
                    }
                }
            }
            _ => unreachable!(),
        };

        spans.push(Span::styled(label, Style::default().fg(color)));
    }

    Some(Line::from(spans))
}

/// Estimated daily impact for an active policy. Shows approximate infections
/// or deaths prevented per day based on current disease parameters and counts.
fn impact_estimate(state: &GameState, region_idx: usize, policy_idx: usize) -> Option<Line<'static>> {
    let region = &state.regions[region_idx];
    let pop = region.population as f64;
    if pop <= 0.0 {
        return None;
    }

    // Collect impact across all active detected diseases in this region
    let mut total_impact: f64 = 0.0;
    let mut impact_type = "";

    for inf in &region.infections {
        if inf.infected <= 0.0 {
            continue;
        }
        let Some(disease) = state.diseases.get(inf.disease_idx) else {
            continue;
        };
        if !disease.detected {
            continue;
        }

        let alive = (pop - region.dead).max(0.0);
        let susceptible = alive - inf.infected - inf.immune;

        match policy_idx {
            0 => {
                // Travel Ban: can't easily estimate cross-region prevention
                // Show income penalty instead (already shown elsewhere)
                return None;
            }
            1 => {
                // Quarantine: infections prevented = infected × infectivity × (1 - factor) × susceptible/pop
                if susceptible > 0.0 {
                    let factor = disease.transmission.quarantine_factor();
                    let prevented = inf.infected * disease.infectivity * (1.0 - factor) * (susceptible / pop);
                    total_impact += prevented;
                }
                impact_type = "infections";
            }
            2 => {
                // Hospital Surge: deaths prevented = infected × lethality × 0.5
                let prevented = inf.infected * disease.lethality * 0.5;
                total_impact += prevented;
                impact_type = "deaths";
            }
            3 => {
                // Border Controls: cross-region spread prevention — hard to estimate
                return None;
            }
            4 => {
                // Water Sanitation: infections prevented for waterborne diseases
                if susceptible > 0.0 {
                    let factor = disease.transmission.water_sanitation_factor();
                    if factor < 1.0 {
                        let prevented = inf.infected * disease.infectivity * (1.0 - factor) * (susceptible / pop);
                        total_impact += prevented;
                    }
                }
                impact_type = "infections";
            }
            _ => return None,
        }
    }

    if total_impact <= 0.0 {
        return None;
    }

    let daily_impact = total_impact * TICKS_PER_DAY;
    let formatted = format_number(daily_impact);

    Some(Line::from(vec![
        Span::raw("      "),
        Span::styled(
            format!("Est. ~{formatted} fewer {impact_type}/day"),
            Style::default().fg(Color::Green),
        ),
    ]))
}
