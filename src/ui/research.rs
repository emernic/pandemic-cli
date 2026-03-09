use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, ResearchKind, ResearchTrack, ResearchUiState, KNOWLEDGE_FOR_MEDICINE, KNOWLEDGE_FULL, KNOWLEDGE_NAME, format_days, personnel_speed};
use crate::ui::hint_line;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, lines) = match &state.ui.research_ui {
        Some(ResearchUiState::BrowseCategories) => render_categories(state),
        Some(ResearchUiState::BrowseProjects { track }) => render_projects(state, *track),
        Some(ResearchUiState::ConfirmProject { track, project_idx, double_personnel }) => {
            render_confirm(state, *track, *project_idx, *double_personnel)
        }
        Some(ResearchUiState::ViewActive { track }) => render_active(state, *track),
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

    let categories = [
        ("Field Research", "Identify threats, run clinical trials", ResearchTrack::Field),
        ("Applied Research", "Develop medicines, manufacture doses", ResearchTrack::Applied),
        ("Basic Research", "Unlock new therapeutic technologies", ResearchTrack::Basic),
    ];
    for (i, (name, desc, track)) in categories.iter().enumerate() {
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

        let active = state.research_slot(*track).is_some();
        let (status, status_color) = if active {
            (" [ACTIVE]", match track {
                ResearchTrack::Field => Color::Cyan,
                ResearchTrack::Applied => Color::Magenta,
                ResearchTrack::Basic => Color::Green,
            })
        } else {
            (" [NONE]", Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("    {}", desc), Style::default().fg(Color::DarkGray)),
            Span::styled(status, Style::default().fg(status_color)),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(""));
    lines.push(hint_line(state, "Select", "Close"));

    (" Research ".to_string(), lines)
}

fn track_name(track: ResearchTrack) -> &'static str {
    match track {
        ResearchTrack::Field => "Field",
        ResearchTrack::Applied => "Applied",
        ResearchTrack::Basic => "Basic",
    }
}

fn render_projects(state: &GameState, track: ResearchTrack) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let title = match track {
        ResearchTrack::Field => " Field Research ",
        ResearchTrack::Applied => " Applied Research ",
        ResearchTrack::Basic => " Basic Research ",
    };
    let mut has_selectable_items = false;

    // Breadcrumb so the player knows where they are in the hierarchy
    lines.push(Line::from(Span::styled(
        format!("  Research > {}", track_name(track)),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let active = state.research_slot(track);

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
        let remaining = (project.required_ticks - project.progress).max(0.0);
        let speed = project.speed(&state.medicines);
        let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
        lines.push(Line::from(Span::styled(
            format!("{}[ACTIVE] {}", marker, format_kind(&project.kind, state)),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!("    Progress: {:.0}% — {} remaining", pct, format_days(effective_remaining)),
            Style::default().fg(Color::Green),
        )));
    } else {
        let projects = state.available_projects(track);

        if projects.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No projects available.",
                Style::default().fg(Color::DarkGray),
            )));
            let hint = match track {
                ResearchTrack::Applied => Some("(Identify diseases to unlock medicine development)"),
                ResearchTrack::Basic => Some("(Identify a pathogen to unlock basic research)"),
                ResearchTrack::Field => None,
            };
            if let Some(hint_text) = hint {
                lines.push(Line::from(Span::styled(
                    format!("  {}", hint_text),
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

fn render_confirm(state: &GameState, track: ResearchTrack, project_idx: usize, double_personnel: bool) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let projects = state.available_projects(track);

    if let Some(kind) = projects.get(project_idx) {
        let (base_personnel, ticks, funding) = kind.costs(&state.medicines);
        let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };
        let has_personnel = state.personnel_available() >= personnel;
        let has_funding = state.resources.funding >= funding;

        // Breadcrumb
        lines.push(Line::from(Span::styled(
            format!("  Research > {} > Confirm", track_name(track)),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

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

        // Toggle checkbox for 2x personnel
        let checkbox = if double_personnel { "[X]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", checkbox)),
            Span::styled("Assign 2x personnel", Style::default().fg(
                if double_personnel { Color::Yellow } else { Color::DarkGray }
            )),
            Span::styled("  [X] toggle", Style::default().fg(Color::DarkGray)),
        ]));

        // Show effective speed based on personnel ratio
        let speed = personnel_speed(personnel, base_personnel);
        let effective_ticks = ticks / speed;
        lines.push(Line::from(vec![
            Span::raw("  Duration: "),
            Span::styled(format_days(effective_ticks), Style::default().fg(Color::White)),
            Span::styled(format!("  ({:.1}x speed)", speed), Style::default().fg(
                if speed > 1.0 { Color::Green } else { Color::DarkGray }
            )),
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

fn render_active(state: &GameState, track: ResearchTrack) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let project = state.research_slot(track);

    if let Some(project) = project {
        let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
        let remaining = (project.required_ticks - project.progress).max(0.0);

        // Breadcrumb
        lines.push(Line::from(Span::styled(
            format!("  Research > {} > Active", track_name(track)),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

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
        let speed_color = if speed >= 1.4 { Color::Green } else if speed >= 1.0 { Color::Cyan } else { Color::Red };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} personnel assigned", project.personnel_assigned),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("  ({:.1}x speed)", speed),
                Style::default().fg(speed_color),
            ),
        ]));

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

    let title = match track {
        ResearchTrack::Field => " Active: Field ",
        ResearchTrack::Applied => " Active: Applied ",
        ResearchTrack::Basic => " Active: Basic ",
    };
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
        ResearchKind::BasicResearch { tech } => tech.name().to_string(),
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
        ResearchKind::BasicResearch { tech } => {
            Some(tech.description().to_string())
        }
        _ => None,
    }
}
