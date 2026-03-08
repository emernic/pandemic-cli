use rand::Rng;

use crate::action::Action;
use crate::state::{
    map_navigate, DeployTarget, GameOutcome, GameState, MapDirection, MedicineUiState, Panel,
    PolicyUiState, RegionDiseaseState, ResearchKind, ResearchProject, ResearchUiState,
    BOOST_RP_COST, BOOST_TICKS, HOSPITAL_SURGE_PERSONNEL, KNOWLEDGE_FOR_MEDICINE,
    KNOWLEDGE_FULL, KNOWLEDGE_NAME, LOSE_DEATH_FRACTION, QUARANTINE_PERSONNEL,
};

/// Ensure policies vec matches regions length (for saves that predate the policy system).
fn ensure_policies(state: &mut GameState) {
    while state.policies.len() < state.regions.len() {
        state.policies.push(crate::state::RegionPolicy::default());
    }
}

/// Advance the simulation by one tick.
pub fn tick(state: &GameState) -> GameState {
    let mut new = state.clone();
    ensure_policies(&mut new);

    // Don't advance simulation after game over
    if new.outcome != GameOutcome::Playing {
        return new;
    }

    // Clone the RNG out so we can mutably borrow both `rng` and `new.regions`
    // simultaneously. Written back to `new.rng` at the end of the function.
    // WARNING: Do not use `new.rng` between here and the write-back line.
    let mut rng = new.rng.clone();

    // Disease spread within each region
    for (region_idx, region) in new.regions.iter_mut().enumerate() {
        let pop = region.population as f64;
        let policy = new.policies.get(region_idx);
        let quarantine_active = policy.is_some_and(|p| p.quarantine);
        let hospital_active = policy.is_some_and(|p| p.hospital_surge);

        for inf in &mut region.infections {
            if let Some(disease) = state.diseases.get(inf.disease_idx) {
                let susceptible = pop - inf.infected - inf.dead - inf.immune;
                if susceptible <= 0.0 {
                    continue;
                }

                let noise: f64 = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.1;
                let infectivity = if quarantine_active {
                    disease.infectivity * 0.5
                } else {
                    disease.infectivity
                };
                let new_infections =
                    infectivity * inf.infected * (susceptible / pop) * noise;
                let new_infections = new_infections.max(0.0).min(susceptible);

                // Deaths and recoveries are concurrent outflows from the infected pool.
                // Compute both, then scale proportionally if they exceed infected.
                let lethality = if hospital_active {
                    disease.lethality * 0.5
                } else {
                    disease.lethality
                };
                let mut new_deaths = (lethality * inf.infected * noise).max(0.0);
                let mut new_recoveries = (disease.recovery_rate * inf.infected * noise).max(0.0);
                let total_outflow = new_deaths + new_recoveries;
                if total_outflow > inf.infected {
                    let scale = inf.infected / total_outflow;
                    new_deaths *= scale;
                    new_recoveries *= scale;
                }

                inf.infected = inf.infected + new_infections - new_deaths - new_recoveries;
                // Snap to zero when below 1 person to avoid floating-point residue
                if inf.infected < 0.5 {
                    inf.infected = 0.0;
                }
                inf.immune += new_recoveries;
                inf.dead += new_deaths;
            }
        }
    }

    // Cross-region spread
    let regions_snapshot: Vec<_> = new.regions.clone();
    for (i, region) in new.regions.iter_mut().enumerate() {
        let dest_has_travel_ban = new.policies.get(i).is_some_and(|p| p.travel_ban);

        for (d_idx, disease) in state.diseases.iter().enumerate() {
            let connected_infected: f64 = regions_snapshot[i]
                .connections
                .iter()
                .filter_map(|&conn_idx| {
                    let source_has_travel_ban =
                        new.policies.get(conn_idx).is_some_and(|p| p.travel_ban);
                    // Travel ban on either end reduces spread by 90%
                    let ban_factor = if source_has_travel_ban || dest_has_travel_ban {
                        0.1
                    } else {
                        1.0
                    };
                    regions_snapshot[conn_idx]
                        .disease_state(d_idx)
                        .map(|inf| inf.infected * ban_factor)
                })
                .sum();

            if connected_infected <= 0.0 {
                continue;
            }

            let has_active_infection = region
                .infections
                .iter()
                .any(|inf| inf.disease_idx == d_idx && inf.infected > 0.0);

            if !has_active_infection {
                let roll: f64 = rng.r#gen();
                let chance = disease.cross_region_spread * (connected_infected / 10_000.0);
                if roll < chance.min(0.5) {
                    // Check if there's an existing entry (e.g., from vaccination)
                    if let Some(existing) = region
                        .infections
                        .iter_mut()
                        .find(|inf| inf.disease_idx == d_idx)
                    {
                        existing.infected = 1.0;
                    } else {
                        region.infections.push(RegionDiseaseState {
                            disease_idx: d_idx,
                            infected: 1.0,
                            dead: 0.0,
                            immune: 0.0,
                        });
                    }
                }
            }
        }
    }

    // Disease mutation
    for disease in &mut new.diseases {
        let mutation_chance = disease.pathogen_type.mutation_rate();
        if rng.r#gen::<f64>() < mutation_chance {
            disease.strain_generation += 1;
            // Small random parameter changes (±10% of current value), clamped to
            // prevent runaway drift over many mutations.
            let inf_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            disease.infectivity = (disease.infectivity * inf_factor).clamp(0.005, 0.5);
            let leth_factor = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.2;
            disease.lethality = (disease.lethality * leth_factor).clamp(0.0005, 0.1);
        }
    }

    // Research progress
    if let Some(ref mut project) = new.field_research {
        project.progress += 1.0;
        if project.is_complete() {
            match &project.kind {
                ResearchKind::IdentifyThreat { disease_idx } => {
                    let d_idx = *disease_idx;
                    if let Some(disease) = new.diseases.get_mut(d_idx) {
                        disease.knowledge = (disease.knowledge + 0.50).min(KNOWLEDGE_FULL);
                    }
                }
                ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
                    let m_idx = *medicine_idx;
                    let d_idx = *disease_idx;
                    if let Some(medicine) = new.medicines.get_mut(m_idx) {
                        if !medicine.tested_against.contains(&d_idx) {
                            medicine.tested_against.push(d_idx);
                        }
                        // Update strain calibration to current disease generation
                        if let Some(pos) = medicine.target_diseases.iter().position(|&d| d == d_idx) {
                            let current_gen = new.diseases.get(d_idx)
                                .map_or(0, |d| d.strain_generation);
                            // Extend strain_generations if needed
                            while medicine.strain_generations.len() <= pos {
                                medicine.strain_generations.push(0);
                            }
                            medicine.strain_generations[pos] = current_gen;
                        }
                    }
                }
                ResearchKind::DevelopMedicine { .. } => {}
            }
            new.field_research = None;
        }
    }
    if let Some(ref mut project) = new.bench_research {
        project.progress += 1.0;
        if project.is_complete() {
            match &project.kind {
                ResearchKind::DevelopMedicine { medicine_idx } => {
                    let m_idx = *medicine_idx;
                    if let Some(medicine) = new.medicines.get_mut(m_idx) {
                        medicine.unlocked = true;
                        // Calibrate to current strain generations of all target diseases
                        medicine.strain_generations = medicine.target_diseases.iter()
                            .map(|&d_idx| new.diseases.get(d_idx)
                                .map_or(0, |d| d.strain_generation))
                            .collect();
                    }
                }
                _ => {}
            }
            new.bench_research = None;
        }
    }

    // Policy costs — deducted before income. Suspend all if insolvent.
    let policy_cost = new.total_policy_funding_cost();
    if policy_cost > 0.0 {
        if new.resources.funding >= policy_cost {
            new.resources.funding -= policy_cost;
        } else {
            // Funding crisis: suspend all policies
            for p in &mut new.policies {
                p.clear_all();
            }
            new.ui.status_message = Some("FUNDING CRISIS: All policies suspended!".to_string());
        }
    }

    // Passive resource generation
    new.resources.funding += 5.0;
    new.resources.research_points += 1.0;

    new.rng = rng;
    new.tick += 1;

    // Check win/lose conditions (only while still playing)
    if new.outcome == GameOutcome::Playing {
        let total_dead = new.total_dead();
        let death_threshold = new.initial_population() * LOSE_DEATH_FRACTION;

        if total_dead >= death_threshold {
            new.outcome = GameOutcome::Lost;
            new.paused = true;
            new.ui.open_panel = Panel::None;
        } else if new.total_infected() < 1.0 {
            // Win requires player engagement: all diseases must be identified
            let all_identified = new.diseases.iter().all(|d| d.knowledge >= KNOWLEDGE_NAME);
            if all_identified {
                new.outcome = GameOutcome::Won;
                new.paused = true;
                new.ui.open_panel = Panel::None;
            }
        }
    }

    new
}

/// Find or create a RegionDiseaseState entry for the given disease in a region.
fn get_or_create_infection(region: &mut crate::state::Region, disease_idx: usize) -> &mut RegionDiseaseState {
    let pos = region.infections.iter().position(|i| i.disease_idx == disease_idx);
    if let Some(idx) = pos {
        &mut region.infections[idx]
    } else {
        region.infections.push(RegionDiseaseState {
            disease_idx,
            infected: 0.0,
            dead: 0.0,
            immune: 0.0,
        });
        region.infections.last_mut().unwrap()
    }
}

/// Execute medicine deployment: deduct funds, apply doses (with adverse effect
/// roll for untested medicines). Pure game logic — does NOT modify UI state.
///
/// Returns (navigate_back, message):
/// - `navigate_back`: true if the caller should return to SelectRegion
/// - `message`: status feedback to display (if any)
fn deploy_medicine(
    state: &mut GameState,
    medicine_idx: usize,
    region_idx: usize,
    target_selection: usize,
) -> (bool, Option<String>) {
    // Block after game over
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    let med = &state.medicines[medicine_idx];
    let cost = med.cost;
    let med_name = med.name.clone();
    let therapy_type = med.therapy_type;
    let target = med.decode_deploy_target(target_selection);

    if let Some(target) = target {
        if state.resources.funding < cost {
            return (false, Some(insufficient_funds_message(cost, state.resources.funding)));
        }

        let disease_idx = match &target {
            DeployTarget::Vaccinate { disease_idx } => *disease_idx,
            DeployTarget::Treat { disease_idx } => *disease_idx,
        };

        // Efficacy: therapy type × pathogen type × strain match
        let pathogen = &state.diseases[disease_idx].pathogen_type;
        let therapy_efficacy = therapy_type.efficacy(pathogen);
        let strain_eff = state.medicines[medicine_idx].strain_efficacy(disease_idx, &state.diseases);
        let efficacy = therapy_efficacy * strain_eff;
        let effective_doses = state.medicines[medicine_idx].doses * efficacy;

        let region = &mut state.regions[region_idx];
        let region_name = region.name.clone();
        let pop = region.population as f64;

        // Look up existing infection state (don't create yet — avoid ghost entries)
        let existing = region.infections.iter().find(|i| i.disease_idx == disease_idx);
        let infected = existing.map(|i| i.infected).unwrap_or(0.0);
        let dead = existing.map(|i| i.dead).unwrap_or(0.0);
        let immune = existing.map(|i| i.immune).unwrap_or(0.0);

        let is_tested = state.medicines[medicine_idx]
            .tested_against
            .contains(&disease_idx);

        let msg = match target {
            DeployTarget::Vaccinate { .. } => {
                let susceptible = (pop - infected - dead - immune).max(0.0);
                let actual = effective_doses.min(susceptible);
                if actual > 0.0 {
                    // Now create entry if needed
                    let inf = get_or_create_infection(region, disease_idx);
                    let mut adverse = false;
                    if !is_tested {
                        let roll: f64 = state.rng.r#gen();
                        if roll < 0.25 {
                            adverse = true;
                            let harmed = (actual * 0.2).min(susceptible);
                            inf.dead += harmed;
                            inf.immune += actual - harmed;
                        } else {
                            inf.immune += actual;
                        }
                    } else {
                        inf.immune += actual;
                    }
                    state.resources.funding -= cost;
                    deploy_feedback(&med_name, &region_name, "Vaccinated", actual, cost, adverse, efficacy)
                } else {
                    format!("No susceptible population in {region_name}")
                }
            }
            DeployTarget::Treat { .. } => {
                let actual = effective_doses.min(infected);
                if actual > 0.0 {
                    let inf = get_or_create_infection(region, disease_idx);
                    inf.infected -= actual;
                    let mut adverse = false;
                    if !is_tested {
                        let roll: f64 = state.rng.r#gen();
                        if roll < 0.25 {
                            adverse = true;
                            let harmed = actual * 0.2;
                            inf.dead += harmed;
                            inf.immune += actual - harmed;
                        } else {
                            inf.immune += actual;
                        }
                    } else {
                        inf.immune += actual;
                    }
                    state.resources.funding -= cost;
                    deploy_feedback(&med_name, &region_name, "Treated", actual, cost, adverse, efficacy)
                } else {
                    format!("No infected population in {region_name}")
                }
            }
        };

        return (true, Some(msg));
    }

    (true, None)
}

fn insufficient_funds_message(cost: f64, have: f64) -> String {
    format!("Insufficient funds! Need ${cost:.0}, have ${have:.0}")
}

fn deploy_feedback(med: &str, region: &str, action: &str, doses: f64, cost: f64, adverse: bool, efficacy: f64) -> String {
    let doses_str = crate::format_number(doses);
    let eff_note = if efficacy < 1.0 {
        format!(" ({:.0}% efficacy)", efficacy * 100.0)
    } else {
        String::new()
    };
    if adverse {
        let killed = crate::format_number(doses * 0.2);
        format!("{action} {doses_str} in {region} with {med}{eff_note} (-${cost:.0}) -- ADVERSE REACTION: {killed} died")
    } else {
        format!("{action} {doses_str} in {region} with {med}{eff_note} (-${cost:.0})")
    }
}

fn toggle_panel(ui: &mut crate::state::UiState, panel: Panel) {
    if ui.open_panel == panel {
        ui.open_panel = Panel::None;
    } else {
        ui.open_panel = panel;
        ui.panel_selection = 0;
    }
}

/// Apply a player action to the game state.
pub fn apply_action(state: &GameState, action: &Action) -> GameState {
    let mut new = state.clone();
    ensure_policies(&mut new);
    new.ui.status_message = None;

    match action {
        Action::TogglePause => {
            // Can't unpause after game over
            if new.outcome == GameOutcome::Playing {
                new.paused = !new.paused;
            }
        }
        Action::OpenThreats => toggle_panel(&mut new.ui, Panel::Threats),
        Action::OpenResearch => {
            toggle_panel(&mut new.ui, Panel::Research);
            if new.ui.open_panel == Panel::Research {
                new.ui.research_ui = Some(ResearchUiState::BrowseCategories);
            } else {
                new.ui.research_ui = None;
            }
        }
        Action::OpenMedicines => {
            toggle_panel(&mut new.ui, Panel::Medicines);
            if new.ui.open_panel == Panel::Medicines {
                new.ui.medicine_ui = Some(MedicineUiState::BrowseMedicines);
            } else {
                new.ui.medicine_ui = None;
            }
        }
        Action::OpenPolicy => {
            toggle_panel(&mut new.ui, Panel::Policy);
            if new.ui.open_panel == Panel::Policy {
                new.ui.policy_ui = Some(PolicyUiState::BrowseRegions);
            } else {
                new.ui.policy_ui = None;
            }
        }
        Action::OpenHelp => toggle_panel(&mut new.ui, Panel::Help),
        Action::ClosePanel => {
            if new.ui.open_panel == Panel::Medicines {
                match new.ui.medicine_ui.clone() {
                    Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, target_selection }) => {
                        new.ui.medicine_ui = Some(MedicineUiState::SelectTarget {
                            medicine_idx,
                            region_idx,
                        });
                        new.ui.panel_selection = target_selection;
                    }
                    Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                        new.ui.medicine_ui =
                            Some(MedicineUiState::SelectRegion { medicine_idx });
                        new.ui.panel_selection = 0;
                    }
                    Some(MedicineUiState::SelectRegion { .. }) => {
                        new.ui.medicine_ui = Some(MedicineUiState::BrowseMedicines);
                        new.ui.panel_selection = 0;
                    }
                    _ => {
                        new.ui.open_panel = Panel::None;
                        new.ui.panel_selection = 0;
                        new.ui.medicine_ui = None;
                    }
                }
            } else if new.ui.open_panel == Panel::Policy {
                match &new.ui.policy_ui {
                    Some(PolicyUiState::ManagePolicies { .. }) => {
                        new.ui.policy_ui = Some(PolicyUiState::BrowseRegions);
                        new.ui.panel_selection = 0;
                    }
                    _ => {
                        new.ui.open_panel = Panel::None;
                        new.ui.panel_selection = 0;
                        new.ui.policy_ui = None;
                    }
                }
            } else if new.ui.open_panel == Panel::Research {
                match &new.ui.research_ui {
                    Some(ResearchUiState::ConfirmProject { bench, .. }) => {
                        new.ui.research_ui = Some(ResearchUiState::BrowseProjects { bench: *bench });
                        new.ui.panel_selection = 0;
                    }
                    Some(ResearchUiState::ViewActive { bench }) => {
                        new.ui.research_ui = Some(ResearchUiState::BrowseProjects { bench: *bench });
                        new.ui.panel_selection = 0;
                    }
                    Some(ResearchUiState::BrowseProjects { .. }) => {
                        new.ui.research_ui = Some(ResearchUiState::BrowseCategories);
                        new.ui.panel_selection = 0;
                    }
                    _ => {
                        new.ui.open_panel = Panel::None;
                        new.ui.panel_selection = 0;
                        new.ui.research_ui = None;
                    }
                }
            } else {
                new.ui.open_panel = Panel::None;
                new.ui.panel_selection = 0;
                new.ui.medicine_ui = None;
                new.ui.research_ui = None;
                new.ui.policy_ui = None;
            }
        }
        Action::SelectNext => {
            if new.ui.open_panel == Panel::None {
                // Navigate map down
                new.ui.map_selection = map_navigate(
                    new.ui.map_selection,
                    MapDirection::Down,
                    new.regions.len(),
                );
            } else {
                let max = match new.ui.open_panel {
                    Panel::Threats => new.diseases.len().saturating_sub(1),
                    Panel::Medicines => match &new.ui.medicine_ui {
                        Some(MedicineUiState::BrowseMedicines) => {
                            new.medicines
                                .iter()
                                .filter(|m| m.unlocked)
                                .count()
                                .saturating_sub(1)
                        }
                        Some(MedicineUiState::SelectRegion { .. }) => {
                            new.regions.len().saturating_sub(1)
                        }
                        Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                            new.medicines[*medicine_idx]
                                .num_deploy_targets()
                                .saturating_sub(1)
                        }
                        Some(MedicineUiState::ConfirmDeploy { .. }) | None => 0,
                    },
                    Panel::Research => research_panel_max(&new),
                    Panel::Policy => match &new.ui.policy_ui {
                        Some(PolicyUiState::BrowseRegions) => {
                            new.regions.len().saturating_sub(1)
                        }
                        Some(PolicyUiState::ManagePolicies { .. }) => 2, // 3 policy types
                        None => 0,
                    },
                    _ => 0,
                };
                if new.ui.panel_selection < max {
                    new.ui.panel_selection += 1;
                }
            }
        }
        Action::SelectPrev => {
            if new.ui.open_panel == Panel::None {
                // Navigate map up
                new.ui.map_selection = map_navigate(
                    new.ui.map_selection,
                    MapDirection::Up,
                    new.regions.len(),
                );
            } else if new.ui.panel_selection > 0 {
                new.ui.panel_selection -= 1;
            }
        }
        Action::SelectLeft => {
            new.ui.map_selection = map_navigate(
                new.ui.map_selection,
                MapDirection::Left,
                new.regions.len(),
            );
        }
        Action::SelectRight => {
            new.ui.map_selection = map_navigate(
                new.ui.map_selection,
                MapDirection::Right,
                new.regions.len(),
            );
        }
        Action::Confirm => {
            // Block all Confirm actions after game over (no deploying or starting research).
            // Players can still browse panels via arrow keys and open/close.
            if new.outcome != GameOutcome::Playing {
                // no-op
            } else if new.ui.open_panel == Panel::Research {
                handle_research_confirm(&mut new);
            } else if new.ui.open_panel == Panel::Medicines {
                match new.ui.medicine_ui.clone() {
                    Some(MedicineUiState::BrowseMedicines) => {
                        let unlocked: Vec<usize> = new
                            .medicines
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| m.unlocked)
                            .map(|(i, _)| i)
                            .collect();
                        if let Some(&med_idx) = unlocked.get(new.ui.panel_selection) {
                            new.ui.medicine_ui =
                                Some(MedicineUiState::SelectRegion { medicine_idx: med_idx });
                            new.ui.panel_selection = 0;
                        }
                    }
                    Some(MedicineUiState::SelectRegion { medicine_idx }) => {
                        let region_idx = new.ui.panel_selection;
                        if region_idx < new.regions.len() {
                            new.ui.medicine_ui = Some(MedicineUiState::SelectTarget {
                                medicine_idx,
                                region_idx,
                            });
                            new.ui.panel_selection = 0;
                        }
                    }
                    Some(MedicineUiState::SelectTarget {
                        medicine_idx,
                        region_idx,
                    }) => {
                        let target_selection = new.ui.panel_selection;
                        let med = &new.medicines[medicine_idx];
                        if let Some(target) = med.decode_deploy_target(target_selection) {
                            // Check funds before anything else — no point warning
                            // about untested risks if the player can't afford it
                            if new.resources.funding < med.cost {
                                new.ui.status_message = Some(
                                    insufficient_funds_message(med.cost, new.resources.funding),
                                );
                            } else {
                                let disease_idx = match &target {
                                    DeployTarget::Vaccinate { disease_idx } => *disease_idx,
                                    DeployTarget::Treat { disease_idx } => *disease_idx,
                                };
                                let is_tested = med.tested_against.contains(&disease_idx);

                                if !is_tested {
                                    // Untested: require confirmation
                                    new.ui.medicine_ui = Some(MedicineUiState::ConfirmDeploy {
                                        medicine_idx,
                                        region_idx,
                                        target_selection,
                                    });
                                } else {
                                    let (nav_back, msg) = deploy_medicine(&mut new, medicine_idx, region_idx, target_selection);
                                    new.ui.status_message = msg;
                                    if nav_back {
                                        new.ui.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                                        new.ui.panel_selection = 0;
                                    }
                                }
                            }
                        }
                    }
                    Some(MedicineUiState::ConfirmDeploy {
                        medicine_idx,
                        region_idx,
                        target_selection,
                    }) => {
                        let (nav_back, msg) = deploy_medicine(&mut new, medicine_idx, region_idx, target_selection);
                        new.ui.status_message = msg;
                        if nav_back {
                            new.ui.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                            new.ui.panel_selection = 0;
                        }
                    }
                    None => {}
                }
            } else if new.ui.open_panel == Panel::Policy {
                match new.ui.policy_ui.clone() {
                    Some(PolicyUiState::BrowseRegions) => {
                        // Pure UI navigation: drill into region's policy management
                        let region_idx = new.ui.panel_selection;
                        if region_idx < new.regions.len() {
                            new.ui.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx });
                            new.ui.panel_selection = 0;
                        }
                    }
                    Some(PolicyUiState::ManagePolicies { region_idx }) => {
                        // Game command: toggle the selected policy
                        let policy_idx = new.ui.panel_selection;
                        if let Some(msg) = toggle_policy(&mut new, region_idx, policy_idx) {
                            new.ui.status_message = Some(msg);
                        }
                    }
                    None => {}
                }
            }
        }
        Action::Quit => {} // Handled by the caller
    }

    new
}

/// Toggle a policy for a region. Returns an error message if the toggle fails
/// (e.g., insufficient personnel). Does not touch UI state.
fn toggle_policy(state: &mut GameState, region_idx: usize, policy_idx: usize) -> Option<String> {
    if region_idx >= state.policies.len() {
        return None;
    }
    let available_personnel = state.personnel_available();
    match policy_idx {
        0 => {
            state.policies[region_idx].travel_ban = !state.policies[region_idx].travel_ban;
            None
        }
        1 => {
            if state.policies[region_idx].quarantine {
                state.policies[region_idx].quarantine = false;
                None
            } else if available_personnel >= QUARANTINE_PERSONNEL {
                state.policies[region_idx].quarantine = true;
                None
            } else {
                Some(format!(
                    "Not enough personnel for quarantine (need {})", QUARANTINE_PERSONNEL
                ))
            }
        }
        2 => {
            if state.policies[region_idx].hospital_surge {
                state.policies[region_idx].hospital_surge = false;
                None
            } else if available_personnel >= HOSPITAL_SURGE_PERSONNEL {
                state.policies[region_idx].hospital_surge = true;
                None
            } else {
                Some(format!(
                    "Not enough personnel for hospital surge (need {})", HOSPITAL_SURGE_PERSONNEL
                ))
            }
        }
        _ => None,
    }
}

/// Compute available field research projects (excludes the currently active one).
pub fn available_field_projects(state: &GameState) -> Vec<ResearchKind> {
    let active_kind = state.field_research.as_ref().map(|p| &p.kind);
    let mut projects = Vec::new();
    // Identify Threat: diseases not fully known
    for (i, disease) in state.diseases.iter().enumerate() {
        if disease.knowledge < KNOWLEDGE_FULL {
            let kind = ResearchKind::IdentifyThreat { disease_idx: i };
            if active_kind != Some(&kind) {
                projects.push(kind);
            }
        }
    }
    // Clinical Trial: unlocked medicines not yet tested, OR tested but strain-outdated
    for (i, med) in state.medicines.iter().enumerate() {
        if !med.unlocked {
            continue;
        }
        for (target_pos, &d_idx) in med.target_diseases.iter().enumerate() {
            let needs_trial = if !med.tested_against.contains(&d_idx) {
                true // Never tested
            } else {
                // Tested, but check if strain has drifted
                let med_gen = med.strain_generations.get(target_pos).copied().unwrap_or(0);
                let disease_gen = state.diseases.get(d_idx)
                    .map_or(0, |d| d.strain_generation);
                disease_gen > med_gen
            };
            if needs_trial {
                let kind = ResearchKind::ClinicalTrial {
                    medicine_idx: i,
                    disease_idx: d_idx,
                };
                if active_kind != Some(&kind) {
                    projects.push(kind);
                }
            }
        }
    }
    projects
}

/// Compute available bench research projects (excludes the currently active one).
pub fn available_bench_projects(state: &GameState) -> Vec<ResearchKind> {
    let active_kind = state.bench_research.as_ref().map(|p| &p.kind);
    let mut projects = Vec::new();
    for (i, med) in state.medicines.iter().enumerate() {
        if med.unlocked {
            continue;
        }
        // Check if at least one target disease has enough knowledge
        let has_knowledge = med.target_diseases.iter().any(|&d_idx| {
            state.diseases.get(d_idx).map_or(false, |d| d.knowledge >= KNOWLEDGE_FOR_MEDICINE)
        });
        if has_knowledge {
            let kind = ResearchKind::DevelopMedicine { medicine_idx: i };
            if active_kind != Some(&kind) {
                projects.push(kind);
            }
        }
    }
    projects
}

/// Max selection index for the current research UI state.
fn research_panel_max(state: &GameState) -> usize {
    match &state.ui.research_ui {
        Some(ResearchUiState::BrowseCategories) => 1, // Field(0), Bench(1)
        Some(ResearchUiState::BrowseProjects { bench }) => {
            let active = if *bench { state.bench_research.is_some() } else { state.field_research.is_some() };
            if active {
                0 // Only "View Active" entry
            } else {
                let count = if *bench {
                    available_bench_projects(state).len()
                } else {
                    available_field_projects(state).len()
                };
                count.saturating_sub(1)
            }
        }
        Some(ResearchUiState::ConfirmProject { .. }) => 0,
        Some(ResearchUiState::ViewActive { .. }) => 0,
        None => 0,
    }
}

/// Project costs: (rp_cost, personnel, duration_ticks)
pub fn project_costs(kind: &ResearchKind) -> (f64, u32, f64) {
    match kind {
        ResearchKind::IdentifyThreat { .. } => (10.0, 5, 20.0),
        ResearchKind::DevelopMedicine { .. } => (30.0, 10, 40.0),
        ResearchKind::ClinicalTrial { .. } => (15.0, 5, 25.0),
    }
}

/// Start a research project. Pure game logic — does NOT modify UI state.
///
/// Returns true if the project was successfully started.
fn start_research(state: &mut GameState, bench: bool, project_idx: usize) -> bool {
    if state.outcome != GameOutcome::Playing {
        return false;
    }
    let occupied = if bench { state.bench_research.is_some() } else { state.field_research.is_some() };
    if occupied {
        return false;
    }

    let projects = if bench {
        available_bench_projects(state)
    } else {
        available_field_projects(state)
    };

    if let Some(kind) = projects.get(project_idx) {
        let (rp_cost, personnel, duration) = project_costs(kind);

        if state.resources.research_points >= rp_cost
            && state.personnel_available() >= personnel
        {
            let project = ResearchProject {
                kind: kind.clone(),
                progress: 0.0,
                required_ticks: duration,
                personnel_assigned: personnel,
                rp_cost,
            };
            state.resources.research_points -= rp_cost;

            if bench {
                state.bench_research = Some(project);
            } else {
                state.field_research = Some(project);
            }
            return true;
        }
    }
    false
}

/// Boost an active research project. Pure game logic — does NOT modify UI state.
///
/// Returns an optional status message.
fn boost_research(state: &mut GameState, bench: bool) -> Option<String> {
    let project = if bench { &mut state.bench_research } else { &mut state.field_research };
    if let Some(project) = project {
        if !project.is_complete() && state.resources.research_points >= BOOST_RP_COST {
            state.resources.research_points -= BOOST_RP_COST;
            project.progress = (project.progress + BOOST_TICKS).min(project.required_ticks);
            Some(format!(
                "Boosted research! (-{:.0} RP, +{:.0} ticks)",
                BOOST_RP_COST, BOOST_TICKS
            ))
        } else if state.resources.research_points < BOOST_RP_COST {
            Some(format!(
                "Need {:.0} RP to boost (have {:.0})",
                BOOST_RP_COST, state.resources.research_points
            ))
        } else {
            None
        }
    } else {
        None
    }
}

fn handle_research_confirm(state: &mut GameState) {
    let research_ui = state.ui.research_ui.clone();
    match research_ui {
        Some(ResearchUiState::BrowseCategories) => {
            let bench = state.ui.panel_selection == 1;
            state.ui.research_ui = Some(ResearchUiState::BrowseProjects { bench });
            state.ui.panel_selection = 0;
        }
        Some(ResearchUiState::BrowseProjects { bench }) => {
            let sel = state.ui.panel_selection;
            let active = if bench { &state.bench_research } else { &state.field_research };

            if active.is_some() {
                state.ui.research_ui = Some(ResearchUiState::ViewActive { bench });
                state.ui.panel_selection = 0;
            } else {
                let count = if bench {
                    available_bench_projects(state).len()
                } else {
                    available_field_projects(state).len()
                };
                if count > 0 {
                    state.ui.research_ui = Some(ResearchUiState::ConfirmProject { bench, project_idx: sel });
                    state.ui.panel_selection = 0;
                }
            }
        }
        Some(ResearchUiState::ConfirmProject { bench, project_idx }) => {
            if start_research(state, bench, project_idx) {
                state.ui.research_ui = Some(ResearchUiState::BrowseProjects { bench });
                state.ui.panel_selection = 0;
            }
        }
        Some(ResearchUiState::ViewActive { bench }) => {
            state.ui.status_message = boost_research(state, bench);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;

    /// Helper: unlock all medicines and mark them tested (for tests that predate the research system).
    fn unlock_all_medicines(state: &mut GameState) {
        for med in &mut state.medicines {
            med.unlocked = true;
            med.tested_against = med.target_diseases.clone();
        }
    }

    #[test]
    fn tick_increases_infections() {
        let state = GameState::new_default(42);
        let initial = state.total_infected();
        let after = tick(&state);
        assert!(
            after.total_infected() > initial,
            "infections should grow: {} -> {}",
            initial,
            after.total_infected()
        );
    }

    #[test]
    fn tick_causes_deaths() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..20 {
            s = tick(&s);
        }
        assert!(s.total_dead() > 0.0, "should have some deaths after 20 ticks");
    }

    #[test]
    fn tick_advances_state() {
        let state = GameState::new_default(42);
        let after = tick(&state);
        assert_eq!(after.tick, state.tick + 1);
        assert!(after.total_infected() > state.total_infected());
    }

    #[test]
    fn multi_tick_determinism() {
        let state = GameState::new_default(42);
        let mut a = state.clone();
        let mut b = state;
        for _ in 0..50 {
            a = tick(&a);
            b = tick(&b);
        }
        assert_eq!(a.total_infected(), b.total_infected());
        assert_eq!(a.total_dead(), b.total_dead());
        assert_eq!(a.total_immune(), b.total_immune());
    }

    #[test]
    fn recovery_accumulates() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..50 {
            s = tick(&s);
        }
        assert!(
            s.total_immune() > 0.0,
            "should have immune (recovered) after 50 ticks, got {}",
            s.total_immune()
        );
    }

    #[test]
    fn population_conservation() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..100 {
            s = tick(&s);
        }
        for region in &s.regions {
            let pop = region.population as f64;
            for inf in &region.infections {
                let accounted = inf.infected + inf.immune + inf.dead;
                assert!(
                    accounted <= pop + 1.0,
                    "region {} disease {}: accounted {} > population {}",
                    region.name,
                    inf.disease_idx,
                    accounted,
                    pop
                );
                assert!(
                    inf.infected >= 0.0 && inf.immune >= 0.0 && inf.dead >= 0.0,
                    "region {} disease {}: negative values: infected={}, immune={}, dead={}",
                    region.name,
                    inf.disease_idx,
                    inf.infected,
                    inf.immune,
                    inf.dead
                );
            }
        }
    }

    #[test]
    fn cross_region_spread_eventually() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..200 {
            s = tick(&s);
        }
        let infected_regions = s
            .regions
            .iter()
            .filter(|r| !r.infections.is_empty())
            .count();
        assert!(
            infected_regions > 1,
            "disease should spread to more than 1 region after 200 ticks, got {}",
            infected_regions
        );
    }

    #[test]
    fn toggle_pause() {
        let state = GameState::new_default(42);
        assert!(!state.paused);
        let s = apply_action(&state, &Action::TogglePause);
        assert!(s.paused);
        let s = apply_action(&s, &Action::TogglePause);
        assert!(!s.paused);
    }

    #[test]
    fn open_close_panels() {
        let state = GameState::new_default(42);
        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::Threats);
        let s = apply_action(&s, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::None);
        let s = apply_action(&s, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::Threats);
        let s = apply_action(&s, &Action::ClosePanel);
        assert_eq!(s.ui.open_panel, Panel::None);
    }

    #[test]
    fn panel_navigation() {
        use crate::state::Disease;

        let mut state = GameState::new_default(42);
        // Add a third disease so we can test navigation bounds
        state.diseases.push(Disease {
            name: "Strain Gamma".into(),
            pathogen_type: crate::state::PathogenType::DnaVirus,
            infectivity: 0.1,
            lethality: 0.01,
            cross_region_spread: 0.005,
            recovery_rate: 0.05,
            knowledge: 0.0,
            strain_generation: 0,
        });

        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.panel_selection, 0);
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 1);
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 2);
        // Can't go past the last item
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 2);
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.panel_selection, 1);
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.panel_selection, 0);
        // Can't go below 0
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.panel_selection, 0);
    }

    #[test]
    fn immune_reduces_susceptible_pool() {
        let mut state = GameState::new_default(42);
        state.regions[4].infections[0].immune = 4_000_000_000.0;
        let before = state.regions[4].infections[0].infected;
        let after = tick(&state);
        let growth = after.regions[4].infections[0].infected - before;

        let state2 = GameState::new_default(42);
        let after2 = tick(&state2);
        let growth2 = after2.regions[4].infections[0].infected
            - state2.regions[4].infections[0].infected;

        assert!(
            growth < growth2,
            "immunity should reduce infection growth: {} vs {}",
            growth,
            growth2
        );
    }

    #[test]
    fn disease_can_spread_into_vaccinated_region() {
        let mut state = GameState::new_default(42);
        state.regions[0].infections.push(RegionDiseaseState {
            disease_idx: 0,
            infected: 0.0,
            dead: 0.0,
            immune: 100_000_000.0,
        });
        let mut s = state;
        for _ in 0..200 {
            s = tick(&s);
        }
        let na_imm = s.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .map(|i| i.immune)
            .unwrap_or(0.0);
        assert!(
            na_imm >= 100_000_000.0,
            "immune count should be preserved"
        );
    }

    #[test]
    fn medicine_vaccination_deployment() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        assert_eq!(state.ui.open_panel, Panel::Medicines);
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
        ));
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectTarget { .. })
        ));
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.resources.funding, funding_before - 200.0);
        let na_inf = state.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .unwrap();
        assert_eq!(na_inf.immune, 100_000.0);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
        ));
        // Deployment feedback message should be set
        let msg = state.ui.status_message.as_ref().expect("status message should be set after deploy");
        assert!(msg.contains("Vaccinated"), "message should mention vaccination: {msg}");
        assert!(msg.contains("North America"), "message should mention region: {msg}");
        assert!(msg.contains("Antiviral-A"), "message should mention medicine: {msg}");
    }

    #[test]
    fn medicine_treatment_deployment() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        for _ in 0..20 {
            state = tick(&state);
        }
        let asia_infected_before = state.regions[4].infections[0].infected;

        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        for _ in 0..4 {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::SelectNext);
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);

        let asia_infected_after = state.regions[4].infections[0].infected;
        assert!(
            asia_infected_after < asia_infected_before,
            "treatment should reduce infected: {} -> {}",
            asia_infected_before,
            asia_infected_after
        );
        assert_eq!(state.resources.funding, funding_before - 200.0);
    }

    #[test]
    fn medicine_insufficient_funds() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.resources.funding = 50.0;
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.resources.funding, funding_before);
        // Should show error message and stay on SelectTarget
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("Insufficient funds"),
            "expected insufficient funds message, got: {:?}",
            state.ui.status_message
        );
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectTarget { .. })),
            "should stay on SelectTarget, got: {:?}",
            state.ui.medicine_ui
        );
    }

    #[test]
    fn untested_medicine_insufficient_funds_skips_warning() {
        let mut state = GameState::new_default(42);
        unlock_untested(&mut state);
        state.resources.funding = 50.0; // Not enough for any medicine
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // select target
        // Should show funds error, NOT the untested warning
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("Insufficient funds"),
            "expected funds error, got: {:?}",
            state.ui.status_message
        );
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectTarget { .. })),
            "should stay on SelectTarget, not go to ConfirmDeploy, got: {:?}",
            state.ui.medicine_ui
        );
        assert_eq!(state.resources.funding, funding_before);
    }

    #[test]
    fn medicine_esc_backstep() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { .. })
        ));
        state = apply_action(&state, &Action::ClosePanel);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
        assert!(state.ui.medicine_ui.is_none());
    }

    #[test]
    fn medicine_zero_targets_refused() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        // Deploy to North America (region 0) which has no infections for disease 0
        let infections_before = state.regions[0].infections.len();
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        state = apply_action(&state, &Action::Confirm); // select region 0 (NA)
        state = apply_action(&state, &Action::SelectNext); // Treat option
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert_eq!(state.resources.funding, funding_before);
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("No infected"),
            "expected zero-target message, got: {:?}",
            state.ui.status_message
        );
        // Should NOT create a ghost disease entry
        assert_eq!(
            state.regions[0].infections.len(),
            infections_before,
            "failed deployment should not create ghost disease entry"
        );
    }

    #[test]
    fn open_medicines_resets_to_browse() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm);
        state = apply_action(&state, &Action::OpenThreats);
        state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        assert_eq!(state.ui.panel_selection, 0);
    }

    /// Helper: unlock medicines but leave them untested.
    fn unlock_untested(state: &mut GameState) {
        for med in &mut state.medicines {
            med.unlocked = true;
        }
    }

    #[test]
    fn untested_medicine_requires_confirmation() {
        let mut state = GameState::new_default(42);
        unlock_untested(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine 0
        state = apply_action(&state, &Action::Confirm); // select region 0 (NA)
        // Confirm target → should go to ConfirmDeploy, NOT deploy
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::ConfirmDeploy { .. })),
            "untested medicine should show confirmation, got {:?}",
            state.ui.medicine_ui
        );
        assert_eq!(state.resources.funding, funding_before, "should not have deployed yet");

        // Confirm again → actually deploys
        state = apply_action(&state, &Action::Confirm);
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectRegion { .. })),
            "should return to SelectRegion after deploy"
        );
        assert!(state.resources.funding < funding_before, "should have spent funding");
    }

    #[test]
    fn untested_medicine_cancel_returns_to_target() {
        let mut state = GameState::new_default(42);
        unlock_untested(&mut state);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        state = apply_action(&state, &Action::Confirm); // → ConfirmDeploy
        assert!(matches!(state.ui.medicine_ui, Some(MedicineUiState::ConfirmDeploy { .. })));

        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::ClosePanel); // cancel
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectTarget { .. })),
            "Esc should return to SelectTarget"
        );
        assert_eq!(state.resources.funding, funding_before, "should not have deployed");
    }

    #[test]
    fn tested_medicine_deploys_immediately() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state); // tested
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // deploy immediately
        assert!(
            matches!(state.ui.medicine_ui, Some(MedicineUiState::SelectRegion { .. })),
            "tested medicine should deploy without confirmation"
        );
        assert!(state.resources.funding < funding_before);
    }

    #[test]
    fn map_navigation_right_left() {
        let state = GameState::new_default(42);
        assert_eq!(state.ui.map_selection, 0); // NA
        let s = apply_action(&state, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 2); // EU
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 4); // AS
        // Can't go past rightmost column
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 4);
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 2); // EU
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 0); // NA
        // Can't go past leftmost column
        let s = apply_action(&s, &Action::SelectLeft);
        assert_eq!(s.ui.map_selection, 0);
    }

    #[test]
    fn map_navigation_up_down_no_panel() {
        let state = GameState::new_default(42);
        assert_eq!(state.ui.map_selection, 0); // NA (row 0)
        let s = apply_action(&state, &Action::SelectNext);
        assert_eq!(s.ui.map_selection, 1); // SA (row 1)
        // Can't go past bottom row
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.map_selection, 1);
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.map_selection, 0); // NA
        // Can't go past top row
        let s = apply_action(&s, &Action::SelectPrev);
        assert_eq!(s.ui.map_selection, 0);
    }

    #[test]
    fn map_navigation_with_panel_open() {
        let state = GameState::new_default(42);
        // Open threats panel — up/down should navigate panel, not map
        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.map_selection, 0);
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 1); // panel navigated
        assert_eq!(s.ui.map_selection, 0); // map unchanged
        // But left/right should still navigate map
        let s = apply_action(&s, &Action::SelectRight);
        assert_eq!(s.ui.map_selection, 2); // EU
        assert_eq!(s.ui.panel_selection, 1); // panel unchanged
    }

    #[test]
    fn research_identify_increases_knowledge() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;
        // Start identify project on disease 0
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #1
        state = apply_action(&state, &Action::Confirm); // Confirm start
        assert!(state.field_research.is_some());
        assert_eq!(state.diseases[0].knowledge, 0.0);

        // Advance to completion (20 ticks)
        for _ in 0..20 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none()); // Project completed
        assert!((state.diseases[0].knowledge - 0.50).abs() < 0.01);
    }

    #[test]
    fn research_develop_medicine_unlocks() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0; // Fully identified

        assert!(!state.medicines[0].unlocked);

        // Start bench research: Develop Antiviral-A
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::SelectNext); // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        state = apply_action(&state, &Action::Confirm);     // Select Develop Antiviral-A
        state = apply_action(&state, &Action::Confirm);     // Confirm

        assert!(state.bench_research.is_some());

        for _ in 0..40 {
            state = tick(&state);
        }
        assert!(state.bench_research.is_none());
        assert!(state.medicines[0].unlocked);
    }

    #[test]
    fn research_clinical_trial_marks_tested() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;
        state.medicines[0].unlocked = true; // Pre-unlock for testing

        assert!(state.medicines[0].tested_against.is_empty());

        // Start field research: Clinical Trial
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        // First two items are identify projects, then clinical trials
        state = apply_action(&state, &Action::SelectNext); // Skip identify #2
        state = apply_action(&state, &Action::SelectNext); // Clinical trial
        state = apply_action(&state, &Action::Confirm);    // Select
        state = apply_action(&state, &Action::Confirm);    // Confirm

        assert!(state.field_research.is_some());

        for _ in 0..25 {
            state = tick(&state);
        }
        assert!(state.field_research.is_none());
        assert!(state.medicines[0].tested_against.contains(&0));
    }

    #[test]
    fn research_insufficient_rp_blocks_start() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 0.0; // No RP

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify
        state = apply_action(&state, &Action::Confirm); // Try to confirm

        // Should not have started — still on confirm screen
        assert!(state.field_research.is_none());
    }

    #[test]
    fn research_panel_navigation() {
        let mut state = GameState::new_default(42);
        state = apply_action(&state, &Action::OpenResearch);

        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseCategories)));
        assert_eq!(state.ui.panel_selection, 0);

        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 1);

        // Can't go past last
        state = apply_action(&state, &Action::SelectNext);
        assert_eq!(state.ui.panel_selection, 1);

        // Esc closes
        state = apply_action(&state, &Action::ClosePanel);
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_esc_backstep() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseProjects { bench: false })));

        state = apply_action(&state, &Action::Confirm); // Select project
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ConfirmProject { .. })));

        state = apply_action(&state, &Action::ClosePanel); // Back to projects
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseProjects { .. })));

        state = apply_action(&state, &Action::ClosePanel); // Back to categories
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::BrowseCategories)));

        state = apply_action(&state, &Action::ClosePanel); // Close panel
        assert_eq!(state.ui.open_panel, Panel::None);
    }

    #[test]
    fn research_confirm_noop_on_empty_list() {
        let mut state = GameState::new_default(42);
        // Make all diseases fully known so no identify projects are available
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
        }
        // No medicines are unlocked, so no clinical trials either
        // => available_field_projects returns empty

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Enter Field Research
        assert!(matches!(
            state.ui.research_ui,
            Some(ResearchUiState::BrowseProjects { bench: false })
        ));

        // Pressing Enter on empty list should stay on BrowseProjects
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.research_ui,
            Some(ResearchUiState::BrowseProjects { bench: false })
        ));
    }

    #[test]
    fn research_boost_spends_rp_and_advances() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 100.0;

        // Start a field research project
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts project

        // Should be back at BrowseProjects with an active field project
        assert!(state.field_research.is_some());
        let progress_before = state.field_research.as_ref().unwrap().progress;
        let rp_before = state.resources.research_points;

        // Navigate to ViewActive and boost
        state = apply_action(&state, &Action::Confirm); // → ViewActive
        assert!(matches!(state.ui.research_ui, Some(ResearchUiState::ViewActive { bench: false })));

        state = apply_action(&state, &Action::Confirm); // Boost!
        assert_eq!(
            state.resources.research_points,
            rp_before - BOOST_RP_COST,
            "should spend {} RP", BOOST_RP_COST
        );
        assert_eq!(
            state.field_research.as_ref().unwrap().progress,
            progress_before + BOOST_TICKS,
            "should advance by {} ticks", BOOST_TICKS
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Boosted"));
    }

    #[test]
    fn research_boost_insufficient_rp() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 15.0; // Enough to start (10 RP) but not boost again

        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select first project
        state = apply_action(&state, &Action::Confirm); // Confirm → starts (costs 10 RP, leaves 5)

        assert!(state.field_research.is_some());
        assert_eq!(state.resources.research_points, 5.0);

        state = apply_action(&state, &Action::Confirm); // → ViewActive
        let rp_before = state.resources.research_points;
        let progress_before = state.field_research.as_ref().unwrap().progress;

        state = apply_action(&state, &Action::Confirm); // Try to boost — should fail
        assert_eq!(state.resources.research_points, rp_before, "should not spend RP");
        assert_eq!(
            state.field_research.as_ref().unwrap().progress,
            progress_before,
            "should not advance"
        );
        assert!(state.ui.status_message.as_ref().unwrap().contains("Need"));
    }

    #[test]
    fn diseases_start_unknown() {
        let state = GameState::new_default(42);
        for disease in &state.diseases {
            assert_eq!(disease.knowledge, 0.0);
        }
    }

    #[test]
    fn lose_condition_triggers_on_mass_death() {
        let mut state = GameState::new_default(42);
        // Run until game over
        for _ in 0..2000 {
            state = tick(&state);
            if state.outcome != GameOutcome::Playing {
                break;
            }
        }
        assert_eq!(state.outcome, GameOutcome::Lost);
        assert!(state.paused);
        // Deaths should be just over the threshold
        let threshold = state.initial_population() * LOSE_DEATH_FRACTION;
        assert!(state.total_dead() >= threshold);
    }

    #[test]
    fn win_requires_identified_diseases() {
        let mut state = GameState::new_default(42);
        // Clear all infections to simulate eradication
        for region in &mut state.regions {
            region.infections.clear();
        }
        // Diseases NOT identified — should not trigger win
        state = tick(&state);
        assert_eq!(state.outcome, GameOutcome::Playing);

        // Now identify all diseases
        for disease in &mut state.diseases {
            disease.knowledge = 1.0;
        }
        state = tick(&state);
        assert_eq!(state.outcome, GameOutcome::Won);
        assert!(state.paused);
    }

    #[test]
    fn no_research_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.resources.research_points = 100.0;
        let rp_before = state.resources.research_points;
        // Try to start research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select project
        state = apply_action(&state, &Action::Confirm); // Try to confirm
        assert!(state.field_research.is_none(), "should not start research after game over");
        assert_eq!(state.resources.research_points, rp_before, "should not spend RP after game over");
    }

    #[test]
    fn no_deploy_after_game_over() {
        let mut state = GameState::new_default(42);
        unlock_all_medicines(&mut state);
        state.outcome = GameOutcome::Lost;
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        state = apply_action(&state, &Action::Confirm); // try to deploy
        assert_eq!(state.resources.funding, funding_before, "should not spend funds after game over");
    }

    #[test]
    fn no_unpause_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.paused = true;
        let s = apply_action(&state, &Action::TogglePause);
        assert!(s.paused, "should not be able to unpause after game over");
    }

    #[test]
    fn tick_does_not_advance_after_game_over() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        let tick_before = state.tick;
        state = tick(&state);
        assert_eq!(state.tick, tick_before, "tick should not advance after game over");
    }

    #[test]
    fn tiny_infected_snaps_to_zero() {
        let mut state = GameState::new_default(42);
        // Set up a region with sub-person infected count
        state.regions[4].infections[0].infected = 0.3;
        state = tick(&state);
        // Should have snapped to 0
        assert_eq!(
            state.regions[4].infections[0].infected, 0.0,
            "infected below 0.5 should snap to zero"
        );
    }

    #[test]
    fn concurrent_field_and_bench_research() {
        let mut state = GameState::new_default(42);
        state.resources.research_points = 200.0;
        state.diseases[0].knowledge = 1.0;

        // Start field research
        state = apply_action(&state, &Action::OpenResearch);
        state = apply_action(&state, &Action::Confirm); // Field Research
        state = apply_action(&state, &Action::Confirm); // Select Identify #2
        state = apply_action(&state, &Action::Confirm); // Confirm
        assert!(state.field_research.is_some());

        // Start bench research
        state = apply_action(&state, &Action::ClosePanel); // Back to categories
        state = apply_action(&state, &Action::SelectNext);  // Bench Research
        state = apply_action(&state, &Action::Confirm);     // Enter Bench
        state = apply_action(&state, &Action::Confirm);     // Select Develop
        state = apply_action(&state, &Action::Confirm);     // Confirm
        assert!(state.bench_research.is_some());

        // Both running simultaneously
        assert!(state.field_research.is_some());
        assert!(state.bench_research.is_some());
    }

    #[test]
    fn policy_travel_ban_reduces_spread() {
        let mut state = GameState::new_default(42);
        // Run without travel ban
        let mut no_ban = state.clone();
        for _ in 0..100 {
            no_ban = tick(&no_ban);
        }
        let no_ban_regions_infected: usize = no_ban.regions.iter()
            .filter(|r| r.total_infected() > 0.0)
            .count();

        // Run with travel bans on all regions (with enough funding)
        state.resources.funding = 100_000.0;
        for p in &mut state.policies {
            p.travel_ban = true;
        }
        let mut with_ban = state;
        for _ in 0..100 {
            with_ban = tick(&with_ban);
        }
        let ban_regions_infected: usize = with_ban.regions.iter()
            .filter(|r| r.total_infected() > 0.0)
            .count();

        assert!(
            ban_regions_infected <= no_ban_regions_infected,
            "travel bans should not increase spread: {} vs {} regions infected",
            ban_regions_infected, no_ban_regions_infected
        );
    }

    #[test]
    fn policy_quarantine_reduces_infections() {
        let mut state = GameState::new_default(42);
        // Run without quarantine
        let mut no_q = state.clone();
        for _ in 0..50 {
            no_q = tick(&no_q);
        }

        // Run with quarantine on Asia (where Strain Alpha starts)
        state.policies[4].quarantine = true;
        let mut with_q = state;
        for _ in 0..50 {
            with_q = tick(&with_q);
        }

        assert!(
            with_q.regions[4].total_infected() < no_q.regions[4].total_infected(),
            "quarantine should reduce infections in Asia: {} vs {}",
            with_q.regions[4].total_infected(), no_q.regions[4].total_infected()
        );
    }

    #[test]
    fn policy_hospital_surge_reduces_deaths() {
        let mut state = GameState::new_default(42);
        // Run without hospital surge
        let mut no_h = state.clone();
        for _ in 0..50 {
            no_h = tick(&no_h);
        }

        // Run with hospital surge on Asia
        state.policies[4].hospital_surge = true;
        let mut with_h = state;
        for _ in 0..50 {
            with_h = tick(&with_h);
        }

        assert!(
            with_h.regions[4].total_dead() < no_h.regions[4].total_dead(),
            "hospital surge should reduce deaths in Asia: {} vs {}",
            with_h.regions[4].total_dead(), no_h.regions[4].total_dead()
        );
    }

    #[test]
    fn policy_costs_deducted_each_tick() {
        let mut state = GameState::new_default(42);
        state.policies[0].travel_ban = true; // $10/tick
        let initial_funding = state.resources.funding;
        state = tick(&state);
        // Should deduct $10 then add $5 passive income = net -$5
        assert_eq!(state.resources.funding, initial_funding - 10.0 + 5.0);
    }

    #[test]
    fn policy_funding_crisis_suspends_all() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 5.0; // Less than travel ban cost
        state.policies[0].travel_ban = true; // $10/tick
        state = tick(&state);
        // Should have suspended all policies
        assert!(!state.policies[0].travel_ban, "travel ban should be suspended");
        assert!(
            state.ui.status_message.as_ref().unwrap().contains("FUNDING CRISIS"),
            "should show funding crisis message"
        );
    }

    #[test]
    fn policy_toggle_via_confirm() {
        let mut state = GameState::new_default(42);
        state = apply_action(&state, &Action::OpenPolicy);
        assert_eq!(state.ui.open_panel, Panel::Policy);

        // Select Asia (index 4)
        for _ in 0..4 {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.policy_ui,
            Some(PolicyUiState::ManagePolicies { region_idx: 4 })
        ));

        // Toggle travel ban (selection 0)
        state = apply_action(&state, &Action::Confirm);
        assert!(state.policies[4].travel_ban);

        // Toggle it off
        state = apply_action(&state, &Action::Confirm);
        assert!(!state.policies[4].travel_ban);
    }

    #[test]
    fn disease_mutates_over_time() {
        let mut state = GameState::new_default(42);
        // RNA virus (Strain Alpha) has mutation_rate 0.008, so over 500 ticks
        // we expect ~4 mutations. Run enough ticks to virtually guarantee at least one.
        let original_infectivity = state.diseases[0].infectivity;
        for _ in 0..500 {
            state = tick(&state);
        }
        assert!(
            state.diseases[0].strain_generation > 0,
            "RNA virus should have mutated at least once in 500 ticks"
        );
        assert_ne!(
            state.diseases[0].infectivity, original_infectivity,
            "infectivity should have changed after mutation"
        );
    }

    #[test]
    fn mutation_is_deterministic() {
        let state = GameState::new_default(42);
        let mut a = state.clone();
        let mut b = state;
        for _ in 0..300 {
            a = tick(&a);
            b = tick(&b);
        }
        assert_eq!(a.diseases[0].strain_generation, b.diseases[0].strain_generation);
        assert_eq!(a.diseases[0].infectivity, b.diseases[0].infectivity);
        assert_eq!(a.diseases[0].lethality, b.diseases[0].lethality);
    }

    #[test]
    fn strain_efficacy_degrades_with_mutation() {
        use crate::state::{Disease, Medicine, TherapyType, PathogenType};

        let diseases = vec![Disease {
            name: "Test".into(),
            pathogen_type: PathogenType::RnaVirus,
            infectivity: 0.05,
            lethality: 0.01,
            cross_region_spread: 0.01,
            recovery_rate: 0.03,
            knowledge: 1.0,
            strain_generation: 3,
        }];

        let med = Medicine {
            name: "TestMed".into(),
            therapy_type: TherapyType::Antiviral,
            target_diseases: vec![0],
            cost: 100.0,
            doses: 1000.0,
            unlocked: true,
            tested_against: vec![0],
            strain_generations: vec![0], // calibrated at gen 0, disease is at gen 3
        };

        // 3 generations behind = 1.0 - 3*0.25 = 0.25
        let eff = med.strain_efficacy(0, &diseases);
        assert!((eff - 0.25).abs() < 0.001, "expected 0.25, got {eff}");

        // Re-calibrated medicine should have full efficacy
        let med_current = Medicine {
            strain_generations: vec![3],
            ..med.clone()
        };
        let eff2 = med_current.strain_efficacy(0, &diseases);
        assert!((eff2 - 1.0).abs() < 0.001, "expected 1.0, got {eff2}");
    }

    #[test]
    fn develop_medicine_sets_strain_generation() {
        let mut state = GameState::new_default(42);
        // Manually mutate disease 0 to gen 2
        state.diseases[0].strain_generation = 2;
        state.diseases[0].knowledge = 1.0;
        state.resources.research_points = 100.0;

        // Start and complete DevelopMedicine for medicine 0 (targets disease 0)
        state.bench_research = Some(ResearchProject {
            kind: ResearchKind::DevelopMedicine { medicine_idx: 0 },
            progress: 39.0, // will complete on next tick
            required_ticks: 40.0,
            personnel_assigned: 10,
            rp_cost: 30.0,
        });

        state = tick(&state);
        assert!(state.medicines[0].unlocked);
        assert_eq!(
            state.medicines[0].strain_generations,
            vec![2], // should match disease gen at time of completion
            "medicine should be calibrated to disease generation at completion"
        );
    }

    #[test]
    fn clinical_trial_updates_strain_generation() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 3;
        state.medicines[0].unlocked = true;
        state.medicines[0].strain_generations = vec![0]; // outdated
        state.resources.research_points = 100.0;

        state.field_research = Some(ResearchProject {
            kind: ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 },
            progress: 24.0, // will complete on next tick
            required_ticks: 25.0,
            personnel_assigned: 5,
            rp_cost: 15.0,
        });

        state = tick(&state);
        assert!(state.medicines[0].tested_against.contains(&0));
        // strain_generation should be updated to current disease gen
        // Note: disease might have mutated during this tick too, so check >= 3
        assert!(
            state.medicines[0].strain_generations[0] >= 3,
            "clinical trial should update strain calibration"
        );
    }

    #[test]
    fn outdated_strain_shows_retrial_available() {
        let mut state = GameState::new_default(42);
        state.diseases[0].strain_generation = 2;
        state.medicines[0].unlocked = true;
        state.medicines[0].tested_against = vec![0]; // already tested
        state.medicines[0].strain_generations = vec![0]; // but outdated

        let field_projects = available_field_projects(&state);
        let has_retrial = field_projects.iter().any(|k| matches!(k,
            ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 }
        ));
        assert!(has_retrial, "should offer clinical trial for strain-outdated medicine");
    }
}
