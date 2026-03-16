use rand::Rng;
use crate::state::{
    AUTO_MANUFACTURE_THRESHOLD, BasicTech, GameEvent, GameOutcome, WorldState,
    ResearchKind, ResearchProject, KNOWLEDGE_FULL, KNOWLEDGE_NAME,
    TRAIN_PERSONNEL_BATCH,
    LAB_LEVEL_1_COST, LAB_LEVEL_2_COST,
    TrialRigor, Medicine, MechanismOfAction, TherapyType,
};

/// Start a research project. Pure game logic — does NOT modify UI state.
/// Takes a `ResearchKind` directly rather than an index, so the caller doesn't need
/// to worry about index stability across dynamic list rebuilds.
///
/// Returns (success, message).
pub(super) fn start_research(state: &mut WorldState, kind: &ResearchKind, double_personnel: bool) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }

    // Verify this project is actually still available
    let projects = state.all_available_projects();
    if !projects.contains(kind) {
        return (false, Some("Project is no longer available".into()));
    }

    let (base_personnel, duration, funding_cost) = state.effective_costs(kind);
    let personnel = if double_personnel { base_personnel * 2 } else { base_personnel };

    if state.resources.funding < funding_cost {
        return (false, Some(super::medicine::insufficient_funds_message(funding_cost, state.resources.funding)));
    }
    if state.personnel_available() < personnel {
        return (false, Some(format!(
            "Need {} personnel, only {} available",
            personnel, state.personnel_available(),
        )));
    }
    state.resources.funding -= funding_cost;
    // Clinical trial speed modifier: Human Trials decree
    let effective_duration = if matches!(kind, ResearchKind::ClinicalTrial { .. }) {
        let mut d = duration;
        if state.enacted_decrees.authorize_human_trials {
            d *= crate::state::HUMAN_TRIALS_SPEED;
        }
        d
    } else {
        duration
    };
    let project = ResearchProject {
        kind: kind.clone(),
        progress: 0.0,
        required_ticks: effective_duration,
        personnel_assigned: personnel,
    };

    state.active_research.push(project);
    (true, None)
}


/// Start a clinical trial from a screening hit.
/// Removes the hit from the pool, creates a new Medicine with randomized stats,
/// and starts a ClinicalTrial research project.
pub(super) fn start_trial(state: &mut WorldState, hit_index: usize, rigor: TrialRigor) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    if hit_index >= state.screening_hits.len() {
        return (false, Some("Invalid hit index".into()));
    }

    let (personnel, duration, funding_cost) = rigor.costs();
    if state.resources.funding < funding_cost {
        return (false, Some(super::medicine::insufficient_funds_message(funding_cost, state.resources.funding)));
    }
    if state.personnel_available() < personnel {
        return (false, Some(format!(
            "Need {} personnel, only {} available",
            personnel, state.personnel_available(),
        )));
    }

    // Remove hit from pool
    let hit = state.screening_hits.remove(hit_index);

    // Determine therapy type from disease's pathogen type
    let pathogen_type = state.diseases.get(hit.disease_idx)
        .map(|d| d.pathogen_type)
        .unwrap_or(crate::state::PathogenType::RnaVirus);
    let therapy_type = pathogen_type.matched_therapy().unwrap_or(TherapyType::BroadSpectrum);

    // Pick a random mechanism of action for this pathogen type
    let mechs: &[MechanismOfAction] = match pathogen_type {
        crate::state::PathogenType::Bacterium => MechanismOfAction::bacterial_mechanisms(),
        crate::state::PathogenType::Fungus => MechanismOfAction::fungal_mechanisms(),
        crate::state::PathogenType::RnaVirus | crate::state::PathogenType::DnaVirus => MechanismOfAction::viral_mechanisms(),
        crate::state::PathogenType::Prion => &[],
    };
    let mechanism = if mechs.is_empty() {
        None
    } else {
        let idx = (state.rng_spread.r#gen::<u64>() as usize) % mechs.len();
        Some(mechs[idx])
    };

    // Randomize stats from the hit's Kd:
    // Lower Kd = better binding = higher efficacy
    // Kd range is roughly 0.1-500 nM. We map to efficacy 0.3-0.95.
    let kd_factor = (hit.kd_nm.ln() - 0.1_f64.ln()) / (500.0_f64.ln() - 0.1_f64.ln());
    let base_efficacy = 0.95 - (kd_factor.clamp(0.0, 1.0) * 0.65);
    // Add some randomness (±15%)
    let noise: f64 = (state.rng_spread.r#gen::<f64>() - 0.5) * 0.30;
    let efficacy = (base_efficacy + noise).clamp(0.15, 0.98);

    // Side effect rate: 0-25%, inversely correlated with rigor (charade hides bad drugs)
    let side_effect_rate = state.rng_spread.r#gen::<f64>() * 0.25;

    // Resistance rate: how quickly resistance builds (0.0-1.0 scale, lower is better)
    let resistance_rate: f64 = if let Some(mech) = mechanism {
        // Fast/cheap mechanisms build resistance faster
        let base: f64 = mech.dev_cost_multiplier();
        let mapped: f64 = (1.5 - base) / 0.7; // 1.0 for cheapest, 0.0 for most expensive
        (mapped * 0.6 + state.rng_spread.r#gen::<f64>() * 0.4).clamp(0.0, 1.0)
    } else {
        0.5
    };

    let max_doses = mechanism.map_or(500_000.0, |m| m.base_doses());

    // Create the medicine — uses compound_id as temporary name until trial completes
    let medicine = Medicine {
        name: hit.compound_id.clone(),
        therapy_type,
        mechanism,
        target_diseases: vec![hit.disease_idx],
        doses: 0.0,
        max_doses,
        unlocked: false,
        tested_against: vec![],
        deployed_count: 0,
        total_treated: 0.0,
        manufacturer_corp_idx: None,
        trial_efficacy: Some(efficacy),
        side_effect_rate,
        resistance_rate,
        trial_rigor: Some(rigor),
        reported_efficacy: None,
        reported_side_effects: None,
        reported_resistance: None,
    };

    let medicine_idx = state.medicines.len();
    state.medicines.push(medicine);

    // Deduct costs and start the trial project
    state.resources.funding -= funding_cost;
    let mut effective_duration = duration;
    // MetagenomicSurveillance: clinical trials 25% faster
    if state.unlocked_techs.contains(&BasicTech::MetagenomicSurveillance) {
        effective_duration *= 0.75;
    }
    // Human Trials decree: halve trial duration
    if state.enacted_decrees.authorize_human_trials {
        effective_duration *= crate::state::HUMAN_TRIALS_SPEED;
    }
    state.active_research.push(ResearchProject {
        kind: ResearchKind::ClinicalTrial {
            medicine_idx,
            disease_idx: hit.disease_idx,
            rigor,
        },
        progress: 0.0,
        required_ticks: effective_duration,
        personnel_assigned: personnel,
    });

    (true, Some(format!("Trial started: {} ({})", hit.compound_id, rigor.label())))
}

/// Discard a screening hit the player doesn't want to trial.
pub(super) fn discard_hit(state: &mut WorldState, hit_index: usize) -> (bool, Option<String>) {
    if hit_index >= state.screening_hits.len() {
        return (false, Some("Invalid hit index".into()));
    }
    let hit = state.screening_hits.remove(hit_index);
    (true, Some(format!("Discarded {}", hit.compound_id)))
}

/// Generate reported stats for a medicine based on trial rigor.
///
/// Full: exact percentages. Abbreviated: offset ranges (midpoint ≠ real value).
/// Compassionate: very wide offset ranges, nearly useless. Charade: no data at all.
///
/// The offset is critical: without it, the player can just take the midpoint of the
/// range to recover the real value, making lower-rigor trials free information.
fn generate_reported_stats(medicine: &mut Medicine, rigor: TrialRigor, rng: &mut impl rand::Rng) {
    let real_eff = medicine.trial_efficacy.unwrap_or(0.5);
    let real_se = medicine.side_effect_rate;
    let real_res = medicine.resistance_rate;

    /// Build an offset percentage range. The range always contains the real value,
    /// but the center is shifted randomly so the midpoint is NOT the real value.
    /// `half_width`: half the total range width. `max_offset`: how far the center
    /// can shift from the real value (the range is then re-anchored to still contain it).
    fn offset_pct_range(real: f64, half_width: f64, max_offset: f64, rng: &mut impl rand::Rng) -> String {
        // Shift center away from real value by up to max_offset in either direction
        let offset: f64 = (rng.r#gen::<f64>() * 2.0 - 1.0) * max_offset;
        let center = (real + offset).clamp(half_width, 1.0 - half_width);
        // Ensure the range still contains the real value by expanding if needed
        let lo = center - half_width;
        let hi = center + half_width;
        // The range must contain real, so clamp lo/hi to guarantee inclusion
        let lo = lo.min(real);
        let hi = hi.max(real);
        let lo_pct = (lo * 100.0).max(0.0);
        let hi_pct = (hi * 100.0).min(100.0);
        format!("{:.0}-{:.0}%", lo_pct, hi_pct)
    }

    match rigor {
        TrialRigor::Full => {
            medicine.reported_efficacy = Some(format!("{:.0}%", real_eff * 100.0));
            medicine.reported_side_effects = Some(format!("{:.0}%", real_se * 100.0));
            medicine.reported_resistance = Some(format!("{:.0}%", real_res * 100.0));
        }
        TrialRigor::Abbreviated => {
            // ±15% range, offset up to ±10% — useful but imprecise
            medicine.reported_efficacy = Some(offset_pct_range(real_eff, 0.15, 0.10, rng));
            medicine.reported_side_effects = Some(offset_pct_range(real_se, 0.15, 0.10, rng));
            medicine.reported_resistance = Some(offset_pct_range(real_res, 0.15, 0.10, rng));
        }
        TrialRigor::Compassionate => {
            // ±35% range, offset up to ±25% — nearly useless, very wide
            medicine.reported_efficacy = Some(offset_pct_range(real_eff, 0.35, 0.25, rng));
            medicine.reported_side_effects = Some(offset_pct_range(real_se, 0.35, 0.25, rng));
            medicine.reported_resistance = Some(offset_pct_range(real_res, 0.35, 0.25, rng));
        }
        TrialRigor::Charade => {
            medicine.reported_efficacy = Some("???".into());
            medicine.reported_side_effects = Some("???".into());
            medicine.reported_resistance = Some("???".into());
        }
    }
}

/// Generate a medicine name from mechanism of action.
///
/// Builds pharmaceutical-sounding names from random syllable components plus
/// a mechanism-based suffix (following real INN stem conventions).
fn generate_medicine_name(mechanism: Option<MechanismOfAction>, rng: &mut impl rand::Rng) -> String {
    // INN-style suffixes by mechanism (these are real pharmaceutical stems)
    let suffixes: &[&str] = match mechanism {
        Some(MechanismOfAction::CellWallInhibitor) => &["cillin", "penem", "cef"],
        Some(MechanismOfAction::RibosomeInhibitor) => &["mycin", "cycline", "thrin"],
        Some(MechanismOfAction::DnaGyraseInhibitor) => &["floxacin", "oxacin"],
        Some(MechanismOfAction::MetabolicInhibitor) => &["trimol", "zolid", "prim"],
        Some(MechanismOfAction::PolymeraseInhibitor) => &["vir", "buvir", "asvir"],
        Some(MechanismOfAction::ProteaseInhibitor) => &["navir", "previr", "gravir"],
        Some(MechanismOfAction::EntryInhibitor) => &["mab", "viroc", "lukast"],
        Some(MechanismOfAction::ErgosterolInhibitor) => &["azole", "conazole"],
        Some(MechanismOfAction::MembraneDisruptor) => &["fungin", "micin"],
        Some(MechanismOfAction::GlucanSynthaseInhibitor) => &["candin", "fundin"],
        None => &["plex", "mide", "drex"],
    };

    // Syllable components for building pharmaceutical-sounding prefixes.
    // These are modeled on common patterns in real drug names.
    const ONSETS: &[&str] = &[
        "ab", "al", "am", "ar", "at", "bal", "bel", "bir", "bor", "bri",
        "car", "cel", "cef", "cip", "cor", "dal", "dar", "del", "dex", "dol",
        "ef", "el", "em", "er", "et", "fal", "fen", "fil", "flu", "for",
        "gal", "gem", "gil", "glu", "gor", "hal", "hep", "hex", "ib", "im",
        "in", "ir", "kan", "kel", "kor", "lan", "lem", "lin", "lor", "lum",
        "mal", "mel", "met", "mil", "mol", "nal", "neb", "nif", "nor", "nul",
        "ol", "or", "ox", "pal", "par", "pen", "pir", "pol", "pra", "ral",
        "rem", "rib", "rof", "rul", "sal", "sel", "sim", "sol", "sul", "tal",
        "tel", "ten", "til", "tor", "tol", "val", "vel", "vin", "vol", "vor",
        "xal", "xel", "xim", "zan", "zel", "zil", "zor",
    ];

    const MIDS: &[&str] = &[
        "a", "e", "i", "o", "u",
        "ab", "ac", "ad", "af", "ag", "ak", "an", "ap", "as", "at",
        "eb", "ec", "ed", "ef", "el", "en", "ep", "es", "et",
        "ib", "ic", "id", "if", "ig", "il", "im", "in", "ip", "is", "it",
        "ob", "oc", "od", "of", "ol", "on", "op", "os", "ot",
        "ub", "uc", "ud", "uf", "ul", "un", "up", "us", "ut",
    ];

    let suffix = suffixes[rng.r#gen::<usize>() % suffixes.len()];

    // Build prefix: 1-2 syllable components
    let onset = ONSETS[rng.r#gen::<usize>() % ONSETS.len()];
    let use_mid = rng.r#gen::<bool>();
    let mut prefix = String::from(onset);
    if use_mid {
        prefix.push_str(MIDS[rng.r#gen::<usize>() % MIDS.len()]);
    }

    // Capitalize first letter, append suffix
    let mut name = String::new();
    let mut chars = prefix.chars();
    if let Some(first) = chars.next() {
        name.extend(first.to_uppercase());
    }
    name.extend(chars);
    name.push_str(suffix);
    name
}

/// Advance research projects by one tick and handle completions.
/// Progress scales with diminishing returns: 2x personnel = 1.5x speed (peak),
/// beyond 2x personnel = negative returns (too many cooks).
/// Returns the number of research completions that should trigger board notifications
/// (ClinicalTrial and BasicResearch completions boost Technocrat satisfaction).
pub(super) fn tick_research(state: &mut WorldState, rng: &mut impl rand::Rng, events: &mut Vec<GameEvent>) -> u32 {
    // Proactively auto-repeat on idle categories
    try_auto_repeat(state);
    // Auto-start queued techs when prerequisites and resources become available
    try_queued_starts(state);

    let mut board_notify_count: u32 = 0;

    let lab_mult = state.lab_speed_multiplier();
    // Biotech sector bonus: best regional bonus boosts research speed
    let biotech_bonus = (0..state.regions.len())
        .map(|r| state.sector_bonus(r, crate::state::CorporationSector::Biotech))
        .fold(0.0_f64, f64::max);
    let biotech_mult = 1.0 + crate::state::CorporationSector::Biotech.max_bonus_pct() / 100.0 * biotech_bonus;

    // Advance all research projects
    for project in &mut state.active_research {
        let speed = project.speed(&state.medicines);
        project.progress += speed * lab_mult * biotech_mult;
    }

    // Collect completed projects (drain_filter pattern via retain)
    let mut completed: Vec<ResearchProject> = Vec::new();
    state.active_research.retain(|p| {
        if p.is_complete() {
            completed.push(p.clone());
            false
        } else {
            true
        }
    });
    // Track which categories had completions for post-processing
    let had_field_completion = completed.iter().any(|p| p.kind.is_field_work());

    for project in &completed {
        match &project.kind {
            ResearchKind::IdentifyThreat { disease_idx } => {
                let d_idx = *disease_idx;
                let was_unknown = state.diseases.get(d_idx)
                    .is_some_and(|d| d.knowledge < KNOWLEDGE_NAME);
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.knowledge = (disease.knowledge + 0.50).min(KNOWLEDGE_FULL);
                }
                if was_unknown && state.diseases.get(d_idx)
                    .is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME)
                {
                    events.push(GameEvent::PathogenIdentified { disease_idx: d_idx });
                }
            }
            ResearchKind::ClinicalTrial { medicine_idx, disease_idx, rigor } => {
                let m_idx = *medicine_idx;
                let d_idx = *disease_idx;
                let rigor = *rigor;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.unlocked = true;
                    if !medicine.tested_against.contains(&d_idx) {
                        medicine.tested_against.push(d_idx);
                    }
                    if !medicine.target_diseases.contains(&d_idx) {
                        medicine.target_diseases.push(d_idx);
                    }
                    // Generate reported stats based on rigor level
                    generate_reported_stats(medicine, rigor, rng);
                    // Give the medicine a random name on completion
                    medicine.name = generate_medicine_name(medicine.mechanism, rng);
                }
                events.push(GameEvent::TrialCompleted {
                    medicine_idx: m_idx,
                    disease_idx: d_idx,
                });
                board_notify_count += 1;
                while state.deploy_enabled.len() <= m_idx {
                    state.deploy_enabled.push(false);
                }
                state.deploy_enabled[m_idx] = true;
                if state.enacted_decrees.authorize_human_trials {
                    let roll: f64 = rng.r#gen();
                    if roll < crate::state::HUMAN_TRIALS_ADVERSE_CHANCE {
                        let kill_frac = crate::state::HUMAN_TRIALS_KILL_FRACTION;
                        let mut total_killed = 0.0;
                        for region in &mut state.regions {
                            if let Some(inf) = region.infections.iter_mut()
                                .find(|i| i.disease_idx == d_idx)
                            {
                                let killed = inf.infected * kill_frac;
                                inf.infected -= killed;
                                inf.dead += killed;
                                region.dead += killed;
                                total_killed += killed;
                            }
                        }
                        if total_killed > 0.0 {
                            events.push(GameEvent::HumanTrialAdverseEvent {
                                disease_idx: d_idx,
                                deaths: total_killed,
                            });
                        }
                    }
                }

                // Boost manufacturer corporation if applicable
                if let Some(corp_idx) = state.medicines.get(m_idx).and_then(|m| m.manufacturer_corp_idx) {
                    if let Some(corp) = state.corporations.get_mut(corp_idx) {
                        if corp.board_seat && !corp.bankrupt {
                            let boost = corp.max_reserves * 0.25;
                            corp.reserves = (corp.reserves + boost).min(corp.max_reserves);
                        }
                    }
                }
            }
            ResearchKind::GenomicSequencing { disease_idx } => {
                let d_idx = *disease_idx;
                if let Some(disease) = state.diseases.get_mut(d_idx) {
                    disease.sequencing_count += 1;
                }
            }
            ResearchKind::ManufactureDoses { medicine_idx } => {
                let m_idx = *medicine_idx;
                if let Some(medicine) = state.medicines.get_mut(m_idx) {
                    medicine.doses = medicine.max_doses;
                }
            }
            ResearchKind::TrainPersonnel => {
                state.resources.personnel += TRAIN_PERSONNEL_BATCH;
            }
            ResearchKind::BasicResearch { tech } => {
                let tech = *tech;
                if !state.unlocked_techs.contains(&tech) {
                    state.unlocked_techs.push(tech);
                    events.push(GameEvent::TechUnlocked { tech });
                    board_notify_count += 1;
                }
            }
        }
    }

    // Notify player if field completions enabled screening
    if had_field_completion {
        // After identification completes, prompt player to start screening
        for project in &completed {
            if let ResearchKind::IdentifyThreat { disease_idx } = &project.kind {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "pathogen".to_string());
                events.push(GameEvent::ResearchHandoff {
                    message: format!("{} identified — start screening in Lab [L]", name),
                });
            }
        }
    }

    // Auto-repeat completed repeatable projects
    for project in &completed {
        if state.auto_repeat_research.contains(&project.kind) {
            // Manufacturing only auto-repeats when doses drop to threshold
            if let ResearchKind::ManufactureDoses { medicine_idx } = &project.kind {
                let dose_frac = state.medicines.get(*medicine_idx)
                    .map(|m| if m.max_doses > 0.0 { m.doses / m.max_doses } else { 1.0 })
                    .unwrap_or(1.0);
                if dose_frac > AUTO_MANUFACTURE_THRESHOLD {
                    continue;
                }
            }
            let (_ok, _) = start_research(state, &project.kind, false);
        }
    }

    board_notify_count
}

/// Try to auto-repeat any repeatable research that has auto-repeat enabled.
/// Called at the start of each tick.
fn try_auto_repeat(state: &mut WorldState) {
    let kinds_to_repeat: Vec<ResearchKind> = state.auto_repeat_research.clone();
    for kind in &kinds_to_repeat {
        // Manufacturing only auto-repeats when doses drop to threshold
        if let ResearchKind::ManufactureDoses { medicine_idx } = kind {
            let dose_frac = state.medicines.get(*medicine_idx)
                .map(|m| if m.max_doses > 0.0 { m.doses / m.max_doses } else { 1.0 })
                .unwrap_or(1.0);
            if dose_frac > AUTO_MANUFACTURE_THRESHOLD {
                continue;
            }
        }
        let (_, _, cost) = state.effective_costs(kind);
        if state.resources.funding < cost {
            continue;
        }
        let (_ok, _) = start_research(state, kind, false);
    }
}

/// Try to auto-start queued techs whose prerequisites and resources are now available.
/// Removes techs from the queue once started (or if already unlocked/researching).
fn try_queued_starts(state: &mut WorldState) {
    let queued: Vec<BasicTech> = state.queued_techs.clone();
    for tech in &queued {
        // Already unlocked or researching — silently remove from queue
        if state.unlocked_techs.contains(tech) {
            state.queued_techs.retain(|t| t != tech);
            continue;
        }
        let already_researching = state.active_research.iter().any(|r| {
            matches!(r.kind, ResearchKind::BasicResearch { tech: t } if t == *tech)
        });
        if already_researching {
            state.queued_techs.retain(|t| t != tech);
            continue;
        }

        // Check prerequisites
        if !tech.prerequisites_met(state) {
            continue;
        }

        // Try to start
        let target_kind = ResearchKind::BasicResearch { tech: *tech };
        let (ok, _) = start_research(state, &target_kind, false);
        if ok {
            state.queued_techs.retain(|t| t != tech);
        }
    }
}

/// Upgrade the global research lab (level 0→1 or 1→2). One-time funding cost.
/// Returns (success, message).
pub(super) fn upgrade_lab(state: &mut WorldState) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    let (cost, next_name) = match state.lab_level {
        0 => (LAB_LEVEL_1_COST, "Enhanced Sequencing Lab"),
        1 => (LAB_LEVEL_2_COST, "Advanced Genomics Center"),
        _ => return (false, Some("Research lab is already at maximum level.".into())),
    };
    if state.resources.funding < cost {
        return (false, Some(format!("Not enough funding (need ¥{:.0})", cost)));
    }
    state.resources.funding -= cost;
    state.lab_level += 1;
    (true, Some(format!("Lab upgraded to {}. Research speed +{}%.", next_name,
        if state.lab_level == 1 { 30 } else { 60 })))
}

#[cfg(test)]
mod tests {
    use crate::action::Action;
    use crate::apply_action;
    use crate::engine::tick;
    use crate::state::{
        GameOutcome, AppState, ResearchFlatItem, ResearchKind, ResearchProject,
    };

    /// Helper: open Lab panel, navigate to first available item matching `kind_pred`, and confirm through.
    /// For BasicResearch, use `start_basic_research` instead (those are in the Research panel now).
    fn start_research_matching(state: &AppState, kind_pred: impl Fn(&ResearchKind) -> bool) -> AppState {
        use crate::state::{LabTab, InfraItem};
        // Ensure panel is closed first, then open fresh
        let mut s = if state.ui.open_panel == crate::state::Panel::Lab {
            apply_action(state, &Action::ClosePanel)
        } else {
            state.clone()
        };
        s = apply_action(&s, &Action::OpenLab);
        let available = s.all_available_projects();

        // Check Infra tab for TrainPersonnel (it uses InfraItem, not ResearchFlatItem)
        if kind_pred(&ResearchKind::TrainPersonnel) {
            let infra_items = s.infra_tab_items();
            if let Some(idx) = infra_items.iter().position(|item| matches!(item, InfraItem::TrainPersonnel)) {
                s.ui.lab_ui = Some(crate::state::LabUiState::Browse { tab: LabTab::Infra });
                s.ui.panel_selection = idx;
                s = apply_action(&s, &Action::Confirm); // ConfirmProject
                s = apply_action(&s, &Action::Confirm); // Start
                return s;
            }
        }

        // Find which tab contains the matching item and its index within that tab
        let mut found = None;
        for tab in LabTab::ALL {
            let items = s.lab_tab_items(tab);
            if let Some(idx) = items.iter().position(|item| {
                if let ResearchFlatItem::Available(proj_idx) = item {
                    available.get(*proj_idx).map_or(false, &kind_pred)
                } else {
                    false
                }
            }) {
                found = Some((tab, idx));
                break;
            }
        }
        let (tab, idx) = found.expect("expected matching research item in some tab");
        // Navigate to the correct tab
        s.ui.lab_ui = Some(crate::state::LabUiState::Browse { tab });
        s.ui.panel_selection = idx;
        s = apply_action(&s, &Action::Confirm); // ConfirmProject
        s = apply_action(&s, &Action::Confirm); // Start
        s
    }

    /// Helper: open Research (tech tree) panel, find first available BasicResearch, and confirm.
    fn start_basic_research(state: &AppState) -> AppState {
        let mut s = apply_action(state, &Action::ClosePanel);
        s = apply_action(&s, &Action::OpenResearch);
        let techs = crate::ui::tech_tree::layout_techs();
        let idx = techs.iter().position(|tech| {
            !s.unlocked_techs.contains(tech)
                && tech.prerequisites_met(&s.world)
                && !s.active_research.iter().any(|r| {
                    matches!(r.kind, ResearchKind::BasicResearch { tech: t } if t == *tech)
                })
        }).expect("expected available BasicResearch in tech tree");
        s.ui.panel_selection = idx;
        s = apply_action(&s, &Action::Confirm);
        s
    }

    #[test]
    fn research_identify_increases_knowledge() {
        use crate::state::{LabTab, LabUiState};
        let mut state = AppState::new_default(42);
        // Start identify project on disease 0 (first item in Sequencing tab)
        state = apply_action(&state, &Action::OpenLab);
        state.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Sequencing });
        state.ui.panel_selection = 0;
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Start
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (160 ticks at 1x speed)
        for _ in 0..160 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    // research_develop_medicine_unlocks — removed: DevelopMedicine no longer exists.
    // Medicines are created from screening hits via clinical trials.

    #[test]
    fn research_clinical_trial_marks_tested() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against.clear(); // Clear so we can test the trial adds it

        assert!(state.medicines[0].tested_against.is_empty());

        // Directly add a clinical trial research project (trials are now started via wizard, not flat list)
        state.active_research.push(ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0, rigor: crate::state::TrialRigor::Full },
            progress: 0.0,
            required_ticks: 120.0,
            personnel_assigned: 4,
        });

        assert!(!state.active_research.is_empty(), "clinical trial should be active");

        for _ in 0..160 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ClinicalTrial { .. })).collect::<Vec<_>>().is_empty(),
            "clinical trial should have completed");
        assert!(state.medicines[0].tested_against.contains(&0));
    }

    #[test]
    fn research_insufficient_personnel_blocks_start() {
        let mut state = AppState::new_default(42);
        state.resources.personnel = 0; // No personnel

        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start

        // Should not have started
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
    }

    #[test]
    fn more_personnel_means_faster_progress() {
        let mut state = AppState::new_default(42);

        // Create a project with base 5 personnel, assign 10 (2x base)
        // With diminishing returns: speed = 1 + (2-1)*(3-2)/2 = 1.5x
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 10, // 2x base (5) — peak of diminishing returns
        }];

        state = state.with_world(tick(&state).0);
        // At 2x ratio, diminishing returns gives 1.5x speed
        let expected = 1.5;
        assert!(
            (state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress - expected).abs() < 0.01,
            "2x personnel should give 1.5x speed, got {}",
            state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress
        );
    }

    #[test]
    fn diminishing_returns_beyond_double() {
        let mut state = AppState::new_default(42);

        // Assign 3x base personnel — should be back to 1.0x speed
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 15, // 3x base (5)
        }];

        state = state.with_world(tick(&state).0);
        let expected = 1.0;
        assert!(
            (state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress - expected).abs() < 0.01,
            "3x personnel should give 1.0x speed, got {}",
            state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().first().unwrap().progress
        );
    }

    #[test]
    fn concurrent_field_and_applied_research() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 1000.0; // enough for both projects

        // Start field research (first item in flat list)
        state = start_research_matching(&state, |k| k.is_field_work());
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        // Start applied research (TrainPersonnel is always available)
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        // Both running simultaneously
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());
    }

    #[test]
    fn research_requires_funding() {
        use crate::state::{LabTab, LabUiState};
        let mut state = AppState::new_default(42);
        // Identify costs $350; set funding to $100 so it fails
        state.resources.funding = 100.0;

        state = apply_action(&state, &Action::OpenLab);
        state.ui.lab_ui = Some(LabUiState::Browse { tab: LabTab::Sequencing });
        state.ui.panel_selection = 0;
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "should not start without funding");
        assert!(state.session.status_message.as_ref().unwrap().contains("Insufficient funds"));

        // Give enough funding, should succeed (still on ConfirmProject screen)
        state.resources.funding = 500.0;
        state = apply_action(&state, &Action::Confirm); // Try again
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "should start with sufficient funding");
        assert!(state.resources.funding < 500.0, "funding should be deducted");
    }

    // develop_medicine_unlocks — removed: DevelopMedicine no longer exists.

    #[test]
    fn clinical_trial_adds_target_and_tested() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = true;

        state.active_research = vec![ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0, rigor: crate::state::TrialRigor::Full },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        }];

        state = state.with_world(tick(&state).0);
        assert!(state.medicines[0].tested_against.contains(&0));
        assert!(state.medicines[0].target_diseases.contains(&0));
    }

    #[test]
    fn clinical_trial_enables_deploy() {
        let mut state = AppState::new_default(42);
        state.medicines[0].unlocked = true;

        state.active_research = vec![ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0, rigor: crate::state::TrialRigor::Full },
            progress: 24.0,
            required_ticks: 25.0,
            personnel_assigned: 5,
        }];

        // deploy should be disabled before trial completes
        assert!(!state.deploy_enabled.get(0).copied().unwrap_or(false),
            "deploy should be off before trial");

        state = state.with_world(tick(&state).0);

        assert!(state.medicines[0].tested_against.contains(&0),
            "medicine should be tested after trial");
        assert!(state.deploy_enabled.get(0).copied().unwrap_or(false),
            "deploy should be enabled automatically after trial completes");
    }

    // narrow_medicine_cheaper_to_develop_than_broad — removed: DevelopMedicine no longer exists.


    #[test]
    fn manufacture_doses_restores_supply() {
        use crate::engine::execute_command;
        use crate::state::GameCommand;

        let mut state = AppState::new_default(42);
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
        state.medicines[0].doses = 0.0;
        state.resources.funding = 10000.0;

        // Configure reactor to produce medicine 0
        execute_command(&mut state, &GameCommand::ConfigureReactor { reactor_idx: 0, medicine_idx: Some(0) });
        assert_eq!(state.reactors[0].medicine_idx, Some(0));
        // Disable auto-deploy so doses accumulate in stockpile for this test
        execute_command(&mut state, &GameCommand::ToggleReactorAutoDeploy { reactor_idx: 0 });
        assert!(!state.reactors[0].auto_deploy);

        // Start a batch
        execute_command(&mut state, &GameCommand::StartReactorBatch { reactor_idx: 0 });
        assert!(state.reactors[0].active, "reactor should be running");

        // Fast-forward batch to near completion
        state.reactors[0].batch_progress = state.reactors[0].batch_required - 1.0;

        state = state.with_world(tick(&state).0);

        assert!(!state.reactors[0].active, "reactor batch should be complete");
        assert_eq!(
            state.medicines[0].doses, state.medicines[0].max_doses,
            "doses should be restored to max_doses"
        );
    }

    #[test]
    fn genomic_sequencing_reduces_mutation_rate() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        let original_rate = state.diseases[0].pathogen_type.mutation_rate();

        state = start_research_matching(&state, |k| matches!(k, ResearchKind::GenomicSequencing { .. }));
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        for _ in 0..200 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert_eq!(state.diseases[0].sequencing_count, 1);

        let effective_rate = original_rate * 0.5_f64.powi(state.diseases[0].sequencing_count as i32);
        assert!((effective_rate - original_rate * 0.5).abs() < 0.0001);
    }

    #[test]
    fn train_personnel_increases_count() {
        let mut state = AppState::new_default(42);
        let initial_personnel = state.resources.personnel;

        state = start_research_matching(&state, |k| matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        for _ in 0..160 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().is_empty());
        assert_eq!(state.resources.personnel, initial_personnel + 5);
    }

    #[test]
    fn basic_research_unlocks_tech() {
        let mut state = AppState::new_default(42);
        // Prereq for TargetedDrugDesign: identify any pathogen
        state.diseases[0].knowledge = 0.5;
        state.resources.funding = 1000.0;
        assert!(state.unlocked_techs.is_empty());

        // Navigate: Research panel → find first available BasicResearch → Confirm
        state = start_basic_research(&state);
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().first().is_some(), "basic research should have started");

        // Advance to completion (240 ticks at 1x speed)
        for _ in 0..240 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().is_empty(), "project should be complete");
        assert!(
            state.unlocked_techs.contains(&crate::state::BasicTech::TargetedDrugDesign),
            "TargetedDrugDesign should be unlocked"
        );
    }

    #[test]
    fn three_concurrent_research_projects() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 2000.0;
        state.unlocked_techs.push(crate::state::BasicTech::TargetedDrugDesign);
        state.resources.personnel = 30;

        // Use the shared helper that handles panel toggle correctly
        // (it closes the panel first if already open)

        // Start field research
        state = start_research_matching(&state, |k| k.is_field_work());
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        // Start applied research (TrainPersonnel is always available)
        state = start_research_matching(&state, |k| matches!(k, ResearchKind::TrainPersonnel));
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());

        // Start basic research (via Research/tech tree panel)
        state = start_basic_research(&state);
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().first().is_some());

        // All three running simultaneously
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::ManufactureDoses { .. } | ResearchKind::TrainPersonnel)).collect::<Vec<_>>().first().is_some());
        assert!(state.active_research.iter().filter(|p| matches!(p.kind, ResearchKind::BasicResearch { .. })).collect::<Vec<_>>().first().is_some());
    }

    #[test]
    fn no_research_after_game_over() {
        let mut state = AppState::new_default(42);
        state.outcome = GameOutcome::Lost;
        // Try to start research
        state = apply_action(&state, &Action::OpenLab);
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Try to start
        assert!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty(), "should not start research after game over");
    }

    #[test]
    fn parallel_research_runs_and_completes_independently() {
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against.clear();
        state.resources.funding = 3000.0;
        state.resources.personnel = 30;

        // Start two projects in parallel: IdentifyThreat (field work) + ClinicalTrial (trial)
        state.active_research = vec![
            ResearchProject {
                kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
                progress: 0.0,
                required_ticks: 50.0,
                personnel_assigned: 5,
            },
            ResearchProject {
                kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0, rigor: crate::state::TrialRigor::Full },
                progress: 0.0,
                required_ticks: 100.0,
                personnel_assigned: 5,
            },
        ];

        assert_eq!(state.active_research.len(), 2, "should have 2 parallel projects");
        assert_eq!(state.personnel_busy(), 10, "10 personnel busy across 2 projects");

        // Advance until first project completes but second hasn't
        for _ in 0..55 {
            state = state.with_world(tick(&state).0);
        }
        assert_eq!(state.active_research.len(), 1, "first project should have completed");
        assert!(matches!(&state.active_research[0].kind, ResearchKind::ClinicalTrial { .. }),
            "remaining project should be the clinical trial");

        // Advance until second completes
        for _ in 0..50 {
            state = state.with_world(tick(&state).0);
        }
        assert!(state.active_research.is_empty(), "both projects should have completed");
    }

    #[test]
    fn research_only_gated_by_personnel_and_funding() {
        let mut state = AppState::new_default(42);
        state.resources.personnel = 50;
        state.resources.funding = 5000.0;

        // Fill 3 field projects (the old MAX_FIELD_RESEARCH limit)
        state.active_research.push(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });
        state.active_research.push(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 1 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });
        state.active_research.push(ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 2 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5,
        });
        assert_eq!(state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().len(), 3);

        // With no capacity limits, a 4th project should start if we have resources
        let available = state.all_available_projects();
        assert!(!available.is_empty(), "should still have available projects with 3 active");
        let first_available = available[0].clone();
        let (ok, _msg) = super::start_research(&mut state, &first_available, false);
        assert!(ok, "should start a 4th project — no capacity limit, only personnel/funding");
        assert!(state.active_research.len() >= 4, "should have 4+ active projects");
    }

    #[test]
    fn rapid_sequencing_unlocks_after_sequencing() {
        let mut state = AppState::new_default(42);
        // No sequencing done yet — RapidSequencing should not be available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::RapidSequencing }
        )), "RapidSequencing should not be available without sequencing");

        // Complete one sequencing
        state.diseases[0].sequencing_count = 1;
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: crate::state::BasicTech::RapidSequencing }
        )), "RapidSequencing should be available after sequencing");
    }

    #[test]
    fn rapid_sequencing_halves_genomic_sequencing_duration() {
        let mut state = AppState::new_default(42);
        let kind = ResearchKind::GenomicSequencing { disease_idx: 0 };

        let (_, base_dur, _) = state.effective_costs(&kind);
        assert_eq!(base_dur, 200.0, "base genomic sequencing should be 200 ticks");

        state.unlocked_techs.push(crate::state::BasicTech::RapidSequencing);
        let (_, rapid_dur, _) = state.effective_costs(&kind);
        assert_eq!(rapid_dur, 100.0, "with RapidSequencing, should be 100 ticks");
    }

    #[test]
    fn combination_therapy_prereqs() {
        use crate::state::{BasicTech, Medicine, TherapyType, MechanismOfAction};
        let mut state = AppState::new_default(42);
        // No chain prereqs or deployed medicines → not available
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::CombinationTherapy }
        )), "CombinationTherapy should not be available without prereqs");

        // Unlock column 1 chain (RapidSeq → ResSurv → MetaSurv → EpiForecasting)
        state.unlocked_techs.push(BasicTech::RapidSequencing);
        state.unlocked_techs.push(BasicTech::ResistanceSurveillance);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        state.unlocked_techs.push(BasicTech::EpidemiologicalForecasting);

        // Chain prereq met but no deployed medicines → still not available
        state.medicines[0].deployed_count = 1;
        let basic = state.available_basic_projects();
        assert!(!basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::CombinationTherapy }
        )), "CombinationTherapy should not be available with only 1 deployed medicine");

        // Add a second medicine and deploy it → available
        state.medicines.push(Medicine {
            name: "Test Antibiotic".into(),
            therapy_type: TherapyType::Antibiotic,
            mechanism: Some(MechanismOfAction::CellWallInhibitor),
            target_diseases: vec![0],
            doses: 500_000.0,
            max_doses: 500_000.0,
            unlocked: true,
            tested_against: vec![0],
            deployed_count: 1,
            total_treated: 0.0,
                manufacturer_corp_idx: None,
            trial_efficacy: None,
            side_effect_rate: 0.0,
            resistance_rate: 0.0,
            trial_rigor: None,
            reported_efficacy: None,
            reported_side_effects: None,
            reported_resistance: None,
        });
        let basic = state.available_basic_projects();
        assert!(basic.iter().any(|k| matches!(k,
            ResearchKind::BasicResearch { tech: BasicTech::CombinationTherapy }
        )), "CombinationTherapy should be available with EpiForecasting + 2+ deployed medicines");
    }

    #[test]
    fn combination_therapy_halves_resistance() {
        let mut state = AppState::new_default(42);
        assert_eq!(state.resistance_multiplier(), 1.0);

        state.unlocked_techs.push(crate::state::BasicTech::CombinationTherapy);
        assert_eq!(state.resistance_multiplier(), 0.5);
    }


    #[test]
    fn genomic_sequencing_unavailable_after_effective_rate_drops() {
        use crate::state::PathogenType;

        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.diseases[0].pathogen_type = PathogenType::RnaVirus; // base rate 0.0002
        // Ensure disease has infected population so sequencing can be considered
        state.regions[0].get_or_create_infection(0).infected = 1000.0;
        state.active_research.clear();

        // After 6 sequencings: 0.0002 * 0.5^6 = 0.000003125 < 0.000005 threshold
        state.diseases[0].sequencing_count = 6;
        let field_projects = state.available_field_projects();
        assert!(
            !field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should not be available when effective rate ({}) is below threshold",
            state.diseases[0].effective_variant_rate()
        );

        // After 2 sequencings: 0.0002 * 0.5^2 = 0.00005 > 0.000005 — still available
        state.diseases[0].sequencing_count = 2;
        let field_projects = state.available_field_projects();
        assert!(
            field_projects.iter().any(|k| matches!(k,
                ResearchKind::GenomicSequencing { disease_idx: 0 }
            )),
            "sequencing should still be available when effective rate ({}) is above threshold",
            state.diseases[0].effective_variant_rate()
        );
    }

    #[test]
    fn human_trials_halves_clinical_trial_duration() {
        use crate::state::{ScreeningHit, ScreeningModality, TrialRigor};
        let mut state = AppState::new_default(42);
        state.diseases[0].knowledge = 1.0;
        state.resources.funding = 10_000.0;
        state.resources.personnel = 20;

        // Create a screening hit so we can start a trial via start_trial
        state.screening_hits.push(ScreeningHit {
            compound_id: "TEST-001".into(),
            disease_idx: 0,
            modality: ScreeningModality::SmallMolecule,
            kd_nm: 5.0,
            well_index: 0,
        });

        // Start trial WITHOUT human trials decree
        let (ok, _) = super::start_trial(&mut state, 0, TrialRigor::Full);
        assert!(ok, "trial should start");
        let normal_duration = state.active_research.last().unwrap().required_ticks;
        state.active_research.clear();
        // Reset: re-add the screening hit (start_trial consumed it)
        state.screening_hits.push(ScreeningHit {
            compound_id: "TEST-002".into(),
            disease_idx: 0,
            modality: ScreeningModality::SmallMolecule,
            kd_nm: 5.0,
            well_index: 0,
        });

        // Enact human trials and start the same trial
        state.enacted_decrees.authorize_human_trials = true;
        let (ok, _) = super::start_trial(&mut state, 0, TrialRigor::Full);
        assert!(ok, "trial should start with human trials");
        let fast_duration = state.active_research.last().unwrap().required_ticks;

        // Duration should be halved
        assert!(
            (fast_duration - normal_duration * crate::state::HUMAN_TRIALS_SPEED).abs() < 1.0,
            "human trials should halve duration: normal={normal_duration}, fast={fast_duration}"
        );
    }


    #[test]
    fn lab_upgrade_increases_research_speed() {
        use crate::state::LAB_LEVEL_1_COST;

        let mut state = AppState::new_default(42);
        state.resources.funding = 1000.0;
        state.active_research = vec![ResearchProject {
            kind: ResearchKind::IdentifyThreat { disease_idx: 0 },
            progress: 0.0,
            required_ticks: 160.0,
            personnel_assigned: 5, // base personnel, 1.0x speed
        }];

        // Baseline: one tick at standard lab
        let (base_state, _) = tick(&state);
        let base_progress = base_state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>()[0].progress;

        // Upgrade to level 1 (1.3x multiplier)
        state.lab_level = 1;
        let (upgraded_state, _) = tick(&state);
        let upgraded_progress = upgraded_state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>()[0].progress;

        assert!(
            (upgraded_progress / base_progress - 1.3).abs() < 0.01,
            "Lab level 1 should give 1.3x speed, got {}x",
            upgraded_progress / base_progress
        );

        // Verify upgrade_lab deducts cost and increments level
        let mut s = AppState::new_default(42);
        s.resources.funding = 1000.0;
        let (ok, msg) = super::upgrade_lab(&mut s);
        assert!(ok);
        assert!(msg.is_some(), "upgrade should return a message");
        assert_eq!(s.lab_level, 1);
        assert!((s.resources.funding - (1000.0 - LAB_LEVEL_1_COST)).abs() < 0.01);
    }

    #[test]
    fn handoff_notification_after_identification() {
        use crate::state::GameEvent;
        let mut state = AppState::new_default(42);
        // Reset broad-spectrum to locked so identification can trigger the handoff
        for med in &mut state.medicines {
            if med.therapy_type == crate::state::TherapyType::BroadSpectrum {
                med.unlocked = false;
                med.doses = 0.0;
            }
        }
        // Start identify on disease 0 (first item in Sequencing tab)
        state = apply_action(&state, &Action::OpenLab);
        state.ui.lab_ui = Some(crate::state::LabUiState::Browse { tab: crate::state::LabTab::Sequencing });
        state.ui.panel_selection = 0;
        state = apply_action(&state, &Action::Confirm); // ConfirmProject
        state = apply_action(&state, &Action::Confirm); // Start
        assert!(!state.active_research.iter().filter(|p| p.kind.is_field_work()).collect::<Vec<_>>().is_empty());

        // Advance to completion, checking events each tick (clone-and-mutate means
        // events are only on the state returned by the tick that generated them)
        let mut found_handoff = false;
        for _ in 0..200 {
            let tick_result = tick(&state);
            state = state.with_world(tick_result.0);
            let tick_events = tick_result.1;
            if tick_events.iter().any(|e|
                matches!(e, GameEvent::ResearchHandoff { message } if message.contains("screening"))
            ) {
                found_handoff = true;
            }
        }
        assert!(state.diseases[0].knowledge >= 0.5, "Disease should be identified");
        assert!(found_handoff, "Should notify about screening after identification");
    }

    // handoff_notification_after_medicine_developed — removed: DevelopMedicine no longer exists.
    // Medicines are created from screening hits via clinical trials.


    // blocked_medicine_developments_shows_identified_but_unresearched — removed:
    // blocked_medicine_developments() method no longer exists.

    // blocked_medicine_developments_not_duplicated_when_already_available — removed:
    // blocked_medicine_developments() method no longer exists.

    #[test]
    fn medicine_names_are_diverse() {
        use crate::state::MechanismOfAction;
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        use std::collections::HashSet;

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut names = HashSet::new();

        let mechanisms = [
            Some(MechanismOfAction::CellWallInhibitor),
            Some(MechanismOfAction::RibosomeInhibitor),
            Some(MechanismOfAction::PolymeraseInhibitor),
            Some(MechanismOfAction::ProteaseInhibitor),
            Some(MechanismOfAction::EntryInhibitor),
            None,
        ];

        // Generate 30 names (5 per mechanism) — all should be unique
        for mech in &mechanisms {
            for _ in 0..5 {
                let name = super::generate_medicine_name(*mech, &mut rng);
                assert!(!name.is_empty());
                // First letter should be uppercase
                assert!(name.chars().next().unwrap().is_uppercase(),
                    "Name should start uppercase: {}", name);
                names.insert(name);
            }
        }

        // With 30 generated names, we should have at least 25 unique (high diversity)
        assert!(names.len() >= 25,
            "Expected at least 25 unique names from 30 generated, got {}: {:?}",
            names.len(), names);

        // Consecutive calls with the same mechanism should produce different names
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);
        let a = super::generate_medicine_name(Some(MechanismOfAction::PolymeraseInhibitor), &mut rng2);
        let b = super::generate_medicine_name(Some(MechanismOfAction::PolymeraseInhibitor), &mut rng2);
        assert_ne!(a, b, "Consecutive calls should produce different names");
    }
}
