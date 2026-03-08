use rand::Rng;

use crate::action::Action;
use crate::state::{DeployTarget, GameState, MedicineUiState, Panel, RegionDiseaseState};

/// Advance the simulation by one tick.
pub fn tick(state: &GameState) -> GameState {
    let mut new = state.clone();

    // Clone the RNG out so we can mutably borrow both `rng` and `new.regions`
    // simultaneously. Written back to `new.rng` at the end of the function.
    // WARNING: Do not use `new.rng` between here and the write-back line.
    let mut rng = new.rng.clone();

    // Disease spread within each region
    for region in &mut new.regions {
        let pop = region.population as f64;

        for inf in &mut region.infections {
            if let Some(disease) = state.diseases.get(inf.disease_idx) {
                // Each disease has its own independent susceptible pool. A person
                // infected with disease A can also be infected with disease B.
                // When displaying aggregate stats, the UI may need to estimate
                // "infected by any disease" (e.g. via inclusion-exclusion or capping).
                let susceptible = pop - inf.infected - inf.dead - inf.immune;
                if susceptible <= 0.0 {
                    continue;
                }

                let noise: f64 = 1.0 + (rng.r#gen::<f64>() - 0.5) * 0.1;
                let new_infections =
                    disease.infectivity * inf.infected * (susceptible / pop) * noise;
                let new_infections = new_infections.max(0.0).min(susceptible);

                let new_deaths = disease.lethality * inf.infected * noise;
                let new_deaths = new_deaths.max(0.0).min(inf.infected);

                inf.infected = inf.infected + new_infections - new_deaths;
                inf.dead += new_deaths;
            }
        }
    }

    // Cross-region spread
    let regions_snapshot: Vec<_> = new.regions.clone();
    for (i, region) in new.regions.iter_mut().enumerate() {
        for (d_idx, disease) in state.diseases.iter().enumerate() {
            let connected_infected: f64 = regions_snapshot[i]
                .connections
                .iter()
                .filter_map(|&conn_idx| {
                    regions_snapshot[conn_idx]
                        .infections
                        .iter()
                        .find(|inf| inf.disease_idx == d_idx)
                        .map(|inf| inf.infected)
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
                let chance = disease.cross_region_spread * (connected_infected / 1_000_000.0);
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

    // Passive resource generation
    new.resources.funding += 5.0;
    new.resources.research_points += 1.0;

    new.rng = rng;
    new.tick += 1;
    new
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

    match action {
        Action::TogglePause => {
            new.paused = !new.paused;
        }
        Action::OpenThreats => toggle_panel(&mut new.ui, Panel::Threats),
        Action::OpenResearch => toggle_panel(&mut new.ui, Panel::Research),
        Action::OpenMedicines => {
            toggle_panel(&mut new.ui, Panel::Medicines);
            if new.ui.open_panel == Panel::Medicines {
                new.ui.medicine_ui = Some(MedicineUiState::BrowseMedicines);
            } else {
                new.ui.medicine_ui = None;
            }
        }
        Action::OpenPolicy => toggle_panel(&mut new.ui, Panel::Policy),
        Action::OpenHelp => toggle_panel(&mut new.ui, Panel::Help),
        Action::ClosePanel => {
            match &new.ui.medicine_ui {
                Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                    new.ui.medicine_ui =
                        Some(MedicineUiState::SelectRegion { medicine_idx: *medicine_idx });
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
        }
        Action::SelectNext => {
            let max = match new.ui.open_panel {
                Panel::Threats => new.diseases.len().saturating_sub(1),
                Panel::Medicines => match &new.ui.medicine_ui {
                    Some(MedicineUiState::BrowseMedicines) => {
                        new.medicines.iter().filter(|m| m.unlocked).count().saturating_sub(1)
                    }
                    Some(MedicineUiState::SelectRegion { .. }) => {
                        new.regions.len().saturating_sub(1)
                    }
                    Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                        new.medicines[*medicine_idx].num_deploy_targets().saturating_sub(1)
                    }
                    None => 0,
                },
                _ => 0,
            };
            if new.ui.panel_selection < max {
                new.ui.panel_selection += 1;
            }
        }
        Action::SelectPrev => {
            if new.ui.panel_selection > 0 {
                new.ui.panel_selection -= 1;
            }
        }
        Action::Confirm => {
            if new.ui.open_panel == Panel::Medicines {
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
                        let med = &new.medicines[medicine_idx];
                        let cost = med.cost;
                        let doses = med.doses;
                        let target = med.decode_deploy_target(new.ui.panel_selection);

                        if let Some(target) = target {
                            if new.resources.funding >= cost {
                                let disease_idx = match &target {
                                    DeployTarget::Vaccinate { disease_idx } => *disease_idx,
                                    DeployTarget::Treat { disease_idx } => *disease_idx,
                                };

                                let region = &mut new.regions[region_idx];
                                let pop = region.population as f64;

                                // Find or create RegionDiseaseState entry
                                let inf_pos = region
                                    .infections
                                    .iter()
                                    .position(|i| i.disease_idx == disease_idx);
                                let inf_idx = if let Some(pos) = inf_pos {
                                    pos
                                } else {
                                    region.infections.push(RegionDiseaseState {
                                        disease_idx,
                                        infected: 0.0,
                                        dead: 0.0,
                                        immune: 0.0,
                                    });
                                    region.infections.len() - 1
                                };

                                let inf = &mut region.infections[inf_idx];

                                match target {
                                    DeployTarget::Vaccinate { .. } => {
                                        let susceptible =
                                            (pop - inf.infected - inf.dead - inf.immune).max(0.0);
                                        let actual = doses.min(susceptible);
                                        if actual > 0.0 {
                                            inf.immune += actual;
                                            new.resources.funding -= cost;
                                        }
                                    }
                                    DeployTarget::Treat { .. } => {
                                        let actual = doses.min(inf.infected);
                                        if actual > 0.0 {
                                            inf.infected -= actual;
                                            inf.immune += actual;
                                            new.resources.funding -= cost;
                                        }
                                    }
                                }
                            }
                        }

                        // Return to SelectRegion for rapid multi-region deployment
                        new.ui.medicine_ui =
                            Some(MedicineUiState::SelectRegion { medicine_idx });
                        new.ui.panel_selection = 0;
                    }
                    None => {}
                }
            }
        }
        Action::Quit => {} // Handled by the caller
    }

    new
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;

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
        // Run several ticks to accumulate deaths
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
    }

    #[test]
    fn cross_region_spread_eventually() {
        let state = GameState::new_default(42);
        let mut s = state;
        for _ in 0..200 {
            s = tick(&s);
        }
        // At least one region besides Asia should be infected
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
        assert!(state.paused);
        let s = apply_action(&state, &Action::TogglePause);
        assert!(!s.paused);
        let s = apply_action(&s, &Action::TogglePause);
        assert!(s.paused);
    }

    #[test]
    fn open_close_panels() {
        let state = GameState::new_default(42);
        // Open Threats
        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::Threats);
        // Press Threats again — should toggle closed
        let s = apply_action(&s, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::None);
        // Open again then close with Esc
        let s = apply_action(&s, &Action::OpenThreats);
        assert_eq!(s.ui.open_panel, Panel::Threats);
        let s = apply_action(&s, &Action::ClosePanel);
        assert_eq!(s.ui.open_panel, Panel::None);
    }

    #[test]
    fn panel_navigation() {
        use crate::state::Disease;

        let mut state = GameState::new_default(42);
        // Add a second disease so we can test navigation
        state.diseases.push(Disease {
            name: "Strain Beta".into(),
            infectivity: 0.1,
            lethality: 0.01,
            cross_region_spread: 0.005,
        });

        let s = apply_action(&state, &Action::OpenThreats);
        assert_eq!(s.ui.panel_selection, 0);
        let s = apply_action(&s, &Action::SelectNext);
        assert_eq!(s.ui.panel_selection, 1);
        // Can't go past the last item
        let s = apply_action(&s, &Action::SelectNext);
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
        // Give Asia a large immune population for disease 0
        state.regions[4].infections[0].immune = 4_000_000_000.0;
        let before = state.regions[4].infections[0].infected;
        let after = tick(&state);
        let growth = after.regions[4].infections[0].infected - before;

        // Compare with no immunity
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
        // Pre-vaccinate North America for disease 0 (no active infection)
        state.regions[0].infections.push(RegionDiseaseState {
            disease_idx: 0,
            infected: 0.0,
            dead: 0.0,
            immune: 100_000_000.0,
        });
        // Run many ticks — disease should still be able to spread there
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
        // Open medicines panel
        state = apply_action(&state, &Action::OpenMedicines);
        assert_eq!(state.ui.open_panel, Panel::Medicines);
        // Select first medicine (Antiviral-A, idx 0)
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
        ));
        // Select first region (North America, idx 0)
        state = apply_action(&state, &Action::Confirm);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectTarget { .. })
        ));
        // Select first target option (Vaccinate susceptible for disease 0)
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        // Should have deducted cost and added immune
        assert_eq!(state.resources.funding, funding_before - 100.0);
        let na_inf = state.regions[0]
            .infections
            .iter()
            .find(|i| i.disease_idx == 0)
            .unwrap();
        assert_eq!(na_inf.immune, 10_000.0);
        // Should be back at SelectRegion
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
        ));
    }

    #[test]
    fn medicine_treatment_deployment() {
        let mut state = GameState::new_default(42);
        // Run some ticks to build up infections in Asia
        for _ in 0..20 {
            state = tick(&state);
        }
        let asia_infected_before = state.regions[4].infections[0].infected;

        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select Antiviral-A
        // Navigate to Asia (index 4)
        for _ in 0..4 {
            state = apply_action(&state, &Action::SelectNext);
        }
        state = apply_action(&state, &Action::Confirm); // select Asia
        // Select "Treat infected" (index 1 for single-disease medicine)
        state = apply_action(&state, &Action::SelectNext);
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // deploy treatment

        let asia_infected_after = state.regions[4].infections[0].infected;
        assert!(
            asia_infected_after < asia_infected_before,
            "treatment should reduce infected: {} -> {}",
            asia_infected_before,
            asia_infected_after
        );
        assert_eq!(state.resources.funding, funding_before - 100.0);
    }

    #[test]
    fn medicine_insufficient_funds() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 50.0; // not enough for Antiviral-A ($100)
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        state = apply_action(&state, &Action::Confirm); // select region
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm); // try to deploy
        assert_eq!(state.resources.funding, funding_before); // unchanged
    }

    #[test]
    fn medicine_esc_backstep() {
        let mut state = GameState::new_default(42);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // -> SelectRegion
        state = apply_action(&state, &Action::Confirm); // -> SelectTarget
        state = apply_action(&state, &Action::ClosePanel); // back to SelectRegion
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::SelectRegion { .. })
        ));
        state = apply_action(&state, &Action::ClosePanel); // back to BrowseMedicines
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        state = apply_action(&state, &Action::ClosePanel); // close panel
        assert_eq!(state.ui.open_panel, Panel::None);
        assert!(state.ui.medicine_ui.is_none());
    }

    #[test]
    fn medicine_zero_targets_refused() {
        let mut state = GameState::new_default(42);
        // Try to treat in a region with 0 infected
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // select medicine
        // Region 0 (North America) has no infection
        state = apply_action(&state, &Action::Confirm); // select North America
        // Select "Treat infected" (index 1)
        state = apply_action(&state, &Action::SelectNext);
        let funding_before = state.resources.funding;
        state = apply_action(&state, &Action::Confirm);
        // Should not deduct — no infected to treat
        assert_eq!(state.resources.funding, funding_before);
    }

    #[test]
    fn open_medicines_resets_to_browse() {
        let mut state = GameState::new_default(42);
        state = apply_action(&state, &Action::OpenMedicines);
        state = apply_action(&state, &Action::Confirm); // -> SelectRegion
        // Switch away and back
        state = apply_action(&state, &Action::OpenThreats);
        state = apply_action(&state, &Action::OpenMedicines);
        assert!(matches!(
            state.ui.medicine_ui,
            Some(MedicineUiState::BrowseMedicines)
        ));
        assert_eq!(state.ui.panel_selection, 0);
    }
}
