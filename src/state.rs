use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameState {
    pub tick: u64,
    pub paused: bool,
    pub rng: ChaCha8Rng,
    pub resources: Resources,
    pub regions: Vec<Region>,
    pub diseases: Vec<Disease>,
    pub ui: UiState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resources {
    pub funding: f64,
    pub research_points: f64,
    pub personnel: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub population: u64,
    pub connections: Vec<usize>,
    pub infections: Vec<RegionInfection>,
}

/// Per-disease infection state within a region.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionInfection {
    pub disease_idx: usize,
    pub infected: f64,
    pub dead: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disease {
    pub name: String,
    pub infectivity: f64,
    pub severity: f64,
    pub lethality: f64,
    pub cross_region_spread: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Panel {
    None,
    Threats,
    Research,
    Medicines,
    Policy,
    Help,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiState {
    pub open_panel: Panel,
    pub panel_selection: usize,
}

impl GameState {
    pub fn new_default(seed: u64) -> Self {
        let regions = vec![
            Region {
                name: "North America".into(),
                population: 500_000_000,
                connections: vec![1, 2, 5],
                infections: vec![],
            },
            Region {
                name: "South America".into(),
                population: 430_000_000,
                connections: vec![0, 2],
                infections: vec![],
            },
            Region {
                name: "Europe".into(),
                population: 750_000_000,
                connections: vec![0, 1, 3, 4],
                infections: vec![],
            },
            Region {
                name: "Africa".into(),
                population: 1_400_000_000,
                connections: vec![2, 4],
                infections: vec![],
            },
            Region {
                name: "Asia".into(),
                population: 4_700_000_000,
                connections: vec![2, 3, 5],
                infections: vec![RegionInfection {
                    disease_idx: 0,
                    infected: 1000.0,
                    dead: 0.0,
                }],
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![0, 4],
                infections: vec![],
            },
        ];

        let diseases = vec![Disease {
            name: "Strain Alpha".into(),
            infectivity: 0.15,
            severity: 0.05,
            lethality: 0.02,
            cross_region_spread: 0.01,
        }];

        Self {
            tick: 0,
            paused: true,
            rng: ChaCha8Rng::seed_from_u64(seed),
            resources: Resources {
                funding: 1000.0,
                research_points: 0.0,
                personnel: 50,
            },
            regions,
            diseases,
            ui: UiState {
                open_panel: Panel::None,
                panel_selection: 0,
            },
        }
    }

    pub fn total_infected(&self) -> f64 {
        self.regions
            .iter()
            .flat_map(|r| &r.infections)
            .map(|i| i.infected)
            .sum()
    }

    pub fn total_dead(&self) -> f64 {
        self.regions
            .iter()
            .flat_map(|r| &r.infections)
            .map(|i| i.dead)
            .sum()
    }

    pub fn total_population(&self) -> u64 {
        self.regions.iter().map(|r| r.population).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let state = GameState::new_default(42);
        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: GameState = serde_json::from_str(&json).unwrap();
        assert_eq!(state.tick, restored.tick);
        assert_eq!(state.regions.len(), restored.regions.len());
        assert_eq!(state.diseases.len(), restored.diseases.len());

        // Roundtrip again
        let json2 = serde_json::to_string_pretty(&restored).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn default_state_has_initial_infection() {
        let state = GameState::new_default(1);
        assert!(state.total_infected() > 0.0);
        assert_eq!(state.total_dead(), 0.0);
    }
}
