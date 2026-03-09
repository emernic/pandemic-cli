use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameOutcome, GameState, ResearchKind, SimState, KNOWLEDGE_NAME, TICKS_PER_DAY, ticks_to_days};
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
        GameOutcome::Playing => match &state.sim_state {
            SimState::Running => {
                let speed = state.ui.speed_multiplier.max(1);
                if speed > 1 {
                    Span::styled(format!(" ▶▶ {}x ", speed), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled(" RUNNING ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                }
            }
            SimState::Paused => Span::styled(" PAUSED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            SimState::Event { .. } => Span::styled(" EVENT ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        },
    };

    let line1 = Line::from(vec![
        pause_indicator,
        Span::raw("  "),
        Span::styled(
            format!("Day: {:.1}", ticks_to_days(state.tick as f64)),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Funds: ${:.0}", state.resources.funding),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!(" (+${:.0}/day)", state.funding_income_rate() * TICKS_PER_DAY),
            Style::default().fg(Color::DarkGray),
        ),
        {
            let ban_penalty = state.travel_ban_income_penalty() * TICKS_PER_DAY;
            if ban_penalty > 0.5 {
                Span::styled(
                    format!(" (−${:.0} bans)", ban_penalty),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                Span::raw("")
            }
        },
        {
            let cost = state.total_policy_funding_cost();
            if cost > 0.0 {
                Span::styled(
                    format!(" −${:.0}/day policy", cost * TICKS_PER_DAY),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                Span::raw("")
            }
        },
        Span::raw("  "),
        Span::styled(
            format!("RP: {:.0}", state.resources.research_points),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(
            format!(" (+{:.0}/day)", state.rp_income_rate() * TICKS_PER_DAY),
            Style::default().fg(Color::DarkGray),
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
        ResearchKind::GenomicSequencing { disease_idx } => {
            let name = state.diseases.get(*disease_idx)
                .filter(|d| d.knowledge >= KNOWLEDGE_NAME)
                .map(|d| d.name.as_str())
                .unwrap_or("Unknown");
            format!("Sequencing {}", name)
        }
        ResearchKind::TrainPersonnel => "Training".to_string(),
    }
}
