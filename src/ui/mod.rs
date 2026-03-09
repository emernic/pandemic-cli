pub mod home;
pub mod hotkey_bar;
pub mod medicines;
pub mod policy;
pub mod research;
pub mod resources;
pub mod threats;
pub mod region_list;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameEvent, GameOutcome, GameState, Panel, ticks_to_days};
use crate::format_number;

/// Build a hint line like "[Enter] Select  [Esc] Close", omitting the Enter
/// portion when the game is over (Confirm is blocked post-game).
pub fn hint_line(state: &GameState, enter_label: &str, esc_label: &str) -> Line<'static> {
    let hint = if state.outcome == GameOutcome::Playing {
        format!("  [Enter] {enter_label}  [Esc] {esc_label}")
    } else {
        format!("  [Esc] {esc_label}")
    };
    Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)))
}

/// Convert game events from the most recent tick into a status message.
/// Called after each tick by the game loop / snapshot runner. This keeps
/// human-facing strings in the UI layer, not in engine.rs.
///
/// Game-rule state transitions (pausing on game-over, disease detection,
/// region collapse, crisis events) are handled in tick(). This function
/// only handles UI presentation responses.
pub fn process_events(state: &mut GameState) {
    if state.events.is_empty() {
        return;
    }

    // UI responses to game events
    if state.events.iter().any(|e| matches!(e, GameEvent::GameOver)) {
        state.ui.open_panel = Panel::None;
    }
    if state.events.iter().any(|e| matches!(e, GameEvent::CrisisStarted)) {
        state.ui.crisis_selection = 0;
        state.ui.crisis_auto_resolve = false;
    }
    // Reset speed display when tick() auto-paused on critical events
    if state.events.iter().any(|e| matches!(e,
        GameEvent::RegionCollapsed { .. } | GameEvent::DiseaseDetected { .. }))
    {
        state.ui.speed_multiplier = 1;
    }

    // Pick the most important event to display as status message.
    let suspended: Vec<_> = state.events.iter()
        .filter_map(|e| match e {
            GameEvent::PolicySuspended { region_idx, policy_name } => {
                let region = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str())
                    .unwrap_or("Unknown");
                Some(format!("{} in {}", policy_name, region))
            }
            _ => None,
        })
        .collect();

    // Priority: RegionCollapsed > DiseaseDetected > PolicySuspended > FundingWarning > DiseaseMutated
    let msg = if let Some(GameEvent::RegionCollapsed { region_idx }) =
        state.events.iter().find(|e| matches!(e, GameEvent::RegionCollapsed { .. }))
    {
        let region_name = state.regions.get(*region_idx)
            .map(|r| r.name.as_str())
            .unwrap_or("Unknown");
        let remaining = state.regions.iter().filter(|r| !r.collapsed).count();
        format!("COLLAPSE: {region_name} has fallen! {remaining} regions remain.")
    } else if let Some(GameEvent::DiseaseDetected { disease_idx }) =
        state.events.iter().find(|e| matches!(e, GameEvent::DiseaseDetected { .. }))
    {
        // Find which regions have this disease
        let affected: Vec<&str> = state.regions.iter()
            .filter(|r| r.disease_state(*disease_idx).is_some_and(|inf| inf.infected > 0.0))
            .map(|r| r.name.as_str())
            .collect();
        if affected.len() > 1 {
            format!("NEW THREAT detected spreading across {} regions! Use [R] Research to identify it.", affected.len())
        } else {
            let region_name = affected.first().unwrap_or(&"unknown");
            format!("NEW THREAT detected in {region_name}! Use [R] Research to identify it.")
        }
    } else if !suspended.is_empty() {
        format!("Funding crisis: suspended {}", suspended.join(", "))
    } else if state.events.iter().any(|e| matches!(e, GameEvent::FundingWarning)) {
        "LOW FUNDS: Policies at risk of suspension!".to_string()
    } else if let Some(GameEvent::PersonnelAttrition { count }) =
        state.events.iter().find(|e| matches!(e, GameEvent::PersonnelAttrition { .. }))
    {
        format!("{count} personnel resigned — no funding for wages")
    } else if let Some(GameEvent::DiseaseMutated { disease_idx, infectivity_factor, lethality_factor, .. }) =
        state.events.iter().find(|e| matches!(e, GameEvent::DiseaseMutated { .. }))
    {
        // Only show mutation messages when the player has medicines affected by the drift.
        // Without an outdated medicine, mutations are invisible to gameplay — showing
        // "X has mutated" is noise the player can't act on.
        if state.has_outdated_medicine(*disease_idx) {
            let name = state.diseases.get(*disease_idx)
                .map(|d| d.display_name(*disease_idx))
                .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
            // Find the worst strain efficacy across affected medicines
            let worst_eff = state.medicines.iter()
                .filter(|m| m.target_diseases.contains(disease_idx)
                    && (m.tested_against.contains(disease_idx) || m.unlocked))
                .map(|m| m.strain_efficacy(*disease_idx, &state.diseases))
                .fold(1.0_f64, f64::min);
            // With Rapid Sequencing unlocked, show stat change details
            let detail = if state.unlocked_techs.contains(&crate::state::BasicTech::RapidSequencing) {
                let inf_pct = (infectivity_factor - 1.0) * 100.0;
                let leth_pct = (lethality_factor - 1.0) * 100.0;
                format!(" (spread {:+.0}%, lethality {:+.0}%)", inf_pct, leth_pct)
            } else {
                String::new()
            };
            format!("{name} mutated{detail} — efficacy {:.0}%! Re-trial in [R].",
                worst_eff * 100.0)
        } else {
            return; // No actionable medicine — suppress noise
        }
    } else if state.events.iter().any(|e| matches!(e, GameEvent::CrisisAutoResolved)) {
        "Crisis auto-resolved (saved preference)".to_string()
    } else if let Some(GameEvent::DiseaseSpreadToRegion { region_idx, .. }) =
        state.events.iter().find(|e| matches!(e, GameEvent::DiseaseSpreadToRegion { .. }))
    {
        let region_name = state.regions.get(*region_idx)
            .map(|r| r.name.as_str())
            .unwrap_or("Unknown");
        let any_policy_active = state.policies.iter().any(|p| p.any_active());
        if any_policy_active {
            format!("Disease spreading to {region_name}.")
        } else {
            format!("Disease spreading to {region_name}! Use [P] Policy to contain.")
        }
    } else {
        return;
    };
    state.ui.status_message = Some(msg);
}

pub fn render(f: &mut Frame, state: &GameState) {
    let header_height = resources::height(state);
    let has_extra_line = state.ui.status_message.is_some() || state.outcome != GameOutcome::Playing;
    let hotkey_height = if has_extra_line { 3 } else { 2 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),  // resources bar (expands when research active)
            Constraint::Min(8),              // main area
            Constraint::Length(hotkey_height), // hotkey bar (+ status line)
        ])
        .split(f.area());

    resources::render(f, chunks[0], state);
    hotkey_bar::render(f, chunks[2], state);

    // All views share the same 50/50 horizontal split: region list left, panel right.
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    region_list::render(f, split[0], state);

    // Right panel: crisis overlay takes priority, then panel or default view.
    if let Some(crisis) = &state.active_crisis {
        render_crisis(f, split[1], crisis, state.ui.crisis_selection, state);
    } else {
        match &state.ui.open_panel {
            Panel::None if state.outcome != GameOutcome::Playing => {
                render_game_over(f, split[1], state);
            }
            Panel::None => home::render(f, split[1], state),
            Panel::Threats => threats::render(f, split[1], state),
            Panel::Medicines => medicines::render(f, split[1], state),
            Panel::Research => research::render(f, split[1], state),
            Panel::Policy => policy::render(f, split[1], state),
            panel => render_placeholder_panel(f, split[1], panel),
        }
    }
}

fn render_crisis(f: &mut Frame, area: Rect, crisis: &crate::state::CrisisEvent, selection: usize, state: &GameState) {
    let auto_resolve = state.ui.crisis_auto_resolve;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    // Flashing warning symbols: toggle every ~500ms using wall-clock time
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let warning = if (millis / 500) % 2 == 0 { "⚠" } else { " " };
    lines.push(Line::from(Span::styled(
        format!("  {} {} {}", warning, crisis.title, warning),
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(""));

    // Word-wrap description manually for the panel width
    let desc = &crisis.description;
    let max_width = area.width.saturating_sub(4) as usize;
    for chunk in textwrap(desc, max_width) {
        lines.push(Line::from(format!("  {}", chunk)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ── Choose your response ──",
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    let options = [&crisis.option_a, &crisis.option_b];
    let labels = ["A", "B"];
    for (i, (option, label)) in options.iter().zip(labels.iter()).enumerate() {
        let affordable = option.cost.as_ref().map_or(true, |c| c.affordable(state));
        let marker = if selection == i { "▶ " } else { "  " };

        let style = if !affordable {
            Style::default().fg(Color::Red)
        } else if selection == i {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let suffix = if !affordable { " (not enough resources)" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("  {}{}: {}{}", marker, label, option.label, suffix),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!("      {}", option.description),
            if !affordable { Style::default().fg(Color::Red) } else { Style::default().fg(Color::DarkGray) },
        )));
        lines.push(Line::from(""));
    }

    // Auto-resolve toggle indicator
    if auto_resolve {
        lines.push(Line::from(Span::styled(
            "  [X] Always pick selected option",
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(""));
    }

    let auto_hint = if auto_resolve { "[X] Auto:ON " } else { "[X] Auto " };
    lines.push(Line::from(Span::styled(
        format!("  [↑/↓] Select  [Enter] Confirm  {}", auto_hint),
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .title(" CRISIS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

/// Simple word wrap: split a string into lines that fit within max_width.
fn textwrap(s: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in s.split_whitespace() {
        if current.is_empty() {
            current = word.to_string();
        } else if current.len() + 1 + word.len() > max_width {
            lines.push(current);
            current = word.to_string();
        } else {
            current.push(' ');
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn render_placeholder_panel(f: &mut Frame, area: Rect, panel: &Panel) {
    let title = match panel {
        Panel::Research => " Research ",
        Panel::Help => " Help ",
        _ => " Panel ",
    };

    let content = match panel {
        Panel::Help => vec![
            Line::from(""),
            Line::from(Span::styled("Pandemic Defense", Style::default().fg(Color::Cyan))),
            Line::from(""),
            Line::from("Defend humanity against disease outbreaks."),
            Line::from(""),
            Line::from(Span::styled("Controls:", Style::default().fg(Color::Yellow))),
            Line::from("  [T] View active threats"),
            Line::from("  [R] Research panel"),
            Line::from("  [M] Medicines panel"),
            Line::from("  [P] Policy panel"),
            Line::from("  [Space] Pause/Resume"),
            Line::from("  [Z] Speed up (1x→2x→4x→6x, pause resets)"),
            Line::from("  [X] Auto-resolve crisis (toggle during event)"),
            Line::from("  [←/→] Cycle regions  [↑/↓] Panel items"),
            Line::from("  [Esc] Close panel"),
            Line::from("  [Q] Save & Quit"),
        ],
        _ => vec![
            Line::from(""),
            Line::from(Span::styled(
                "Coming soon...",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from("This panel will be implemented"),
            Line::from("as game mechanics are designed."),
        ],
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let widget = Paragraph::new(content).block(block);
    f.render_widget(widget, area);
}

fn render_game_over(f: &mut Frame, area: Rect, state: &GameState) {
    let (title, border_color) = (" DEFEAT ", Color::Red);

    let total_dead = state.total_dead();
    let total_immune = state.total_immune();
    let initial_pop = state.initial_population();
    let survivors = (initial_pop - total_dead).max(0.0);
    let survival_pct = if initial_pop > 0.0 { (survivors / initial_pop) * 100.0 } else { 0.0 };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    let defeat_msg = if state.mercy_rule {
        "  Your organization has run out of resources. The pandemic will run its course."
    } else {
        "  Humanity has fallen. Too many lives were lost."
    };
    lines.push(Line::from(Span::styled(
        defeat_msg,
        Style::default().fg(Color::Red),
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ── Summary ──",
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    let stat_label = Style::default().fg(Color::DarkGray);
    let stat_value = Style::default().fg(Color::White);

    lines.push(Line::from(vec![
        Span::styled("  Duration:       ", stat_label),
        Span::styled(format!("{:.1} days", ticks_to_days(state.tick as f64)), stat_value),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Total Dead:     ", stat_label),
        Span::styled(
            format_number(total_dead),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!("  ({:.1}% of population)", (total_dead / initial_pop) * 100.0 + 0.0),
            stat_label,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Survivors:      ", stat_label),
        Span::styled(
            format_number(survivors),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!("  ({survival_pct:.1}%)"),
            stat_label,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Immunized:      ", stat_label),
        Span::styled(
            format_number(total_immune),
            Style::default().fg(Color::Cyan),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Still Infected: ", stat_label),
        Span::styled(
            format_number(state.total_infected()),
            Style::default().fg(if state.total_infected() > 0.0 { Color::Yellow } else { Color::DarkGray }),
        ),
    ]));

    // Collapse timeline
    let mut collapse_order: Vec<(usize, Option<u64>)> = state.regions.iter().enumerate()
        .map(|(i, r)| (i, r.collapsed_at_tick))
        .collect();
    collapse_order.sort_by_key(|(_, tick)| tick.unwrap_or(u64::MAX));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ── Collapse Timeline ──",
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    for (region_idx, collapsed_tick) in &collapse_order {
        let region = &state.regions[*region_idx];
        let dead = region.total_dead();
        let pop = region.population as f64;
        let dead_pct = if pop > 0.0 { ((dead / pop) * 100.0).min(100.0) } else { 0.0 };
        let timing = if let Some(tick) = collapsed_tick {
            format!("Day {:>5.1}", ticks_to_days(*tick as f64))
        } else {
            "       ".to_string()
        };
        let status_color = if region.collapsed { Color::Red } else { Color::Green };
        let status = if region.collapsed { "FELL" } else { "held" };
        lines.push(Line::from(vec![
            Span::styled(format!("  {timing}  "), stat_label),
            Span::styled(format!("{:<16}", region.name), stat_value),
            Span::styled(
                format!("{status:<4}"),
                Style::default().fg(status_color),
            ),
            Span::styled(
                format!("  {} dead ({:.1}%)", format_number(dead), dead_pct),
                Style::default().fg(if dead > 0.0 { Color::Red } else { Color::DarkGray }),
            ),
        ]));
    }

    // Per-disease breakdown with pathogen reveal
    if !state.diseases.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── Pathogen Report ──",
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));

        for (d_idx, disease) in state.diseases.iter().enumerate() {
            // Sum deaths across all regions for this disease
            let disease_dead: f64 = state.regions.iter()
                .flat_map(|r| r.infections.iter())
                .filter(|inf| inf.disease_idx == d_idx)
                .map(|inf| inf.dead)
                .sum();

            // Always reveal the true name on defeat
            let revealed = disease.name != disease.display_name(d_idx);
            let name_str = if revealed {
                format!("{} (was Unknown Pathogen #{})", disease.name, d_idx + 1)
            } else {
                disease.name.clone()
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {name_str}"), stat_value),
                Span::styled(
                    format!("  {} · {}", disease.pathogen_type.label(), disease.transmission.label()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("      Deaths: ", stat_label),
                Span::styled(
                    format_number(disease_dead),
                    Style::default().fg(if disease_dead > 0.0 { Color::Red } else { Color::DarkGray }),
                ),
                Span::styled(
                    format!("  ({:.1}% of total)", if total_dead > 0.0 { disease_dead / total_dead * 100.0 } else { 0.0 }),
                    stat_label,
                ),
            ]));
        }
    }

    // Score — rewards surviving longer with more people alive
    let days = ticks_to_days(state.tick as f64);
    let score = (days * survival_pct).round() as u64;
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ── Score ──",
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Score:          ", stat_label),
        Span::styled(
            format!("{score}"),
            Style::default().fg(Color::Yellow).add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(
            "  (days × survival %)".to_string(),
            stat_label,
        ),
    ]));

    // Strategic tips
    let tips = state.defeat_tips();
    if !tips.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── What to try next time ──",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
        for tip in &tips {
            lines.push(Line::from(Span::styled(
                format!("  • {tip}"),
                Style::default().fg(Color::White),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Q] Save & Quit  [T/R/M] Browse panels",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
