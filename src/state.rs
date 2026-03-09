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

// Disease emergence constants.
/// First new disease can emerge after this many ticks.
pub const EMERGENCE_MIN_TICK: u64 = 100;
/// Per-tick probability of a new disease emerging (after min tick).
pub const EMERGENCE_CHANCE_PER_TICK: f64 = 0.01; // ~1 every 100 ticks
/// Maximum number of simultaneous diseases.
pub const MAX_DISEASES: usize = 8;

// Policy cost constants — single source of truth.
pub const BASE_FUNDING_INCOME: f64 = 5.0;
pub const BASE_RP_INCOME: f64 = 0.4;
pub const TRAVEL_BAN_INCOME_PENALTY: f64 = 0.5;
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

    /// Biologically plausible stat ranges: (infectivity, lethality, recovery, cross_region).
    /// Each tuple is (min, max) for that stat.
    fn stat_ranges(&self) -> DiseaseStatRanges {
        match self {
            // RNA viruses: fast-spreading, variable lethality, quick recovery
            // Ranges tightened to ~60% of original width to reduce seed difficulty variance
            PathogenType::RnaVirus => DiseaseStatRanges {
                infectivity: (0.015, 0.027),
                lethality: (0.002, 0.006),
                recovery: (0.012, 0.018),
                cross_region: (0.007, 0.013),
            },
            // DNA viruses: moderate spread, higher lethality, slower recovery
            PathogenType::DnaVirus => DiseaseStatRanges {
                infectivity: (0.011, 0.018),
                lethality: (0.004, 0.008),
                recovery: (0.008, 0.012),
                cross_region: (0.005, 0.009),
            },
            // Bacteria: moderate spread, low lethality, moderate recovery
            PathogenType::Bacterium => DiseaseStatRanges {
                infectivity: (0.011, 0.019),
                lethality: (0.002, 0.004),
                recovery: (0.006, 0.011),
                cross_region: (0.006, 0.010),
            },
            // Prions: very slow but devastating, almost no recovery
            PathogenType::Prion => DiseaseStatRanges {
                infectivity: (0.003, 0.007),
                lethality: (0.007, 0.013),
                recovery: (0.001, 0.003),
                cross_region: (0.002, 0.004),
            },
        }
    }

    /// Name pools for procedural disease name generation.
    pub fn name_pool(&self) -> &'static [&'static str] {
        match self {
            PathogenType::RnaVirus => &[
                "CORVID", "H7N3 Avian Flu", "Marburg-X", "Nipah-7",
                "Zika Variant C", "RSV-Delta", "MERS-CoV-3", "Hantavirus Sigma",
                "Rift Valley Fever X", "Lassa-9", "Dengue-Phi",
            ],
            PathogenType::DnaVirus => &[
                "Variola Nova", "Monkeypox-Zeta", "Adeno-X47", "Herpes-Omega",
                "Papilloma-R3", "Mimivirus Delta", "Baculovirus-H",
            ],
            PathogenType::Bacterium => &[
                "Yersinia-Omega", "Vibrio Fortis", "Mycobacterium Sigma",
                "Burkholderia-X", "Clostridium Rex", "Rickettsia Tau",
                "Streptococcus Phi", "Klebsiella Nova",
            ],
            PathogenType::Prion => &[
                "PrP-Sigma Fold", "TSE-7 Variant", "Kuru-X Prion",
                "CJD-Delta", "Fatal Insomnia Tau", "BSE-Omega",
            ],
        }
    }

    /// The therapy type that's most effective against this pathogen.
    pub fn matched_therapy(&self) -> TherapyType {
        match self {
            PathogenType::RnaVirus | PathogenType::DnaVirus => TherapyType::Antiviral,
            PathogenType::Bacterium => TherapyType::Antibiotic,
            PathogenType::Prion => TherapyType::BroadSpectrum,
        }
    }
}

/// Stat ranges for procedural disease generation.
struct DiseaseStatRanges {
    infectivity: (f64, f64),
    lethality: (f64, f64),
    recovery: (f64, f64),
    cross_region: (f64, f64),
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
    /// Number of times genomic sequencing has been completed on this disease.
    /// Each sequencing halves the effective mutation rate.
    #[serde(default)]
    pub sequencing_count: u32,
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

    /// Effective mutation rate after genomic sequencing reductions.
    /// Each sequencing halves the rate: base_rate * 0.5^sequencing_count.
    pub fn effective_mutation_rate(&self) -> f64 {
        self.pathogen_type.mutation_rate() * 0.5_f64.powi(self.sequencing_count as i32)
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
    /// Sequence a disease's genome to slow its mutation rate permanently.
    GenomicSequencing { disease_idx: usize },
    /// Train new personnel to expand the available workforce.
    TrainPersonnel,
}

impl ResearchKind {
    /// Project costs: (rp_cost, personnel, duration_ticks).
    ///
    /// DevelopMedicine costs scale with medicine target count:
    /// narrow (1 target) is cheaper/faster, broad (2+ targets) is more expensive/slower.
    pub fn costs(&self, medicines: &[Medicine]) -> (f64, u32, f64) {
        match self {
            ResearchKind::IdentifyThreat { .. } => (15.0, 5, 80.0),
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let targets = medicines.get(*medicine_idx)
                    .map_or(1, |m| m.target_diseases.len());
                if targets <= 1 {
                    (20.0, 3, 100.0)  // narrow: fast and cheap, single-target
                } else {
                    (60.0, 10, 250.0) // broad: slow and expensive, covers all
                }
            }
            ResearchKind::ClinicalTrial { .. } => (20.0, 5, 80.0),
            ResearchKind::ManufactureDoses { .. } => (15.0, 3, 60.0),
            ResearchKind::GenomicSequencing { .. } => (25.0, 5, 120.0),
            ResearchKind::TrainPersonnel => (20.0, 0, 100.0),
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
    /// An individual policy was suspended due to insufficient funding.
    PolicySuspended {
        region_idx: usize,
        policy_name: String,
    },
    /// Funding is low — player has only a few ticks of policy runway left.
    FundingWarning,
    /// A disease mutated, changing its strain generation and stats.
    DiseaseMutated {
        disease_idx: usize,
        new_generation: u32,
    },
    /// A new disease emerged mid-game. UI should notify the player.
    NewDiseaseEmerged {
        disease_idx: usize,
        region_idx: usize,
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

/// A game command produced by UI wizard completion. The engine executes these
/// without knowing about wizard steps, panel states, or selection indices.
#[derive(Clone, Debug, PartialEq)]
pub enum GameCommand {
    DeployMedicine {
        medicine_idx: usize,
        region_idx: usize,
        target_selection: usize,
    },
    StartResearch {
        bench: bool,
        project_idx: usize,
    },
    BoostResearch {
        bench: bool,
    },
    TogglePolicy {
        region_idx: usize,
        policy_idx: usize,
    },
}

/// Fraction of initial world population that, when dead, triggers game over.
pub const LOSE_DEATH_FRACTION: f64 = 0.10;
/// Win when total infected drops below this threshold (with other conditions met).
/// Individual region infections snap to 0.0 at < 1.0, so this means "truly eradicated."
pub const WIN_INFECTED_THRESHOLD: f64 = 1.0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

impl UiState {
    /// Toggle a panel open/closed. Resets selection and initializes panel-specific UI state.
    pub fn toggle_panel(&mut self, panel: Panel) {
        if self.open_panel == panel {
            self.open_panel = Panel::None;
            self.panel_selection = 0;
            match panel {
                Panel::Medicines => self.medicine_ui = None,
                Panel::Research => self.research_ui = None,
                Panel::Policy => self.policy_ui = None,
                _ => {}
            }
        } else {
            self.open_panel = panel;
            self.panel_selection = 0;
            match panel {
                Panel::Medicines => self.medicine_ui = Some(MedicineUiState::BrowseMedicines),
                Panel::Research => self.research_ui = Some(ResearchUiState::BrowseCategories),
                Panel::Policy => self.policy_ui = Some(PolicyUiState::BrowseRegions),
                _ => {}
            }
        }
    }

    /// Handle Escape — go back one step in the current panel's wizard, or close the panel.
    pub fn close_panel(&mut self) {
        match self.open_panel {
            Panel::Medicines => {
                match self.medicine_ui.clone() {
                    Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, target_selection }) => {
                        self.medicine_ui = Some(MedicineUiState::SelectTarget {
                            medicine_idx,
                            region_idx,
                        });
                        self.panel_selection = target_selection;
                    }
                    Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                        self.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                        self.panel_selection = 0;
                    }
                    Some(MedicineUiState::SelectRegion { .. }) => {
                        self.medicine_ui = Some(MedicineUiState::BrowseMedicines);
                        self.panel_selection = 0;
                    }
                    _ => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.medicine_ui = None;
                    }
                }
            }
            Panel::Policy => {
                match &self.policy_ui {
                    Some(PolicyUiState::ManagePolicies { .. }) => {
                        self.policy_ui = Some(PolicyUiState::BrowseRegions);
                        self.panel_selection = 0;
                    }
                    _ => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.policy_ui = None;
                    }
                }
            }
            Panel::Research => {
                match &self.research_ui {
                    Some(ResearchUiState::ConfirmProject { bench, .. }) => {
                        self.research_ui = Some(ResearchUiState::BrowseProjects { bench: *bench });
                        self.panel_selection = 0;
                    }
                    Some(ResearchUiState::ViewActive { bench }) => {
                        self.research_ui = Some(ResearchUiState::BrowseProjects { bench: *bench });
                        self.panel_selection = 0;
                    }
                    Some(ResearchUiState::BrowseProjects { .. }) => {
                        self.research_ui = Some(ResearchUiState::BrowseCategories);
                        self.panel_selection = 0;
                    }
                    _ => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.research_ui = None;
                    }
                }
            }
            _ => {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                self.medicine_ui = None;
                self.research_ui = None;
                self.policy_ui = None;
            }
        }
    }


    /// Navigate down (in map) or to the next item (in a panel).
    /// `num_regions` is needed for map navigation; `panel_max` bounds panel selection.
    pub fn select_next(&mut self, num_regions: usize, panel_max: usize) {
        if self.open_panel == Panel::None {
            self.map_selection = map_navigate(
                self.map_selection,
                MapDirection::Down,
                num_regions,
            );
        } else if self.panel_selection < panel_max {
            self.panel_selection += 1;
        }
    }

    /// Navigate up (in map) or to the previous item (in a panel).
    pub fn select_prev(&mut self, num_regions: usize) {
        if self.open_panel == Panel::None {
            self.map_selection = map_navigate(
                self.map_selection,
                MapDirection::Up,
                num_regions,
            );
        } else if self.panel_selection > 0 {
            self.panel_selection -= 1;
        }
    }

    /// Navigate left on the map.
    pub fn select_left(&mut self, num_regions: usize) {
        self.map_selection = map_navigate(
            self.map_selection,
            MapDirection::Left,
            num_regions,
        );
    }

    /// Navigate right on the map.
    pub fn select_right(&mut self, num_regions: usize) {
        self.map_selection = map_navigate(
            self.map_selection,
            MapDirection::Right,
            num_regions,
        );
    }

    /// Handle a Confirm keypress. Advances wizard state machines and returns
    /// a GameCommand when the wizard completes and a game action should fire.
    /// All UI transitions happen here; the engine only executes returned commands.
    pub fn handle_confirm(&mut self, state: &GameState) -> Option<GameCommand> {
        match self.open_panel {
            Panel::Medicines => self.handle_medicine_confirm(state),
            Panel::Research => self.handle_research_confirm(state),
            Panel::Policy => self.handle_policy_confirm(state),
            _ => None,
        }
    }

    fn handle_medicine_confirm(&mut self, state: &GameState) -> Option<GameCommand> {
        match self.medicine_ui.clone() {
            Some(MedicineUiState::BrowseMedicines) => {
                let unlocked: Vec<usize> = state
                    .medicines
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.unlocked)
                    .map(|(i, _)| i)
                    .collect();
                if let Some(&med_idx) = unlocked.get(self.panel_selection) {
                    self.medicine_ui =
                        Some(MedicineUiState::SelectRegion { medicine_idx: med_idx });
                    self.panel_selection = 0;
                }
                None
            }
            Some(MedicineUiState::SelectRegion { medicine_idx }) => {
                let region_idx = self.panel_selection;
                if region_idx < state.regions.len() {
                    self.medicine_ui = Some(MedicineUiState::SelectTarget {
                        medicine_idx,
                        region_idx,
                    });
                    self.panel_selection = 0;
                }
                None
            }
            Some(MedicineUiState::SelectTarget {
                medicine_idx,
                region_idx,
            }) => {
                let target_selection = self.panel_selection;
                let med = &state.medicines[medicine_idx];
                if let Some(target) = med.decode_deploy_target(target_selection) {
                    if state.resources.funding < med.cost {
                        self.status_message = Some(
                            format!("Insufficient funds! Need ${:.0}, have ${:.0}",
                                med.cost, state.resources.funding),
                        );
                        None
                    } else {
                        let disease_idx = match &target {
                            DeployTarget::Vaccinate { disease_idx } => *disease_idx,
                            DeployTarget::Treat { disease_idx } => *disease_idx,
                        };
                        let is_tested = med.tested_against.contains(&disease_idx);

                        if !is_tested {
                            self.medicine_ui = Some(MedicineUiState::ConfirmDeploy {
                                medicine_idx,
                                region_idx,
                                target_selection,
                            });
                            None
                        } else {
                            Some(GameCommand::DeployMedicine {
                                medicine_idx,
                                region_idx,
                                target_selection,
                            })
                        }
                    }
                } else {
                    None
                }
            }
            Some(MedicineUiState::ConfirmDeploy {
                medicine_idx,
                region_idx,
                target_selection,
            }) => {
                Some(GameCommand::DeployMedicine {
                    medicine_idx,
                    region_idx,
                    target_selection,
                })
            }
            None => None,
        }
    }

    fn handle_research_confirm(&mut self, state: &GameState) -> Option<GameCommand> {
        match self.research_ui.clone() {
            Some(ResearchUiState::BrowseCategories) => {
                let bench = self.panel_selection == 1;
                self.research_ui = Some(ResearchUiState::BrowseProjects { bench });
                self.panel_selection = 0;
                None
            }
            Some(ResearchUiState::BrowseProjects { bench }) => {
                let active = if bench {
                    &state.bench_research
                } else {
                    &state.field_research
                };
                if active.is_some() {
                    self.research_ui = Some(ResearchUiState::ViewActive { bench });
                    self.panel_selection = 0;
                } else {
                    let count = if bench {
                        state.available_bench_projects().len()
                    } else {
                        state.available_field_projects().len()
                    };
                    if count > 0 {
                        self.research_ui = Some(ResearchUiState::ConfirmProject {
                            bench,
                            project_idx: self.panel_selection,
                        });
                        self.panel_selection = 0;
                    }
                }
                None
            }
            Some(ResearchUiState::ConfirmProject { bench, project_idx }) => {
                Some(GameCommand::StartResearch { bench, project_idx })
            }
            Some(ResearchUiState::ViewActive { bench }) => {
                Some(GameCommand::BoostResearch { bench })
            }
            None => None,
        }
    }

    /// Update UI navigation after a game command completes.
    /// Called by the action handler after execute_command returns.
    pub fn apply_command_result(&mut self, cmd: &GameCommand, success: bool) {
        match cmd {
            GameCommand::DeployMedicine { medicine_idx, .. } => {
                if success {
                    self.medicine_ui =
                        Some(MedicineUiState::SelectRegion { medicine_idx: *medicine_idx });
                    self.panel_selection = 0;
                }
            }
            GameCommand::StartResearch { bench, .. } => {
                if success {
                    self.research_ui =
                        Some(ResearchUiState::BrowseProjects { bench: *bench });
                    self.panel_selection = 0;
                }
            }
            GameCommand::BoostResearch { .. } | GameCommand::TogglePolicy { .. } => {
                // No UI navigation change needed
            }
        }
    }

    fn handle_policy_confirm(&mut self, state: &GameState) -> Option<GameCommand> {
        match self.policy_ui.clone() {
            Some(PolicyUiState::BrowseRegions) => {
                let region_idx = self.panel_selection;
                if region_idx < state.regions.len() {
                    self.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx });
                    self.panel_selection = 0;
                }
                None
            }
            Some(PolicyUiState::ManagePolicies { region_idx }) => {
                let policy_idx = self.panel_selection;
                Some(GameCommand::TogglePolicy {
                    region_idx,
                    policy_idx,
                })
            }
            None => None,
        }
    }
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

        // --- Procedural disease generation ---
        // Generate 2-3 diseases with different pathogen types
        let disease_count = 2 + (rng.r#gen::<usize>() % 2); // 2 or 3

        // Pick distinct pathogen types (weighted: prions are rare)
        let mut available_types = vec![
            PathogenType::RnaVirus,
            PathogenType::RnaVirus,   // 2× weight
            PathogenType::DnaVirus,
            PathogenType::Bacterium,
            PathogenType::Bacterium,  // 2× weight
        ];
        // Only add prion with 20% chance per game
        if rng.r#gen::<f64>() < 0.2 {
            available_types.push(PathogenType::Prion);
        }

        let mut chosen_types = Vec::new();
        for _ in 0..disease_count {
            if available_types.is_empty() {
                break;
            }
            let idx = rng.r#gen::<usize>() % available_types.len();
            chosen_types.push(available_types.remove(idx));
        }

        let range_val = |rng: &mut ChaCha8Rng, range: (f64, f64)| -> f64 {
            range.0 + rng.r#gen::<f64>() * (range.1 - range.0)
        };

        let mut diseases = Vec::new();
        let mut used_names: Vec<String> = Vec::new();
        for pathogen_type in &chosen_types {
            let pool = pathogen_type.name_pool();
            // Pick a name not already used (with fallback to avoid infinite loop)
            let available: Vec<_> = pool.iter()
                .filter(|n| !used_names.contains(&n.to_string()))
                .collect();
            let name = if available.is_empty() {
                format!("Pathogen-{}", used_names.len() + 1)
            } else {
                let idx = rng.r#gen::<usize>() % available.len();
                available[idx].to_string()
            };
            used_names.push(name.clone());

            let ranges = pathogen_type.stat_ranges();
            diseases.push(Disease {
                name,
                pathogen_type: *pathogen_type,
                infectivity: range_val(&mut rng, ranges.infectivity),
                lethality: range_val(&mut rng, ranges.lethality),
                cross_region_spread: range_val(&mut rng, ranges.cross_region),
                recovery_rate: range_val(&mut rng, ranges.recovery),
                knowledge: 0.0,
                strain_generation: 0,
                sequencing_count: 0,
            });
        }

        // --- Place initial outbreaks in distinct regions ---
        let region_count = regions.len();
        let mut available_regions: Vec<usize> = (0..region_count).collect();
        for (disease_idx, _disease) in diseases.iter().enumerate() {
            if available_regions.is_empty() {
                break;
            }
            let pick = rng.r#gen::<usize>() % available_regions.len();
            let region_idx = available_regions.remove(pick);

            // First disease gets larger outbreak, subsequent ones get smaller
            let infected = if disease_idx == 0 {
                5_000.0 + rng.r#gen::<f64>() * 15_000.0
            } else {
                1_000.0 + rng.r#gen::<f64>() * 4_000.0
            };

            regions[region_idx].infections.push(RegionDiseaseState {
                disease_idx,
                infected,
                dead: 0.0,
                immune: 0.0,
            });
        }

        // --- Generate medicines to match diseases ---
        let mut medicines = Vec::new();

        // One targeted medicine per disease (matched therapy type)
        for (i, disease) in diseases.iter().enumerate() {
            let therapy = disease.pathogen_type.matched_therapy();
            let name = format!("{}-{}", therapy.label(), (b'A' + i as u8) as char);
            medicines.push(Medicine {
                name,
                therapy_type: therapy,
                target_diseases: vec![i],
                cost: 100.0,
                doses: 100_000.0,
                max_doses: 100_000.0,
                unlocked: false,
                tested_against: vec![],
                strain_generations: vec![],
            });
        }

        // One broad-spectrum medicine targeting all diseases
        let all_disease_indices: Vec<usize> = (0..diseases.len()).collect();
        medicines.push(Medicine {
            name: "Broad-Spectrum".into(),
            therapy_type: TherapyType::BroadSpectrum,
            target_diseases: all_disease_indices,
            cost: 200.0,
            doses: 100_000.0,
            max_doses: 100_000.0,
            unlocked: false,
            tested_against: vec![],
            strain_generations: vec![],
        });

        Self {
            tick: 0,
            paused: false,
            rng,
            resources: Resources {
                funding: 300.0,
                research_points: 0.0,
                personnel: 20,
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
                TRAVEL_BAN_INCOME_PENALTY
            } else {
                1.0
            };
            income += BASE_FUNDING_INCOME * region_share * healthy_frac * travel_ban_factor;
        }
        income
    }

    /// RP income per tick, degraded by pandemic damage (mirrors funding income).
    /// As deaths mount, research infrastructure collapses — labs close, scientists
    /// die or flee, supply chains break. At the loss threshold, RP income is near zero.
    pub fn rp_income_rate(&self) -> f64 {
        let initial_pop = self.initial_population();
        if initial_pop <= 0.0 {
            return 0.0;
        }
        let death_fraction = self.total_dead() / initial_pop;
        let health_multiplier = (1.0 - death_fraction / LOSE_DEATH_FRACTION).max(0.1);
        BASE_RP_INCOME * health_multiplier
    }

    /// Total initial population across all regions (before any deaths).
    pub fn initial_population(&self) -> f64 {
        self.regions.iter().map(|r| r.population as f64).sum()
    }

    /// Spawn a new disease mid-game: generates a random disease, places an initial
    /// outbreak in a random region, and creates a matching targeted medicine.
    /// Returns `(disease_idx, region_idx)` if successful, or `None` if at the cap.
    /// Uses `self.rng` — caller must have extracted rng if borrowing mutably.
    pub fn spawn_disease(&mut self, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
        use rand::Rng;

        if self.diseases.len() >= MAX_DISEASES {
            return None;
        }

        // Pick a pathogen type (weighted: prions rare)
        let mut types = vec![
            PathogenType::RnaVirus,
            PathogenType::RnaVirus,
            PathogenType::DnaVirus,
            PathogenType::Bacterium,
            PathogenType::Bacterium,
        ];
        if rng.r#gen::<f64>() < 0.15 {
            types.push(PathogenType::Prion);
        }
        let pathogen_type = types[rng.r#gen::<usize>() % types.len()];

        // Pick a unique name
        let used_names: Vec<&str> = self.diseases.iter().map(|d| d.name.as_str()).collect();
        let pool = pathogen_type.name_pool();
        let available: Vec<&&str> = pool.iter().filter(|n| !used_names.contains(*n)).collect();
        let name = if available.is_empty() {
            format!("Pathogen-{}", self.diseases.len() + 1)
        } else {
            available[rng.r#gen::<usize>() % available.len()].to_string()
        };

        // Generate stats — mid-game threats are biased toward the upper end
        let ranges = pathogen_type.stat_ranges();
        let range_val = |rng: &mut ChaCha8Rng, (lo, hi): (f64, f64)| -> f64 {
            lo + rng.r#gen::<f64>() * (hi - lo)
        };
        let biased_range_val = |rng: &mut ChaCha8Rng, (lo, hi): (f64, f64)| -> f64 {
            let base = lo + rng.r#gen::<f64>() * (hi - lo);
            (base + (hi - lo) * 0.2).min(hi)
        };

        let disease_idx = self.diseases.len();
        self.diseases.push(Disease {
            name,
            pathogen_type,
            infectivity: biased_range_val(rng, ranges.infectivity),
            lethality: biased_range_val(rng, ranges.lethality),
            cross_region_spread: range_val(rng, ranges.cross_region),
            recovery_rate: range_val(rng, ranges.recovery),
            knowledge: 0.0,
            strain_generation: 0,
            sequencing_count: 0,
        });

        // Place initial outbreak in a random region
        let region_idx = rng.r#gen::<usize>() % self.regions.len();
        let initial_infected = 500.0 + rng.r#gen::<f64>() * 2_000.0;
        self.regions[region_idx].infections.push(RegionDiseaseState {
            disease_idx,
            infected: initial_infected,
            dead: 0.0,
            immune: 0.0,
        });

        // Generate a matching targeted medicine
        let therapy = pathogen_type.matched_therapy();
        let letter = (b'A' + disease_idx as u8) as char;
        self.medicines.push(Medicine {
            name: format!("{}-{}", therapy.label(), letter),
            therapy_type: therapy,
            target_diseases: vec![disease_idx],
            cost: 100.0,
            doses: 100_000.0,
            max_doses: 100_000.0,
            unlocked: false,
            tested_against: vec![],
            strain_generations: vec![],
        });

        // Update broad-spectrum medicine to also target new disease
        for med in &mut self.medicines {
            if med.therapy_type == TherapyType::BroadSpectrum
                && !med.target_diseases.contains(&disease_idx)
            {
                med.target_diseases.push(disease_idx);
            }
        }

        Some((disease_idx, region_idx))
    }

    /// Whether any tested/unlocked medicines targeting this disease have fallen behind
    /// the current strain generation (i.e., mutation has reduced their efficacy).
    pub fn has_outdated_medicine(&self, disease_idx: usize) -> bool {
        self.medicines.iter().any(|m| {
            m.target_diseases.contains(&disease_idx)
                && (m.tested_against.contains(&disease_idx) || m.unlocked)
                && m.strain_efficacy(disease_idx, &self.diseases) < 1.0
        })
    }

    /// Generate strategic tips based on what the player did (or didn't do) before defeat.
    /// Returns up to 2 actionable tips, most impactful first.
    pub fn defeat_tips(&self) -> Vec<String> {
        let mut tips = Vec::new();

        // Check if any diseases were never identified
        let unidentified = self.diseases.iter()
            .filter(|d| d.knowledge < KNOWLEDGE_NAME)
            .count();
        if unidentified == self.diseases.len() {
            tips.push(
                "You never identified any threats. Use [R] Research → Field Research → Identify to learn about diseases."
                    .to_string(),
            );
        } else if unidentified > 0 {
            tips.push(format!(
                "{} of {} threats were never identified. Identifying threats unlocks medicine development.",
                unidentified,
                self.diseases.len()
            ));
        }

        // Check if medicines were developed but never deployed
        let unlocked_meds = self.medicines.iter().filter(|m| m.unlocked).count();
        let deployed_any = self.medicines.iter().any(|m| m.unlocked && m.doses < m.max_doses);
        if unlocked_meds > 0 && !deployed_any {
            tips.push(
                "You developed medicines but never deployed them. Use [M] Medicines to vaccinate or treat regions."
                    .to_string(),
            );
        } else if unlocked_meds == 0 && unidentified < self.diseases.len() {
            // Identified threats but never developed medicine
            tips.push(
                "You identified threats but never developed a medicine. Use Bench Research to develop treatments."
                    .to_string(),
            );
        }

        // Check if policies were ever used
        let any_policy_active = self.policies.iter().any(|p| p.travel_ban || p.quarantine || p.hospital_surge);
        if !any_policy_active && tips.len() < 2 {
            // Find the worst-hit region
            let worst_region = self.regions.iter()
                .max_by(|a, b| a.total_dead().partial_cmp(&b.total_dead()).unwrap());
            if let Some(region) = worst_region {
                if region.total_dead() > 0.0 {
                    tips.push(format!(
                        "{} lost the most lives. Travel bans [P] can slow spread between regions.",
                        region.name
                    ));
                }
            }
        }

        // If no specific tips, give a general one
        if tips.is_empty() {
            tips.push(
                "Try acting faster — research, develop, and deploy medicines before infections spread."
                    .to_string(),
            );
        }

        tips.truncate(2);
        tips
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
        // Genomic Sequencing: fully identified diseases that still mutate
        for (i, disease) in self.diseases.iter().enumerate() {
            if disease.knowledge >= KNOWLEDGE_FULL
                && disease.pathogen_type.mutation_rate() > 0.0001
            {
                let kind = ResearchKind::GenomicSequencing { disease_idx: i };
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
        // Train Personnel: always available as a bench project
        let kind = ResearchKind::TrainPersonnel;
        if active_kind != Some(&kind) {
            projects.push(kind);
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

        // Verify idempotent roundtrip: deserialize→serialize→deserialize→serialize
        // should produce identical JSON. (The first serialize may differ from the
        // in-memory f64 due to float representation, but subsequent roundtrips
        // must be stable.)
        let json2 = serde_json::to_string_pretty(&restored).unwrap();
        let restored2: GameState = serde_json::from_str(&json2).unwrap();
        let json3 = serde_json::to_string_pretty(&restored2).unwrap();
        assert_eq!(json2, json3);
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
        let disease_count = state.diseases.len();
        assert!(disease_count >= 2 && disease_count <= 3, "expected 2-3 diseases, got {}", disease_count);
        // One targeted medicine per disease + one broad-spectrum
        assert_eq!(state.medicines.len(), disease_count + 1);
        // Medicines start locked — must be developed via research
        assert!(state.medicines.iter().all(|m| !m.unlocked));
        // Last medicine is always broad-spectrum
        assert_eq!(state.medicines.last().unwrap().therapy_type, TherapyType::BroadSpectrum);
        // Each targeted medicine has exactly one target disease
        for i in 0..disease_count {
            assert_eq!(state.medicines[i].target_diseases.len(), 1);
            assert_eq!(state.medicines[i].target_diseases[0], i);
        }
        // Broad-spectrum targets all diseases
        let broad = state.medicines.last().unwrap();
        assert_eq!(broad.target_diseases.len(), disease_count);
    }

    #[test]
    fn procedural_generation_varies_by_seed() {
        let state1 = GameState::new_default(1);
        let state2 = GameState::new_default(999);
        // Different seeds should produce different disease names (with very high probability)
        let names1: Vec<_> = state1.diseases.iter().map(|d| d.name.clone()).collect();
        let names2: Vec<_> = state2.diseases.iter().map(|d| d.name.clone()).collect();
        assert_ne!(names1, names2, "different seeds should produce different diseases");
    }

    #[test]
    fn procedural_generation_is_deterministic() {
        let state1 = GameState::new_default(42);
        let state2 = GameState::new_default(42);
        let names1: Vec<_> = state1.diseases.iter().map(|d| d.name.clone()).collect();
        let names2: Vec<_> = state2.diseases.iter().map(|d| d.name.clone()).collect();
        assert_eq!(names1, names2, "same seed should produce same diseases");
        assert_eq!(state1.diseases.len(), state2.diseases.len());
    }

    #[test]
    fn each_disease_starts_in_different_region() {
        // Test across several seeds
        for seed in 0..20 {
            let state = GameState::new_default(seed);
            let mut infected_regions: Vec<usize> = Vec::new();
            for (ri, region) in state.regions.iter().enumerate() {
                if !region.infections.is_empty() {
                    infected_regions.push(ri);
                }
            }
            // Each disease should be in a unique region
            let unique: std::collections::HashSet<_> = infected_regions.iter().collect();
            assert_eq!(unique.len(), infected_regions.len(),
                "seed {}: diseases placed in overlapping regions", seed);
        }
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

    #[test]
    fn defeat_tips_no_research() {
        let state = GameState::new_default(42);
        let tips = state.defeat_tips();
        assert!(!tips.is_empty());
        assert!(tips[0].contains("identified"), "should suggest identifying threats: {:?}", tips);
    }

    #[test]
    fn defeat_tips_with_identified_but_no_medicine() {
        let mut state = GameState::new_default(42);
        for d in &mut state.diseases {
            d.knowledge = KNOWLEDGE_NAME;
        }
        let tips = state.defeat_tips();
        assert!(tips.iter().any(|t| t.contains("develop") || t.contains("Bench")),
            "should suggest developing medicine: {:?}", tips);
    }
}
