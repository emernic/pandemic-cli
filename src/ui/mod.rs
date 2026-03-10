pub mod home;
pub mod hotkey_bar;
pub mod medicines;
pub mod policy;
pub mod research;
pub mod resources;
pub mod scientists;
pub mod threats;
pub mod region_list;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameEvent, GameOutcome, GameState, Panel, ResearchTrack, ticks_to_days};
use crate::format_number;

const EVENT_LOG_MAX: usize = 50;

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

/// Convert game events from the most recent tick into status/log messages.
/// Called after each tick by the game loop / snapshot runner.
///
/// Note: this handles TICK-TIME event formatting. Command responses
/// (deploy, research, policy) are formatted in engine command handlers
/// and returned via CommandResult.message — see target-architecture.md.
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
    // Slow to 1x on critical events so the player has time to read the notification.
    // (We no longer pause for non-crisis events — the top-right notification handles visibility.)
    if state.events.iter().any(|e| matches!(e,
        GameEvent::RegionCollapsed { .. } | GameEvent::DiseaseDetected { .. }
        | GameEvent::ThreatEscalation { .. } | GameEvent::ThreatLevelChanged { .. }
        | GameEvent::PathogenIdentified { .. }
        | GameEvent::MedicineDeveloped { .. } | GameEvent::TrialCompleted { .. })
        || state.events.iter().any(|e| matches!(e,
            GameEvent::InfrastructureBreakpoint { threshold, .. } if *threshold <= 0.25))
    )
    {
        state.ui.speed_multiplier = 1;
    }

    // Build log messages for each event, then pick the best for the status bar.
    let day = ticks_to_days(state.tick as f64);
    let mut log_entries: Vec<String> = Vec::new();
    let mut status_msg: Option<(u8, String)> = None; // (priority, message) — lower = more important

    for event in &state.events {
        let (priority, msg) = match event {
            GameEvent::RegionCollapsed { region_idx } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let remaining = state.regions.iter().filter(|r| !r.collapsed).count();
                (0, format!("COLLAPSE: {} has fallen! {} regions remain", region_name, remaining))
            }
            GameEvent::DiseaseDetected { disease_idx, silent_days } => {
                let affected: Vec<&str> = state.regions.iter()
                    .filter(|r| r.disease_state(*disease_idx).is_some_and(|inf| inf.infected > 0.0))
                    .map(|r| r.name.as_str()).collect();
                let location = if affected.len() > 1 {
                    format!("{} regions", affected.len())
                } else {
                    affected.first().unwrap_or(&"unknown").to_string()
                };
                let msg = if *silent_days > 0.5 {
                    format!("NEW THREAT detected in {}. Spreading silently for {:.1} days.", location, silent_days)
                } else {
                    format!("NEW THREAT detected in {}", location)
                };
                (1, msg)
            }
            GameEvent::IntelBriefing { message } => {
                (3, message.clone())
            }
            GameEvent::ThreatEscalation { disease_idx, deaths, has_medicine } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                let deaths_str = format_number(*deaths);
                let msg = if *has_medicine {
                    format!("{name}: {deaths_str} dead. Deploy medicine!")
                } else {
                    format!("{name}: {deaths_str} dead. No medicine available.")
                };
                (2, msg)
            }
            GameEvent::HumanTrialAdverseEvent { disease_idx, deaths } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                (3, format!("ADVERSE EVENT: {} trial killed {:.0} patients", name, deaths))
            }
            GameEvent::PathogenIdentified { disease_idx } => {
                let d = state.diseases.get(*disease_idx);
                let name = d.map(|d| d.name.clone()).unwrap_or_else(|| "?".to_string());
                let ptype = d.map(|d| d.pathogen_type.label()).unwrap_or("Unknown");
                let transmission = d.map(|d| d.transmission.label()).unwrap_or("Unknown");
                (1, format!("IDENTIFIED: {} [{} / {} transmission]", name, ptype, transmission))
            }
            GameEvent::MedicineDeveloped { medicine_idx } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("Unknown");
                (2, format!("BREAKTHROUGH: {} developed. Ready for clinical trials.", med_name))
            }
            GameEvent::TrialCompleted { medicine_idx, disease_idx } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let disease_name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let efficacy = state.medicines.get(*medicine_idx)
                    .map(|m| m.effective_efficacy(*disease_idx, &state.diseases) * 100.0)
                    .unwrap_or(0.0);
                (2, format!("TRIAL SUCCESS: {} effective against {} ({:.0}%). Auto-deploy ON. Press [X] in Medicines to disable.", med_name, disease_name, efficacy))
            }
            GameEvent::TechUnlocked { tech } => {
                (3, format!("TECH UNLOCKED: {} [{}]", tech.name(), tech.description()))
            }
            GameEvent::PolicySuspended { region_idx, policy_name } => {
                let region = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                (4, format!("Funding crisis: suspended {} in {}", policy_name, region))
            }
            GameEvent::FundingWarning => {
                (5, "LOW FUNDS: Policies at risk of suspension".to_string())
            }
            GameEvent::PersonnelAttrition { count } => {
                (6, format!("{} personnel resigned, no funding", count))
            }
            GameEvent::DiseaseMutated { disease_idx, infectivity_factor, lethality_factor, .. } => {
                if !state.has_outdated_medicine(*disease_idx) {
                    continue;
                }
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| format!("Pathogen #{}", disease_idx + 1));
                let worst_eff = state.medicines.iter()
                    .filter(|m| m.target_diseases.contains(disease_idx)
                        && (m.tested_against.contains(disease_idx) || m.unlocked))
                    .map(|m| m.strain_efficacy(*disease_idx, &state.diseases))
                    .fold(1.0_f64, f64::min);
                let detail = if state.unlocked_techs.contains(&crate::state::BasicTech::RapidSequencing) {
                    let inf_pct = (infectivity_factor - 1.0) * 100.0;
                    let leth_pct = (lethality_factor - 1.0) * 100.0;
                    format!(" (spread {:+.0}%, lethality {:+.0}%)", inf_pct, leth_pct)
                } else {
                    String::new()
                };
                (7, format!("{} mutated{}. Efficacy {:.0}%.", name, detail, worst_eff * 100.0))
            }
            GameEvent::ResearchAutoStarted { track } => {
                let track_name = match track {
                    ResearchTrack::Field => "Field",
                    ResearchTrack::Applied => "Applied",
                    ResearchTrack::Basic => "Basic",
                };
                (8, format!("Auto-started {} research", track_name))
            }
            GameEvent::ContainmentAdaptation { disease_idx, level } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let pct = (level * 100.0).round() as u32;
                (3, format!("{} adapting to containment. Quarantine/travel ban {}% less effective.", name, pct))
            }
            GameEvent::CrisisAutoResolved => {
                // Don't log auto-resolves — they're noise
                continue;
            }
            GameEvent::ResistanceTransferred { from_disease_idx, to_disease_idx } => {
                let from_name = state.diseases.get(*from_disease_idx)
                    .map(|d| d.display_name(*from_disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let to_name = state.diseases.get(*to_disease_idx)
                    .map(|d| d.display_name(*to_disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                (9, format!("Gene transfer: {} → {}", from_name, to_name))
            }
            GameEvent::DiseaseSpreadToRegion { region_idx, .. } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                (10, format!("Disease spreading to {}", region_name))
            }
            GameEvent::ScientistBurnout { scientist_name } => {
                (6, format!("{} burned out, unavailable for 3 days", scientist_name))
            }
            GameEvent::ScientistInfected { scientist_name } => {
                (5, format!("{} contracted disease in the field, unavailable for 4 days", scientist_name))
            }
            GameEvent::ScientistBreakthrough { scientist_name } => {
                (3, format!("{} had a breakthrough. Research accelerated.", scientist_name))
            }
            GameEvent::ArkProtocolActivated { region_idx } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                (1, format!("⚠ ARK PROTOCOL: all resources consolidated in {}", region_name))
            }
            GameEvent::GovernorAction { description, .. } => {
                (4, description.clone())
            }
            GameEvent::ThreatLevelChanged { to, .. } => {
                (1, format!("DEFCON {}: Threat level {}", to.defcon(), to.label()))
            }
            GameEvent::MedicineShipped { medicine_idx, region_idx, doses } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let dose_str = format_number(*doses);
                let pop = state.regions.get(*region_idx)
                    .map(|r| r.population as f64).unwrap_or(1.0);
                let coverage = *doses / pop * 100.0;
                let msg = if coverage >= 50.0 {
                    format!("{dose_str} doses of {med_name} dispatched to {region_name} ({coverage:.0}% population coverage)")
                } else {
                    format!("{dose_str} doses of {med_name} en route to {region_name}")
                };
                (9, msg)
            }
            GameEvent::ShipmentBlocked { medicine_idx, region_idx } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                (4, format!("⚠ {} shipment blocked at {}. Travel ban in effect. Lift ban to deliver.", med_name, region_name))
            }
            GameEvent::ShipmentDelivered { medicine_idx, region_idx, doses, adverse } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let dose_str = format_number(*doses);
                if *adverse {
                    (3, format!("⚠ {dose_str} doses of {med_name} delivered to {region_name}. ADVERSE REACTION reported."))
                } else {
                    (9, format!("{med_name} delivered to {region_name}, {dose_str} doses administered"))
                }
            }
            GameEvent::InfrastructureBreakpoint { region_idx, system, threshold } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let system_label = match system.as_str() {
                    "healthcare" => "Healthcare",
                    "supply_lines" => "Supply Lines",
                    "civil_order" => "Civil Order",
                    _ => system.as_str(),
                };
                let severity = if *threshold <= 0.0 {
                    "FAILED"
                } else if *threshold <= 0.25 {
                    "CRITICAL"
                } else {
                    "STRESSED"
                };
                (2, format!("⚠ {region_name}: {system_label} {severity}"))
            }
            GameEvent::GameOver | GameEvent::CrisisStarted => continue,
        };

        log_entries.push(msg.clone());

        // Track highest-priority message for status bar
        if status_msg.as_ref().is_none_or(|(p, _)| priority < *p) {
            status_msg = Some((priority, msg));
        }
    }

    // Append to persistent event log
    for entry in log_entries {
        state.event_log.push_back((day, entry));
    }
    while state.event_log.len() > EVENT_LOG_MAX {
        state.event_log.pop_front();
    }

    // Update the top-right event notification area.
    if let Some((priority, msg)) = status_msg {
        let notification = match priority {
            0 => format!("{}. Personnel lost.", msg),
            1 if msg.starts_with("NEW THREAT") => {
                format!("{}! Use [R] Research to identify it.", msg)
            }
            10 if !state.policies.iter().any(|p| p.any_active()) => {
                format!("{}! Use [P] Policy to contain.", msg)
            }
            _ => msg,
        };
        state.ui.event_notification = Some(notification);
    }
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
            Panel::Scientists => scientists::render(f, split[1], state),
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

    let defeat_msg = if state.ark_protocol.is_some() {
        "  The Ark has fallen. Humanity's last refuge could not hold.".to_string()
    } else if state.mercy_rule {
        let collapsed = state.regions.iter().filter(|r| r.collapsed).count();
        if collapsed >= 4 {
            format!("  {collapsed} of {} regions lost. No viable research remains.", state.regions.len())
        } else {
            "  No funding, no research, no medicine. The pandemic runs its course.".to_string()
        }
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
