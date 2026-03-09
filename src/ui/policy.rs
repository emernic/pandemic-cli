use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, PolicyUiState, ScreeningLevel, TICKS_PER_DAY,
    TRAVEL_BAN_COST, TRAVEL_BAN_PERSONNEL,
    QUARANTINE_COST, QUARANTINE_PERSONNEL,
    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL,
    BORDER_SCREENING_COST, BORDER_SCREENING_PERSONNEL,
    WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
    SCREENING_LOW_COST, SCREENING_MEDIUM_COST, SCREENING_HIGH_COST,
    grid_reading_order, POLICY_POL_THRESHOLDS,
};
use crate::ui::hint_line;
use crate::format_number;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines) = match &state.ui.policy_ui {
        Some(PolicyUiState::ManagePolicies { region_idx }) => {
            render_manage(state, *region_idx)
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
        let has_active = policy.is_some_and(|p| p.any_active());

        let mut spans = vec![
            Span::styled(format!("{}{:<16}", marker, region.name), style),
        ];

        if has_active {
            let cost = policy.map(|p| p.funding_cost()).unwrap_or(0.0);
            let mut labels: Vec<&str> = [
                policy.is_some_and(|p| p.travel_ban).then_some("Travel Ban"),
                policy.is_some_and(|p| p.quarantine).then_some("Quarantine"),
                policy.is_some_and(|p| p.hospital_surge).then_some("Hospital"),
                policy.is_some_and(|p| p.border_screening).then_some("Border"),
                policy.is_some_and(|p| p.water_sanitation).then_some("Sanitation"),
            ].into_iter().flatten().collect();
            if let Some(p) = policy {
                match p.screening {
                    ScreeningLevel::Low => labels.push("Screen:Lo"),
                    ScreeningLevel::Medium => labels.push("Screen:Med"),
                    ScreeningLevel::High => labels.push("Screen:Hi"),
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

    // Policy toggles — costs derived from constants in state.rs
    let policies: Vec<(&str, bool, String, &str, Option<u32>)> = vec![
        ("Travel Ban", policy.travel_ban,
         format!("${:.0}/day + {} pers.", TRAVEL_BAN_COST * TICKS_PER_DAY, TRAVEL_BAN_PERSONNEL),
         "Blocks 90% spread, halves region income", Some(TRAVEL_BAN_PERSONNEL)),
        ("Quarantine", policy.quarantine,
         format!("${:.0}/day + {} pers.", QUARANTINE_COST * TICKS_PER_DAY, QUARANTINE_PERSONNEL),
         "Halves infection rate", Some(QUARANTINE_PERSONNEL)),
        ("Hospital Surge", policy.hospital_surge,
         format!("${:.0}/day + {} pers.", HOSPITAL_SURGE_COST * TICKS_PER_DAY, HOSPITAL_SURGE_PERSONNEL),
         "Halves lethality", Some(HOSPITAL_SURGE_PERSONNEL)),
        ("Border Screening", policy.border_screening,
         format!("${:.0}/day + {} pers.", BORDER_SCREENING_COST * TICKS_PER_DAY, BORDER_SCREENING_PERSONNEL),
         "Blocks 50% spread, no income penalty", Some(BORDER_SCREENING_PERSONNEL)),
        ("Water Sanitation", policy.water_sanitation,
         format!("${:.0}/day + {} pers.", WATER_SANITATION_COST * TICKS_PER_DAY, WATER_SANITATION_PERSONNEL),
         "Halves waterborne disease spread", Some(WATER_SANITATION_PERSONNEL)),
        ("Low Screening", policy.screening == ScreeningLevel::Low,
         format!("${:.0}/day + 1 pers.", SCREENING_LOW_COST * TICKS_PER_DAY),
         "40% infection visibility, faster detection", Some(1)),
        ("Med Screening", policy.screening == ScreeningLevel::Medium,
         format!("${:.0}/day + 2 pers.", SCREENING_MEDIUM_COST * TICKS_PER_DAY),
         "70% infection visibility, faster detection", Some(2)),
        ("High Screening", policy.screening == ScreeningLevel::High,
         format!("${:.0}/day + 3 pers.", SCREENING_HIGH_COST * TICKS_PER_DAY),
         "90% infection visibility, fastest detection", Some(3)),
    ];

    for (i, (name, active, cost_str, desc, personnel_needed)) in policies.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };
        let pol_unlocked = state.policy_unlocked(region_idx, i);

        let can_afford_personnel = personnel_needed
            .map(|need| {
                let mut avail = state.personnel_available();
                if *active {
                    // If already active, its personnel would be freed on disable
                    avail += need;
                } else if i >= 5 && i <= 7 {
                    // Screening upgrade: personnel from current tier would be freed
                    avail += policy.screening.personnel_cost();
                }
                avail >= need
            })
            .unwrap_or(true);

        if !*active && !pol_unlocked {
            // Locked by POL — show as unavailable
            let threshold = POLICY_POL_THRESHOLDS[i];
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
