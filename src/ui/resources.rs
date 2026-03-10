use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameOutcome, GameState, ResearchKind, ResearchTrack, SimState, ThreatLevel, KNOWLEDGE_NAME, TICKS_PER_DAY, ticks_to_days};
use crate::format_number;

/// Returns the height this bar needs: always 3 to show research status.
pub fn height(_state: &GameState) -> u16 {
    3
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let pause_indicator = match &state.outcome {
        GameOutcome::Lost => Span::styled(" DEFEAT ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
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
        {
            let (label, color) = match state.threat_level {
                ThreatLevel::Normal => ("DEFCON-5", Color::Green),
                ThreatLevel::Elevated => ("DEFCON-4", Color::Yellow),
                ThreatLevel::Crisis => ("DEFCON-3", Color::Red),
                ThreatLevel::Catastrophe => ("DEFCON-2", Color::LightRed),
                ThreatLevel::Extinction => ("DEFCON-1", Color::LightRed),
            };
            Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD))
        },
        Span::raw("  "),
        {
            let pol = state.resources.political_power;
            let pol_color = if pol >= 0.5 { Color::Green } else if pol >= 0.2 { Color::Yellow } else { Color::Red };
            Span::styled(
                format!("POL: {:.0}%", pol * 100.0),
                Style::default().fg(pol_color),
            )
        },
        // POL trend arrow: compare current POL to its drift target
        {
            let target = state.pol_target();
            let pol = state.resources.political_power;
            let gap = target - pol;
            if gap > 0.02 {
                Span::styled(" \u{25b2}", Style::default().fg(Color::Green)) // ▲ rising
            } else if gap < -0.02 {
                Span::styled(" \u{25bc}", Style::default().fg(Color::Red)) // ▼ falling
            } else {
                Span::styled(" \u{2014}", Style::default().fg(Color::DarkGray)) // — stable
            }
        },
        // Next POL unlock hint
        match state.next_pol_unlock() {
            Some((name, threshold)) => Span::styled(
                format!(" ({}@{:.0}%)", name, threshold * 100.0),
                Style::default().fg(Color::DarkGray),
            ),
            None => Span::raw(""),
        },
        Span::raw("  "),
        Span::styled(
            format!("Funds: ¥{:.0}", state.resources.funding),
            Style::default().fg(Color::Green),
        ),
        {
            let gross = state.funding_income_rate() * TICKS_PER_DAY;
            let upkeep = state.personnel_upkeep_rate() * TICKS_PER_DAY;
            let policy = state.total_policy_funding_cost() * TICKS_PER_DAY;
            let net = gross - upkeep - policy;
            let (sign, color) = if net >= 0.0 {
                ("+", Color::DarkGray)
            } else {
                ("", Color::Red)
            };
            Span::styled(
                format!(" ({sign}¥{net:.0}/day)"),
                Style::default().fg(color),
            )
        },
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
        {
            let screened = state.total_infected_screened();
            let any_estimated = state.regions.iter().enumerate()
                .any(|(i, _)| state.screening_visibility(i) < 1.0);
            let prefix = if any_estimated { "Infected: ~" } else { "Infected: " };
            Span::styled(
                format!("{}{}", prefix, format_number(screened)),
                Style::default().fg(Color::Red),
            )
        },
        // Infection trend arrow (compared to ~1 day ago)
        // When infections drop because people are dying (not recovering),
        // show a red ▼ instead of green — declining infected is bad news
        // when the death rate is accelerating.
        match state.infection_trend() {
            Some(ratio) if ratio > 1.05 => Span::styled(" \u{25b2}", Style::default().fg(Color::Red)),
            Some(ratio) if ratio < 0.95 => {
                let deaths_rising = state.death_trend().is_some_and(|d| d > 1.05);
                if deaths_rising {
                    Span::styled(" \u{25bc}", Style::default().fg(Color::Red))
                } else {
                    Span::styled(" \u{25bc}", Style::default().fg(Color::Green))
                }
            }
            Some(_) => Span::styled(" \u{2014}", Style::default().fg(Color::DarkGray)),
            None => Span::raw(""),
        },
        Span::raw("  "),
        Span::styled(
            format!("Dead: {}", format_number(state.total_dead_detected())),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let mut lines = vec![line1];

    // Always show research status line so empty slots are visible
    {
        let mut spans: Vec<Span> = Vec::new();

        // Field research: show all active projects (or "None")
        let field_auto = if state.auto_research[ResearchTrack::Field.index()] { "Field(A): " } else { "Field: " };
        spans.push(Span::styled(field_auto, Style::default().fg(Color::DarkGray)));
        if state.field_research.is_empty() {
            spans.push(Span::styled("None", Style::default().fg(Color::DarkGray)));
        } else {
            for (i, project) in state.field_research.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(", ", Style::default().fg(Color::DarkGray)));
                }
                let pct = (project.progress / project.required_ticks * 100.0).min(100.0) as u32;
                spans.push(Span::styled(
                    format!("{} {}%", compact_research_label(&project.kind, state), pct),
                    Style::default().fg(Color::Cyan),
                ));
            }
        }

        // Applied and Basic tracks
        for (label, track, project, color) in [
            ("Applied", ResearchTrack::Applied, &state.applied_research, Color::Magenta),
            ("Basic", ResearchTrack::Basic, &state.basic_research, Color::Green),
        ] {
            spans.push(Span::styled("  │  ", Style::default().fg(Color::DarkGray)));
            let auto_tag = if state.auto_research[track.index()] { "(A)" } else { "" };
            spans.push(Span::styled(format!("{}{}: ", label, auto_tag), Style::default().fg(Color::DarkGray)));
            if let Some(project) = project {
                let pct = (project.progress / project.required_ticks * 100.0).min(100.0) as u32;
                spans.push(Span::styled(
                    format!("{} {}%", compact_research_label(&project.kind, state), pct),
                    Style::default().fg(color),
                ));
            } else {
                spans.push(Span::styled("None", Style::default().fg(Color::DarkGray)));
            }
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
            let disease = state.diseases.get(*disease_idx);
            let name = disease
                .filter(|d| d.knowledge >= KNOWLEDGE_NAME)
                .map(|d| d.name.as_str())
                .unwrap_or("Unknown");
            let verb = if disease.is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME) {
                "Studying"
            } else {
                "Identifying"
            };
            format!("{} {}", verb, name)
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
        ResearchKind::BasicResearch { tech } => tech.name().to_string(),
        ResearchKind::SuppressPathogen { disease_idx } => {
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Suppress {}", name)
        }
    }
}
