use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, ResearchKind, ResearchUiState, KNOWLEDGE_FOR_MEDICINE, KNOWLEDGE_FULL, KNOWLEDGE_NAME, format_days};
use crate::ui::hint_line;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines) = match &state.ui.research_ui {
        Some(ResearchUiState::BrowseCategories) => render_categories(state),
        Some(ResearchUiState::BrowseProjects { bench }) => render_projects(state, *bench),
        Some(ResearchUiState::ConfirmProject { bench, project_idx }) => {
            render_confirm(state, *bench, *project_idx)
        }
        Some(ResearchUiState::ViewActive { bench }) => render_active(state, *bench),
        None => (" Research ".to_string(), vec![]),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn render_categories(state: &GameState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    let categories = ["Field Research", "Bench Research"];
    for (i, name) in categories.iter().enumerate() {
        let selected = state.ui.panel_selection == i;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(Span::styled(
            format!("{}{}", marker, name),
            style,
        )));

        let (desc_text, status, status_color) = match i {
            0 => {
                let (s, c) = if state.field_research.is_some() {
                    (" [ACTIVE]", Color::Cyan)
                } else {
                    (" [NONE]", Color::DarkGray)
                };
                ("    Identify threats, run clinical trials", s, c)
            }
            _ => {
                let (s, c) = if state.bench_research.is_some() {
                    (" [ACTIVE]", Color::Magenta)
                } else {
                    (" [NONE]", Color::DarkGray)
                };
                ("    Develop medicines, manufacture doses", s, c)
            }
        };
        lines.push(Line::from(vec![
            Span::styled(desc_text, Style::default().fg(Color::DarkGray)),
            Span::styled(status, Style::default().fg(status_color)),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Select", "Close"));

    (" Research ".to_string(), lines)
}

fn render_projects(state: &GameState, bench: bool) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let title = if bench { " Bench Research " } else { " Field Research " };
    let mut has_selectable_items = false;

    let active = if bench { &state.bench_research } else { &state.field_research };

    // When a project is active, only show it — no starting new projects until it completes
    if let Some(project) = active {
        has_selectable_items = true;
        let selected = state.ui.panel_selection == 0;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };

        let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
        lines.push(Line::from(Span::styled(
            format!("{}[ACTIVE] {}", marker, format_kind(&project.kind, state)),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!("    Progress: {:.0}% ({}/{})", pct, format_days(project.progress), format_days(project.required_ticks)),
            Style::default().fg(Color::Green),
        )));
    } else {
        let projects = if bench {
            state.available_bench_projects()
        } else {
            state.available_field_projects()
        };

        if projects.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No projects available.",
                Style::default().fg(Color::DarkGray),
            )));
            if bench {
                lines.push(Line::from(Span::styled(
                    "  (Identify diseases to unlock medicine development)",
                    Style::default().fg(Color::DarkGray),
                )));
            }
        } else {
            has_selectable_items = true;
            for (i, kind) in projects.iter().enumerate() {
                let selected = state.ui.panel_selection == i;
                let marker = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(Span::styled(
                    format!("{}{}", marker, format_kind(kind, state)),
                    style,
                )));

                if let Some(detail) = format_detail(kind, state) {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", detail),
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                let (personnel, ticks, funding) = kind.costs(&state.medicines);
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("${:.0}", funding), Style::default().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::styled(format!("{} personnel", personnel), Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled(format_days(ticks), Style::default().fg(Color::DarkGray)),
                ]));
                lines.push(Line::from(""));
            }
        }
    }

    lines.push(Line::from(""));
    if has_selectable_items {
        lines.push(hint_line(state, "Select", "Back"));
    } else {
        lines.push(Line::from(Span::styled(
            "  [Esc] Back",
            Style::default().fg(Color::DarkGray),
        )));
    }

    (title.to_string(), lines)
}

fn render_confirm(state: &GameState, bench: bool, project_idx: usize) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let projects = if bench {
        state.available_bench_projects()
    } else {
        state.available_field_projects()
    };

    if let Some(kind) = projects.get(project_idx) {
        let (personnel, ticks, funding) = kind.costs(&state.medicines);
        let has_personnel = state.personnel_available() >= personnel;
        let has_funding = state.resources.funding >= funding;

        lines.push(Line::from(Span::styled(
            format!("  Start: {}", format_kind(kind, state)),
            Style::default().fg(Color::Cyan),
        )));
        if let Some(detail) = format_detail(kind, state) {
            lines.push(Line::from(Span::styled(
                format!("  {}", detail),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  Cost: "),
            Span::styled(format!("${:.0}", funding), Style::default().fg(
                if has_funding { Color::Green } else { Color::Red }
            )),
            Span::styled(
                format!("  (have ${:.0})", state.resources.funding),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  Personnel: "),
            Span::styled(format!("{}", personnel), Style::default().fg(
                if has_personnel { Color::Green } else { Color::Red }
            )),
            Span::styled(
                format!("  ({} available)", state.personnel_available()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  Duration: "),
            Span::styled(format_days(ticks), Style::default().fg(Color::White)),
        ]));

        let can_afford = has_personnel && has_funding;

        lines.push(Line::from(""));
        if can_afford {
            lines.push(hint_line(state, "Confirm", "Back"));
        } else {
            lines.push(Line::from(Span::styled(
                "  Insufficient resources!",
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(Span::styled(
                "  [Esc] Back",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    (" Confirm Research ".to_string(), lines)
}

fn render_active(state: &GameState, bench: bool) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let project = if bench { &state.bench_research } else { &state.field_research };

    if let Some(project) = project {
        let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
        let remaining = (project.required_ticks - project.progress).max(0.0);

        lines.push(Line::from(Span::styled(
            format!("  {}", format_kind(&project.kind, state)),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));

        // Progress bar
        let bar_width = 30;
        let filled = (pct / 100.0 * bar_width as f64) as usize;
        let empty = bar_width - filled;
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "█".repeat(filled),
                Style::default().fg(Color::Green),
            ),
            Span::styled(
                "░".repeat(empty),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(format!(" {:.0}%", pct)),
        ]));
        let speed = project.speed(&state.medicines);
        let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {} remaining", format_days(effective_remaining)),
            Style::default().fg(Color::White),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {} personnel assigned ({}x speed)", project.personnel_assigned, format!("{:.1}", speed)),
            Style::default().fg(Color::Cyan),
        )));

        // Personnel adjustment controls
        if !project.is_complete() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  [↑/k] ", Style::default().fg(Color::DarkGray)),
                Span::styled("Add personnel", Style::default().fg(
                    if state.personnel_available() >= 1 { Color::Green } else { Color::Red }
                )),
                Span::styled(format!("  ({} available)", state.personnel_available()), Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  [↓/j] ", Style::default().fg(Color::DarkGray)),
                Span::styled("Remove personnel", Style::default().fg(
                    if project.personnel_assigned > 1 { Color::Yellow } else { Color::Red }
                )),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  No active project.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc] Back",
        Style::default().fg(Color::DarkGray),
    )));

    let title = if bench { " Active: Bench " } else { " Active: Field " };
    (title.to_string(), lines)
}

fn format_kind(kind: &ResearchKind, state: &GameState) -> String {
    match kind {
        ResearchKind::IdentifyThreat { disease_idx } => {
            let disease = state.diseases.get(*disease_idx);
            let name = disease
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            let verb = if disease.is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME) {
                "Study"
            } else {
                "Identify"
            };
            format!("{}: {}", verb, name)
        }
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            format!("Develop: {}", name)
        }
        ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
            let med_name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            let dis_name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Trial: {} vs {}", med_name, dis_name)
        }
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let name = state.medicines.get(*medicine_idx)
                .map(|m| m.name.as_str())
                .unwrap_or("Unknown");
            format!("Manufacture: {}", name)
        }
        ResearchKind::GenomicSequencing { disease_idx } => {
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Sequence: {}", name)
        }
        ResearchKind::TrainPersonnel => "Train Personnel (+5)".to_string(),
    }
}

/// Supplementary detail line for a research project (targets, knowledge, etc).
fn format_detail(kind: &ResearchKind, state: &GameState) -> Option<String> {
    match kind {
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let names: Vec<String> = med.target_diseases.iter()
                .filter_map(|&d_idx| {
                    state.diseases.get(d_idx)
                        .map(|d| d.display_name(d_idx))
                })
                .collect();
            Some(format!("Targets: {}", names.join(", ")))
        }
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            Some(format!("Restores to {} doses", crate::format_number(med.max_doses)))
        }
        ResearchKind::GenomicSequencing { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            let current_rate = disease.effective_mutation_rate();
            let new_rate = current_rate * 0.5;
            Some(format!("Mutation rate: {:.4} → {:.4}", current_rate, new_rate))
        }
        ResearchKind::TrainPersonnel => {
            Some(format!("Current: {} personnel", state.resources.personnel))
        }
        ResearchKind::IdentifyThreat { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            if disease.knowledge >= KNOWLEDGE_NAME {
                // Already identified — explain what further study unlocks
                let next = if disease.knowledge < KNOWLEDGE_FOR_MEDICINE {
                    "Unlocks medicine development"
                } else if disease.knowledge < KNOWLEDGE_FULL {
                    "Reveals full pathogen stats"
                } else {
                    "Fully studied"
                };
                Some(format!("Knowledge: {:.0}% — {}", disease.knowledge * 100.0, next))
            } else if disease.knowledge > 0.0 {
                Some(format!("Knowledge: {:.0}%", disease.knowledge * 100.0))
            } else {
                None
            }
        }
        _ => None,
    }
}
