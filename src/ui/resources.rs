use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameOutcome, GameState, ResearchKind, KNOWLEDGE_NAME};
use crate::format_number;

/// Returns the height this bar needs: 2 normally, 3 when research is active.
pub fn height(state: &GameState) -> u16 {
    if state.field_research.is_some() || state.bench_research.is_some() {
        3
    } else {
        2
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let pause_indicator = match &state.outcome {
        GameOutcome::Lost => Span::styled(" DEFEAT ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        GameOutcome::Won => Span::styled(" VICTORY ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        GameOutcome::Playing if state.paused => Span::styled(" PAUSED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        GameOutcome::Playing => Span::styled(" RUNNING ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
    };

    let line1 = Line::from(vec![
        Span::styled("PANDEMIC DEFENSE", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        pause_indicator,
        Span::raw("  "),
        Span::styled(
            format!("Tick: {}", state.tick),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Funds: ${:.0}", state.resources.funding),
            Style::default().fg(Color::Green),
        ),
        {
            let income = state.funding_income_rate();
            let cost = state.total_policy_funding_cost();
            let net = income - cost;
            if cost > 0.0 {
                let color = if net >= 0.0 { Color::Green } else { Color::Red };
                let label = if net >= 0.0 {
                    format!(" (+${:.0}/t)", net)
                } else {
                    format!(" (-${:.0}/t)", -net)
                };
                Span::styled(label, Style::default().fg(color))
            } else {
                Span::styled(
                    format!(" (+${:.0}/t)", income),
                    Style::default().fg(Color::DarkGray),
                )
            }
        },
        Span::raw("  "),
        Span::styled(
            format!("RP: {:.0}", state.resources.research_points),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("  "),
        Span::styled(
            {
                let avail = state.personnel_available();
                let total = state.resources.personnel;
                if avail < total {
                    format!("Personnel: {}/{}", avail, total)
                } else {
                    format!("Personnel: {}", total)
                }
            },
            Style::default().fg(Color::White),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Infected: {}", format_number(state.total_infected())),
            Style::default().fg(Color::Red),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Dead: {}", format_number(state.total_dead())),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let mut lines = vec![line1];

    // Show active research on a second line when any research is running
    if state.field_research.is_some() || state.bench_research.is_some() {
        let mut spans: Vec<Span> = Vec::new();

        if let Some(ref project) = state.field_research {
            let pct = (project.progress / project.required_ticks * 100.0).min(100.0) as u32;
            spans.push(Span::styled("Field: ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("{} {}%", compact_research_label(&project.kind, state), pct),
                Style::default().fg(Color::Cyan),
            ));
        }

        if let Some(ref project) = state.bench_research {
            if !spans.is_empty() {
                spans.push(Span::styled("  │  ", Style::default().fg(Color::DarkGray)));
            }
            let pct = (project.progress / project.required_ticks * 100.0).min(100.0) as u32;
            spans.push(Span::styled("Bench: ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("{} {}%", compact_research_label(&project.kind, state), pct),
                Style::default().fg(Color::Magenta),
            ));
        }

        lines.push(Line::from(spans));
    }

    let widget = Paragraph::new(lines).block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(widget, area);
}

/// Compact research description for the header status line.
fn compact_research_label(kind: &ResearchKind, state: &GameState) -> String {
    match kind {
        ResearchKind::IdentifyThreat { disease_idx } => {
            let name = state.diseases.get(*disease_idx)
                .filter(|d| d.knowledge >= KNOWLEDGE_NAME)
                .map(|d| d.name.as_str())
                .unwrap_or("Unknown");
            format!("Identifying {}", name)
        }
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            name.to_string()
        }
        ResearchKind::ClinicalTrial { medicine_idx, .. } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            format!("Trial: {}", name)
        }
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            format!("Mfg: {}", name)
        }
    }
}
