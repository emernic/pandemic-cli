pub mod board;
pub mod home;
pub mod hotkey_bar;
pub mod ledger;
pub mod medicines;
pub mod operations;
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

use crate::state::{GameOutcome, GameState, Panel, UiState, ticks_to_days};
use crate::format_number;

/// Minimum terminal dimensions for playable layout.
pub const MIN_COLS: u16 = 80;
pub const MIN_ROWS: u16 = 24;

/// Returns true when the size warning overlay is being displayed.
pub fn is_size_warning_active(state: &GameState, cols: u16, rows: u16) -> bool {
    !state.session.size_warning_dismissed && (cols < MIN_COLS || rows < MIN_ROWS)
}

/// Maximum selection index for the current panel and UI sub-state.
/// Dispatches to each panel module's `selection_max` so item-count logic
/// lives alongside the renderers instead of being centralised in state.rs.
pub fn panel_selection_max(ui: &UiState, state: &GameState) -> usize {
    match ui.open_panel {
        Panel::Threats => threats::selection_max(state),
        Panel::Medicines => match &ui.medicine_ui {
            Some(s) => medicines::selection_max(s, state),
            None => 0,
        },
        Panel::Research => match &ui.research_ui {
            Some(s) => research::selection_max(s, state),
            None => 0,
        },
        Panel::Policy => match &ui.policy_ui {
            Some(s) => policy::selection_max(s, state),
            None => 0,
        },
        Panel::Operations => match &ui.operations_ui {
            Some(s) => operations::selection_max(s, state),
            None => 0,
        },
        Panel::Board => board::selection_max(state),
        Panel::Ledger => match &ui.ledger_ui {
            Some(s) => ledger::selection_max(s, state),
            None => 0,
        },
        Panel::None | Panel::Help => 0,
    }
}

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

/// Render a sparkline from price history using Unicode block characters.
/// `width` controls how many data points (from the tail) are shown.
/// Returns (sparkline_string, trend_color) where trend_color is Green if the
/// price is up over the visible window, Red if down, DarkGray if flat.
pub fn sparkline(history: &[f64], width: usize) -> (String, Color) {
    if history.is_empty() {
        return (String::new(), Color::DarkGray);
    }
    // All 8 levels are visible — no space character, so the minimum value
    // still renders as ▁ rather than disappearing.
    let bars = ['\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    let min = history.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = history.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(0.01);
    let start = history.len().saturating_sub(width);
    let slice = &history[start..];
    let chart: String = slice.iter().map(|v| {
        let normalized = ((v - min) / range * 7.0).round() as usize;
        bars[normalized.min(7)]
    }).collect();
    // Color based on overall trend across the visible window.
    let first = slice.first().copied().unwrap_or(0.0);
    let last = slice.last().copied().unwrap_or(0.0);
    let color = if last > first + 0.01 {
        Color::Green
    } else if last < first - 0.01 {
        Color::Red
    } else {
        Color::DarkGray
    };
    (chart, color)
}

pub fn render(f: &mut Frame, state: &GameState) {
    let area = f.area();
    if is_size_warning_active(state, area.width, area.height) {
        render_size_warning(f, area);
        return;
    }

    let header_height = resources::height(state);
    let has_extra_line = state.session.status_message.is_some() || state.outcome != GameOutcome::Playing;
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
            Panel::Operations => operations::render(f, split[1], state),
            Panel::Board => board::render(f, split[1], state),
            Panel::Ledger => ledger::render(f, split[1], state),
            panel => render_placeholder_panel(f, split[1], panel),
        }
    }
}

fn render_size_warning(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "TERMINAL TOO SMALL",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from("Resize your terminal or press F11 for full screen."),
        Line::from(""),
        Line::from(format!(
            "Current: {}x{}  Minimum: {}x{}",
            area.width, area.height, MIN_COLS, MIN_ROWS,
        )),
        Line::from(""),
        Line::from(Span::styled(
            "[X] Dismiss",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let paragraph = Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("PANDEMIC DEFENSE"));
    f.render_widget(paragraph, area);
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
    let flash_on = (millis / 500) % 2 == 0;
    let warning_style = if flash_on {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Black)
    };
    let title_style = Style::default().fg(Color::Yellow);
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("⚠", warning_style),
        Span::raw("  "),
        Span::styled(crisis.title.clone(), title_style),
        Span::raw("  "),
        Span::styled("⚠", warning_style),
    ]));
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

    let labels = ["A", "B", "C", "D", "E", "F"];
    for (i, option) in crisis.options.iter().enumerate() {
        let label = labels.get(i).unwrap_or(&"?");
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
/// Respects explicit newlines in the input — each `\n` forces a line break.
fn textwrap(s: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in s.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
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
            Line::from("  [O] Orders panel"),
            Line::from("  [B] Board panel"),
            Line::from("  [Space] Pause/Resume"),
            Line::from("  [Z] Speed up (1x→2x→4x→6x, pause resets)"),
            Line::from("  [X] Auto-resolve crisis (toggle during event)"),
            Line::from("  [←/→] Cycle regions  [↑/↓] Panel items"),
            Line::from("  [Esc] Close panel"),
            Line::from("  [Q] Save & Quit"),
            Line::from(""),
            Line::from(Span::styled("Infrastructure:", Style::default().fg(Color::Yellow))),
            Line::from("  Each region has three infrastructure systems."),
            Line::from(""),
            Line::from(Span::styled("  Healthcare (HC)", Style::default().fg(Color::Cyan))),
            Line::from("  Degrades from infection load (overwhelmed hospitals)."),
            Line::from("  Below 50%: 2x lethality. Below 25%: 4x lethality."),
            Line::from("  Field hospitals and low infection allow recovery."),
            Line::from(""),
            Line::from(Span::styled("  Supply Lines (SL)", Style::default().fg(Color::Cyan))),
            Line::from("  Degrades from high death rates and travel bans."),
            Line::from("  Below 50%: 1.5x policy cost. Below 25%: 2x deploy time."),
            Line::from("  At 0%: no medicine deployment possible."),
            Line::from(""),
            Line::from(Span::styled("  Civil Order (CO)", Style::default().fg(Color::Cyan))),
            Line::from("  Degrades from deaths, restrictive policies, and low HC."),
            Line::from("  At 0%: +50% within-region spread (anarchy)."),
            Line::from(""),
            Line::from(Span::styled("  Delivery Efficiency", Style::default().fg(Color::Cyan))),
            Line::from("  When deploying medicine, effective doses = HC × SL."),
            Line::from("  Example: 40% HC and 60% SL = only 24% of doses effective."),
            Line::from("  Use Field Operations (Research) to restore systems."),
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

    let defeat_msg = if let Some(ark_idx) = state.ark_protocol {
        let region_name = state.regions.get(ark_idx)
            .map(|r| r.name.as_str())
            .unwrap_or("the last region");
        format!("  {} collapsed. No remaining operational sites.", region_name)
    } else {
        let collapsed = state.regions.iter().filter(|r| r.collapsed).count();
        format!("  All {collapsed} regions collapsed. Global health infrastructure has ceased to function.")
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
        Span::styled("  Immune:         ", stat_label),
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

    // Show collapse secondary deaths if any occurred
    let total_collapse_dead: f64 = state.regions.iter().map(|r| r.collapse_deaths).sum();
    if total_collapse_dead > 0.0 {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Secondary causes (starvation, violence)", stat_label),
        ]));
        lines.push(Line::from(vec![
            Span::styled("      Deaths: ", stat_label),
            Span::styled(
                format_number(total_collapse_dead),
                Style::default().fg(Color::Red),
            ),
            Span::styled(
                format!("  ({:.1}% of total)", if total_dead > 0.0 { total_collapse_dead / total_dead * 100.0 } else { 0.0 }),
                stat_label,
            ),
        ]));
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

    // Biological footprint — what the player actually did
    let total_deployments: u32 = state.medicines.iter().map(|m| m.deployed_count).sum();
    let interventions = state.pathogens_suppressed + state.pathogens_attenuated + state.pathogens_interdicted;
    if total_deployments > 0 || interventions > 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── Mission Report ──",
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(vec![
            Span::styled("  Deployments:    ", stat_label),
            Span::styled(format!("{total_deployments}"), stat_value),
            Span::styled(
                format!("  ({} total doses)", format_number(state.total_doses_deployed)),
                stat_label,
            ),
        ]));

        let coverage_pct = if initial_pop > 0.0 {
            state.total_doses_deployed / initial_pop * 100.0
        } else { 0.0 };
        lines.push(Line::from(vec![
            Span::styled("  Coverage:       ", stat_label),
            Span::styled(
                format!("{coverage_pct:.1}% of global population"),
                if coverage_pct >= 100.0 { Style::default().fg(Color::Yellow) } else { stat_value },
            ),
        ]));

        if state.pathogens_suppressed > 0 {
            lines.push(Line::from(vec![
                Span::styled("  Suppressed:     ", stat_label),
                Span::styled(format!("{} pathogens", state.pathogens_suppressed), stat_value),
            ]));
        }
        if state.pathogens_attenuated > 0 {
            lines.push(Line::from(vec![
                Span::styled("  Attenuated:     ", stat_label),
                Span::styled(format!("{} pathogens", state.pathogens_attenuated), stat_value),
            ]));
        }
        if state.pathogens_interdicted > 0 {
            lines.push(Line::from(vec![
                Span::styled("  Interdicted:    ", stat_label),
                Span::styled(format!("{} pathogens", state.pathogens_interdicted), stat_value),
            ]));
        }
    }

    // Strategic tips
    let tips = state.defeat_tips();
    if !tips.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── Debrief ──",
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

/// Compute scroll offset to keep the full selected item visible in a panel.
///
/// Items are separated by blank lines (width == 0). We scan forward from the
/// selected item's first line to find where the item ends, then ensure the
/// scroll offset keeps the entire item in the viewport.
pub fn scroll_offset_for_selection(
    lines: &[Line],
    selected_line: Option<usize>,
    inner_height: u16,
) -> u16 {
    let Some(start) = selected_line else {
        return 0;
    };

    // Find the last content line of the selected item by scanning forward
    // for the next blank-line separator (or end of content).
    let end = if start + 1 < lines.len() {
        lines[start + 1..]
            .iter()
            .position(|line| line.width() == 0)
            .map(|off| start + off) // last content line is one before the blank
            .unwrap_or(lines.len().saturating_sub(1))
    } else {
        start
    };

    let end_u16 = end as u16;
    if end_u16 < inner_height {
        // Entire item fits without scrolling
        0
    } else {
        // Position the item's last line at ~2/3 down the viewport
        let offset = end_u16.saturating_sub(inner_height * 2 / 3);
        // Don't scroll past the item's first line
        offset.min(start as u16)
    }
}
