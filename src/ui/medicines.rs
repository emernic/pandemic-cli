use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{DeployTarget, GameState, MedicineUiState};
use crate::ui::research::disease_display_name;
use crate::ui::hint_line;
use crate::format_number;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines) = match &state.ui.medicine_ui {
        Some(MedicineUiState::SelectRegion { medicine_idx }) => {
            render_select_region(state, *medicine_idx)
        }
        Some(MedicineUiState::SelectTarget { medicine_idx, region_idx }) => {
            render_select_target(state, *medicine_idx, *region_idx)
        }
        Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, target_selection }) => {
            render_confirm_deploy(state, *medicine_idx, *region_idx, *target_selection)
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

            let disease_names: Vec<String> = med
                .target_diseases
                .iter()
                .filter_map(|&idx| state.diseases.get(idx).map(|d| disease_display_name(d, idx)))
                .collect();

            // Check tested status
            let tested_count = med.target_diseases.iter()
                .filter(|d| med.tested_against.contains(d))
                .count();
            let total_targets = med.target_diseases.len();
            let tested_label = if tested_count == total_targets {
                Span::styled(" [Tested]", Style::default().fg(Color::Green))
            } else if tested_count > 0 {
                Span::styled(
                    format!(" [Tested: {}/{}]", tested_count, total_targets),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                Span::styled(" [UNTESTED]", Style::default().fg(Color::Red))
            };

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
                tested_label,
            ]));
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
        lines.push(hint_line(state, "Select", "Close"));
    }

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

        // Show region stats: population, total infected, total dead
        let infected = region.total_infected();
        let dead = region.total_dead();

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
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Select", "Back"));

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

        let disease_idx = match &target {
            DeployTarget::Vaccinate { disease_idx } => *disease_idx,
            DeployTarget::Treat { disease_idx } => *disease_idx,
        };
        let disease_name = state.diseases.get(disease_idx)
            .map(|d| disease_display_name(d, disease_idx))
            .unwrap_or_else(|| "Unknown".to_string());

        let inf = region.infections.iter().find(|i| i.disease_idx == disease_idx);

        match &target {
            DeployTarget::Vaccinate { .. } => {
                let infected = inf.map(|i| i.infected).unwrap_or(0.0);
                let dead = inf.map(|i| i.dead).unwrap_or(0.0);
                let immune = inf.map(|i| i.immune).unwrap_or(0.0);
                let susceptible = (pop - infected - dead - immune).max(0.0);
                let empty = susceptible == 0.0;

                let marker = if selected { "▶ " } else { "  " };
                let style = if empty {
                    Style::default().fg(Color::DarkGray)
                } else if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(Span::styled(
                    format!("{}Vaccinate susceptible ({})", marker, disease_name),
                    style,
                )));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("{} susceptible", format_number(susceptible)),
                        Style::default().fg(if empty { Color::DarkGray } else { Color::Cyan }),
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
                let empty = infected == 0.0;

                let marker = if selected { "▶ " } else { "  " };
                let style = if empty {
                    Style::default().fg(Color::DarkGray)
                } else if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(Span::styled(
                    format!("{}Treat infected ({})", marker, disease_name),
                    style,
                )));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("{} infected", format_number(infected)),
                        Style::default().fg(if empty { Color::DarkGray } else { Color::Red }),
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
    lines.push(hint_line(state, "Deploy", "Back"));

    (format!(" {} → {} ", med.name, region.name), lines)
}

fn render_confirm_deploy(
    state: &GameState,
    medicine_idx: usize,
    region_idx: usize,
    target_selection: usize,
) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let med = &state.medicines[medicine_idx];
    let region = &state.regions[region_idx];
    let target = med.decode_deploy_target(target_selection);

    let action_desc = match &target {
        Some(DeployTarget::Vaccinate { disease_idx }) => {
            let name = disease_display_name(&state.diseases[*disease_idx], *disease_idx);
            format!("Vaccinate {} against {}", region.name, name)
        }
        Some(DeployTarget::Treat { disease_idx }) => {
            let name = disease_display_name(&state.diseases[*disease_idx], *disease_idx);
            format!("Treat {} in {}", name, region.name)
        }
        None => "Deploy".to_string(),
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
        "  This medicine has not completed clinical trials.",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "  25% chance of adverse effects — 20% of doses",
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
