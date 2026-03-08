use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::engine::{available_bench_projects, available_field_projects};
use crate::state::{GameState, ResearchKind, ResearchUiState};
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

        let desc = match i {
            0 => {
                let active = if state.field_research.is_some() {
                    " [ACTIVE]"
                } else {
                    ""
                };
                format!("    Identify threats, run clinical trials{}", active)
            }
            _ => {
                let active = if state.bench_research.is_some() {
                    " [ACTIVE]"
                } else {
                    ""
                };
                format!("    Develop new medicines{}", active)
            }
        };
        lines.push(Line::from(Span::styled(
            desc,
            Style::default().fg(Color::DarkGray),
        )));
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
            format!("    Progress: {:.0}% ({:.0}/{:.0} ticks)", pct, project.progress, project.required_ticks),
            Style::default().fg(Color::Green),
        )));
    } else {
        let projects = if bench {
            available_bench_projects(state)
        } else {
            available_field_projects(state)
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

                // Show target diseases for medicine development
                if let Some(targets) = format_targets(kind, state) {
                    lines.push(Line::from(Span::styled(
                        format!("    Targets: {}", targets),
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                let (rp, personnel, ticks) = crate::engine::project_costs(kind);
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("{:.0} RP", rp), Style::default().fg(Color::Magenta)),
                    Span::raw("  "),
                    Span::styled(format!("{} personnel", personnel), Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled(format!("{:.0} ticks", ticks), Style::default().fg(Color::DarkGray)),
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
        available_bench_projects(state)
    } else {
        available_field_projects(state)
    };

    if let Some(kind) = projects.get(project_idx) {
        let (rp, personnel, ticks) = crate::engine::project_costs(kind);

        lines.push(Line::from(Span::styled(
            format!("  Start: {}", format_kind(kind, state)),
            Style::default().fg(Color::Cyan),
        )));
        if let Some(targets) = format_targets(kind, state) {
            lines.push(Line::from(Span::styled(
                format!("  Targets: {}", targets),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  Cost: "),
            Span::styled(format!("{:.0} RP", rp), Style::default().fg(
                if state.resources.research_points >= rp { Color::Green } else { Color::Red }
            )),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  Personnel: "),
            Span::styled(format!("{}", personnel), Style::default().fg(
                if state.personnel_available() >= personnel { Color::Green } else { Color::Red }
            )),
            Span::styled(
                format!("  ({} available)", state.personnel_available()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  Duration: "),
            Span::styled(format!("{:.0} ticks", ticks), Style::default().fg(Color::White)),
        ]));

        let can_afford = state.resources.research_points >= rp
            && state.personnel_available() >= personnel;

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
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {:.0} ticks remaining", remaining),
            Style::default().fg(Color::White),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {} personnel assigned", project.personnel_assigned),
            Style::default().fg(Color::DarkGray),
        )));
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
            let name = state.diseases.get(*disease_idx)
                .map(|d| disease_display_name(d, *disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Identify: {}", name)
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
                .map(|d| disease_display_name(d, *disease_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            format!("Trial: {} vs {}", med_name, dis_name)
        }
    }
}

/// For DevelopMedicine projects, return the target disease names.
fn format_targets(kind: &ResearchKind, state: &GameState) -> Option<String> {
    match kind {
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let names: Vec<String> = med.target_diseases.iter()
                .filter_map(|&d_idx| {
                    state.diseases.get(d_idx)
                        .map(|d| disease_display_name(d, d_idx))
                })
                .collect();
            Some(names.join(", "))
        }
        _ => None,
    }
}

/// Display name for a disease based on knowledge level.
pub fn disease_display_name(disease: &crate::state::Disease, idx: usize) -> String {
    if disease.knowledge >= crate::state::KNOWLEDGE_NAME {
        disease.name.clone()
    } else {
        format!("Unknown Pathogen #{}", idx + 1)
    }
}
