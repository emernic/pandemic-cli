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

use crate::state::{GameEvent, GameOutcome, GameState, Panel, UiState, ticks_to_days};
use crate::format_number;

/// Minimum terminal dimensions for playable layout.
pub const MIN_COLS: u16 = 80;
pub const MIN_ROWS: u16 = 24;

/// Returns true when the size warning overlay is being displayed.
pub fn is_size_warning_active(state: &GameState, cols: u16, rows: u16) -> bool {
    !state.ui.size_warning_dismissed && (cols < MIN_COLS || rows < MIN_ROWS)
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
///
/// Event suppression: some events are filtered out here if they would add
/// noise without actionable content (e.g., DiseaseMutated is suppressed
/// when no medicine has been affected by the mutation).
pub(crate) fn process_events(state: &mut GameState) {
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
    // Build log messages for each event, then pick the best for the status bar.
    // Each event produces a (priority, log_msg, notification_msg) triple.
    // notification_msg may include contextual action hints not shown in the log.
    // lower priority number = more important (shown in status bar over higher numbers).
    let day = ticks_to_days(state.tick as f64);
    let mut log_entries: Vec<String> = Vec::new();
    // Tracks the highest-priority notification for the top-right status area.
    // Lower priority number = more important.
    let mut best_notification: Option<(u8, String)> = None; // (priority, notification_msg)

    for event in &state.events {
        let (priority, msg, notification) = match event {
            GameEvent::RegionCollapsed { region_idx, personnel_lost } => {
                let region = state.regions.get(*region_idx);
                let region_name = region.map(|r| r.name.as_str()).unwrap_or("Unknown");
                let remaining = state.regions.iter().filter(|r| !r.collapsed).count();
                let spec_suffix = match region.and_then(|r| r.specialization) {
                    Some(spec) => format!(". {} lost", spec.label()),
                    None => String::new(),
                };
                let msg = format!("COLLAPSE: {} has fallen! {} regions remain{}", region_name, remaining, spec_suffix);
                let notification = if *personnel_lost > 0 {
                    format!("{}. {} personnel lost.", msg, personnel_lost)
                } else {
                    msg.clone()
                };
                (0u8, msg, notification)
            }
            GameEvent::RegionAbandoned { region_idx } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let remaining = state.regions.iter().filter(|r| !r.collapsed).count();
                let msg = format!("ABANDONED: {} withdrawn from operations. {} regions remain", region_name, remaining);
                (0u8, msg.clone(), msg)
            }
            GameEvent::CollapseSecondaryDeaths { region_idx, deaths } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let deaths_str = format_number(*deaths);
                let msg = format!("{}: ~{}/day dying from secondary causes", region_name, deaths_str);
                (3, msg.clone(), msg)
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
                let notification = format!("{}! Use [R] Research to identify it.", msg.trim_end_matches('.'));
                (1, msg, notification)
            }
            GameEvent::IntelBriefing { message } => {
                (3, message.clone(), message.clone())
            }
            GameEvent::IntelAnalysis { message, .. } => {
                (2, message.clone(), message.clone())
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
                (2, msg.clone(), msg)
            }
            GameEvent::HumanTrialAdverseEvent { disease_idx, deaths } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let msg = format!("ADVERSE EVENT: {} trial killed {:.0} patients", name, deaths);
                (3, msg.clone(), msg)
            }
            GameEvent::PathogenIdentified { disease_idx } => {
                let d = state.diseases.get(*disease_idx);
                let name = d.map(|d| d.name.clone()).unwrap_or_else(|| "?".to_string());
                let ptype = d.map(|d| d.pathogen_type.label()).unwrap_or("Unknown");
                let transmission = d.map(|d| d.transmission.label()).unwrap_or("Unknown");
                let is_prion = d.is_some_and(|d| !d.pathogen_type.is_treatable());
                let base = format!("IDENTIFIED: {} [{} / {} transmission]", name, ptype, transmission);
                if is_prion {
                    let msg = format!("{} — NO TREATMENT POSSIBLE, containment only", base);
                    (3, msg.clone(), msg)
                } else {
                    (1, base.clone(), base)
                }
            }
            GameEvent::MedicineDeveloped { medicine_idx } => {
                let med = state.medicines.get(*medicine_idx);
                let med_name = med.map(|m| m.name.as_str()).unwrap_or("Unknown");
                let contract_note = med.and_then(|m| m.manufacturer_corp_idx)
                    .and_then(|ci| state.corporations.get(ci))
                    .map(|corp| {
                        if corp.board_seat {
                            format!(" Mfg contract: {} (board satisfied)", corp.name)
                        } else {
                            format!(" Mfg contract: {}", corp.name)
                        }
                    })
                    .unwrap_or_default();
                let msg = format!("BREAKTHROUGH: {} developed.{} Ready for clinical trials.", med_name, contract_note);
                (2, msg.clone(), msg)
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
                let deploy_status = if state.medicines.get(*medicine_idx).map_or(false, |m| m.doses > 0.0) {
                    "auto-deploy on"
                } else {
                    "auto-deploy on, pending doses"
                };
                let msg = format!("TRIAL SUCCESS: {} effective against {} ({:.0}%), {}", med_name, disease_name, efficacy, deploy_status);
                (2, msg.clone(), msg)
            }
            GameEvent::TechUnlocked { tech } => {
                let msg = format!("TECH UNLOCKED: {} [{}]", tech.name(), tech.description());
                (3, msg.clone(), msg)
            }
            GameEvent::DecreeUnlocked { decree } => {
                let name = decree.display_name();
                let msg = format!("DECREE AVAILABLE: {}", name);
                let notification = format!("{}. Open Orders [O] to enact.", msg);
                (2, msg, notification)
            }
            GameEvent::PathogenSuppressed { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let msg = format!("Suppression complete: {} infectivity reduced 20%", name);
                (3, msg.clone(), msg)
            }
            GameEvent::PathogenAttenuated { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let msg = format!("Attenuation complete: {} lethality reduced 30%", name);
                (3, msg.clone(), msg)
            }
            GameEvent::PathogenInterdicted { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let msg = format!("Interdiction complete: {} cross-region transmission eliminated", name);
                (3, msg.clone(), msg)
            }
            GameEvent::InfrastructureStabilized { region_idx, system } => {
                let region = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("{} stabilized in {}", system.label(), region);
                (3, msg.clone(), msg)
            }
            GameEvent::PolicySuspended { region_idx, policy_name } => {
                let region = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("Funding crisis: suspended {} in {}", policy_name, region);
                (4, msg.clone(), msg)
            }
            GameEvent::FundingWarning => {
                let msg = "LOW FUNDS: Policies at risk of suspension".to_string();
                (5, msg.clone(), msg)
            }
            GameEvent::PersonnelAttrition { count } => {
                let msg = format!("{} personnel resigned, no funding", count);
                (6, msg.clone(), msg)
            }
            GameEvent::DiseaseMutated { disease_idx, infectivity_factor, lethality_factor, .. } => {
                // Suppress mutation events when the player has no medicine affected by this mutation.
                // The mutation still happened; we just don't generate noise when there's nothing to act on.
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
                    format!(" (within-region spread {:+.0}%, lethality {:+.0}%)", inf_pct, leth_pct)
                } else {
                    String::new()
                };
                let eff_pct = (worst_eff * 100.0).round() as u32;
                let msg = if worst_eff < 0.25 {
                    format!("CRITICAL: {} medicine only {}% effective{}. Re-trial now!", name, eff_pct, detail)
                } else if worst_eff < 0.50 {
                    format!("WARNING: {} medicine degraded to {}% efficacy{}. Re-trial recommended.", name, eff_pct, detail)
                } else {
                    format!("{} mutated{}. Efficacy {}%.", name, detail, eff_pct)
                };
                let priority = if worst_eff < 0.25 { 2 } else if worst_eff < 0.50 { 4 } else { 7 };
                (priority, msg.clone(), msg)
            }
            GameEvent::CrisisAutoResolved { message } => {
                let msg = format!("Auto-resolved: {}", message);
                (5, msg.clone(), msg)
            }
            GameEvent::ResistanceTransferred { from_disease_idx, to_disease_idx } => {
                let from_name = state.diseases.get(*from_disease_idx)
                    .map(|d| d.display_name(*from_disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let to_name = state.diseases.get(*to_disease_idx)
                    .map(|d| d.display_name(*to_disease_idx))
                    .unwrap_or_else(|| "?".to_string());
                let msg = format!("Gene transfer: {} → {}", from_name, to_name);
                (9, msg.clone(), msg)
            }
            GameEvent::DiseaseSpreadToRegion { region_idx, .. } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("Disease spreading to {}", region_name);
                let notification = if !state.policies.iter().any(|p| p.any_active()) {
                    format!("{}! Use [P] Policy to contain.", msg)
                } else {
                    msg.clone()
                };
                (10, msg, notification)
            }
            GameEvent::ArkProtocolActivated { region_idx } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("⚠ Emergency consolidation: all operations moved to {}", region_name);
                (1, msg.clone(), msg)
            }
            GameEvent::GovernorAction { description, .. } => {
                (4, description.clone(), description.clone())
            }
            GameEvent::GovernorDied { region_idx, name } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("{} of {} has died. Region is leaderless.", name, region_name);
                (1, msg.clone(), msg)
            }
            GameEvent::GovernorSucceeded { region_idx, name } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("New governor in {}: {}", region_name, name);
                (2, msg.clone(), msg)
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
                (9, msg.clone(), msg)
            }
            GameEvent::ShipmentDelivered { medicine_idx, region_idx, doses, adverse, efficiency, doses_wasted, people_treated, people_protected } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let dose_str = format_number(*doses);
                let outcome = if *people_treated > 0.0 {
                    format!(", {} treated", format_number(*people_treated))
                } else if *people_protected > 0.0 {
                    format!(", {} protected", format_number(*people_protected))
                } else {
                    String::new()
                };
                let waste_note = if *doses_wasted > 100.0 {
                    format!(", {} wasted (no surveillance)", format_number(*doses_wasted))
                } else {
                    String::new()
                };
                let msg = if *adverse {
                    format!("⚠ {med_name} delivered to {region_name}. ADVERSE REACTION — {dose_str} doses{outcome}")
                } else if *efficiency < 0.90 || *doses_wasted > 100.0 {
                    let eff_pct = (*efficiency * 100.0) as u32;
                    if *efficiency < 0.90 {
                        format!("{med_name} delivered to {region_name}{outcome} ({eff_pct}% infra efficiency){waste_note}")
                    } else {
                        format!("{med_name} delivered to {region_name}{outcome}{waste_note}")
                    }
                } else {
                    format!("{med_name} delivered to {region_name}{outcome}")
                };
                let priority = if *adverse { 3 } else if *efficiency < 0.90 || *doses_wasted > 100.0 { 6 } else { 9 };
                (priority, msg.clone(), msg)
            }
            GameEvent::InfrastructureBreakpoint { region_idx, system, threshold } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let severity = if *threshold <= 0.0 {
                    "FAILED"
                } else if *threshold <= 0.25 {
                    "CRITICAL"
                } else {
                    "STRESSED"
                };
                let msg = format!("⚠ {region_name}: {} {severity}", system.label());
                (2, msg.clone(), msg)
            }
            GameEvent::PolicyAutoActivated { policy_name, .. } => {
                let msg = format!("Standing order: {policy_name} auto-activated");
                (8, msg.clone(), msg)
            }
            GameEvent::NetworkDisruption { disrupted_region_idx, collapsed_region_idx } => {
                let disrupted = state.regions.get(*disrupted_region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let collapsed = state.regions.get(*collapsed_region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");
                let msg = format!("Network disruption: supply routes through {} severed. {} medicine deployment +50% for 10 days",
                    collapsed, disrupted);
                (2, msg.clone(), msg)
            }
            GameEvent::ResearchHandoff { message } => {
                (2, message.clone(), message.clone())
            }
            GameEvent::ContractOffered { name } => {
                let msg = format!("TERMS RECEIVED: {}. Respond via crisis popup", name);
                (5, msg.clone(), msg)
            }
            GameEvent::ContractWarning { member_name, reason } => {
                let msg = format!("NOTICE: {}: {}", member_name, reason);
                (2, msg.clone(), msg)
            }
            GameEvent::ContractRevoked { name, reason } => {
                let msg = format!("FUNDING CUT: {}: {}", name, reason);
                (2, msg.clone(), msg)
            }
            GameEvent::PatronBonus { member_name, description } => {
                let msg = format!("BONUS: {} — {}", member_name, description);
                (3, msg.clone(), msg)
            }
            GameEvent::CorporationBankrupt { corp_idx, region_idx } => {
                let corp_name = state.corporations.get(*corp_idx)
                    .map(|c| c.name.as_str()).unwrap_or("Unknown");
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("BANKRUPT: {} ({}) has failed", corp_name, region_name);
                (1, msg.clone(), msg)
            }
            GameEvent::CrisisTeamReturned { label, personnel } => {
                let msg = format!("{} returned ({} personnel freed)", label, personnel);
                (3, msg.clone(), msg)
            }
            GameEvent::EmergencySampleDelivered { medicine_idx, region_idx, cooperation_change, adverse } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("Unknown");
                let gov_name = state.regions.get(*region_idx)
                    .map(|r| r.governor.name.as_str()).unwrap_or("Unknown");
                let msg = if *adverse {
                    format!("Adverse reaction to {} samples. {} cooperation {:.0}", med_name, gov_name, cooperation_change)
                } else {
                    format!("Delivered {} samples to {}. Cooperation +{:.0}", med_name, gov_name, cooperation_change)
                };
                let priority = if *adverse { 1 } else { 2 };
                (priority, msg.clone(), msg)
            }
            GameEvent::PolicyAuthorized { policy } => {
                let name = policy.display_name();
                let msg = format!("Board authorized policy: {}", name);
                let notification = format!("{}. Open [P] Policy to deploy.", msg);
                (3, msg, notification)
            }
            GameEvent::ResearchAutoRestarted { kind } => {
                let msg = format!("Auto-restarted: {}", kind.display_label(&state.diseases, &state.medicines, &state.regions));
                (8, msg.clone(), msg)
            }
            GameEvent::AutoDeployBlocked { medicine_idx } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let msg = format!("{} auto-deploy halted — resistance too high", med_name);
                (3, msg.clone(), msg)
            }
            GameEvent::GameOver | GameEvent::CrisisStarted => continue,
        };

        log_entries.push(msg);

        // Track highest-priority notification for the status bar
        if best_notification.as_ref().is_none_or(|(p, _)| priority < *p) {
            best_notification = Some((priority, notification));
        }
    }

    // Append to persistent event log
    for entry in log_entries {
        state.event_log.push_back((day, entry));
    }
    while state.event_log.len() > EVENT_LOG_MAX {
        state.event_log.pop_front();
    }

    // Update the top-right event notification area with the highest-priority event's notification.
    if let Some((_, notification)) = best_notification {
        state.ui.event_notification = Some(notification);
    }
}

pub fn render(f: &mut Frame, state: &GameState) {
    let area = f.area();
    if is_size_warning_active(state, area.width, area.height) {
        render_size_warning(f, area);
        return;
    }

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
        let status_color = if region.abandoned { Color::Yellow } else if region.collapsed { Color::Red } else { Color::Green };
        let status = if region.abandoned { "ABDN" } else if region.collapsed { "FELL" } else { "held" };
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
