use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, KNOWLEDGE_NAME, KNOWLEDGE_PARTIAL_STATS};
use crate::ui::research::disease_display_name;
use super::format_number;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();

    if state.diseases.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active threats.",
            Style::default().fg(Color::Green),
        )));
    } else {
        for (i, disease) in state.diseases.iter().enumerate() {
            let selected = state.ui.panel_selection == i;
            let marker = if selected { "▶ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let display_name = disease_display_name(disease, i);
            lines.push(Line::from(Span::styled(
                format!("{}{}", marker, display_name),
                style,
            )));

            if disease.knowledge < KNOWLEDGE_NAME {
                // Completely unknown — show nothing
                lines.push(Line::from(Span::styled(
                    "    ???",
                    Style::default().fg(Color::DarkGray),
                )));
            } else if disease.knowledge < KNOWLEDGE_PARTIAL_STATS {
                // Name known, partial stats
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("Infect: {:.0}%", disease.infectivity * 100.0),
                        Style::default().fg(Color::Red),
                    ),
                    Span::raw("  "),
                    Span::styled("Lethal: ?", Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled("Recov: ?", Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                // Full stats visible
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("Infect: {:.0}%", disease.infectivity * 100.0),
                        Style::default().fg(Color::Red),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("Lethal: {:.1}%", disease.lethality * 100.0),
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("Recov: {:.0}%", disease.recovery_rate * 100.0),
                        Style::default().fg(Color::Green),
                    ),
                ]));
            }

            // Show knowledge bar
            if disease.knowledge < 1.0 {
                let pct = (disease.knowledge * 100.0).min(100.0);
                lines.push(Line::from(Span::styled(
                    format!("    Knowledge: {:.0}%", pct),
                    Style::default().fg(Color::Blue),
                )));
            }

            if selected {
                render_disease_detail(&mut lines, state, i);
            }

            lines.push(Line::from(""));
        }
    }

    let block = Block::default()
        .title(" Threats ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_disease_detail(lines: &mut Vec<Line>, state: &GameState, disease_idx: usize) {
    let hdr = Style::default().fg(Color::DarkGray);
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("{:<16}", "Region"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Infected"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Immune"), hdr),
        Span::raw("  "),
        Span::styled(format!("{:>8}", "Dead"), hdr),
    ]));

    let mut total_infected = 0.0;
    let mut total_immune = 0.0;
    let mut total_dead = 0.0;

    for region in &state.regions {
        if let Some(inf) = region.disease_state(disease_idx) {
            if inf.infected <= 0.0 && inf.immune <= 0.0 && inf.dead <= 0.0 {
                continue;
            }
            total_infected += inf.infected;
            total_immune += inf.immune;
            total_dead += inf.dead;

            let name = format!("{:<16}", region.name);
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(
                    format!("{:>8}", format_number(inf.infected)),
                    Style::default().fg(Color::LightRed),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:>8}", format_number(inf.immune)),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:>8}", format_number(inf.dead)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    // Totals
    lines.push(Line::from(vec![
        Span::styled("    ────────────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
        Span::styled("──────────", Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("{:<16}", "TOTAL"),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_infected)),
            Style::default().fg(Color::LightRed),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_immune)),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:>8}", format_number(total_dead)),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
}
