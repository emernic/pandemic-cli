use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{
    DecreeId, GameState, OpsUiState, TICKS_PER_DAY,
    DECREE_COUNT, STANDING_ORDER_COUNT,
};
use super::hint_line;
use super::policy::{decree_description, render_confirm_decree, render_region_select};

/// Number of "field operations" items (Emergency Sample Delivery, Fire Personnel).
const FIELD_OPS_COUNT: usize = 2;

/// Returns eligible medicine indices for emergency sample delivery.
pub fn emergency_delivery_medicines(state: &GameState) -> Vec<usize> {
    state.medicines.iter().enumerate()
        .filter(|(_, m)| m.unlocked && m.doses > 0.0)
        .map(|(i, _)| i)
        .collect()
}

/// Maximum selection index for the operations panel in its current sub-state.
pub fn selection_max(ui_state: &OpsUiState, state: &GameState) -> usize {
    match ui_state {
        OpsUiState::BrowseOps => {
            (DECREE_COUNT + STANDING_ORDER_COUNT + FIELD_OPS_COUNT + state.loans.len())
                .saturating_sub(1)
        }
        OpsUiState::SelectSacrificeRegion
        | OpsUiState::SelectFortifyRegion => {
            state.regions.iter().filter(|r| !r.collapsed).count().saturating_sub(1)
        }
        OpsUiState::ConfirmDecree { .. } => 0,
        OpsUiState::SelectEmergencyMedicine => {
            emergency_delivery_medicines(state).len().saturating_sub(1)
        }
        OpsUiState::ConfirmEmergencyDelivery { .. } => 0,
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    match &state.ui.operations_ui {
        Some(OpsUiState::BrowseOps) | None => render_browse(f, area, state),
        Some(OpsUiState::ConfirmDecree { decree }) => {
            let (title, lines, _) = render_confirm_decree(state, *decree);
            render_panel(f, area, &title, lines);
        }
        Some(OpsUiState::SelectSacrificeRegion) => {
            let (title, lines, _) = render_region_select(
                state,
                "SACRIFICE REGION",
                "Sacrifice",
                "Choose a region to abandon. Its population is written off; income from remaining regions increases permanently.",
            );
            render_panel(f, area, &title, lines);
        }
        Some(OpsUiState::SelectFortifyRegion) => {
            let (title, lines, _) = render_region_select(
                state,
                "FORTIFY REGION",
                "Fortify",
                "Choose a region to restore to full infrastructure. All other regions suffer a permanent infrastructure penalty.",
            );
            render_panel(f, area, &title, lines);
        }
        Some(OpsUiState::SelectEmergencyMedicine) => {
            render_select_medicine(f, area, state);
        }
        Some(OpsUiState::ConfirmEmergencyDelivery { medicine_idx }) => {
            render_confirm_delivery(f, area, state, *medicine_idx);
        }
    }
}

fn render_browse(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let selected = state.ui.panel_selection;
    let mut row = 0;
    let mut selected_line: Option<usize> = None;

    // Crisis operations (temporary personnel commitments)
    if !state.crisis_operations.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Active Operations",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        for op in &state.crisis_operations {
            let days_left = op.ticks_remaining / TICKS_PER_DAY;
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(&op.label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" ({:.1}d left, {} personnel tied up)", days_left, op.personnel),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        lines.push(Line::raw(""));
    }

    // Emergency Decrees
    lines.push(Line::from(Span::styled(
        "  Emergency Decrees",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: loop count must equal DECREE_COUNT
    for &decree in &DecreeId::ALL {
        let is_selected = row == selected;
        if is_selected { selected_line = Some(lines.len()); }
        let marker = if is_selected { "▶ " } else { "  " };
        let name = decree.display_name();
        let enacted = state.enacted_decrees.is_enacted(decree);
        let unlocked = state.decree_unlocked(decree);

        if !enacted && !unlocked {
            // Locked — show 🔒 icon with unlock hint, matching Policies panel style
            let name_style = if is_selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let hint = GameState::decree_unlock_hint(decree);
            lines.push(Line::from(vec![
                Span::styled(marker, name_style),
                Span::styled("🔒 ", Style::default().fg(Color::DarkGray)),
                Span::styled(name, name_style),
                Span::styled(
                    format!("  ({})", hint),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(""));
        } else {
            // Enacted or unlocked — show with description
            let cost_pct = (decree.chairman_cost().abs() * 100.0) as u32;
            let (name_color, desc_color, suffix) = if enacted {
                (Color::DarkGray, Color::DarkGray, " [ENACTED]".to_string())
            } else if is_selected {
                (Color::Yellow, Color::DarkGray, format!(" [-{}% chairman]", cost_pct))
            } else {
                (Color::Red, Color::DarkGray, format!(" [-{}% chairman]", cost_pct))
            };

            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Yellow)),
                Span::styled(name, Style::default().fg(name_color).add_modifier(Modifier::BOLD)),
                Span::styled(suffix, Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(decree_description(decree), Style::default().fg(desc_color)),
            ]));
        }
        row += 1;
    }

    // Standing Orders
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  Standing Orders",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: must equal STANDING_ORDER_COUNT (= 2)
    let standing_orders = [
        (
            "Auto-Quarantine at HIGH",
            "Automatically enable Quarantine when infections exceed HIGH threshold (10K).",
            state.standing_orders.auto_quarantine_at_high,
        ),
        (
            "Auto-Travel Ban at CRIT",
            "Automatically enable Travel Ban when infections exceed CRIT threshold (100K).",
            state.standing_orders.auto_travel_ban_at_crit,
        ),
    ];

    for (name, desc, enabled) in &standing_orders {
        let is_selected = row == selected;
        if is_selected { selected_line = Some(lines.len()); }
        let marker = if is_selected { "▶ " } else { "  " };
        let name_color = if is_selected { Color::Yellow } else { Color::White };
        let status = if *enabled { "[ON] " } else { "[OFF]" };
        let status_color = if *enabled { Color::Green } else { Color::DarkGray };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(status, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::styled(" ", Style::default()),
            Span::styled(*name, Style::default().fg(name_color).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(*desc, Style::default().fg(Color::DarkGray)),
        ]));
        row += 1;
    }

    // Field Operations
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  Field Operations",
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    )));

    // COUPLING CHECK: must equal FIELD_OPS_COUNT (= 2)
    {
        let is_selected = row == selected;
        if is_selected { selected_line = Some(lines.len()); }
        let marker = if is_selected { "▶ " } else { "  " };
        let has_medicine = !emergency_delivery_medicines(state).is_empty();
        let name_color = if !has_medicine {
            Color::DarkGray
        } else if is_selected {
            Color::Yellow
        } else {
            Color::Green
        };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(
                "Emergency Sample Delivery",
                Style::default().fg(name_color).add_modifier(Modifier::BOLD),
            ),
            if !has_medicine {
                Span::styled("  (no medicines available)", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(
                    format!("  (to {})", state.regions[state.ui.map_selection].name),
                    Style::default().fg(Color::DarkGray),
                )
            },
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "Send experimental samples to a regional governor. Costs doses and personnel.",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        row += 1;
    }

    // Fire Personnel
    {
        let is_selected = row == selected;
        if is_selected { selected_line = Some(lines.len()); }
        let marker = if is_selected { "▶ " } else { "  " };
        let available = state.personnel_available();
        let name_color = if available == 0 {
            Color::DarkGray
        } else if is_selected {
            Color::Yellow
        } else {
            Color::Green
        };
        let upkeep_per_day = state.resources.personnel as f64
            * crate::state::PERSONNEL_UPKEEP_COST
            * crate::state::TICKS_PER_DAY;

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(
                "Fire Personnel",
                Style::default().fg(name_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({} available, {} total, ¥{:.0}/day upkeep)", available, state.resources.personnel, upkeep_per_day),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "Dismiss 5 unassigned personnel to reduce upkeep costs.",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        row += 1;
    }

    // Outstanding Loans
    if !state.loans.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  Outstanding Loans",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));

        for loan in &state.loans {
            let is_selected = row == selected;
            if is_selected { selected_line = Some(lines.len()); }
            let marker = if is_selected { "▶ " } else { "  " };
            let highlight = if is_selected { Color::Yellow } else { Color::White };
            let interest_per_day = loan.interest_per_tick() * TICKS_PER_DAY;
            let days_left = (loan.due_day - state.tick as f64 / TICKS_PER_DAY).max(0.0);

            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Yellow)),
                Span::styled(&loan.lender_name, Style::default().fg(highlight).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" — ¥{:.0} outstanding (+¥{:.1}/day, {:.1}d left)",
                        loan.outstanding, interest_per_day, days_left),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            row += 1;
        }
    }

    let _ = row; // suppress unused variable warning

    lines.push(hint_line(state, "Select", "Close"));

    let block = Block::default()
        .title(" ORDERS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines).block(block).scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

fn render_select_medicine(f: &mut Frame, area: Rect, state: &GameState) {
    let eligible = emergency_delivery_medicines(state);
    let selected = state.ui.panel_selection;
    let region_name = &state.regions[state.ui.map_selection].name;
    let gov_name = &state.regions[state.ui.map_selection].governor.name;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  Target: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} ({})", gov_name, region_name),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  Select medicine to send:",
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    )));

    for (row, &med_idx) in eligible.iter().enumerate() {
        let med = &state.medicines[med_idx];
        let is_selected = row == selected;
        let marker = if is_selected { "▶ " } else { "  " };
        let name_color = if is_selected { Color::Yellow } else { Color::White };

        let dose_cost = state.emergency_delivery_dose_cost(med_idx);

        // Check if tested against diseases in the target region
        let region_diseases: Vec<usize> = state.regions[state.ui.map_selection].infections.iter()
            .map(|inf| inf.disease_idx)
            .collect();
        let any_untested = region_diseases.iter()
            .any(|d_idx| !med.tested_against.contains(d_idx));
        let status = if region_diseases.is_empty() {
            "no active diseases in region"
        } else if any_untested {
            "UNTESTED: adverse reaction risk"
        } else {
            "tested"
        };
        let status_color = if any_untested && !region_diseases.is_empty() {
            Color::Red
        } else {
            Color::DarkGray
        };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Yellow)),
            Span::styled(&med.name, Style::default().fg(name_color).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  ({:.0} doses)", dose_cost),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(status, Style::default().fg(status_color)),
        ]));
    }

    if eligible.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No medicines available. Develop one first.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(hint_line(state, "Select", "Back"));

    let block = Block::default()
        .title(" EMERGENCY SAMPLE DELIVERY ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_confirm_delivery(f: &mut Frame, area: Rect, state: &GameState, medicine_idx: usize) {
    let med = &state.medicines[medicine_idx];
    let region_idx = state.ui.map_selection;
    let region = &state.regions[region_idx];
    let gov = &region.governor;

    let dose_cost = state.emergency_delivery_dose_cost(medicine_idx);

    let region_diseases: Vec<usize> = region.infections.iter()
        .map(|inf| inf.disease_idx)
        .collect();
    let any_untested = region_diseases.iter()
        .any(|d_idx| !med.tested_against.contains(d_idx));

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("  Send {} to {}?", med.name, gov.name),
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("  Region: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&region.name, Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Cooperation: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{:.0}", gov.cooperation), Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Cost: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:.0} doses + 2 personnel (1 day)", dose_cost),
            Style::default().fg(Color::Yellow),
        ),
    ]));

    if any_untested && !region_diseases.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  WARNING: Untested against local pathogens.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  25% chance of adverse reaction. Governor cooperation will drop if it goes wrong.",
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  Tested medicine. Low risk, strong cooperation boost.",
            Style::default().fg(Color::Green),
        )));
    }

    lines.push(Line::raw(""));
    lines.push(hint_line(state, "Confirm", "Back"));

    let block = Block::default()
        .title(" CONFIRM DELIVERY ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_panel(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>) {
    let block = Block::default()
        .title(title.to_string())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
