use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    GameState, PolicyUiState, TICKS_PER_DAY,
    TRAVEL_BAN_COST, QUARANTINE_COST, QUARANTINE_PERSONNEL,
    HOSPITAL_SURGE_COST, HOSPITAL_SURGE_PERSONNEL,
    BORDER_SCREENING_COST, WATER_SANITATION_COST, WATER_SANITATION_PERSONNEL,
    grid_reading_order,
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
            let labels: Vec<&str> = [
                policy.is_some_and(|p| p.travel_ban).then_some("Travel Ban"),
                policy.is_some_and(|p| p.quarantine).then_some("Quarantine"),
                policy.is_some_and(|p| p.hospital_surge).then_some("Hospital"),
                policy.is_some_and(|p| p.border_screening).then_some("Screening"),
                policy.is_some_and(|p| p.water_sanitation).then_some("Sanitation"),
            ].into_iter().flatten().collect();

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

    let infected = region.total_infected();
    let dead = region.total_dead();
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
    let policies: [(&str, bool, String, &str, Option<u32>); 5] = [
        ("Travel Ban", policy.travel_ban,
         format!("${:.0}/day", TRAVEL_BAN_COST * TICKS_PER_DAY),
         "Blocks 90% spread, halves region income", None),
        ("Quarantine", policy.quarantine,
         format!("${:.0}/day + {} pers.", QUARANTINE_COST * TICKS_PER_DAY, QUARANTINE_PERSONNEL),
         "Halves infection rate", Some(QUARANTINE_PERSONNEL)),
        ("Hospital Surge", policy.hospital_surge,
         format!("${:.0}/day + {} pers.", HOSPITAL_SURGE_COST * TICKS_PER_DAY, HOSPITAL_SURGE_PERSONNEL),
         "Halves lethality", Some(HOSPITAL_SURGE_PERSONNEL)),
        ("Border Screening", policy.border_screening,
         format!("${:.0}/day", BORDER_SCREENING_COST * TICKS_PER_DAY),
         "Blocks 50% spread, no income penalty", None),
        ("Water Sanitation", policy.water_sanitation,
         format!("${:.0}/day + {} pers.", WATER_SANITATION_COST * TICKS_PER_DAY, WATER_SANITATION_PERSONNEL),
         "Halves waterborne disease spread", Some(WATER_SANITATION_PERSONNEL)),
    ];

    for (i, (name, active, cost_str, desc, personnel_needed)) in policies.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };

        let can_afford_personnel = personnel_needed
            .map(|need| {
                let avail = if *active {
                    // If already active, those personnel are already counted as busy;
                    // toggling off would free them, so show as affordable
                    state.personnel_available() + need
                } else {
                    state.personnel_available()
                };
                avail >= need
            })
            .unwrap_or(true);

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
