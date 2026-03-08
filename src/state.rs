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
    #[serde(default)]
    pub medicines: Vec<Medicine>,
    /// Active field research project (Identify Threat or Clinical Trial).
    #[serde(default)]
    pub field_research: Option<ResearchProject>,
    /// Active bench research project (Develop Medicine).
    #[serde(default)]
    pub bench_research: Option<ResearchProject>,
    #[serde(default)]
    pub outcome: GameOutcome,
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
    pub infections: Vec<RegionDiseaseState>,
}

impl Region {
    /// Current living population: starting population minus total deaths.
    pub fn alive(&self) -> f64 {
        self.population as f64 - self.total_dead()
    }

    pub fn total_infected(&self) -> f64 {
        self.infections.iter().map(|i| i.infected).sum()
    }

    pub fn total_dead(&self) -> f64 {
        self.infections.iter().map(|i| i.dead).sum()
    }

    pub fn total_immune(&self) -> f64 {
        self.infections.iter().map(|i| i.immune).sum()
    }

    pub fn disease_state(&self, disease_idx: usize) -> Option<&RegionDiseaseState> {
        self.infections.iter().find(|i| i.disease_idx == disease_idx)
    }
}

/// Per-disease state within a region: infection, deaths, and immunity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionDiseaseState {
    pub disease_idx: usize,
    pub infected: f64,
    pub dead: f64,
    #[serde(default)]
    pub immune: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disease {
    pub name: String,
    pub infectivity: f64,
    pub lethality: f64,
    pub cross_region_spread: f64,
    #[serde(default)]
    pub recovery_rate: f64,
    #[serde(default)]
    pub knowledge: f64,
}

/// Knowledge thresholds for progressive disease revelation.
pub const KNOWLEDGE_NAME: f64 = 0.33;
pub const KNOWLEDGE_PARTIAL_STATS: f64 = 0.66;
pub const KNOWLEDGE_FULL: f64 = 1.0;
/// Minimum knowledge required to develop a medicine targeting this disease.
pub const KNOWLEDGE_FOR_MEDICINE: f64 = 0.75;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Medicine {
    pub name: String,
    pub target_diseases: Vec<usize>,
    pub cost: f64,
    pub doses: f64,
    pub unlocked: bool,
    /// Disease indices this medicine has been clinically trialed against.
    #[serde(default)]
    pub tested_against: Vec<usize>,
}

/// What a medicine deployment targets: vaccinate susceptible or treat infected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeployTarget {
    Vaccinate { disease_idx: usize },
    Treat { disease_idx: usize },
}

impl Medicine {
    /// Number of target options in the UI (vaccinate + treat per target disease).
    pub fn num_deploy_targets(&self) -> usize {
        2 * self.target_diseases.len()
    }

    /// Decode a UI selection index into a deploy target.
    /// Indices 0..n are vaccinate options, n..2n are treat options.
    pub fn decode_deploy_target(&self, selection: usize) -> Option<DeployTarget> {
        let n = self.target_diseases.len();
        if selection < n {
            Some(DeployTarget::Vaccinate { disease_idx: self.target_diseases[selection] })
        } else if selection < 2 * n {
            Some(DeployTarget::Treat { disease_idx: self.target_diseases[selection - n] })
        } else {
            None
        }
    }
}

/// An active research project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResearchProject {
    pub kind: ResearchKind,
    pub progress: f64,
    pub required_ticks: f64,
    pub personnel_assigned: u32,
    pub rp_cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResearchKind {
    IdentifyThreat { disease_idx: usize },
    DevelopMedicine { medicine_idx: usize },
    ClinicalTrial { medicine_idx: usize, disease_idx: usize },
}

impl ResearchProject {
    pub fn is_complete(&self) -> bool {
        self.progress >= self.required_ticks
    }
}

/// Game outcome — checked each tick after simulation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum GameOutcome {
    #[default]
    Playing,
    Won,
    Lost,
}

/// Fraction of initial world population that, when dead, triggers game over.
pub const LOSE_DEATH_FRACTION: f64 = 0.10;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Panel {
    None,
    Threats,
    Research,
    Medicines,
    Policy,
    Help,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MedicineUiState {
    BrowseMedicines,
    SelectRegion { medicine_idx: usize },
    SelectTarget { medicine_idx: usize, region_idx: usize },
    ConfirmDeploy { medicine_idx: usize, region_idx: usize, target_selection: usize },
}

/// Research panel UI state machine, following the medicines panel pattern.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResearchUiState {
    /// Top level: choose Field Research or Bench Research category.
    BrowseCategories,
    /// Browsing available projects in the selected category.
    BrowseProjects { bench: bool },
    /// Confirming a project before starting it.
    ConfirmProject { bench: bool, project_idx: usize },
    /// Viewing the active project in a category.
    ViewActive { bench: bool },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiState {
    pub open_panel: Panel,
    pub panel_selection: usize,
    #[serde(default)]
    pub medicine_ui: Option<MedicineUiState>,
    #[serde(default)]
    pub map_selection: usize,
    #[serde(default)]
    pub research_ui: Option<ResearchUiState>,
    /// Temporary status message shown above the hotkey bar (cleared on next action).
    #[serde(default)]
    pub status_message: Option<String>,
}

/// Grid layout for the world map: 3 columns × 2 rows.
/// Maps region index to (col, row). Hardcoded for 6 regions.
const MAP_GRID: [(u16, u16); 6] = [
    (0, 0), // 0: North America
    (0, 1), // 1: South America
    (1, 0), // 2: Europe
    (1, 1), // 3: Africa
    (2, 0), // 4: Asia
    (2, 1), // 5: Oceania
];

pub const MAP_GRID_LEN: usize = MAP_GRID.len();

pub fn map_grid_pos(region_idx: usize) -> Option<(u16, u16)> {
    MAP_GRID.get(region_idx).copied()
}

pub fn region_at_grid(col: u16, row: u16) -> Option<usize> {
    MAP_GRID.iter().position(|&(c, r)| c == col && r == row)
}

/// Navigate the map selection in a direction. Returns the new selection index.
pub fn map_navigate(current: usize, direction: MapDirection, num_regions: usize) -> usize {
    if num_regions == 0 || current >= num_regions || current >= MAP_GRID.len() {
        return current;
    }
    let (col, row) = MAP_GRID[current];
    let (new_col, new_row) = match direction {
        MapDirection::Up => (col, row.wrapping_sub(1)),
        MapDirection::Down => (col, row + 1),
        MapDirection::Left => (col.wrapping_sub(1), row),
        MapDirection::Right => (col + 1, row),
    };
    region_at_grid(new_col, new_row)
        .filter(|&idx| idx < num_regions)
        .unwrap_or(current)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapDirection {
    Up,
    Down,
    Left,
    Right,
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
                infections: vec![RegionDiseaseState {
                    disease_idx: 1,
                    infected: 500.0,
                    dead: 0.0,
                    immune: 0.0,
                }],
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
                infections: vec![RegionDiseaseState {
                    disease_idx: 0,
                    infected: 50_000.0,
                    dead: 0.0,
                    immune: 0.0,
                }],
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![0, 4],
                infections: vec![],
            },
        ];

        let diseases = vec![
            Disease {
                name: "Strain Alpha".into(),
                infectivity: 0.06,
                lethality: 0.008,
                cross_region_spread: 0.02,
                recovery_rate: 0.04,
                knowledge: 0.0,
            },
            Disease {
                name: "Strain Beta".into(),
                infectivity: 0.04,
                lethality: 0.002,
                cross_region_spread: 0.03,
                recovery_rate: 0.015,
                knowledge: 0.0,
            },
        ];

        let medicines = vec![
            Medicine {
                name: "Antiviral-A".into(),
                target_diseases: vec![0],
                cost: 200.0,
                doses: 100_000.0,
                unlocked: false,
                tested_against: vec![],
            },
            Medicine {
                name: "Broad-Spectrum Antiviral".into(),
                target_diseases: vec![0, 1],
                cost: 500.0,
                doses: 500_000.0,
                unlocked: false,
                tested_against: vec![],
            },
        ];

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
            medicines,
            field_research: None,
            bench_research: None,
            outcome: GameOutcome::Playing,
            ui: UiState {
                open_panel: Panel::None,
                panel_selection: 0,
                medicine_ui: None,
                map_selection: 0,
                research_ui: None,
                status_message: None,
            },
        }
    }

    pub fn total_infected(&self) -> f64 {
        self.regions.iter().map(|r| r.total_infected()).sum()
    }

    pub fn total_dead(&self) -> f64 {
        self.regions.iter().map(|r| r.total_dead()).sum()
    }

    pub fn total_immune(&self) -> f64 {
        self.regions.iter().map(|r| r.total_immune()).sum()
    }

    pub fn personnel_busy(&self) -> u32 {
        let field = self.field_research.as_ref().map_or(0, |p| p.personnel_assigned);
        let bench = self.bench_research.as_ref().map_or(0, |p| p.personnel_assigned);
        field + bench
    }

    pub fn personnel_available(&self) -> u32 {
        self.resources.personnel.saturating_sub(self.personnel_busy())
    }

    /// Total initial population across all regions (before any deaths).
    pub fn initial_population(&self) -> f64 {
        self.regions.iter().map(|r| r.population as f64).sum()
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

    #[test]
    fn default_state_has_medicines() {
        let state = GameState::new_default(1);
        assert_eq!(state.medicines.len(), 2);
        // Medicines start locked — must be developed via research
        assert!(!state.medicines[0].unlocked);
        assert!(!state.medicines[1].unlocked);
    }
}
