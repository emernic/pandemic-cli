use rand::Rng;

use crate::action::Action;
use crate::state::{GameState, Panel, RegionInfection};

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
                let susceptible = pop - inf.infected - inf.dead;
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

            let has_infection = region
                .infections
                .iter()
                .any(|inf| inf.disease_idx == d_idx);

            if !has_infection {
                let roll: f64 = rng.r#gen();
                let chance = disease.cross_region_spread * (connected_infected / 1_000_000.0);
                if roll < chance.min(0.5) {
                    region.infections.push(RegionInfection {
                        disease_idx: d_idx,
                        infected: 1.0,
                        dead: 0.0,
                    });
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
        Action::OpenMedicines => toggle_panel(&mut new.ui, Panel::Medicines),
        Action::OpenPolicy => toggle_panel(&mut new.ui, Panel::Policy),
        Action::OpenHelp => toggle_panel(&mut new.ui, Panel::Help),
        Action::ClosePanel => {
            new.ui.open_panel = Panel::None;
            new.ui.panel_selection = 0;
        }
        Action::SelectNext => {
            let max = match new.ui.open_panel {
                Panel::Threats => new.diseases.len().saturating_sub(1),
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
            severity: 0.03,
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
}
