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
    /// Per-region active policies.
    #[serde(default)]
    pub policies: Vec<RegionPolicy>,
    #[serde(default)]
    pub outcome: GameOutcome,
    /// Events from the most recent tick. Consumed by the UI layer for status
    /// messages. Cleared at the start of each tick.
    #[serde(skip)]
    pub events: Vec<GameEvent>,
    pub ui: UiState,
}

// Policy cost constants — single source of truth.
pub const TRAVEL_BAN_COST: f64 = 10.0;
pub const QUARANTINE_COST: f64 = 8.0;
pub const QUARANTINE_PERSONNEL: u32 = 2;
pub const HOSPITAL_SURGE_COST: f64 = 5.0;
pub const HOSPITAL_SURGE_PERSONNEL: u32 = 2;

/// Per-region policy toggles. Each costs funding (and optionally personnel) per tick.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegionPolicy {
    /// Blocks 90% of cross-region spread to/from this region.
    /// Also halves the region's contribution to funding income.
    pub travel_ban: bool,
    /// Halves infection rate within the region.
    pub quarantine: bool,
    /// Halves lethality in the region.
    pub hospital_surge: bool,
}

impl RegionPolicy {
    pub fn funding_cost(&self) -> f64 {
        let mut cost = 0.0;
        if self.travel_ban { cost += TRAVEL_BAN_COST; }
        if self.quarantine { cost += QUARANTINE_COST; }
        if self.hospital_surge { cost += HOSPITAL_SURGE_COST; }
        cost
    }

    pub fn personnel_cost(&self) -> u32 {
        let mut cost = 0;
        if self.quarantine { cost += QUARANTINE_PERSONNEL; }
        if self.hospital_surge { cost += HOSPITAL_SURGE_PERSONNEL; }
        cost
    }

    pub fn any_active(&self) -> bool {
        self.travel_ban || self.quarantine || self.hospital_surge
    }

    pub fn clear_all(&mut self) {
        self.travel_ban = false;
        self.quarantine = false;
        self.hospital_surge = false;
    }
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

/// Fundamental category of pathogen — determines behavior characteristics
/// and which therapy types are effective.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathogenType {
    /// Fast-mutating, high infectivity, responds to antivirals
    RnaVirus,
    /// Slower-mutating, stable, responds to antivirals
    DnaVirus,
    /// Responds to antibiotics, can develop resistance
    Bacterium,
    /// Extremely slow-spreading but nearly untreatable
    Prion,
}

impl Default for PathogenType {
    fn default() -> Self {
        PathogenType::RnaVirus
    }
}

impl PathogenType {
    pub fn label(&self) -> &'static str {
        match self {
            PathogenType::RnaVirus => "RNA Virus",
            PathogenType::DnaVirus => "DNA Virus",
            PathogenType::Bacterium => "Bacterium",
            PathogenType::Prion => "Prion",
        }
    }

    /// Per-tick probability that this pathogen type mutates.
    pub fn mutation_rate(&self) -> f64 {
        match self {
            PathogenType::RnaVirus => 0.008,   // ~1 mutation per 125 ticks
            PathogenType::DnaVirus => 0.002,   // ~1 per 500 ticks
            PathogenType::Bacterium => 0.003,  // ~1 per 333 ticks
            PathogenType::Prion => 0.0001,     // ~1 per 10000 ticks
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disease {
    pub name: String,
    #[serde(default)]
    pub pathogen_type: PathogenType,
    pub infectivity: f64,
    pub lethality: f64,
    pub cross_region_spread: f64,
    #[serde(default)]
    pub recovery_rate: f64,
    #[serde(default)]
    pub knowledge: f64,
    /// How many times this disease has mutated. Medicines developed against
    /// earlier generations become less effective.
    #[serde(default)]
    pub strain_generation: u32,
}

impl Disease {
    /// Display name, respecting knowledge level. Unknown diseases show as
    /// "Unknown Pathogen #N" where N is the 1-based index.
    pub fn display_name(&self, idx: usize) -> String {
        if self.knowledge >= KNOWLEDGE_NAME {
            self.name.clone()
        } else {
            format!("Unknown Pathogen #{}", idx + 1)
        }
    }
}

/// Knowledge thresholds for progressive disease revelation.
pub const KNOWLEDGE_NAME: f64 = 0.33;
pub const KNOWLEDGE_PARTIAL_STATS: f64 = 0.66;
pub const KNOWLEDGE_FULL: f64 = 1.0;
/// Minimum knowledge required to develop a medicine targeting this disease.
pub const KNOWLEDGE_FOR_MEDICINE: f64 = 0.75;

/// Cost in RP to boost an active research project.
pub const BOOST_RP_COST: f64 = 10.0;
/// Ticks of progress added per boost.
pub const BOOST_TICKS: f64 = 5.0;

/// Category of therapeutic mechanism — determines efficacy against pathogen types.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TherapyType {
    /// Targets viral replication; effective against RNA and DNA viruses.
    Antiviral,
    /// Kills or inhibits bacteria; effective against bacterial pathogens.
    Antibiotic,
    /// Works across pathogen types but at reduced efficacy.
    BroadSpectrum,
}

impl Default for TherapyType {
    fn default() -> Self {
        TherapyType::BroadSpectrum
    }
}

impl TherapyType {
    pub fn label(&self) -> &'static str {
        match self {
            TherapyType::Antiviral => "Antiviral",
            TherapyType::Antibiotic => "Antibiotic",
            TherapyType::BroadSpectrum => "Broad-Spectrum",
        }
    }

    /// Efficacy multiplier when used against a given pathogen type (0.0–1.0).
    /// Determines what fraction of doses are effective during deployment.
    pub fn efficacy(&self, pathogen: &PathogenType) -> f64 {
        match (self, pathogen) {
            // Matched therapies: full efficacy
            (TherapyType::Antiviral, PathogenType::RnaVirus) => 1.0,
            (TherapyType::Antiviral, PathogenType::DnaVirus) => 0.8,
            (TherapyType::Antibiotic, PathogenType::Bacterium) => 1.0,
            // Broad-spectrum: partial efficacy against everything except prions
            (TherapyType::BroadSpectrum, PathogenType::Prion) => 0.1,
            (TherapyType::BroadSpectrum, _) => 0.5,
            // Mismatched: nearly useless
            (TherapyType::Antiviral, PathogenType::Bacterium) => 0.1,
            (TherapyType::Antibiotic, PathogenType::RnaVirus) => 0.1,
            (TherapyType::Antibiotic, PathogenType::DnaVirus) => 0.1,
            // Prions resist everything
            (_, PathogenType::Prion) => 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Medicine {
    pub name: String,
    #[serde(default)]
    pub therapy_type: TherapyType,
    pub target_diseases: Vec<usize>,
    pub cost: f64,
    pub doses: f64,
    /// Maximum doses this medicine can hold (set on creation, restored by manufacturing).
    /// Defaults to 0.0 on deserialization; `migrate()` sets it from `doses` for old saves.
    #[serde(default)]
    pub max_doses: f64,
    pub unlocked: bool,
    /// Disease indices this medicine has been clinically trialed against.
    #[serde(default)]
    pub tested_against: Vec<usize>,
    /// Strain generation this medicine was calibrated for, per target disease.
    /// Parallel to `target_diseases`. When a disease mutates past this generation,
    /// the medicine becomes less effective. Re-running a clinical trial updates this.
    #[serde(default)]
    pub strain_generations: Vec<u32>,
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

    /// Efficacy multiplier based on how many generations behind this medicine is
    /// for a given disease. Each generation of drift reduces efficacy by 25%,
    /// with a floor at 10%. Returns 1.0 if the medicine hasn't been calibrated yet
    /// (strain_generations not populated — pre-mutation-system medicines).
    pub fn strain_efficacy(&self, disease_idx: usize, diseases: &[Disease]) -> f64 {
        let pos = self.target_diseases.iter().position(|&d| d == disease_idx);
        match pos {
            Some(i) => {
                let med_gen = self.strain_generations.get(i).copied();
                match med_gen {
                    Some(mg) => {
                        let disease_gen = diseases.get(disease_idx)
                            .map_or(0, |d| d.strain_generation);
                        let behind = disease_gen.saturating_sub(mg);
                        (1.0 - behind as f64 * 0.25).max(0.1)
                    }
                    // Not yet calibrated (developed before mutation system) — full efficacy
                    None => 1.0,
                }
            }
            None => 1.0,
        }
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
    ManufactureDoses { medicine_idx: usize },
}

impl ResearchKind {
    /// Project costs: (rp_cost, personnel, duration_ticks).
    ///
    /// DevelopMedicine costs scale with medicine target count:
    /// narrow (1 target) is cheaper/faster, broad (2+ targets) is more expensive/slower.
    pub fn costs(&self, medicines: &[Medicine]) -> (f64, u32, f64) {
        match self {
            ResearchKind::IdentifyThreat { .. } => (10.0, 5, 20.0),
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let targets = medicines.get(*medicine_idx)
                    .map_or(1, |m| m.target_diseases.len());
                if targets <= 1 {
                    (15.0, 5, 25.0)   // narrow: fast and cheap
                } else {
                    (40.0, 10, 50.0)  // broad: slow and expensive
                }
            }
            ResearchKind::ClinicalTrial { .. } => (15.0, 5, 25.0),
            ResearchKind::ManufactureDoses { .. } => (10.0, 3, 15.0),
        }
    }
}

impl ResearchProject {
    pub fn is_complete(&self) -> bool {
        self.progress >= self.required_ticks
    }
}

/// Events generated by game simulation (tick). Read by the UI layer to
/// generate status messages, notifications, etc. Not persisted in saves.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GameEvent {
    /// All policies suspended due to insufficient funding.
    FundingCrisis,
    /// A disease mutated, changing its strain generation and stats.
    DiseaseMutated {
        disease_idx: usize,
        new_generation: u32,
    },
    /// The game just ended (win or lose). UI should pause and close panels.
    /// The actual outcome is on `GameState::outcome`; this just signals the transition.
    GameOver,
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
/// Win when total infected drops below this threshold (with other conditions met).
pub const WIN_INFECTED_THRESHOLD: f64 = 1000.0;

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

/// Policy panel UI state machine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyUiState {
    /// Browse regions and their active policies.
    BrowseRegions,
    /// Manage policies for a specific region.
    ManagePolicies { region_idx: usize },
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
    #[serde(default)]
    pub policy_ui: Option<PolicyUiState>,
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
        use rand::Rng;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        let mut regions = vec![
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
                infections: vec![],
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![0, 4],
                infections: vec![],
            },
        ];

        // Vary disease parameters ±30% from base values
        let vary = |rng: &mut ChaCha8Rng, base: f64| -> f64 {
            base * (0.7 + rng.r#gen::<f64>() * 0.6)
        };

        let diseases = vec![
            Disease {
                name: "Strain Alpha".into(),
                pathogen_type: PathogenType::RnaVirus,
                infectivity: vary(&mut rng, 0.06),
                lethality: vary(&mut rng, 0.008),
                cross_region_spread: vary(&mut rng, 0.02),
                recovery_rate: vary(&mut rng, 0.04),
                knowledge: 0.0,
                strain_generation: 0,
            },
            Disease {
                name: "Strain Beta".into(),
                pathogen_type: PathogenType::Bacterium,
                infectivity: vary(&mut rng, 0.04),
                lethality: vary(&mut rng, 0.002),
                cross_region_spread: vary(&mut rng, 0.03),
                recovery_rate: vary(&mut rng, 0.015),
                knowledge: 0.0,
                strain_generation: 0,
            },
        ];

        // Pick 2 different regions for initial infections
        let region_count = regions.len();
        let region_a = rng.r#gen::<usize>() % region_count;
        let mut region_b = rng.r#gen::<usize>() % (region_count - 1);
        if region_b >= region_a {
            region_b += 1;
        }

        // Primary outbreak: 10K-100K infected
        let infected_a = 10_000.0 + rng.r#gen::<f64>() * 90_000.0;
        regions[region_a].infections.push(RegionDiseaseState {
            disease_idx: 0,
            infected: infected_a,
            dead: 0.0,
            immune: 0.0,
        });

        // Secondary outbreak: 100-5K infected
        let infected_b = 100.0 + rng.r#gen::<f64>() * 4_900.0;
        regions[region_b].infections.push(RegionDiseaseState {
            disease_idx: 1,
            infected: infected_b,
            dead: 0.0,
            immune: 0.0,
        });

        let medicines = vec![
            Medicine {
                name: "Antiviral-A".into(),
                therapy_type: TherapyType::Antiviral,
                target_diseases: vec![0],
                cost: 200.0,
                doses: 100_000.0,
                max_doses: 100_000.0,
                unlocked: false,
                tested_against: vec![],
                strain_generations: vec![],
            },
            Medicine {
                name: "Antibiotic-B".into(),
                therapy_type: TherapyType::Antibiotic,
                target_diseases: vec![1],
                cost: 150.0,
                doses: 100_000.0,
                max_doses: 100_000.0,
                unlocked: false,
                tested_against: vec![],
                strain_generations: vec![],
            },
            Medicine {
                name: "Broad-Spectrum".into(),
                therapy_type: TherapyType::BroadSpectrum,
                target_diseases: vec![0, 1],
                cost: 400.0,
                doses: 200_000.0,
                max_doses: 200_000.0,
                unlocked: false,
                tested_against: vec![],
                strain_generations: vec![],
            },
        ];

        Self {
            tick: 0,
            paused: false,
            rng,
            resources: Resources {
                funding: 1000.0,
                research_points: 0.0,
                personnel: 50,
            },
            policies: vec![RegionPolicy::default(); regions.len()],
            regions,
            diseases,
            medicines,
            field_research: None,
            bench_research: None,
            outcome: GameOutcome::Playing,
            events: vec![],
            ui: UiState {
                open_panel: Panel::None,
                panel_selection: 0,
                medicine_ui: None,
                map_selection: 0,
                research_ui: None,
                policy_ui: None,
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
        let policy: u32 = self.policies.iter().map(|p| p.personnel_cost()).sum();
        field + bench + policy
    }

    pub fn personnel_available(&self) -> u32 {
        self.resources.personnel.saturating_sub(self.personnel_busy())
    }

    pub fn total_policy_funding_cost(&self) -> f64 {
        self.policies.iter().map(|p| p.funding_cost()).sum()
    }

    /// Estimated funding income per tick, based on current population health and policies.
    pub fn funding_income_rate(&self) -> f64 {
        let base_funding = 5.0;
        let total_pop: f64 = self.regions.iter().map(|r| r.population as f64).sum();
        if total_pop <= 0.0 {
            return 0.0;
        }
        let mut income = 0.0;
        for (i, region) in self.regions.iter().enumerate() {
            let pop = region.population as f64;
            let dead: f64 = region.infections.iter().map(|inf| inf.dead).sum();
            let healthy_frac = (pop - dead).max(0.0) / pop;
            let region_share = pop / total_pop;
            let travel_ban_factor = if self.policies.get(i).is_some_and(|p| p.travel_ban) {
                0.5
            } else {
                1.0
            };
            income += base_funding * region_share * healthy_frac * travel_ban_factor;
        }
        income
    }

    /// Total initial population across all regions (before any deaths).
    pub fn initial_population(&self) -> f64 {
        self.regions.iter().map(|r| r.population as f64).sum()
    }

    /// Run all save-migration fixups. Call once after deserializing a save file.
    pub fn migrate(&mut self) {
        // Ensure policies vec matches regions (for saves before the policy system).
        while self.policies.len() < self.regions.len() {
            self.policies.push(RegionPolicy::default());
        }
        // Backfill max_doses for saves before dose depletion.
        for med in &mut self.medicines {
            if med.max_doses == 0.0 && med.doses > 0.0 {
                med.max_doses = med.doses;
            }
        }
    }

    /// Available field research projects (excludes currently active).
    pub fn available_field_projects(&self) -> Vec<ResearchKind> {
        let active_kind = self.field_research.as_ref().map(|p| &p.kind);
        let mut projects = Vec::new();
        // Identify Threat: diseases not fully known
        for (i, disease) in self.diseases.iter().enumerate() {
            if disease.knowledge < KNOWLEDGE_FULL {
                let kind = ResearchKind::IdentifyThreat { disease_idx: i };
                if active_kind != Some(&kind) {
                    projects.push(kind);
                }
            }
        }
        // Clinical Trial: unlocked medicines not yet tested, OR tested but strain-outdated
        for (i, med) in self.medicines.iter().enumerate() {
            if !med.unlocked {
                continue;
            }
            for (target_pos, &d_idx) in med.target_diseases.iter().enumerate() {
                let needs_trial = if !med.tested_against.contains(&d_idx) {
                    true // Never tested
                } else {
                    // Tested, but check if strain has drifted
                    let med_gen = med.strain_generations.get(target_pos).copied().unwrap_or(0);
                    let disease_gen = self.diseases.get(d_idx)
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

    /// Available bench research projects (excludes currently active).
    pub fn available_bench_projects(&self) -> Vec<ResearchKind> {
        let active_kind = self.bench_research.as_ref().map(|p| &p.kind);
        let mut projects = Vec::new();
        for (i, med) in self.medicines.iter().enumerate() {
            if med.unlocked {
                // Unlocked medicines can be manufactured if doses are depleted
                if med.doses < med.max_doses {
                    let kind = ResearchKind::ManufactureDoses { medicine_idx: i };
                    if active_kind != Some(&kind) {
                        projects.push(kind);
                    }
                }
                continue;
            }
            let has_knowledge = med.target_diseases.iter().any(|&d_idx| {
                self.diseases.get(d_idx).map_or(false, |d| d.knowledge >= KNOWLEDGE_FOR_MEDICINE)
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

    /// Maximum selection index for the current panel and UI sub-state.
    /// Used by navigation (SelectNext) to bounds-check panel_selection.
    pub fn panel_selection_max(&self) -> usize {
        match self.ui.open_panel {
            Panel::Threats => self.diseases.len().saturating_sub(1),
            Panel::Medicines => match &self.ui.medicine_ui {
                Some(MedicineUiState::BrowseMedicines) => {
                    self.medicines
                        .iter()
                        .filter(|m| m.unlocked)
                        .count()
                        .saturating_sub(1)
                }
                Some(MedicineUiState::SelectRegion { .. }) => {
                    self.regions.len().saturating_sub(1)
                }
                Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                    self.medicines[*medicine_idx]
                        .num_deploy_targets()
                        .saturating_sub(1)
                }
                Some(MedicineUiState::ConfirmDeploy { .. }) | None => 0,
            },
            Panel::Research => match &self.ui.research_ui {
                Some(ResearchUiState::BrowseCategories) => 1,
                Some(ResearchUiState::BrowseProjects { bench }) => {
                    let active = if *bench {
                        self.bench_research.is_some()
                    } else {
                        self.field_research.is_some()
                    };
                    if active {
                        0
                    } else {
                        let count = if *bench {
                            self.available_bench_projects().len()
                        } else {
                            self.available_field_projects().len()
                        };
                        count.saturating_sub(1)
                    }
                }
                Some(ResearchUiState::ConfirmProject { .. }) => 0,
                Some(ResearchUiState::ViewActive { .. }) => 0,
                None => 0,
            },
            Panel::Policy => match &self.ui.policy_ui {
                Some(PolicyUiState::BrowseRegions) => {
                    self.regions.len().saturating_sub(1)
                }
                Some(PolicyUiState::ManagePolicies { .. }) => 2,
                None => 0,
            },
            _ => 0,
        }
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
        assert_eq!(state.medicines.len(), 3);
        // Medicines start locked — must be developed via research
        assert!(state.medicines.iter().all(|m| !m.unlocked));
        // Verify therapy types
        assert_eq!(state.medicines[0].therapy_type, TherapyType::Antiviral);
        assert_eq!(state.medicines[1].therapy_type, TherapyType::Antibiotic);
        assert_eq!(state.medicines[2].therapy_type, TherapyType::BroadSpectrum);
    }

    #[test]
    fn therapy_efficacy_matches() {
        // Matched therapies: full or near-full efficacy
        assert_eq!(TherapyType::Antiviral.efficacy(&PathogenType::RnaVirus), 1.0);
        assert_eq!(TherapyType::Antiviral.efficacy(&PathogenType::DnaVirus), 0.8);
        assert_eq!(TherapyType::Antibiotic.efficacy(&PathogenType::Bacterium), 1.0);

        // Mismatched: nearly useless
        assert_eq!(TherapyType::Antiviral.efficacy(&PathogenType::Bacterium), 0.1);
        assert_eq!(TherapyType::Antibiotic.efficacy(&PathogenType::RnaVirus), 0.1);

        // Broad-spectrum: partial efficacy
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::RnaVirus), 0.5);
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::Bacterium), 0.5);

        // Prions resist everything
        assert_eq!(TherapyType::Antiviral.efficacy(&PathogenType::Prion), 0.0);
        assert_eq!(TherapyType::Antibiotic.efficacy(&PathogenType::Prion), 0.0);
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::Prion), 0.1);
    }
}
