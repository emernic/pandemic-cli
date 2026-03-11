use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, PolicyUiState, RegionPriority, RegionTrait, ScreeningLevel, TRADE_DEPENDENT_TRAVEL_BAN_MULT, TransmissionVector, TICKS_PER_DAY,
    TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL,
    QUARANTINE_COST, QUARANTINE_PERSONNEL,
    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL, HOSPITAL_SURGE_SPREAD_FACTOR,
    BORDER_CONTROLS_COST, BORDER_CONTROLS_PERSONNEL,
    WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
    MARTIAL_LAW_COST, MARTIAL_LAW_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    FIELD_HOSPITAL_COST, FIELD_HOSPITAL_PERSONNEL,
    MEDICAL_CENTER_COST, MEDICAL_CENTER_PERSONNEL,
    INTEL_STATION_COST, INTEL_STATION_PERSONNEL,
    ADVANCED_INTEL_COST, ADVANCED_INTEL_PERSONNEL,
    SCREENING_BASIC_COST, SCREENING_ANTIGEN_COST, SCREENING_MASS_RAPID_COST,
    POLICY_POL_THRESHOLDS, POLICY_IDX_NUCLEAR, POLICY_IDX_SCREENING_BASE,
    decree_display_name,
    CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY,
    SACRIFICE_INCOME_BONUS, FORTIFY_INFRA_PENALTY,
    COUNTERMEASURE_KILL_FRACTION, COUNTERMEASURE_INFECTIVITY_MULT, COUNTERMEASURE_SPREAD_MULT,
    MANAGE_PRIORITY_POS, MANAGE_APPEASE_POS, MANAGE_BARGAIN_POS,
    policy_display_order, APPEASE_COST, APPEASE_LOYALTY_GAIN,
    BARGAIN_LOYALTY_GAIN, BARGAIN_BLOWHARD_LOYALTY_GAIN,
    BARGAIN_BUFFOON_POL_COST, BARGAIN_BLOWHARD_FUNDING_COST,
    BARGAIN_RECLUSE_PERSONNEL_COST, BARGAIN_HARDLINER_FUNDING_COST,
    BARGAIN_OPERATIVE_INCOME_CUT, BARGAIN_MOBSTER_BASE_COST,
    GovernorPersonality,
};
use crate::ui::hint_line;
use crate::format_number;

/// Section header labels for the policy panel, keyed by display position.
/// Returns Some(header) at the start of each group, None otherwise.
fn policy_section_header(display_pos: usize) -> Option<&'static str> {
    match display_pos {
        0 => Some("Infrastructure"),
        2 => Some("Detection"),
        5 => Some("Containment"),
        8 => Some("Medical"),
        10 => Some("Extreme"),
        _ => None,
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines, selected_line) = match &state.ui.policy_ui {
        Some(PolicyUiState::ManagePolicies { region_idx }) => {
            render_manage(state, *region_idx)
        }
        None => render_manage(state, state.ui.map_selection),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    // Scroll to keep the selected item visible. The panel's inner height
    // is area.height minus 2 (top + bottom border).
    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = selected_line.map(|line| {
        if line as u16 >= inner_height {
            // Keep selected item ~1/3 from bottom so context is visible above
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


fn render_manage(state: &GameState, region_idx: usize) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let region = &state.regions[region_idx];
    let policy = state.policies.get(region_idx).cloned().unwrap_or_default();

    lines.push(Line::from(Span::styled(
        format!("  {}", region.name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    let infected = region.screened_infected();
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

    // Show regional traits
    if !region.traits.is_empty() {
        let trait_labels: Vec<&str> = region.traits.iter().map(|t| t.label()).collect();
        lines.push(Line::from(vec![
            Span::raw("  Traits: "),
            Span::styled(trait_labels.join(", "), Style::default().fg(Color::Yellow)),
        ]));
        lines.push(Line::from(""));
    }

    // Trait-adjusted costs
    let low_infra = region.has_trait(RegionTrait::LowInfrastructure);
    let trade_dep = region.has_trait(RegionTrait::TradeDependent);
    let infra_extra: u32 = if low_infra { 1 } else { 0 };
    let tb_cost = if trade_dep { TRAVEL_BAN_COST * TRADE_DEPENDENT_TRAVEL_BAN_MULT } else { TRAVEL_BAN_COST };

    // Policy toggles — each entry explicitly carries its policy_idx (see POLICY_COUNT
    // doc in state.rs for the index mapping). Display position != policy_idx (grouped
    // by function via policy_display_order()).
    // tick_cost: per-tick ongoing funding cost (0.0 for one-time purchases like hospitals/intel).
    // Used to mute policies when funding ≤ 0 — the engine suspends ongoing-cost policies at that point.
    //                   (policy_idx, name, active, cost_str, desc, personnel_needed, tick_cost)
    let policies: Vec<(usize, &str, bool, String, &str, Option<u32>, f64)> = vec![
        (0, "Travel Ban", policy.travel_ban,
         format!("¥{:.0}/day + {} pers.", tb_cost * TICKS_PER_DAY, TRAVEL_BAN_PERSONNEL + infra_extra),
         if trade_dep { "Reduces cross-region spread, 75% income penalty" }
         else { "Reduces cross-region spread, halves income" },
         Some(TRAVEL_BAN_PERSONNEL + infra_extra), tb_cost),
        (1, "Quarantine", policy.quarantine,
         format!("¥{:.0}/day + {} pers.", QUARANTINE_COST * TICKS_PER_DAY, QUARANTINE_PERSONNEL + infra_extra),
         "Reduces infection rate (varies by transmission)", Some(QUARANTINE_PERSONNEL + infra_extra), QUARANTINE_COST),
        (2, "Hospital Surge", policy.hospital_surge,
         format!("¥{:.0}/day + {} pers.", HOSPITAL_SURGE_COST * TICKS_PER_DAY, HOSPITAL_SURGE_PERSONNEL + infra_extra),
         if region.has_trait(RegionTrait::StrongPublicHealth) {
             "60% lethality reduction, +25% spread (hospital exposure)"
         } else {
             "Halves lethality, +25% spread (hospital exposure)"
         },
         Some(HOSPITAL_SURGE_PERSONNEL + infra_extra), HOSPITAL_SURGE_COST),
        (3, "Border Controls", policy.border_controls,
         format!("¥{:.0}/day + {} pers.", BORDER_CONTROLS_COST * TICKS_PER_DAY, BORDER_CONTROLS_PERSONNEL + infra_extra),
         "Blocks 50% spread into/out of region", Some(BORDER_CONTROLS_PERSONNEL + infra_extra), BORDER_CONTROLS_COST),
        (4, "Water Sanitation", policy.water_sanitation,
         format!("¥{:.0}/day + {} pers.", WATER_SANITATION_COST * TICKS_PER_DAY, WATER_SANITATION_PERSONNEL + infra_extra),
         "Halves waterborne spread within the region", Some(WATER_SANITATION_PERSONNEL + infra_extra), WATER_SANITATION_COST),
        (5, "Basic Screening", policy.screening == ScreeningLevel::Basic,
         format!("¥{:.0}/day + {} pers.", SCREENING_BASIC_COST * TICKS_PER_DAY, 1 + infra_extra),
         "~40% of infections visible, detects outbreaks earlier (~4 day ramp-up)", Some(1 + infra_extra), SCREENING_BASIC_COST),
        (6, "Antigen Screening", policy.screening == ScreeningLevel::Antigen,
         format!("¥{:.0}/day + {} pers.", SCREENING_ANTIGEN_COST * TICKS_PER_DAY, 2 + infra_extra),
         "~75% visible + reveals immune population (~4 day ramp-up)", Some(2 + infra_extra), SCREENING_ANTIGEN_COST),
        (7, "Mass Rapid Screen", policy.screening == ScreeningLevel::MassRapid,
         format!("¥{:.0}/day + {} pers.", SCREENING_MASS_RAPID_COST * TICKS_PER_DAY, 4 + infra_extra),
         "~95% visible, reduces spread by 25% (~4 day ramp-up)", Some(4 + infra_extra), SCREENING_MASS_RAPID_COST),
        (8, "Martial Law", policy.martial_law,
         format!("¥{:.0}/day + {} pers.", MARTIAL_LAW_COST * TICKS_PER_DAY, MARTIAL_LAW_PERSONNEL + infra_extra),
         "+15% collapse resilience (must enact before collapse)", Some(MARTIAL_LAW_PERSONNEL + infra_extra), MARTIAL_LAW_COST),
        (9, "Nuclear Option", policy.nuclear_annihilation,
         format!("One-time: ¥{:.0}", NUCLEAR_ANNIHILATION_COST),
         "Eliminate 99% of population. Stops all disease spread.", None, 0.0),
        (10,
         match region.hospital_level {
             0 => "Build Field Hospital",
             1 => "Upgrade → Medical Center",
             _ => "Medical Center (built)",
         },
         region.hospital_level >= 2,
         match region.hospital_level {
             0 => format!("¥{:.0} + {} pers.", FIELD_HOSPITAL_COST, FIELD_HOSPITAL_PERSONNEL),
             1 => format!("¥{:.0} + {} pers.", MEDICAL_CENTER_COST, MEDICAL_CENTER_PERSONNEL - FIELD_HOSPITAL_PERSONNEL),
             _ => format!("{} pers. ongoing", MEDICAL_CENTER_PERSONNEL),
         },
         match region.hospital_level {
             0 => "25% lethality reduction, +10 governor loyalty",
             1 => "40% lethality reduction, +25% medicine efficacy, +10 loyalty",
             _ => "40% lethality reduction, +25% medicine efficacy",
         },
         None, 0.0),
        (11,
         match region.intel_level {
             0 => "Build Intel Station",
             1 => "Upgrade → Advanced Intel",
             _ => "Advanced Intel (built)",
         },
         region.intel_level >= 2,
         match region.intel_level {
             0 => format!("¥{:.0} + {} pers.", INTEL_STATION_COST, INTEL_STATION_PERSONNEL),
             1 => format!("¥{:.0} + {} pers.", ADVANCED_INTEL_COST, ADVANCED_INTEL_PERSONNEL - INTEL_STATION_PERSONNEL),
             _ => format!("{} pers. ongoing", ADVANCED_INTEL_PERSONNEL),
         },
         match region.intel_level {
             0 => "Detects new pathogens at 3,000 local infections (vs. 10,000)",
             1 => "Detects at 1,000 infections. Generates pre-detection briefings at 500.",
             _ => "Early warning active. Pre-detection surveillance operational.",
         },
         None, 0.0),
    ];

    // Reorder by canonical display order (grouped by function — see policy_display_order() doc).
    // display_pos == panel_selection; confirm handler maps back via policy_display_order().
    let policies: Vec<_> = policy_display_order().iter().map(|&idx| policies[idx].clone()).collect();

    for (display_pos, (policy_idx, name, active, cost_str, desc, personnel_needed, tick_cost)) in policies.iter().enumerate() {
        // Insert section headers between policy groups
        if let Some(header) = policy_section_header(display_pos) {
            if display_pos > 0 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!("  ─── {} ───", header),
                Style::default().fg(Color::DarkGray),
            )));
        }

        let selected = state.ui.panel_selection == display_pos;
        if selected { selected_line = Some(lines.len()); }
        let marker = if selected { "▶ " } else { "  " };

        // Collapsed regions: only nuclear annihilation is available
        // Non-collapsed regions: nuclear annihilation is not available
        // Healthcare investment (idx 10): only available pre-collapse
        let structurally_locked = if region.collapsed {
            *policy_idx != POLICY_IDX_NUCLEAR && !*active
        } else {
            *policy_idx == POLICY_IDX_NUCLEAR
        };

        if structurally_locked {
            let name_style = if selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let reason = if region.collapsed { "collapsed" } else { "not collapsed" };
            // Nuclear uses ☢ icon to match the 🔒 icon pattern for AUTH-locked items.
            let icon = if *policy_idx == POLICY_IDX_NUCLEAR { "☢ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(format!("{}", marker), name_style),
                Span::styled(icon, Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", name), name_style),
                Span::styled(
                    format!("  ({})", reason),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            // No blank line after structurally-locked items to save panel space
            continue;
        }

        let pol_unlocked = state.policy_unlocked(region_idx, *policy_idx);

        let can_afford_personnel = personnel_needed
            .map(|need| {
                let mut avail = state.personnel_available();
                if *active {
                    // If already active, its personnel would be freed on disable
                    avail += need;
                } else if *policy_idx >= POLICY_IDX_SCREENING_BASE && *policy_idx <= POLICY_IDX_SCREENING_BASE + 2 {
                    // Screening upgrade: personnel from current tier would be freed
                    avail += policy.screening.personnel_cost();
                }
                avail >= need
            })
            .unwrap_or(true);

        // Policies with ongoing funding costs will be immediately suspended by the engine
        // when funding ≤ 0. Mute them so the player knows enabling them achieves nothing.
        let can_afford_funding = *active || *tick_cost == 0.0 || state.resources.funding > 0.0;
        let can_afford = can_afford_personnel && can_afford_funding;

        if !*active && !pol_unlocked {
            // Locked by AUTH — show as unavailable
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
                    format!("  (AUTH {:.0}%)", threshold * 100.0),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            // No blank line after AUTH-locked items to save panel space
            continue;
        }

        let status_style = if *active {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else if can_afford {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Red)
        };

        let name_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if !*active && !can_afford {
            // Unaffordable: mute name — player can see but enabling achieves nothing
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let status = if *active { "[ON] " } else { "[OFF]" };

        let mut row = vec![
            Span::styled(format!("{}", marker), name_style),
            Span::styled(format!("{} ", status), status_style),
        ];
        if *policy_idx == POLICY_IDX_NUCLEAR {
            row.push(Span::styled("☢ ", Style::default().fg(Color::Yellow)));
        }
        row.push(Span::styled(format!("{}", name), name_style));
        lines.push(Line::from(row));
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

    // Deployment Priority toggle (MANAGE_PRIORITY_POS)
    if !region.collapsed {
        let selected = state.ui.panel_selection == MANAGE_PRIORITY_POS;
        if selected { selected_line = Some(lines.len()); }
        let marker = if selected { "▶ " } else { "  " };
        let name_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let priority = region.deploy_priority;
        let priority_color = match priority {
            RegionPriority::High => Color::Green,
            RegionPriority::Normal => Color::White,
            RegionPriority::Low => Color::DarkGray,
            RegionPriority::CutOff => Color::Red,
        };
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), name_style),
            Span::styled("Deploy Priority: ", name_style),
            Span::styled(
                format!("[{}]", priority.label()),
                Style::default().fg(priority_color).add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                "Controls auto-deploy targeting order",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Appease Governor action (after repair actions)
    if !region.collapsed {
        let appease_pos = MANAGE_APPEASE_POS;
        let selected = state.ui.panel_selection == appease_pos;
        if selected { selected_line = Some(lines.len()); }
        let gov = &region.governor;
        let marker = if selected { "▶ " } else { "  " };
        let name_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let loyalty_color = if gov.is_defiant() {
            Color::Red
        } else if gov.is_cooperative() {
            Color::Green
        } else {
            Color::Yellow
        };
        let status_label = if gov.is_defiant() {
            "DEFIANT"
        } else if gov.is_cooperative() {
            "cooperative"
        } else {
            ""
        };
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), name_style),
            Span::styled(
                format!("Appease {} ", gov.name),
                name_style,
            ),
            Span::styled(
                format!("(Loyalty: {:.0}", gov.loyalty),
                Style::default().fg(loyalty_color),
            ),
            if !status_label.is_empty() {
                Span::styled(
                    format!(", {}", status_label),
                    Style::default().fg(loyalty_color).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("")
            },
            Span::styled(")", Style::default().fg(loyalty_color)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("Cost: ¥{:.0}  →  +{:.0} loyalty", APPEASE_COST, APPEASE_LOYALTY_GAIN),
                Style::default().fg(Color::Yellow),
            ),
        ]));
        lines.push(Line::from(""));

        // Bargain option (below Appease, only when governor is defiant and bargain available)
        if state.bargain_available(region_idx) {
            let bargain_pos = MANAGE_BARGAIN_POS;
            let selected = state.ui.panel_selection == bargain_pos;
            if selected { selected_line = Some(lines.len()); }
            let marker = if selected { "▶ " } else { "  " };
            let name_style = if selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };

            let (bargain_name, cost_desc, loyalty_gain) = match gov.personality {
                GovernorPersonality::Buffoon => (
                    "Public Praise",
                    format!("-{:.0}% AUTH", BARGAIN_BUFFOON_POL_COST * 100.0),
                    BARGAIN_LOYALTY_GAIN,
                ),
                GovernorPersonality::Blowhard => (
                    "Token Concession",
                    format!("-¥{:.0}", BARGAIN_BLOWHARD_FUNDING_COST),
                    BARGAIN_BLOWHARD_LOYALTY_GAIN,
                ),
                GovernorPersonality::Recluse => (
                    "Send Manager",
                    format!("-{} personnel", BARGAIN_RECLUSE_PERSONNEL_COST),
                    BARGAIN_LOYALTY_GAIN,
                ),
                GovernorPersonality::Hardliner => (
                    "Grant Authority",
                    format!("-¥{:.0}", BARGAIN_HARDLINER_FUNDING_COST),
                    BARGAIN_LOYALTY_GAIN,
                ),
                GovernorPersonality::Operative => (
                    "Income Cut",
                    format!("-{:.0}% of regional income", BARGAIN_OPERATIVE_INCOME_CUT * 100.0),
                    BARGAIN_LOYALTY_GAIN,
                ),
                GovernorPersonality::Mobster => {
                    let count = gov.bargain_count;
                    let cost = BARGAIN_MOBSTER_BASE_COST * 2.0_f64.powi(count as i32);
                    (
                        "Protection Money",
                        format!("-¥{:.0}", cost),
                        BARGAIN_LOYALTY_GAIN,
                    )
                }
            };

            lines.push(Line::from(vec![
                Span::styled(marker.to_string(), name_style),
                Span::styled(
                    format!("Bargain: {} ", bargain_name),
                    name_style,
                ),
                Span::styled(
                    "(DEFIANT)",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    format!("Cost: {}  →  +{:.0} loyalty", cost_desc, loyalty_gain),
                    Style::default().fg(Color::Cyan),
                ),
            ]));
            lines.push(Line::from(""));
        }
    }

    lines.push(hint_line(state, "Toggle", "Back"));

    (format!(" Policy: {} ", region.name), lines, selected_line)
}

/// Returns the short description for a decree, used in both the browse list and confirmation dialog.
pub(crate) fn decree_description(decree_idx: usize) -> String {
    match decree_idx {
        0 => format!("+{} personnel, -¥{:.0}/day income (permanent)",
            CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY * TICKS_PER_DAY),
        1 => "Clinical trials 50% faster, risk of adverse events (permanent)".to_string(),
        2 => format!("Abandon a region, +{:.0}% income from the rest (permanent)",
            (SACRIFICE_INCOME_BONUS - 1.0) * 100.0),
        3 => "Neutralize all governors. No defiance, no cooperation. (permanent)".to_string(),
        4 => format!("Restore one region's infrastructure. Others: -{:.0}% infra. (permanent)",
            FORTIFY_INFRA_PENALTY * 100.0),
        5 => format!("Infectivity -{:.0}%, spread -{:.0}%. Kills {:.0}% of surviving population. (permanent)",
            (1.0 - COUNTERMEASURE_INFECTIVITY_MULT) * 100.0,
            (1.0 - COUNTERMEASURE_SPREAD_MULT) * 100.0,
            COUNTERMEASURE_KILL_FRACTION * 100.0),
        _ => unreachable!("decree_idx {} out of range", decree_idx),
    }
}

pub(crate) fn render_confirm_decree(state: &GameState, decree_idx: usize) -> (String, Vec<Line<'static>>, Option<usize>) {
    let name = decree_display_name(decree_idx);
    let desc = decree_description(decree_idx);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        format!("  {}", name.to_uppercase()),
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Once enacted, this decree is permanent.",
        Style::default().fg(Color::Red),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {}", desc),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(hint_line(state, "Confirm", "Cancel"));

    (format!(" ⚠ CONFIRM DECREE: {} ", name.to_uppercase()), lines, None)
}

pub(crate) fn render_region_select(state: &GameState, title: &str, action: &str, description: &str) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        format!("  ⚠ {}: THIS CANNOT BE UNDONE ⚠", title),
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {}", description),
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
        let infra_str = format!("HC:{:.0}% SL:{:.0}% CO:{:.0}%",
            region.healthcare_capacity * 100.0,
            region.supply_lines * 100.0,
            region.civil_order * 100.0);
        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<16}", marker, region.name), style),
            Span::styled(
                format!("Pop: {}  {}", pop_str, infra_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    let confirm_label = action[..1].to_uppercase() + &action[1..];
    lines.push(hint_line(state, &confirm_label, "Cancel"));

    (format!(" ⚠ {} ", title), lines, None)
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
    let gov_eff = region.policy_effectiveness();

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
                let reduction = (1.0 - vector.travel_ban_factor()) * gov_eff * 100.0;
                let color = if reduction >= 80.0 { Color::Green } else { Color::Yellow };
                (format!("{name} ({}, -{reduction:.0}%)", vector.label()), color)
            }
            1 => { // Quarantine
                let reduction = (1.0 - vector.quarantine_factor()) * gov_eff * 100.0;
                let color = if reduction >= 50.0 { Color::Green }
                    else if reduction >= 30.0 { Color::Yellow }
                    else { Color::Red };
                (format!("{name} ({}, -{reduction:.0}%)", vector.label()), color)
            }
            2 => { // Hospital Surge — universal +25% spread
                let increase = (HOSPITAL_SURGE_SPREAD_FACTOR - 1.0) * gov_eff * 100.0;
                (format!("{name} ({}, +{increase:.0}% spread!)", vector.label()), Color::Red)
            }
            4 => { // Water Sanitation
                match vector {
                    TransmissionVector::Waterborne => {
                        let reduction = 0.5 * gov_eff * 100.0;
                        (format!("{name} (waterborne, -{reduction:.0}%)"), Color::Green)
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
    let gov_eff = region.policy_effectiveness();

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
                    let prevented = inf.infected * disease.infectivity * (1.0 - factor) * gov_eff * (susceptible / pop);
                    total_impact += prevented;
                }
                impact_type = "infections";
            }
            2 => {
                // Hospital Surge: deaths prevented = infected × lethality × 0.5
                let prevented = inf.infected * disease.lethality * 0.5 * gov_eff;
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
                        let prevented = inf.infected * disease.infectivity * (1.0 - factor) * gov_eff * (susceptible / pop);
                        total_impact += prevented;
                    }
                }
                impact_type = "infections";
            }
            10 => {
                // Field Hospital / Medical Center: deaths prevented by lethality reduction
                let reduction = match region.hospital_level {
                    0 => 0.25, // Building Level 1
                    1 => 0.40 - 0.25, // Upgrading from Level 1 to Level 2 (incremental)
                    _ => return None, // Already fully built
                };
                let prevented = inf.infected * disease.lethality * region.healthcare_modifier * reduction;
                total_impact += prevented;
                impact_type = "deaths";
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
