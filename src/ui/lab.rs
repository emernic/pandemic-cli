use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{AppState, LAB_LEVEL_1_COST, LAB_LEVEL_2_COST, Medicine, PERSONNEL_UPKEEP_COST, ResearchKind, LabUiState, TherapyType, KNOWLEDGE_FOR_MEDICINE, KNOWLEDGE_FULL, KNOWLEDGE_NAME, TICKS_PER_DAY, TRAIN_PERSONNEL_BATCH, format_days, personnel_speed};
use crate::ui::hint_line;

/// Maximum selection index for the lab panel in its current sub-state.
pub fn selection_max(ui_state: &LabUiState, state: &AppState) -> usize {
    match ui_state {
        LabUiState::BrowseAll => {
            state.research_flat_items().len().saturating_sub(1)
        }
        LabUiState::ConfirmProject { .. } | LabUiState::ConfirmLabUpgrade => 0,
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState) {
    let (title, lines, selected_line) = match &state.ui.lab_ui {
        Some(LabUiState::BrowseAll) => render_flat(state),
        Some(LabUiState::ConfirmProject { project_idx, double_personnel }) => {
            let (t, l) = render_confirm(state, *project_idx, *double_personnel);
            (t, l, None)
        }
        Some(LabUiState::ConfirmLabUpgrade) => {
            let (t, l) = render_confirm_lab_upgrade(state);
            (t, l, None)
        }
        None => (" Lab ".to_string(), vec![], None),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

/// Render the flat lab panel: one unified list, no section headers.
fn render_flat(state: &AppState) -> (String, Vec<Line<'static>>, Option<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let items = state.research_flat_items();
    let available = state.all_available_projects();

    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No projects available.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    for (item_idx, item) in items.iter().enumerate() {
        let selected = state.ui.panel_selection == item_idx;
        match item {
            crate::state::ResearchFlatItem::Active(ai) => {
                if let Some(project) = state.active_research.get(*ai) {
                    if selected { selected_line = Some(lines.len()); }
                    render_active_project(&mut lines, project, selected, state);
                }
            }
            crate::state::ResearchFlatItem::Available(avail_idx) => {
                if let Some(kind) = available.get(*avail_idx) {
                    if selected { selected_line = Some(lines.len()); }
                    render_available_project(&mut lines, kind, selected, state);
                }
            }
            crate::state::ResearchFlatItem::FullStockpile(kind) => {
                if selected { selected_line = Some(lines.len()); }
                let marker = if selected { "▶ " } else { "  " };
                let auto_tag = if state.auto_repeat_research.contains(kind) { " AUTO" } else { "" };
                lines.push(Line::from(Span::styled(
                    format!("{}{}{} [FULL]", marker, kind.display_label(&state.diseases, &state.medicines), auto_tag),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }
            crate::state::ResearchFlatItem::UpgradeLab => {
                lines.push(Line::from(""));
                if selected { selected_line = Some(lines.len()); }
                let marker = if selected { "▶ " } else { "  " };
                let upgrade_style = if selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Magenta)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}[PURCHASE] Upgrade Lab", marker),
                    upgrade_style,
                )));
                let (cost, next_name, pct) = if state.lab_level == 0 {
                    (LAB_LEVEL_1_COST, "Enhanced Sequencing", 30)
                } else {
                    (LAB_LEVEL_2_COST, "Advanced Genomics Center", 60)
                };
                let can_afford = state.resources.funding >= cost;
                let cost_style = if can_afford { Color::Cyan } else { Color::Red };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {} → {} (+{}% speed)", state.lab_level_name(), next_name, pct),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(" [¥{:.0}]", cost),
                        Style::default().fg(cost_style),
                    ),
                ]));
            }
        }
    }

    // Max lab info when fully upgraded
    if state.lab_level >= 2 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {} (max) — all research 60% faster", state.lab_level_name()),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    if !items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  [↑/↓] Select  [Enter] Confirm  [X] Auto",
            Style::default().fg(Color::DarkGray),
        )));
    }

    (" Lab ".to_string(), lines, selected_line)
}

/// Render an active research project (shows progress).
fn render_active_project(lines: &mut Vec<Line<'static>>, project: &crate::state::ResearchProject, selected: bool, state: &AppState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let pct = (project.progress / project.required_ticks * 100.0).min(100.0);
    let remaining = (project.required_ticks - project.progress).max(0.0);
    let personnel_speed = project.speed(&state.medicines);
    let infra_mult = state.research_infra_multiplier();
    let speed = personnel_speed * infra_mult;
    let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };
    let auto_tag = if state.auto_repeat_research.contains(&project.kind) { " AUTO" } else { "" };
    lines.push(Line::from(Span::styled(
        format!("{}[ACTIVE]{} {}", marker, auto_tag, project.kind.display_label(&state.diseases, &state.medicines)),
        style,
    )));
    if let Some(detail) = format_detail(&project.kind, state) {
        lines.push(Line::from(Span::styled(
            format!("    {}", detail),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!("    Progress: {:.0}%, {} remaining", pct, format_days(effective_remaining)),
        Style::default().fg(Color::Green),
    )));
    let speed_tag = if (speed - 1.0).abs() < 0.01 {
        String::new()
    } else {
        format!("  {:.1}x speed", speed)
    };
    lines.push(Line::from(Span::styled(
        format!("    {} personnel{}", project.personnel_assigned, speed_tag),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
}

/// Render an available (startable) research project with cost details.
fn render_available_project(lines: &mut Vec<Line<'static>>, kind: &ResearchKind, selected: bool, state: &AppState) {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let auto_tag = if state.auto_repeat_research.contains(kind) { " AUTO" } else { "" };
    lines.push(Line::from(Span::styled(
        format!("{}{}{}", marker, kind.display_label(&state.diseases, &state.medicines), auto_tag),
        style,
    )));
    if let Some(detail) = format_detail(kind, state) {
        lines.push(Line::from(Span::styled(
            format!("    {}", detail),
            Style::default().fg(Color::DarkGray),
        )));
    }
    let (personnel, ticks, funding) = state.effective_costs(kind);
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("¥{:.0}", funding), Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(format!("{} personnel", personnel), Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(format_days(ticks), Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));
}

fn render_confirm(state: &AppState, project_idx: usize, double_personnel: bool) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let projects = state.all_available_projects();

    if let Some(kind) = projects.get(project_idx) {
        let (base_personnel, ticks, funding) = state.effective_costs(kind);
        let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };
        let has_personnel = state.personnel_available() >= personnel;
        let has_funding = state.resources.funding >= funding;

        // Breadcrumb
        lines.push(Line::from(Span::styled(
            "  Lab > Confirm",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            format!("  Start: {}", kind.display_label(&state.diseases, &state.medicines)),
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
            Span::styled(format!("¥{:.0}", funding), Style::default().fg(
                if has_funding { Color::Green } else { Color::Red }
            )),
            Span::styled(
                format!("  (have ¥{:.0})", state.resources.funding),
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

    (" Confirm ".to_string(), lines)
}

fn render_confirm_lab_upgrade(state: &AppState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();
    let (cost, next_name, pct) = if state.lab_level == 0 {
        (LAB_LEVEL_1_COST, "Enhanced Sequencing", 30)
    } else {
        (LAB_LEVEL_2_COST, "Advanced Genomics Center", 60)
    };
    let can_afford = state.resources.funding >= cost;

    lines.push(Line::from(Span::styled(
        "  Lab > Lab Upgrade > Confirm",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Upgrade: {} → {}", state.lab_level_name(), next_name),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(Span::styled(
        format!("  All research runs {}% faster", pct),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  Cost: "),
        Span::styled(format!("¥{:.0}", cost), Style::default().fg(
            if can_afford { Color::Green } else { Color::Red }
        )),
        Span::styled(
            format!("  (have ¥{:.0})", state.resources.funding),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(""));
    if can_afford {
        lines.push(hint_line(state, "Confirm", "Back"));
    } else {
        lines.push(Line::from(Span::styled(
            "  Insufficient funding!",
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(Span::styled(
            "  [Esc] Back",
            Style::default().fg(Color::DarkGray),
        )));
    }

    (" Confirm Lab Upgrade ".to_string(), lines)
}

/// Label showing the manufacturing corporation for a medicine.
/// Returns " | Mfg: CorpName (+Approval)" or " | Mfg: CorpName" or "".
fn manufacturer_label(med: &Medicine, state: &AppState) -> String {
    let corp_idx = match med.manufacturer_corp_idx {
        Some(idx) => idx,
        None => return String::new(),
    };
    let corp = match state.corporations.get(corp_idx) {
        Some(c) => c,
        None => return String::new(),
    };
    if corp.board_seat {
        format!(" | Mfg: {} (+Approval)", corp.name)
    } else {
        format!(" | Mfg: {}", corp.name)
    }
}

/// Supplementary detail line for a research project (targets, knowledge, etc).
fn format_detail(kind: &ResearchKind, state: &AppState) -> Option<String> {
    match kind {
        ResearchKind::DevelopMedicine { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let names: Vec<String> = med.target_diseases.iter()
                .filter_map(|&d_idx| {
                    state.diseases.get(d_idx)
                        .map(|d| d.display_name(d_idx))
                })
                .collect();
            let mfg = manufacturer_label(med, state);
            if let Some(mech) = med.mechanism {
                let resist_label = if mech.resistance_rate_multiplier() > 1.2 {
                    "High"
                } else if mech.resistance_rate_multiplier() > 0.7 {
                    "Med"
                } else {
                    "Low"
                };
                Some(format!("{}: {} | Eff {:.0}%, Resist: {}{}",
                    mech.tradeoff_label(),
                    names.join(", "),
                    mech.efficacy_modifier() * 100.0,
                    resist_label,
                    mfg))
            } else {
                Some(format!("Targets: {}{}",
                    names.join(", "),
                    mfg))
            }
        }
        ResearchKind::ManufactureDoses { medicine_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let yield_bonus = state.manufacturing_yield_bonus();
            let target_doses = med.max_doses * yield_bonus;
            let current = crate::format_number(med.doses);
            let target = crate::format_number(target_doses);
            let bonus_note = if (yield_bonus - 1.0).abs() > 0.01 {
                format!(" (+{:.0}% mfg bonus)", (yield_bonus - 1.0) * 100.0)
            } else {
                String::new()
            };
            Some(format!("{} → {} doses{}", current, target, bonus_note))
        }
        ResearchKind::GenomicSequencing { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            let current_rate = disease.effective_variant_rate();
            let new_rate = current_rate * 0.5;
            Some(format!("Variant spawn rate: {:.4} → {:.4}", current_rate, new_rate))
        }
        ResearchKind::TrainPersonnel => {
            let added_upkeep = TRAIN_PERSONNEL_BATCH as f64 * PERSONNEL_UPKEEP_COST * TICKS_PER_DAY;
            Some(format!("Current: {} personnel (+¥{:.0}/day upkeep after)",
                state.resources.personnel, added_upkeep))
        }
        ResearchKind::IdentifyThreat { disease_idx } => {
            let disease = state.diseases.get(*disease_idx)?;
            if disease.knowledge >= KNOWLEDGE_NAME {
                // Already identified — explain what further study unlocks
                let has_targeted_tech = state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign);
                // Broad-spectrum targets all diseases — it's unlockable once ANY disease
                // reaches KNOWLEDGE_FOR_MEDICINE. Don't mislead player about this disease
                // "unlocking" broad-spectrum if it's already available or developed.
                let broad_already_available = state.medicines.iter().any(|m| {
                    m.therapy_type == TherapyType::BroadSpectrum
                        && (m.unlocked
                            || m.target_diseases.iter().any(|&d_idx| {
                                state.diseases.get(d_idx).map_or(false, |d| {
                                    d.knowledge >= KNOWLEDGE_FOR_MEDICINE
                                })
                            }))
                });
                let next = if disease.knowledge < KNOWLEDGE_FOR_MEDICINE {
                    if broad_already_available {
                        "Targeted medicine requires full study. Keep studying"
                    } else {
                        "Unlocks broad-spectrum medicine development"
                    }
                } else if disease.knowledge < KNOWLEDGE_FULL {
                    if has_targeted_tech {
                        "Unlocks targeted medicine development"
                    } else {
                        "Targeted medicines also need Basic Research: Targeted Drug Design"
                    }
                } else {
                    "Fully studied"
                };
                Some(format!("Knowledge: {:.0}% ({})", disease.knowledge * 100.0, next))
            } else if disease.knowledge > 0.0 {
                Some(format!("Knowledge: {:.0}%", disease.knowledge * 100.0))
            } else {
                None
            }
        }
        ResearchKind::BasicResearch { tech } => {
            Some(tech.description().to_string())
        }
        ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
            let med = state.medicines.get(*medicine_idx)?;
            let is_retrial = med.tested_against.contains(disease_idx);
            if is_retrial {
                Some("Re-trial — promotes to primary target (removes cross-reactive penalty)".to_string())
            } else {
                Some("First trial — tests efficacy and enables deployment".to_string())
            }
        }
    }
}
