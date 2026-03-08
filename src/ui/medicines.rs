use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{DeployTarget, GameState, MedicineUiState};
use super::format_number;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines) = match &state.ui.medicine_ui {
        Some(MedicineUiState::SelectRegion { medicine_idx }) => {
            render_select_region(state, *medicine_idx)
        }
        Some(MedicineUiState::SelectTarget { medicine_idx, region_idx }) => {
            render_select_target(state, *medicine_idx, *region_idx)
        }
        _ => render_browse(state),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_browse(state: &GameState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let unlocked: Vec<_> = state.medicines.iter().filter(|m| m.unlocked).collect();

    if unlocked.is_empty() {
        lines.push(Line::from(Span::styled(
            "No medicines available.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, med) in unlocked.iter().enumerate() {
            let selected = state.ui.panel_selection == i;
            let marker = if selected { "▶ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            lines.push(Line::from(Span::styled(
                format!("{}{}", marker, med.name),
                style,
            )));

            let disease_names: Vec<&str> = med
                .target_diseases
                .iter()
                .filter_map(|&idx| state.diseases.get(idx).map(|d| d.name.as_str()))
                .collect();

            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("${:.0}", med.cost),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{} doses", format_number(med.doses)),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw("  "),
                Span::styled(
                    disease_names.join(", "),
                    Style::default().fg(Color::Red),
                ),
            ]));
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Enter] Select  [Esc] Close",
        Style::default().fg(Color::DarkGray),
    )));

    (" Medicines ".to_string(), lines)
}

fn render_select_region(state: &GameState, medicine_idx: usize) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med = &state.medicines[medicine_idx];

    lines.push(Line::from(Span::styled(
        format!("  Deploy: {}", med.name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    for (i, region) in state.regions.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        // Show infection stats for target diseases
        let mut info_parts: Vec<Span> = Vec::new();
        for &d_idx in &med.target_diseases {
            let inf = region.infections.iter().find(|i| i.disease_idx == d_idx);
            let infected = inf.map(|i| i.infected).unwrap_or(0.0);
            let immune = inf.map(|i| i.immune).unwrap_or(0.0);
            let dead = inf.map(|i| i.dead).unwrap_or(0.0);
            let susceptible = (region.population as f64 - infected - dead - immune).max(0.0);

            if !info_parts.is_empty() {
                info_parts.push(Span::raw(" "));
            }
            info_parts.push(Span::styled(
                format!("{}s", format_number(susceptible)),
                Style::default().fg(Color::Cyan),
            ));
            info_parts.push(Span::raw(" "));
            info_parts.push(Span::styled(
                format!("{}i", format_number(infected)),
                Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
            ));
            if immune > 0.0 {
                info_parts.push(Span::raw(" "));
                info_parts.push(Span::styled(
                    format!("{}✓", format_number(immune)),
                    Style::default().fg(Color::Green),
                ));
            }
        }

        let mut spans = vec![
            Span::styled(format!("{}{:<14}", marker, region.name), style),
            Span::raw("  "),
        ];
        spans.extend(info_parts);
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Enter] Select  [Esc] Back",
        Style::default().fg(Color::DarkGray),
    )));

    (format!(" Deploy: {} ", med.name), lines)
}

fn render_select_target(
    state: &GameState,
    medicine_idx: usize,
    region_idx: usize,
) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med = &state.medicines[medicine_idx];
    let region = &state.regions[region_idx];
    let pop = region.population as f64;

    lines.push(Line::from(Span::styled(
        format!("  {} → {}", med.name, region.name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    for i in 0..med.num_deploy_targets() {
        let target = med.decode_deploy_target(i).unwrap();
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let disease_idx = match &target {
            DeployTarget::Vaccinate { disease_idx } => *disease_idx,
            DeployTarget::Treat { disease_idx } => *disease_idx,
        };
        let disease_name = state.diseases.get(disease_idx)
            .map(|d| d.name.as_str())
            .unwrap_or("Unknown");

        let inf = region.infections.iter().find(|i| i.disease_idx == disease_idx);

        match &target {
            DeployTarget::Vaccinate { .. } => {
                let infected = inf.map(|i| i.infected).unwrap_or(0.0);
                let dead = inf.map(|i| i.dead).unwrap_or(0.0);
                let immune = inf.map(|i| i.immune).unwrap_or(0.0);
                let susceptible = (pop - infected - dead - immune).max(0.0);

                lines.push(Line::from(Span::styled(
                    format!("{}Vaccinate susceptible ({})", marker, disease_name),
                    style,
                )));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("{} susceptible", format_number(susceptible)),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(" | "),
                    Span::styled(
                        format!("{} doses", format_number(med.doses)),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            DeployTarget::Treat { .. } => {
                let infected = inf.map(|i| i.infected).unwrap_or(0.0);

                lines.push(Line::from(Span::styled(
                    format!("{}Treat infected ({})", marker, disease_name),
                    style,
                )));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("{} infected", format_number(infected)),
                        Style::default().fg(if infected > 0.0 { Color::Red } else { Color::DarkGray }),
                    ),
                    Span::raw(" | "),
                    Span::styled(
                        format!("{} doses", format_number(med.doses)),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Cost: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.0}", med.cost),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("    "),
        Span::styled("Funding: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.0}", state.resources.funding),
            Style::default().fg(if state.resources.funding >= med.cost {
                Color::Green
            } else {
                Color::Red
            }),
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Enter] Deploy  [Esc] Back",
        Style::default().fg(Color::DarkGray),
    )));

    (format!(" {} → {} ", med.name, region.name), lines)
}
