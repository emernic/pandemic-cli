//! Event consequence application.
//!
//! Converts transient `GameEvent`s (produced by `engine::tick()` and
//! `engine::execute_command()`) into durable state changes (event log,
//! notifications) and runtime UI resets (panel close on game-over,
//! crisis selection reset).
//!
//! This module is called from the coordinator layer (`lib.rs`) — never
//! from the engine or UI/render layer directly.

use crate::format_number;
use crate::state::{GameEvent, AppState, Panel, ticks_to_days};

const EVENT_LOG_MAX: usize = 50;

/// Apply event consequences to durable and runtime state.
///
/// This handles:
/// - `event_log` population and capping
/// - sticky `event_notification` updates
/// - game-over panel closing
/// - crisis popup selection and checkbox reset on `CrisisStarted`
/// - event prioritization and suppression rules
///
/// Events are passed explicitly from the caller (tick or command result).
pub(crate) fn process_events(state: &mut AppState, events: &[GameEvent]) {
    if events.is_empty() {
        return;
    }

    // UI responses to game events
    if events.iter().any(|e| matches!(e, GameEvent::GameOver)) {
        state.ui.open_panel = Panel::None;
    }
    if events.iter().any(|e| matches!(e, GameEvent::CrisisStarted)) {
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

    for event in events {
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
                let msg = format!("Suppression complete: {} within-region spread reduced 20%", name);
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
            GameEvent::VariantEmerged { disease_idx, parent_name } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| format!("Pathogen #{}", disease_idx + 1));
                let msg = format!("NEW VARIANT: {} emerged from {}", name, parent_name);
                (2, msg.clone(), msg)
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
            GameEvent::NuclearImpact { region_idx, killed } => {
                let region_name = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                let msg = format!("☢ {} annihilated. {:.1}M dead. Disease eradicated.",
                    region_name, killed / 1_000_000.0);
                (0, msg.clone(), msg)
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
                let msg = format!("Auto-restarted: {}", kind.display_label(&state.diseases, &state.medicines));
                (8, msg.clone(), msg)
            }
            GameEvent::DeployBlocked { medicine_idx } => {
                let med_name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str()).unwrap_or("?");
                let msg = format!("{} deployment halted — resistance too high", med_name);
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
