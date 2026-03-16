use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    AppState, PolicyUiState, Region, RegionSpecialization, RegionTrait,
    ScreeningLevel, TRADE_DEPENDENT_TRAVEL_BAN_MULT, TransmissionVector, TICKS_PER_DAY,
    REGULATORY_APPARATUS_COST_MULT, KNOWLEDGE_PARTIAL_STATS,
    INFECTION_PRESSURE_CRIT, INFECTION_PRESSURE_HIGH, INFECTION_PRESSURE_MOD,
    TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL,
    QUARANTINE_COST, QUARANTINE_PERSONNEL,
    DISCOURAGE_HOSP_COST, DISCOURAGE_HOSP_PERSONNEL, HOSPITAL_EXPOSURE_FACTOR,
    BORDER_CONTROLS_COST, BORDER_CONTROLS_PERSONNEL,
    MARTIAL_LAW_COST, MARTIAL_LAW_PERSONNEL,
    NUCLEAR_ANNIHILATION_COST,
    FIELD_HOSPITAL_COST, FIELD_HOSPITAL_PERSONNEL,
    MEDICAL_CENTER_COST, MEDICAL_CENTER_PERSONNEL,
    SCREENING_BASIC_COST, SCREENING_ANTIGEN_COST, SCREENING_MASS_RAPID_COST,
    REBUILD_INFRA_COST_PER_POINT, REBUILD_INFRA_MAX_REPAIR, REBUILD_INFRA_AUTO_THRESHOLD,
    DecreeId, PolicyId, POLICY_COUNT,
    CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY,
    SACRIFICE_INCOME_BONUS, FORTIFY_INFRA_PENALTY,
    COUNTERMEASURE_KILL_FRACTION, COUNTERMEASURE_SPREAD_WITHIN_MULT, COUNTERMEASURE_SPREAD_MULT,
    MANAGE_NEGOTIATE_POS, MANAGE_BARGAIN_POS, AUTO_NEGOTIATE_THRESHOLD,
    policy_display_order, NEGOTIATE_COST, NEGOTIATE_COOPERATION_GAIN,
    BARGAIN_COOPERATION_GAIN, BARGAIN_BLOWHARD_COOPERATION_GAIN,
    BARGAIN_BUFFOON_APPROVAL_COST, BARGAIN_BLOWHARD_FUNDING_COST,
    BARGAIN_RECLUSE_PERSONNEL_COST, BARGAIN_HARDLINER_FUNDING_COST,
    BARGAIN_PRAGMATIST_INCOME_CUT, BARGAIN_MOBSTER_BASE_COST,
    GovernorPersonality,
};
use crate::ui::hint_line;
use crate::format_number;

/// Maximum selection index for the policy panel in its current sub-state.
pub fn selection_max(ui_state: &PolicyUiState, state: &AppState) -> usize {
    match ui_state {
        PolicyUiState::ManagePolicies { region_idx } => {
            if state.regions.get(*region_idx).is_some_and(|r| r.collapsed) {
                POLICY_COUNT - 1
            } else if state.bargain_available(*region_idx) {
                MANAGE_BARGAIN_POS
            } else {
                MANAGE_NEGOTIATE_POS
            }
        }
    }
}

/// Section header labels for the policy panel, keyed by display position.
/// Returns Some(header) at the start of each group, None otherwise.
fn policy_section_header(display_pos: usize) -> Option<&'static str> {
    match display_pos {
        0 => Some("Detection"),
        3 => Some("Containment"),
        8 => Some("Infrastructure"),
        11 => Some("Other"),
        _ => None,
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState) {
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

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}


fn render_manage(state: &AppState, region_idx: usize) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let region = &state.regions[region_idx];
    let policy = state.policies.get(region_idx).cloned().unwrap_or_default();


    // Show regional traits with effects
    for t in &region.traits {
        lines.push(Line::from(vec![
            Span::raw(format!("  {}: ", t.label())),
            Span::styled(t.effect(), Style::default().fg(Color::Yellow)),
        ]));
    }
    if !region.traits.is_empty() {
        lines.push(Line::from(""));
    }

    // Trait-adjusted and specialization-adjusted costs
    let low_infra = region.has_trait(RegionTrait::LowInfrastructure);
    let trade_dep = region.has_trait(RegionTrait::TradeDependent);
    let infra_extra: u32 = if low_infra { 1 } else { 0 };
    let spec_mult = if region.has_specialization(RegionSpecialization::RegulatoryApparatus) {
        REGULATORY_APPARATUS_COST_MULT
    } else {
        1.0
    };
    let tb_cost = if trade_dep { TRAVEL_BAN_COST * TRADE_DEPENDENT_TRAVEL_BAN_MULT } else { TRAVEL_BAN_COST };
    let tb_cost = tb_cost * spec_mult;

    // Policy toggles — each entry carries its PolicyId. Display position != index
    // (grouped by function via policy_display_order()).
    // tick_cost: per-tick ongoing funding cost (0.0 for one-time purchases like hospitals/intel).
    // Used to mute policies when funding ≤ 0 — the engine suspends ongoing-cost policies at that point.
    //                   (PolicyId, name, active, cost_str, desc, personnel_needed, tick_cost)
    let policies: Vec<(PolicyId, &str, bool, String, &str, Option<u32>, f64)> = vec![
        (PolicyId::TravelBan, "Travel Ban", policy.travel_ban,
         format!("¥{:.0}/day + {} pers.", tb_cost * TICKS_PER_DAY, TRAVEL_BAN_PERSONNEL + infra_extra),
         if trade_dep { "Blocks 50-95% cross-region spread (varies by pathogen), 30% GDP penalty" }
         else { "Blocks 50-95% cross-region spread (varies by pathogen), 20% GDP penalty" },
         Some(TRAVEL_BAN_PERSONNEL + infra_extra), tb_cost),
        (PolicyId::Quarantine, "Quarantine", policy.quarantine,
         format!("¥{:.0}/day + {} pers.", QUARANTINE_COST * spec_mult * TICKS_PER_DAY, QUARANTINE_PERSONNEL + infra_extra),
         "20-65% within-region spread reduction (varies by pathogen)", Some(QUARANTINE_PERSONNEL + infra_extra), QUARANTINE_COST * spec_mult),
        (PolicyId::DiscourageHosp, "Discourage Hospitalization", policy.discourage_hosp,
         "Free".to_string(),
         "Removes within-region hospital spread penalty, +50% lethality (no hospital care)",
         Some(DISCOURAGE_HOSP_PERSONNEL + infra_extra), DISCOURAGE_HOSP_COST * spec_mult),
        (PolicyId::BorderControls, "Border Controls", policy.border_controls,
         format!("¥{:.0}/day + {} pers.", BORDER_CONTROLS_COST * spec_mult * TICKS_PER_DAY, BORDER_CONTROLS_PERSONNEL + infra_extra),
         "Blocks 30% cross-region spread", Some(BORDER_CONTROLS_PERSONNEL + infra_extra), BORDER_CONTROLS_COST * spec_mult),
        (PolicyId::BasicScreening, "Basic Screening", policy.screening == ScreeningLevel::Basic,
         format!("¥{:.0}/day + {} pers.", SCREENING_BASIC_COST * spec_mult * TICKS_PER_DAY, 1 + infra_extra),
         "40% visible, 10% all spread reduction, 75% dose targeting, 30% faster detection (~4 day ramp-up)", Some(1 + infra_extra), SCREENING_BASIC_COST * spec_mult),
        (PolicyId::AntigenScreening, "Antigen Screening", policy.screening == ScreeningLevel::Antigen,
         format!("¥{:.0}/day + {} pers.", SCREENING_ANTIGEN_COST * spec_mult * TICKS_PER_DAY, 2 + infra_extra),
         "75% visible, 20% all spread reduction, 90% dose targeting, detects incubating and immune (~4 day ramp-up)", Some(2 + infra_extra), SCREENING_ANTIGEN_COST * spec_mult),
        (PolicyId::MassRapidScreen, "Mass Rapid Screen", policy.screening == ScreeningLevel::MassRapid,
         format!("¥{:.0}/day + {} pers.", SCREENING_MASS_RAPID_COST * spec_mult * TICKS_PER_DAY, 4 + infra_extra),
         "95% visible, 30% all spread reduction, 100% dose targeting, detects incubating and immune (~4 day ramp-up)", Some(4 + infra_extra), SCREENING_MASS_RAPID_COST * spec_mult),
        (PolicyId::MartialLaw, "Martial Law", policy.martial_law,
         format!("¥{:.0}/day + {} pers.", MARTIAL_LAW_COST * spec_mult * TICKS_PER_DAY, MARTIAL_LAW_PERSONNEL + infra_extra),
         "Collapse threshold −15% (must enact before collapse)", Some(MARTIAL_LAW_PERSONNEL + infra_extra), MARTIAL_LAW_COST * spec_mult),
        (PolicyId::NuclearOption, "Nuclear Option", policy.nuclear_state.is_active(),
         format!("One-time: ¥{:.0}", NUCLEAR_ANNIHILATION_COST),
         "Eliminate 99% of population. Stops all disease spread.", None, 0.0),
        (PolicyId::FieldHospital,
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
             0 => "25% lethality reduction, +10 co-op",
             1 => "40% lethality reduction, +25% medicine efficacy, +10 co-op",
             _ => "40% lethality reduction, +25% medicine efficacy",
         },
         None, 0.0),
        (PolicyId::RebuildInfra,
         "Rebuild Infrastructure",
         policy.auto_rebuild_infra,
         {
             let repair_needed = rebuild_infra_repair_needed(region);
             if repair_needed > 0.0 {
                 format!("¥{:.0} (proportional)", repair_needed * REBUILD_INFRA_COST_PER_POINT)
             } else {
                 "No repairs needed".to_string()
             }
         },
         "Repairs up to 10% of each degraded infra stat",
         None, 0.0),
    ];

    // Reorder by canonical display order (grouped by function — see PolicyId::DISPLAY_ORDER).
    // display_pos == panel_selection; confirm handler maps back via policy_display_order().
    let display_order = policy_display_order();
    let policies: Vec<_> = display_order.iter().map(|pid| {
        policies.iter().find(|(id, ..)| id == pid).unwrap().clone()
    }).collect();

    for (display_pos, (policy_id, name, active, cost_str, desc, personnel_needed, tick_cost)) in policies.iter().enumerate() {
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
        let structurally_locked = if region.collapsed {
            *policy_id != PolicyId::NuclearOption && !*active
        } else {
            *policy_id == PolicyId::NuclearOption
        };

        if structurally_locked {
            let name_style = if selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let reason = if region.collapsed { "collapsed" } else { "not collapsed" };
            let icon = if *policy_id == PolicyId::NuclearOption { "🔒 " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(format!("{}", marker), name_style),
                Span::styled(icon, Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", name), name_style),
                Span::styled(
                    format!("  ({})", reason),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        let pol_unlocked = state.policy_unlocked(region_idx, *policy_id);

        let can_afford_personnel = personnel_needed
            .map(|need| {
                let mut avail = state.personnel_available();
                if *active {
                    // If already active, its personnel would be freed on disable
                    avail += need;
                } else if policy_id.is_screening() {
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
            // Locked — show as unavailable with the reason
            let research_met = state.policy_research_met(*policy_id);
            let authority_met = match state.effective_authority_requirement(region_idx, *policy_id) {
                Some(req) => state.resources.authority >= req,
                None => true,
            };

            let lock_reason = if !research_met && !authority_met {
                let tech = policy_id.research_prerequisite().unwrap();
                let req = policy_id.authority_requirement()
                    .map(|a| a.label())
                    .unwrap_or("???");
                format!("  (Requires {} + {} authority)", tech.name(), req)
            } else if !research_met {
                let tech = policy_id.research_prerequisite().unwrap();
                format!("  (Requires {})", tech.name())
            } else {
                let req = policy_id.authority_requirement()
                    .map(|a| a.label())
                    .unwrap_or("???");
                format!("  (Requires {} authority)", req)
            };

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
                    lock_reason,
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        // Tiered policies: show level instead of misleading [OFF] for surpassed tiers
        let status = if *policy_id == PolicyId::NuclearOption && *active {
            match policy.nuclear_state {
                crate::state::NuclearState::Dropping { .. } => "[DROPPING]",
                crate::state::NuclearState::Dropped => "[DROPPED] ",
                _ => "[ON] ",
            }
        } else if *policy_id == PolicyId::RebuildInfra && *active {
            "[AUTO]"
        } else if *active {
            "[ON] "
        } else if policy_id.is_screening() {
            // Screening is tiered: check if current level is above this tier
            let tier_level = match policy_id {
                PolicyId::BasicScreening => ScreeningLevel::Basic,
                PolicyId::AntigenScreening => ScreeningLevel::Antigen,
                _ => ScreeningLevel::MassRapid,
            };
            if policy.screening > tier_level {
                "[PAST]"
            } else {
                "[OFF]"
            }
        } else if *policy_id == PolicyId::FieldHospital && region.hospital_level == 1 {
            // Hospital level 1 built — show [Lv1] not [OFF]
            "[Lv1]"
        } else {
            "[OFF]"
        };

        let is_surpassed = status == "[PAST]" || status == "[Lv1]";
        let status_style = if status == "[DROPPING]" {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if status == "[DROPPED] " {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if *active {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else if is_surpassed {
            Style::default().fg(Color::Cyan)
        } else if can_afford {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Red)
        };

        let name_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if !*active && !can_afford {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let mut row = vec![
            Span::styled(format!("{}", marker), name_style),
            Span::styled(format!("{} ", status), status_style),
        ];
        row.push(Span::styled(format!("{}", name), name_style));
        lines.push(Line::from(row));
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(*desc, Style::default().fg(Color::DarkGray)),
        ]));
        // Effectiveness hints for transmission-sensitive policies
        if let Some(hint) = effectiveness_hint(state, region_idx, *policy_id) {
            lines.push(hint);
        }
        // Estimated daily impact for active policies
        if *active {
            if let Some(impact) = impact_estimate(state, region_idx, *policy_id) {
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
        // Dedicated auto-rebuild hint line (matches auto-negotiate hint style)
        if *policy_id == PolicyId::RebuildInfra {
            let auto_hint = if policy.auto_rebuild_infra {
                format!("[X] Auto: ON (rebuilds when infra < {:.0}%)", REBUILD_INFRA_AUTO_THRESHOLD * 100.0)
            } else {
                "[X] Auto: OFF".to_string()
            };
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(auto_hint, Style::default().fg(if policy.auto_rebuild_infra { Color::Green } else { Color::DarkGray })),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Negotiate with Governor action
    if !region.collapsed {
        let negotiate_pos = MANAGE_NEGOTIATE_POS;
        let selected = state.ui.panel_selection == negotiate_pos;
        if selected { selected_line = Some(lines.len()); }
        let gov = &region.governor;
        let marker = if selected { "▶ " } else { "  " };
        let name_style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let auto_negotiate = state.policies[region_idx].auto_negotiate;
        let auto_tag = if auto_negotiate { "[AUTO] " } else { "" };
        if gov.is_dead() {
            lines.push(Line::from(vec![
                Span::styled(marker.to_string(), name_style),
                Span::styled("Governor: ", name_style),
                Span::styled("LEADERLESS", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            ]));
        } else {
            let cooperation_color = if gov.is_hostile() {
                Color::Red
            } else if gov.is_cooperative() {
                Color::Green
            } else {
                Color::Yellow
            };
            let status_label = if gov.is_hostile() {
                "HOSTILE"
            } else if gov.is_cooperative() {
                "cooperative"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::styled(marker.to_string(), name_style),
                if auto_negotiate {
                    Span::styled(auto_tag, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                } else {
                    Span::raw("")
                },
                Span::styled(
                    format!("Negotiate: {} ", gov.name),
                    name_style,
                ),
                Span::styled(
                    format!("(Co-Op: {:.0}", gov.cooperation),
                    Style::default().fg(cooperation_color),
                ),
                if !status_label.is_empty() {
                    Span::styled(
                        format!(", {}", status_label),
                        Style::default().fg(cooperation_color).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("")
                },
                Span::styled(")", Style::default().fg(cooperation_color)),
            ]));
        }
        if gov.is_dead() {
            let eff = gov.policy_effectiveness();
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    format!("⚠ Leaderless: policies only {:.0}% effective", eff * 100.0),
                    Style::default().fg(Color::Red),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    format!("Cost: ¥{:.0}  →  +{:.0} co-op", NEGOTIATE_COST, NEGOTIATE_COOPERATION_GAIN),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
            {
                let eff = gov.policy_effectiveness();
                if eff < 1.0 {
                    lines.push(Line::from(vec![
                        Span::raw("      "),
                        Span::styled(
                            format!("⚠ Hostile: policies only {:.0}% effective in this region", eff * 100.0),
                            Style::default().fg(Color::Red),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw("      "),
                        Span::styled(
                            "Below 40 → hostile. Above 80 → cooperative (−20% policy cost)",
                            Style::default().fg(Color::DarkGray),
                        ),
                ]));
            }
            }
            // Show cooperation pressure drivers
            if !gov.is_dead() {
                let infected: f64 = region.infections.iter().map(|inf| inf.infected).sum();
                let pop = region.population as f64;
                let death_frac = if pop > 0.0 { region.dead / pop } else { 0.0 };
                let restrictive_count = [
                    state.policies[region_idx].travel_ban,
                    state.policies[region_idx].quarantine,
                    state.policies[region_idx].martial_law,
                    state.policies[region_idx].border_controls,
                ].iter().filter(|&&b| b).count();

                let mut pressures: Vec<&str> = Vec::new();
                if infected > INFECTION_PRESSURE_CRIT {
                    pressures.push("infections (severe)");
                } else if infected > INFECTION_PRESSURE_HIGH {
                    pressures.push("infections (rising)");
                } else if infected > INFECTION_PRESSURE_MOD {
                    pressures.push("infections");
                }
                if death_frac > 0.01 {
                    pressures.push("deaths");
                }
                if restrictive_count > 0 {
                    pressures.push("restrictive policies");
                }

                let pressure_text = if pressures.is_empty() {
                    "Pressure: none".to_string()
                } else {
                    format!("Pressure: {}", pressures.join(", "))
                };
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(pressure_text, Style::default().fg(Color::DarkGray)),
                ]));
            }
            // X hotkey hint for auto-negotiate
            let auto_hint = if auto_negotiate {
                format!("[X] Auto: ON (negotiates when co-op < {:.0})", AUTO_NEGOTIATE_THRESHOLD)
            } else {
                "[X] Auto: OFF".to_string()
            };
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(auto_hint, Style::default().fg(if auto_negotiate { Color::Green } else { Color::DarkGray })),
            ]));
        }
        lines.push(Line::from(""));

        // Bargain option (below Negotiate, only when governor is hostile and bargain available)
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

            let (bargain_name, cost_desc, cooperation_gain) = match gov.personality {
                GovernorPersonality::Buffoon => (
                    "Public Praise",
                    format!("-{:.0}% chairman approval", BARGAIN_BUFFOON_APPROVAL_COST * 100.0),
                    BARGAIN_COOPERATION_GAIN,
                ),
                GovernorPersonality::Blowhard => (
                    "Token Concession",
                    format!("-¥{:.0}", BARGAIN_BLOWHARD_FUNDING_COST),
                    BARGAIN_BLOWHARD_COOPERATION_GAIN,
                ),
                GovernorPersonality::Recluse => (
                    "Send Manager",
                    format!("-{} personnel", BARGAIN_RECLUSE_PERSONNEL_COST),
                    BARGAIN_COOPERATION_GAIN,
                ),
                GovernorPersonality::Hardliner => (
                    "Grant Authority",
                    format!("-¥{:.0}", BARGAIN_HARDLINER_FUNDING_COST),
                    BARGAIN_COOPERATION_GAIN,
                ),
                GovernorPersonality::Pragmatist => (
                    "Income Cut",
                    format!("-{:.0}% of regional income", BARGAIN_PRAGMATIST_INCOME_CUT * 100.0),
                    BARGAIN_COOPERATION_GAIN,
                ),
                GovernorPersonality::Mobster => {
                    let count = gov.bargain_count;
                    let cost = BARGAIN_MOBSTER_BASE_COST * 2.0_f64.powi(count as i32);
                    (
                        "Protection Money",
                        format!("-¥{:.0}", cost),
                        BARGAIN_COOPERATION_GAIN,
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
                    "(HOSTILE)",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    format!("Cost: {}  →  +{:.0} co-op", cost_desc, cooperation_gain),
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
pub(crate) fn decree_description(decree: DecreeId) -> String {
    match decree {
        DecreeId::ConscriptResearchers => format!("+{} personnel, -¥{:.0}/day income (permanent)",
            CONSCRIPT_PERSONNEL_GAIN, CONSCRIPT_INCOME_PENALTY * TICKS_PER_DAY),
        DecreeId::AuthorizeHumanTrials => "Clinical trials 50% faster, risk of adverse events (permanent)".to_string(),
        DecreeId::SacrificeRegion => format!("Abandon a region, +{:.0}% income from the rest (permanent)",
            (SACRIFICE_INCOME_BONUS - 1.0) * 100.0),
        DecreeId::FortifyRegion => format!("Restore one region's infrastructure. Others: -{:.0}% infra. (permanent)",
            FORTIFY_INFRA_PENALTY * 100.0),
        DecreeId::EmergencyCountermeasure => format!("Within-region spread -{:.0}%, cross-region spread -{:.0}%. Kills {:.0}% of surviving population. (permanent)",
            (1.0 - COUNTERMEASURE_SPREAD_WITHIN_MULT) * 100.0,
            (1.0 - COUNTERMEASURE_SPREAD_MULT) * 100.0,
            COUNTERMEASURE_KILL_FRACTION * 100.0),
    }
}

pub(crate) fn render_confirm_decree(state: &AppState, decree: DecreeId) -> (String, Vec<Line<'static>>, Option<usize>) {
    let name = decree.display_name();
    let desc = decree_description(decree);
    let cost = decree.chairman_cost();
    let cost_pct = (cost.abs() * 100.0) as u32;

    let mut lines: Vec<Line> = Vec::new();

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
    lines.push(Line::from(vec![
        Span::styled(
            format!("  Chairman satisfaction: -{}%", cost_pct),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(hint_line(state, "Confirm", "Cancel"));

    (format!(" ⚠ CONFIRM DECREE: {} ", name.to_uppercase()), lines, None)
}

pub(crate) fn render_region_select(state: &AppState, title: &str, action: &str, description: &str) -> (String, Vec<Line<'static>>, Option<usize>) {
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
fn effectiveness_hint(state: &AppState, region_idx: usize, policy: PolicyId) -> Option<Line<'static>> {
    // Only transmission-sensitive policies get hints
    if !matches!(policy, PolicyId::TravelBan | PolicyId::Quarantine | PolicyId::DiscourageHosp) {
        return None;
    }

    let region = &state.regions[region_idx];
    let gov_eff = region.policy_effectiveness();

    // Collect identified diseases with active infections in this region.
    // Uses KNOWLEDGE_PARTIAL_STATS threshold to match the threats panel —
    // transmission vector is only revealed at partial stats level.
    let active_diseases: Vec<(String, TransmissionVector)> = region
        .infections
        .iter()
        .filter(|inf| inf.infected > 0.0)
        .filter_map(|inf| {
            let disease = state.diseases.get(inf.disease_idx)?;
            if disease.knowledge >= KNOWLEDGE_PARTIAL_STATS {
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

        let (label, color) = match policy {
            PolicyId::TravelBan => {
                let reduction = (1.0 - vector.travel_ban_factor()) * gov_eff * 100.0;
                let color = if reduction >= 80.0 { Color::Green } else { Color::Yellow };
                (format!("{name} ({}, -{reduction:.0}%)", vector.label()), color)
            }
            PolicyId::Quarantine => {
                let reduction = (1.0 - vector.quarantine_factor()) * gov_eff * 100.0;
                let color = if reduction >= 50.0 { Color::Green }
                    else if reduction >= 30.0 { Color::Yellow }
                    else { Color::Red };
                (format!("{name} ({}, -{reduction:.0}%)", vector.label()), color)
            }
            PolicyId::DiscourageHosp => { // removes hospital exposure
                let reduction = (1.0 - 1.0 / HOSPITAL_EXPOSURE_FACTOR) * gov_eff * 100.0;
                (format!("{name} ({}, -{reduction:.0}% within-region spread)", vector.label()), Color::Green)
            }
            _ => unreachable!(),
        };

        spans.push(Span::styled(label, Style::default().fg(color)));
    }

    Some(Line::from(spans))
}

/// Estimated daily impact for an active policy. Shows approximate infections
/// or deaths prevented per day based on current disease parameters and counts.
fn impact_estimate(state: &AppState, region_idx: usize, policy: PolicyId) -> Option<Line<'static>> {
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
        if disease.knowledge < KNOWLEDGE_PARTIAL_STATS {
            continue;
        }

        let alive = (pop - region.dead).max(0.0);
        let susceptible = alive - inf.infected - inf.immune;

        match policy {
            PolicyId::TravelBan => {
                // Travel Ban: can't easily estimate cross-region prevention
                return None;
            }
            PolicyId::Quarantine => {
                // infections prevented = infected × spread × (1 - factor) × susceptible/pop
                if susceptible > 0.0 {
                    let factor = disease.transmission.quarantine_factor();
                    let prevented = inf.infected * disease.within_region_spread * (1.0 - factor) * gov_eff * (susceptible / pop);
                    total_impact += prevented;
                }
                impact_type = "infections";
            }
            PolicyId::DiscourageHosp => {
                // infections prevented by removing hospital exposure
                if susceptible > 0.0 {
                    // Baseline has HOSPITAL_EXPOSURE_FACTOR; removing it prevents this fraction
                    let prevented = inf.infected * disease.within_region_spread * (HOSPITAL_EXPOSURE_FACTOR - 1.0) * gov_eff * (susceptible / pop);
                    total_impact += prevented;
                }
                impact_type = "infections";
            }
            PolicyId::BorderControls => {
                // cross-region spread prevention — hard to estimate
                return None;
            }
            PolicyId::FieldHospital => {
                // deaths prevented by lethality reduction
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

/// Total infrastructure repair needed for one activation (capped at REBUILD_INFRA_MAX_REPAIR per system).
fn rebuild_infra_repair_needed(region: &Region) -> f64 {
    let mut total = 0.0;
    for &level in &[region.healthcare_capacity, region.supply_lines, region.civil_order] {
        let deficit = 1.0 - level;
        total += deficit.min(REBUILD_INFRA_MAX_REPAIR).max(0.0);
    }
    total
}
