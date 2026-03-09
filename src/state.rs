use std::collections::HashMap;

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SimState {
    Running,
    Paused,
    /// Game is paused for an event. `was_running` tracks pre-event state
    /// so we can restore it on dismissal.
    Event { was_running: bool },
}

impl Default for SimState {
    fn default() -> Self {
        SimState::Running
    }
}

impl SimState {
    pub fn is_running(&self) -> bool {
        matches!(self, SimState::Running)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameState {
    pub tick: u64,
    #[serde(default)]
    pub sim_state: SimState,
    pub rng: ChaCha8Rng,
    pub resources: Resources,
    pub regions: Vec<Region>,
    pub diseases: Vec<Disease>,
    #[serde(default)]
    pub medicines: Vec<Medicine>,
    /// Active field research project (Identify Threat or Clinical Trial).
    #[serde(default)]
    pub field_research: Option<ResearchProject>,
    /// Active applied research project (Develop Medicine).
    #[serde(default, alias = "applied_research")]
    pub applied_research: Option<ResearchProject>,
    /// Active basic research project (tech tree unlocks).
    #[serde(default)]
    pub basic_research: Option<ResearchProject>,
    /// Technologies unlocked via basic research.
    #[serde(default)]
    pub unlocked_techs: Vec<BasicTech>,
    /// Per-region active policies.
    #[serde(default)]
    pub policies: Vec<RegionPolicy>,
    #[serde(default)]
    pub outcome: GameOutcome,
    /// Events from the most recent tick. Consumed by the UI layer for status
    /// messages. Cleared at the start of each tick.
    #[serde(skip)]
    pub events: Vec<GameEvent>,
    /// Active crisis event requiring player decision. Game pauses while active.
    #[serde(default)]
    pub active_crisis: Option<CrisisEvent>,
    /// Per-type cooldowns: crisis tag → tick when it last fired.
    /// Used to prevent the same crisis type repeating within CRISIS_TYPE_COOLDOWN ticks.
    #[serde(default)]
    pub crisis_cooldowns: HashMap<String, u64>,
    /// Auto-resolve preferences: crisis tag → choice index (0 = A, 1 = B).
    /// When a crisis fires whose tag matches, it's resolved immediately without pausing.
    #[serde(default)]
    pub auto_resolve_crises: HashMap<String, usize>,
    /// Historical snapshots for dashboard charts. Recorded every HISTORY_INTERVAL ticks.
    #[serde(default)]
    pub history: Vec<HistorySnapshot>,
    pub ui: UiState,
}

/// A point-in-time snapshot for dashboard sparkline charts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub tick: u64,
    pub total_infected: f64,
    pub total_dead: f64,
}

/// Record a history snapshot every this many ticks (~1 hour of game time).
pub const HISTORY_INTERVAL: u64 = 5;
/// Maximum history entries to retain (covers ~4 days at 5-tick intervals).
pub const HISTORY_MAX: usize = 100;

// Medicine constants.
/// Fraction of infected treated per deployment (before efficacy modifiers).
/// Treatment is proportional — scales with infection size instead of fixed dose count.
pub const TREATMENT_FRACTION: f64 = 0.5;
/// Fraction of susceptible population vaccinated per deployment (before efficacy).
/// Vaccination is proportional like treatment — each deploy protects a meaningful
/// fraction, making repeated deployments build toward herd immunity.
pub const VACCINATION_FRACTION: f64 = 0.02;

// Disease emergence constants.
/// First new disease can emerge after this many ticks (~day 7).
/// Gives the player time to identify disease 0 and start the research pipeline.
pub const EMERGENCE_MIN_TICK: u64 = 840;
/// Per-tick probability of a new disease emerging (after min tick).
/// ~1 new disease every 12 days (1440 ticks) → ~3 new diseases in a 45-day game.
pub const EMERGENCE_CHANCE_PER_TICK: f64 = 0.0007;
/// Maximum number of simultaneous diseases.
pub const MAX_DISEASES: usize = 5;

// Economy constants — single source of truth.
pub const BASE_FUNDING_INCOME: f64 = 3.0;
/// Per-tick cost for each personnel on the roster (busy or idle).
/// 20 personnel × 0.1 = $2/tick = $240/day upkeep vs $360/day gross income → ~$120/day net.
pub const PERSONNEL_UPKEEP_COST: f64 = 0.1;
pub const TRAVEL_BAN_INCOME_PENALTY: f64 = 0.5;
pub const TRAVEL_BAN_COST: f64 = 1.0;
pub const TRAVEL_BAN_PERSONNEL: u32 = 3;
pub const QUARANTINE_COST: f64 = 0.6;
pub const QUARANTINE_PERSONNEL: u32 = 3;
pub const HOSPITAL_SURGE_COST: f64 = 0.4;
pub const HOSPITAL_SURGE_PERSONNEL: u32 = 2;
pub const BORDER_CONTROLS_COST: f64 = 0.1;
pub const BORDER_CONTROLS_PERSONNEL: u32 = 1;
pub const WATER_SANITATION_COST: f64 = 0.3;
pub const WATER_SANITATION_PERSONNEL: u32 = 1;

/// Disease surveillance intensity. Higher levels reveal more infections
/// and help detect new hidden diseases faster. Per-region setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScreeningLevel {
    #[default]
    None,
    Low,
    Medium,
    High,
}

/// Per-tick cost for each screening level.
pub const SCREENING_LOW_COST: f64 = 0.2;
pub const SCREENING_MEDIUM_COST: f64 = 0.4;
pub const SCREENING_HIGH_COST: f64 = 0.6;

impl ScreeningLevel {
    /// Fraction of actual infections visible to the player.
    /// Without screening, only ~15% of cases are reported organically.
    pub fn visibility_rate(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.15,
            ScreeningLevel::Low => 0.40,
            ScreeningLevel::Medium => 0.70,
            ScreeningLevel::High => 0.90,
        }
    }

    /// Multiplier on the detection threshold for hidden diseases.
    /// Lower = detect new threats sooner.
    pub fn detection_multiplier(&self) -> f64 {
        match self {
            ScreeningLevel::None => 1.0,
            ScreeningLevel::Low => 0.7,
            ScreeningLevel::Medium => 0.4,
            ScreeningLevel::High => 0.2,
        }
    }

    /// Per-tick funding cost for this screening level.
    pub fn funding_cost(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.0,
            ScreeningLevel::Low => SCREENING_LOW_COST,
            ScreeningLevel::Medium => SCREENING_MEDIUM_COST,
            ScreeningLevel::High => SCREENING_HIGH_COST,
        }
    }

    /// Personnel required for this screening level.
    pub fn personnel_cost(&self) -> u32 {
        match self {
            ScreeningLevel::None => 0,
            ScreeningLevel::Low => 1,
            ScreeningLevel::Medium => 2,
            ScreeningLevel::High => 3,
        }
    }

    /// Display name for the policy panel.
    pub fn label(&self) -> &'static str {
        match self {
            ScreeningLevel::None => "None",
            ScreeningLevel::Low => "Low",
            ScreeningLevel::Medium => "Medium",
            ScreeningLevel::High => "High",
        }
    }
}

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
    /// Reduces cross-region spread by 50%, no income penalty.
    /// Cheaper alternative to travel ban. Superseded by travel ban if both active.
    #[serde(default, alias = "border_screening")]
    pub border_controls: bool,
    /// Halves waterborne disease infectivity. No effect on airborne/contact.
    #[serde(default)]
    pub water_sanitation: bool,
    /// Disease surveillance level — determines what fraction of infections
    /// are visible to the player and how quickly new diseases are detected.
    #[serde(default)]
    pub screening: ScreeningLevel,
}

/// Total number of policy types available per region.
/// Indices 0-4: boolean policies, 5-7: screening tiers (Low/Medium/High).
pub const POLICY_COUNT: usize = 8;

/// Minimum Political Power (0.0–1.0) required to activate each policy.
/// Ordered by policy_idx: travel_ban, quarantine, hospital_surge, border_controls,
/// water_sanitation, screening_low, screening_medium, screening_high.
pub const POLICY_POL_THRESHOLDS: [f64; POLICY_COUNT] = [
    0.30, // Travel Ban — major action, needs moderate political will
    0.25, // Quarantine — strong measure but regionally justified
    0.15, // Hospital Surge — relatively uncontroversial
    0.05, // Border Controls — mild, early unlock
    0.10, // Water Sanitation — basic public health
    0.00, // Low Disease Screening — available immediately
    0.10, // Medium Disease Screening
    0.15, // High Disease Screening
];

impl RegionPolicy {
    pub fn funding_cost(&self) -> f64 {
        let mut cost = 0.0;
        if self.travel_ban { cost += TRAVEL_BAN_COST; }
        if self.quarantine { cost += QUARANTINE_COST; }
        if self.hospital_surge { cost += HOSPITAL_SURGE_COST; }
        if self.border_controls { cost += BORDER_CONTROLS_COST; }
        if self.water_sanitation { cost += WATER_SANITATION_COST; }
        cost += self.screening.funding_cost();
        cost
    }

    pub fn personnel_cost(&self) -> u32 {
        let mut cost = 0;
        if self.travel_ban { cost += TRAVEL_BAN_PERSONNEL; }
        if self.quarantine { cost += QUARANTINE_PERSONNEL; }
        if self.hospital_surge { cost += HOSPITAL_SURGE_PERSONNEL; }
        if self.border_controls { cost += BORDER_CONTROLS_PERSONNEL; }
        if self.water_sanitation { cost += WATER_SANITATION_PERSONNEL; }
        cost += self.screening.personnel_cost();
        cost
    }

    pub fn any_active(&self) -> bool {
        self.travel_ban || self.quarantine || self.hospital_surge
            || self.border_controls || self.water_sanitation
            || self.screening != ScreeningLevel::None
    }

    pub fn clear_all(&mut self) {
        self.travel_ban = false;
        self.quarantine = false;
        self.hospital_surge = false;
        self.border_controls = false;
        self.water_sanitation = false;
        self.screening = ScreeningLevel::None;
    }

    /// Access a boolean policy field by index (0-4).
    pub fn get_bool(&self, idx: usize) -> bool {
        match idx {
            0 => self.travel_ban,
            1 => self.quarantine,
            2 => self.hospital_surge,
            3 => self.border_controls,
            4 => self.water_sanitation,
            _ => false,
        }
    }

    /// Set a boolean policy field by index (0-4).
    pub fn set_bool(&mut self, idx: usize, val: bool) {
        match idx {
            0 => self.travel_ban = val,
            1 => self.quarantine = val,
            2 => self.hospital_surge = val,
            3 => self.border_controls = val,
            4 => self.water_sanitation = val,
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resources {
    pub funding: f64,
    pub personnel: u32,
    /// Political Power (0.0–1.0). Represents global willingness to act.
    /// Increases based on disease severity and time. Gates policies.
    #[serde(default)]
    pub political_power: f64,
    /// Fractional accumulator for POL-based personnel gains.
    #[serde(default)]
    pub personnel_accum: f64,
    /// Fractional accumulator for personnel attrition (when funding is $0).
    #[serde(default)]
    pub attrition_accum: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub population: u64,
    pub connections: Vec<usize>,
    pub infections: Vec<RegionDiseaseState>,
    /// Fraction of population that must remain alive to avoid collapse.
    /// E.g., 0.75 means the region collapses when alive drops below 75% of initial population.
    /// More developed regions have higher thresholds (fragile); less developed are more resilient.
    #[serde(default = "default_collapse_threshold")]
    pub collapse_threshold: f64,
    /// Whether this region has collapsed. Collapsed regions lose all policies,
    /// cannot conduct field research, and are cut off from flight connections.
    #[serde(default)]
    pub collapsed: bool,
    /// Tick when this region collapsed (None if still standing).
    #[serde(default)]
    pub collapsed_at_tick: Option<u64>,
}

fn default_collapse_threshold() -> f64 {
    0.50
}

impl Region {
    /// Current living population: starting population minus total deaths.
    /// Clamped to 0 because independent per-disease SIR pools can
    /// double-count deaths (same person dies in multiple disease pools).
    pub fn alive(&self) -> f64 {
        (self.population as f64 - self.total_dead()).max(0.0)
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

    /// Total infected from detected diseases only (for UI display).
    pub fn detected_infected(&self, diseases: &[Disease]) -> f64 {
        self.infections.iter()
            .filter(|inf| diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.infected)
            .sum()
    }

    /// Screened infection count: actual detected infections × visibility rate.
    /// This is what the player sees — imperfect surveillance means not all
    /// cases are reported.
    pub fn screened_infected(&self, diseases: &[Disease], visibility: f64) -> f64 {
        self.detected_infected(diseases) * visibility
    }

    /// Total dead from detected diseases only (for UI display).
    /// Deaths are always reported accurately regardless of screening level.
    pub fn detected_dead(&self, diseases: &[Disease]) -> f64 {
        self.infections.iter()
            .filter(|inf| diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.dead)
            .sum()
    }

    /// Total immune from detected diseases only (for UI display).
    pub fn detected_immune(&self, diseases: &[Disease]) -> f64 {
        self.infections.iter()
            .filter(|inf| diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.immune)
            .sum()
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
    /// Tuned so the fastest mutator (RNA virus) averages ~8 days between
    /// mutations, giving the player time to complete the ~5-day research
    /// pipeline before the first mutation hits.
    pub fn mutation_rate(&self) -> f64 {
        match self {
            PathogenType::RnaVirus => 0.0002,    // ~1 mutation per 5000 ticks (~42 days)
            PathogenType::DnaVirus => 0.00008,   // ~1 per 12500 ticks (~104 days)
            PathogenType::Bacterium => 0.00012,  // ~1 per 8333 ticks (~69 days)
            PathogenType::Prion => 0.00001,      // ~1 per 100000 ticks (~833 days)
        }
    }

    /// Stat ranges tuned for a ~45-50 day game arc.
    /// R0 = infectivity / (lethality + recovery) targets 3-5 for most types.
    /// Daily growth ≈ 2-3× with 120 ticks/day — first region collapse at ~day 14,
    /// total defeat at ~day 47 without intervention. This gives players time to
    /// research, deploy policies, and develop medicines before collapse cascades.
    fn stat_ranges(&self) -> DiseaseStatRanges {
        match self {
            // RNA viruses: fast spreader (R0 ~3-5), moderate lethality, decent recovery
            PathogenType::RnaVirus => DiseaseStatRanges {
                infectivity: (0.008, 0.014),
                lethality: (0.0008, 0.002),
                recovery: (0.0012, 0.002),
                cross_region: (0.003, 0.005),
            },
            // DNA viruses: moderate spread (R0 ~2.5-4.5), high lethality, slow recovery
            PathogenType::DnaVirus => DiseaseStatRanges {
                infectivity: (0.006, 0.012),
                lethality: (0.0012, 0.0024),
                recovery: (0.0008, 0.0016),
                cross_region: (0.002, 0.004),
            },
            // Bacteria: moderate spread (R0 ~3-5), moderate lethality
            PathogenType::Bacterium => DiseaseStatRanges {
                infectivity: (0.006, 0.010),
                lethality: (0.0006, 0.0014),
                recovery: (0.0006, 0.0014),
                cross_region: (0.002, 0.004),
            },
            // Prions: slow but devastating (R0 ~1.5-3), very high lethality, almost no recovery
            // Infectivity floor must exceed max outflow (0.003+0.0006=0.0036) to ensure R0 > 1.
            PathogenType::Prion => DiseaseStatRanges {
                infectivity: (0.004, 0.007),
                lethality: (0.0016, 0.003),
                recovery: (0.0002, 0.0006),
                cross_region: (0.001, 0.0024),
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

    /// Pick a biologically plausible transmission vector for this pathogen type.
    pub fn random_transmission(&self, rng: &mut ChaCha8Rng) -> TransmissionVector {
        let roll: f64 = rng.r#gen();
        match self {
            // RNA viruses: flu, COVID (airborne), Ebola (contact), rare waterborne
            PathogenType::RnaVirus => {
                if roll < 0.6 { TransmissionVector::Airborne }
                else if roll < 0.9 { TransmissionVector::Contact }
                else { TransmissionVector::Waterborne }
            }
            // DNA viruses: smallpox (airborne/contact), HPV (contact)
            PathogenType::DnaVirus => {
                if roll < 0.4 { TransmissionVector::Airborne }
                else if roll < 0.85 { TransmissionVector::Contact }
                else { TransmissionVector::Waterborne }
            }
            // Bacteria: cholera (waterborne), TB (airborne), MRSA (contact)
            PathogenType::Bacterium => {
                if roll < 0.25 { TransmissionVector::Airborne }
                else if roll < 0.60 { TransmissionVector::Contact }
                else { TransmissionVector::Waterborne }
            }
            // Prions: food/contact (mad cow), never truly airborne
            PathogenType::Prion => {
                if roll < 0.7 { TransmissionVector::Contact }
                else { TransmissionVector::Waterborne }
            }
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

/// How a disease transmits between hosts. Determines which policies are
/// effective and how cross-region spread works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransmissionVector {
    /// Spreads via respiratory droplets/aerosols. Fast cross-region spread.
    /// Quarantine and travel bans are highly effective.
    #[default]
    Airborne,
    /// Spreads via contaminated water/food. Harder to contain with quarantine.
    /// Travel bans are less effective.
    Waterborne,
    /// Requires close physical contact. Slow cross-region spread.
    /// Quarantine is extremely effective. Hospital surge is risky
    /// (healthcare workers exposed).
    Contact,
}

impl TransmissionVector {
    pub fn label(&self) -> &'static str {
        match self {
            TransmissionVector::Airborne => "Airborne",
            TransmissionVector::Waterborne => "Waterborne",
            TransmissionVector::Contact => "Contact",
        }
    }

    /// Infectivity multiplier when quarantine is active in a region.
    /// Lower = quarantine is more effective at reducing spread.
    pub fn quarantine_factor(&self) -> f64 {
        match self {
            TransmissionVector::Airborne => 0.50,   // standard: halves infectivity
            TransmissionVector::Waterborne => 0.75,  // weak: only 25% reduction
            TransmissionVector::Contact => 0.30,     // strong: 70% reduction
        }
    }

    /// Cross-region spread leakage when travel ban is active.
    /// Lower = travel ban blocks more spread.
    pub fn travel_ban_factor(&self) -> f64 {
        match self {
            TransmissionVector::Airborne => 0.10,   // 90% reduction
            TransmissionVector::Waterborne => 0.50,  // only 50% reduction
            TransmissionVector::Contact => 0.05,     // 95% reduction
        }
    }

    /// Multiplier on base cross-region spread chance.
    pub fn cross_region_modifier(&self) -> f64 {
        match self {
            TransmissionVector::Airborne => 1.5,
            TransmissionVector::Waterborne => 0.7,
            TransmissionVector::Contact => 0.5,
        }
    }

    /// Infectivity multiplier when hospital surge is active.
    /// Contact diseases spread faster in hospitals (healthcare workers exposed).
    pub fn hospital_infectivity_factor(&self) -> f64 {
        match self {
            TransmissionVector::Airborne => 1.0,
            TransmissionVector::Waterborne => 1.0,
            TransmissionVector::Contact => 1.15,    // 15% increase
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
    #[serde(default)]
    pub transmission: TransmissionVector,
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
    /// Whether this disease has been detected by global health systems.
    /// Undetected diseases spread silently — the player sees only "?" in the threats panel.
    #[serde(default = "default_true")]
    pub detected: bool,
}

fn default_true() -> bool {
    true
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

    /// Generate a random disease of the given pathogen type.
    ///
    /// If `toughness_bias` is true, infectivity and lethality are biased toward
    /// the upper end of their ranges (used for mid-game emergent diseases).
    pub fn generate(
        rng: &mut ChaCha8Rng,
        pathogen_type: PathogenType,
        used_names: &[String],
        toughness_bias: bool,
    ) -> Disease {
        let pool = pathogen_type.name_pool();
        let available: Vec<_> = pool.iter()
            .filter(|n| !used_names.contains(&n.to_string()))
            .collect();
        let name = if available.is_empty() {
            format!("Pathogen-{}", used_names.len() + 1)
        } else {
            let idx = rng.r#gen::<usize>() % available.len();
            available[idx].to_string()
        };

        let ranges = pathogen_type.stat_ranges();
        let range_val = |rng: &mut ChaCha8Rng, (lo, hi): (f64, f64)| -> f64 {
            lo + rng.r#gen::<f64>() * (hi - lo)
        };
        let biased_val = |rng: &mut ChaCha8Rng, (lo, hi): (f64, f64)| -> f64 {
            let base = lo + rng.r#gen::<f64>() * (hi - lo);
            (base + (hi - lo) * 0.2).min(hi)
        };

        let stat = |rng: &mut ChaCha8Rng, range: (f64, f64), bias: bool| -> f64 {
            if bias { biased_val(rng, range) } else { range_val(rng, range) }
        };

        Disease {
            name,
            pathogen_type,
            transmission: pathogen_type.random_transmission(rng),
            infectivity: stat(rng, ranges.infectivity, toughness_bias),
            lethality: stat(rng, ranges.lethality, toughness_bias),
            cross_region_spread: range_val(rng, ranges.cross_region),
            recovery_rate: range_val(rng, ranges.recovery),
            knowledge: 0.0,
            strain_generation: 0,
            sequencing_count: 0,
            detected: true, // callers override to false for new diseases
        }
    }
}

/// Knowledge thresholds for progressive disease revelation.
pub const KNOWLEDGE_NAME: f64 = 0.33;
pub const KNOWLEDGE_PARTIAL_STATS: f64 = 0.66;
pub const KNOWLEDGE_FULL: f64 = 1.0;
/// Minimum knowledge required to develop a medicine targeting this disease.
/// One identification (0.50 knowledge) is enough to start development.
pub const KNOWLEDGE_FOR_MEDICINE: f64 = 0.50;


/// Number of simulation ticks per in-game day. The UI displays days, not ticks.
/// 120 chosen so 5 ticks = 1 hour exactly (120 / 24 = 5).
pub const TICKS_PER_DAY: f64 = 120.0;

/// Convert ticks to days for display purposes.
pub fn ticks_to_days(ticks: f64) -> f64 {
    ticks / TICKS_PER_DAY
}

/// Convert ticks to a formatted day string for the UI.
/// Uses hours for sub-day values so "5 ticks" reads as "1h" not "0.05 days".
pub fn format_days(ticks: f64) -> String {
    let days = ticks_to_days(ticks);
    if days < 1.0 {
        let hours = days * 24.0;
        format!("{:.0}h", hours)
    } else {
        format!("{:.1} days", days)
    }
}

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

/// What a medicine deployment targets: protect susceptible (preventive) or treat infected (therapeutic).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeployTarget {
    Vaccinate { disease_idx: usize },
    Treat { disease_idx: usize },
}

impl Medicine {
    /// Create a targeted medicine for a disease. Name format: "TherapyType-A", "TherapyType-B", etc.
    pub fn new_targeted(disease_idx: usize, pathogen_type: PathogenType) -> Medicine {
        let therapy = pathogen_type.matched_therapy();
        let letter = (b'A' + disease_idx as u8) as char;
        Medicine {
            name: format!("{}-{}", therapy.label(), letter),
            therapy_type: therapy,
            target_diseases: vec![disease_idx],
            cost: 150.0,
            doses: 5_000_000.0,
            max_doses: 5_000_000.0,
            unlocked: false,
            tested_against: vec![],
            strain_generations: vec![],
        }
    }

    /// Number of target options in the UI (vaccinate + treat per target disease).
    pub fn num_deploy_targets(&self) -> usize {
        2 * self.target_diseases.len()
    }

    /// Efficacy multiplier based on how many generations behind this medicine is
    /// for a given disease. Each generation of drift reduces efficacy by 15%,
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
                        (1.0 - behind as f64 * 0.15).max(0.1)
                    }
                    // Not yet calibrated (developed before mutation system) — full efficacy
                    None => 1.0,
                }
            }
            None => 1.0,
        }
    }

    /// Estimate how many people a vaccination deployment would protect.
    /// Returns the number of doses that would be consumed (capped by available doses).
    pub fn estimate_vaccination(&self, susceptible: f64, efficacy: f64) -> f64 {
        let target = susceptible * VACCINATION_FRACTION * efficacy;
        target.min(self.doses)
    }

    /// Estimate how many people a treatment deployment would treat.
    /// Returns the number of doses that would be consumed (capped by available doses).
    pub fn estimate_treatment(&self, infected: f64, efficacy: f64) -> f64 {
        let target = infected * TREATMENT_FRACTION * efficacy;
        target.min(self.doses)
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

/// Which research track a project belongs to.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResearchTrack {
    Field,
    #[serde(alias = "Bench")]
    Applied,
    Basic,
}

/// An active research project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResearchProject {
    pub kind: ResearchKind,
    pub progress: f64,
    pub required_ticks: f64,
    pub personnel_assigned: u32,
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
    /// Basic research — unlocks a technology in the tech tree.
    BasicResearch { tech: BasicTech },
}

/// Technology nodes in the Basic Research tech tree.
/// Each unlocks new capabilities in Applied Research.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BasicTech {
    /// Unlocks targeted drug development (Antiviral / Antibiotic).
    /// Prereq: identify any pathogen.
    TargetedDrugDesign,
    /// Unlocks monoclonal antibody development for viruses.
    /// Prereq: TargetedDrugDesign + fully studied any virus (knowledge >= 1.0).
    MonoclonalAntibodies,
    /// Unlocks phage therapy development for bacteria.
    /// Prereq: TargetedDrugDesign + fully studied any bacterium (knowledge >= 1.0).
    PhageTherapy,
}

impl BasicTech {
    /// Human-readable name for display.
    pub fn name(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Targeted Drug Design",
            BasicTech::MonoclonalAntibodies => "Monoclonal Antibodies",
            BasicTech::PhageTherapy => "Phage Therapy",
        }
    }

    /// Short description for the research panel.
    pub fn description(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Unlocks targeted Antiviral/Antibiotic development",
            BasicTech::MonoclonalAntibodies => "Unlocks high-efficacy mAb drugs for viruses",
            BasicTech::PhageTherapy => "Unlocks phage therapy drugs for bacteria",
        }
    }

    /// Prerequisites: returns list of (tech prereqs, description of other prereqs).
    pub fn prerequisites_met(&self, state: &GameState) -> bool {
        match self {
            BasicTech::TargetedDrugDesign => {
                // Prereq: identified any pathogen (any disease with knowledge > 0)
                state.diseases.iter().any(|d| d.knowledge > 0.0)
            }
            BasicTech::MonoclonalAntibodies => {
                // Prereq: TargetedDrugDesign + fully studied any virus
                state.unlocked_techs.contains(&BasicTech::TargetedDrugDesign)
                    && state.diseases.iter().any(|d| {
                        d.knowledge >= 1.0
                            && matches!(d.pathogen_type, PathogenType::RnaVirus | PathogenType::DnaVirus)
                    })
            }
            BasicTech::PhageTherapy => {
                // Prereq: TargetedDrugDesign + fully studied any bacterium
                state.unlocked_techs.contains(&BasicTech::TargetedDrugDesign)
                    && state.diseases.iter().any(|d| {
                        d.knowledge >= 1.0 && d.pathogen_type == PathogenType::Bacterium
                    })
            }
        }
    }

    /// What prerequisites are needed (for display when locked).
    pub fn prereq_description(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Identify any pathogen",
            BasicTech::MonoclonalAntibodies => "Targeted Drug Design + study any virus",
            BasicTech::PhageTherapy => "Targeted Drug Design + study any bacterium",
        }
    }

    /// All techs in display order.
    pub fn all() -> &'static [BasicTech] {
        &[
            BasicTech::TargetedDrugDesign,
            BasicTech::MonoclonalAntibodies,
            BasicTech::PhageTherapy,
        ]
    }
}

impl ResearchKind {
    /// Project costs: (personnel, duration_ticks, funding).
    ///
    /// DevelopMedicine costs scale with medicine target count:
    /// narrow (1 target) is cheaper/faster, broad (2+ targets) is more expensive/slower.
    pub fn costs(&self, medicines: &[Medicine]) -> (u32, f64, f64) {
        match self {
            ResearchKind::IdentifyThreat { .. } => (5, 160.0, 200.0),
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let targets = medicines.get(*medicine_idx)
                    .map_or(1, |m| m.target_diseases.len());
                if targets <= 1 {
                    (3, 200.0, 300.0)  // narrow: fast and cheap, single-target
                } else {
                    (10, 400.0, 600.0) // broad: slow and expensive, covers all
                }
            }
            ResearchKind::ClinicalTrial { .. } => (2, 60.0, 100.0),
            ResearchKind::ManufactureDoses { .. } => (3, 120.0, 150.0),
            ResearchKind::GenomicSequencing { .. } => (5, 200.0, 300.0),
            ResearchKind::TrainPersonnel => (1, 160.0, 100.0),
            ResearchKind::BasicResearch { tech } => match tech {
                BasicTech::TargetedDrugDesign => (3, 240.0, 400.0),
                BasicTech::MonoclonalAntibodies => (5, 360.0, 600.0),
                BasicTech::PhageTherapy => (5, 360.0, 600.0),
            },
        }
    }
}

impl ResearchProject {
    pub fn is_complete(&self) -> bool {
        self.progress >= self.required_ticks
    }

    /// Speed multiplier with diminishing returns on personnel.
    pub fn speed(&self, medicines: &[Medicine]) -> f64 {
        let (base, _, _) = self.kind.costs(medicines);
        personnel_speed(self.personnel_assigned, base)
    }
}

/// Diminishing returns speed curve for research personnel.
///
/// Below base: linear (understaffed = proportionally slower).
/// 1x-2x base: diminishing gains, peaks at ~1.5x speed with 2x personnel.
/// Beyond 2x: negative returns — too many cooks in the kitchen.
/// Minimum speed is 0.5x to prevent absurd slowdowns.
pub fn personnel_speed(assigned: u32, base: u32) -> f64 {
    let ratio = assigned as f64 / base.max(1) as f64;
    if ratio <= 1.0 {
        ratio
    } else {
        (1.0 + (ratio - 1.0) * (3.0 - ratio) / 2.0).max(0.5)
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
    /// A previously undetected disease has been detected by health systems.
    DiseaseDetected {
        disease_idx: usize,
    },
    /// A disease spread to a new region via cross-region transmission.
    DiseaseSpreadToRegion {
        disease_idx: usize,
        region_idx: usize,
    },
    /// A region's society has collapsed — too many deaths.
    RegionCollapsed {
        region_idx: usize,
    },
    /// The game just ended (win or lose). UI should pause and close panels.
    /// The actual outcome is on `GameState::outcome`; this just signals the transition.
    GameOver,
    /// A crisis event appeared and needs player attention.
    CrisisStarted,
    /// A crisis was auto-resolved based on player's saved preference.
    CrisisAutoResolved,
    /// Personnel left due to unpaid wages (funding at $0).
    PersonnelAttrition { count: u32 },
}

/// Game outcome — there is no victory. You lose eventually. The question is when.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum GameOutcome {
    #[default]
    Playing,
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
        track: ResearchTrack,
        project_idx: usize,
        double_personnel: bool,
    },
    AddResearchPersonnel {
        track: ResearchTrack,
    },
    RemoveResearchPersonnel {
        track: ResearchTrack,
    },
    TogglePolicy {
        region_idx: usize,
        policy_idx: usize,
    },
    /// Resolve the active crisis by choosing option A (0) or B (1).
    ResolveCrisis {
        choice: usize,
    },
}

/// A crisis event that pauses the game and requires a player decision.
/// Crises create ongoing strategic choices throughout the game — the player
/// must pick one of two options, each with trade-offs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrisisEvent {
    pub kind: CrisisKind,
    pub title: String,
    pub description: String,
    pub option_a: CrisisOption,
    pub option_b: CrisisOption,
    pub tick_created: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrisisOption {
    pub label: String,
    pub description: String,
    /// Resource cost to select this option. None = free.
    #[serde(default)]
    pub cost: Option<CrisisCost>,
}

/// Resources required to select a crisis option.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrisisCost {
    #[serde(default)]
    pub funding: f64,
    #[serde(default)]
    pub personnel: u32,
}

impl CrisisCost {
    /// Check if the player can afford this cost.
    pub fn affordable(&self, state: &GameState) -> bool {
        state.resources.funding >= self.funding
            && state.personnel_available() >= self.personnel
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CrisisKind {
    /// Supply chain disrupted — lose medicine doses or pay to protect them.
    SupplyDisruption { medicine_idx: usize },
    /// Lab accident — lose applied research or spend resources to contain.
    LabAccident,
    /// Political pressure — lift quarantine in a region or pay to resist.
    PoliticalPressure { region_idx: usize },
    /// Staff burnout — lose personnel or pay retention bonus.
    PersonnelCrisis { amount: u32 },
    /// International aid offer — choose funding or personnel.
    InternationalAid { funding: f64, personnel: u32 },
    /// Mutation surge — pay to gain knowledge or let it drift.
    MutationSurge { disease_idx: usize },
}

impl CrisisKind {
    /// Short tag identifying the crisis type (ignoring variant data).
    /// Used for cooldown tracking to prevent back-to-back repeats.
    pub fn tag(&self) -> &'static str {
        match self {
            CrisisKind::SupplyDisruption { .. } => "supply",
            CrisisKind::LabAccident => "lab",
            CrisisKind::PoliticalPressure { .. } => "political",
            CrisisKind::PersonnelCrisis { .. } => "personnel",
            CrisisKind::InternationalAid { .. } => "aid",
            CrisisKind::MutationSurge { .. } => "mutation",
        }
    }
}

/// Crisis events start appearing after this many ticks (~3 days).
pub const CRISIS_MIN_TICK: u64 = 360;
/// Average ticks between crises (~7 days).
pub const CRISIS_INTERVAL: u64 = 840;
/// Minimum ticks before the same crisis type can repeat (~15 days).
pub const CRISIS_TYPE_COOLDOWN: u64 = 1800;

/// Total infected across all regions at which a disease is detected by health systems.
/// Below this, the disease spreads silently and is invisible to the player.
pub const DETECTION_THRESHOLD: f64 = 10_000.0;

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
    /// Shown after a deployment completes, displaying the result prominently.
    DeployResult { medicine_idx: usize, message: String, adverse: bool },
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
    /// Top level: choose Field, Applied, or Basic Research category.
    BrowseCategories,
    /// Browsing available projects in the selected category.
    BrowseProjects { track: ResearchTrack },
    /// Confirming a project before starting it.
    ConfirmProject { track: ResearchTrack, project_idx: usize, double_personnel: bool },
    /// Viewing the active project in a category.
    ViewActive { track: ResearchTrack },
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
    /// Which crisis option is selected (0 = A, 1 = B).
    #[serde(default)]
    pub crisis_selection: usize,
    /// Whether the [X] auto-resolve toggle is active for the current crisis popup.
    #[serde(default)]
    pub crisis_auto_resolve: bool,
    /// Whether the home splash animation has completed (or been skipped).
    /// Once true, the home panel renders fully without animation.
    #[serde(default)]
    pub home_splash_done: bool,
    /// Game speed multiplier (1, 2, 4, 6). Affects real-time tick rate only.
    #[serde(default = "default_speed")]
    pub speed_multiplier: u8,
}

fn default_speed() -> u8 {
    1
}

impl UiState {
    /// Toggle a panel open/closed. Resets selection and initializes panel-specific UI state.
    pub fn toggle_panel(&mut self, panel: Panel, num_regions: usize) {
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
            // Once the player opens any panel, the home splash animation is done.
            self.home_splash_done = true;
            match panel {
                Panel::Medicines => self.medicine_ui = Some(MedicineUiState::BrowseMedicines),
                Panel::Research => self.research_ui = Some(ResearchUiState::BrowseCategories),
                Panel::Policy => {
                    self.policy_ui = Some(PolicyUiState::BrowseRegions);
                    // Pre-select the region matching the current map selection
                    let order = grid_reading_order(num_regions);
                    if let Some(pos) = order.iter().position(|&idx| idx == self.map_selection) {
                        self.panel_selection = pos;
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle Escape — go back one step in the current panel's wizard, or close the panel.
    pub fn close_panel(&mut self) {
        match self.open_panel {
            Panel::Medicines => {
                match self.medicine_ui.clone() {
                    Some(MedicineUiState::DeployResult { medicine_idx, .. }) => {
                        self.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                        self.panel_selection = 0;
                    }
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
                    Some(ResearchUiState::ConfirmProject { track, .. }) => {
                        self.research_ui = Some(ResearchUiState::BrowseProjects { track: *track });
                        self.panel_selection = 0;
                    }
                    Some(ResearchUiState::ViewActive { track }) => {
                        self.research_ui = Some(ResearchUiState::BrowseProjects { track: *track });
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


    /// Maximum selection index for the current panel and UI sub-state.
    /// Used by navigation (SelectNext) to bounds-check panel_selection.
    pub fn panel_selection_max(&self, state: &GameState) -> usize {
        match self.open_panel {
            Panel::Threats => state.diseases.len().saturating_sub(1),
            Panel::Medicines => match &self.medicine_ui {
                Some(MedicineUiState::BrowseMedicines) => {
                    state.medicines
                        .iter()
                        .filter(|m| m.unlocked)
                        .count()
                        .saturating_sub(1)
                }
                Some(MedicineUiState::SelectRegion { .. }) => {
                    state.regions.len().saturating_sub(1)
                }
                Some(MedicineUiState::SelectTarget { medicine_idx, .. }) => {
                    state.medicines[*medicine_idx]
                        .num_deploy_targets()
                        .saturating_sub(1)
                }
                Some(MedicineUiState::ConfirmDeploy { .. })
                | Some(MedicineUiState::DeployResult { .. })
                | None => 0,
            },
            Panel::Research => match &self.research_ui {
                Some(ResearchUiState::BrowseCategories) => 2, // Field, Applied, Basic
                Some(ResearchUiState::BrowseProjects { track }) => {
                    let active = state.research_slot(*track).is_some();
                    if active {
                        0
                    } else {
                        state.available_projects(*track).len().saturating_sub(1)
                    }
                }
                Some(ResearchUiState::ConfirmProject { .. }) => 0,
                Some(ResearchUiState::ViewActive { .. }) => 0,
                None => 0,
            },
            Panel::Policy => match &self.policy_ui {
                Some(PolicyUiState::BrowseRegions) => {
                    state.regions.len().saturating_sub(1)
                }
                Some(PolicyUiState::ManagePolicies { .. }) => POLICY_COUNT - 1,
                None => 0,
            },
            _ => 0,
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

    /// Navigate left on the map (always, regardless of open panel).
    /// Left/right are reserved for region navigation — panels use up/down only.
    pub fn select_left(&mut self, num_regions: usize) {
        self.map_selection = map_navigate(
            self.map_selection,
            MapDirection::Left,
            num_regions,
        );
        self.sync_panel_region();
    }

    /// Navigate right on the map (always, regardless of open panel).
    /// Left/right are reserved for region navigation — panels use up/down only.
    pub fn select_right(&mut self, num_regions: usize) {
        self.map_selection = map_navigate(
            self.map_selection,
            MapDirection::Right,
            num_regions,
        );
        self.sync_panel_region();
    }

    /// Keep region-specific panel views in sync with the map selection.
    fn sync_panel_region(&mut self) {
        if let Some(PolicyUiState::ManagePolicies { region_idx }) = &mut self.policy_ui {
            *region_idx = self.map_selection;
        }
        match &self.medicine_ui {
            Some(MedicineUiState::SelectTarget { region_idx, medicine_idx }) => {
                if *region_idx != self.map_selection {
                    let med = *medicine_idx;
                    self.medicine_ui = Some(MedicineUiState::SelectTarget {
                        medicine_idx: med,
                        region_idx: self.map_selection,
                    });
                    self.panel_selection = 0;
                }
            }
            Some(MedicineUiState::ConfirmDeploy { medicine_idx, .. }) => {
                // Regress to target selection — don't silently change region on confirm screen
                let med = *medicine_idx;
                self.medicine_ui = Some(MedicineUiState::SelectTarget {
                    medicine_idx: med,
                    region_idx: self.map_selection,
                });
                self.panel_selection = 0;
            }
            _ => {}
        }
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
                    // Pre-select the region matching the current map selection
                    let order = grid_reading_order(state.regions.len());
                    self.panel_selection = order.iter()
                        .position(|&idx| idx == self.map_selection)
                        .unwrap_or(0);
                }
                None
            }
            Some(MedicineUiState::SelectRegion { medicine_idx }) => {
                let order = grid_reading_order(state.regions.len());
                let region_idx = order.get(self.panel_selection).copied().unwrap_or(0);
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
            Some(MedicineUiState::DeployResult { medicine_idx, .. }) => {
                self.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                self.panel_selection = 0;
                None
            }
            None => None,
        }
    }

    fn handle_research_confirm(&mut self, state: &GameState) -> Option<GameCommand> {
        match self.research_ui.clone() {
            Some(ResearchUiState::BrowseCategories) => {
                let track = match self.panel_selection {
                    0 => ResearchTrack::Field,
                    1 => ResearchTrack::Applied,
                    _ => ResearchTrack::Basic,
                };
                self.research_ui = Some(ResearchUiState::BrowseProjects { track });
                self.panel_selection = 0;
                None
            }
            Some(ResearchUiState::BrowseProjects { track }) => {
                if state.research_slot(track).is_some() {
                    self.research_ui = Some(ResearchUiState::ViewActive { track });
                    self.panel_selection = 0;
                } else {
                    let count = state.available_projects(track).len();
                    if count > 0 {
                        self.research_ui = Some(ResearchUiState::ConfirmProject {
                            track,
                            project_idx: self.panel_selection,
                            double_personnel: false,
                        });
                        self.panel_selection = 0;
                    }
                }
                None
            }
            Some(ResearchUiState::ConfirmProject { track, project_idx, double_personnel }) => {
                Some(GameCommand::StartResearch { track, project_idx, double_personnel })
            }
            Some(ResearchUiState::ViewActive { track }) => {
                // ViewActive uses up/down for personnel, Confirm goes back
                self.research_ui = Some(ResearchUiState::BrowseProjects { track });
                self.panel_selection = 0;
                None
            }
            None => None,
        }
    }

    fn handle_policy_confirm(&mut self, state: &GameState) -> Option<GameCommand> {
        match self.policy_ui.clone() {
            Some(PolicyUiState::BrowseRegions) => {
                let order = grid_reading_order(state.regions.len());
                let region_idx = order.get(self.panel_selection).copied().unwrap_or(0);
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

/// Reading order of regions: left-to-right, top-to-bottom through the grid.
/// Used for left/right wrap-around navigation and canonical display order.
pub fn grid_reading_order(num_regions: usize) -> Vec<usize> {
    let max_row = MAP_GRID.iter().take(num_regions).map(|&(_, r)| r).max().unwrap_or(0);
    let max_col = MAP_GRID.iter().take(num_regions).map(|&(c, _)| c).max().unwrap_or(0);
    let mut order = Vec::new();
    for r in 0..=max_row {
        for c in 0..=max_col {
            if let Some(idx) = region_at_grid(c, r) {
                if idx < num_regions {
                    order.push(idx);
                }
            }
        }
    }
    order
}

/// Navigate the map selection in a direction. Returns the new selection index.
///
/// Left/right use reading order (left-to-right, top-to-bottom) with full
/// wrap-around: right from the last region goes to the first, left from the
/// first goes to the last. This lets players cycle through all regions with
/// arrow keys even while a panel is open (panels only use up/down).
///
/// Up/down move within the same column without wrapping.
pub fn map_navigate(current: usize, direction: MapDirection, num_regions: usize) -> usize {
    if num_regions == 0 || current >= num_regions || current >= MAP_GRID.len() {
        return current;
    }
    match direction {
        MapDirection::Up | MapDirection::Down => {
            let (col, row) = MAP_GRID[current];
            let (new_col, new_row) = match direction {
                MapDirection::Up => (col, row.wrapping_sub(1)),
                MapDirection::Down => (col, row + 1),
                _ => unreachable!(),
            };
            region_at_grid(new_col, new_row)
                .filter(|&idx| idx < num_regions)
                .unwrap_or(current)
        }
        MapDirection::Left | MapDirection::Right => {
            let order = grid_reading_order(num_regions);
            let pos = order.iter().position(|&idx| idx == current).unwrap_or(0);
            let new_pos = match direction {
                MapDirection::Right => (pos + 1) % order.len(),
                MapDirection::Left => (pos + order.len() - 1) % order.len(),
                _ => unreachable!(),
            };
            order[new_pos]
        }
    }
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

        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // Canonical region connections (indices match vec order):
        //   0: N.America ↔ S.America(1), Europe(2)
        //   1: S.America ↔ N.America(0)               ← refugium
        //   2: Europe    ↔ N.America(0), Africa(3), Asia(4)  ← central hub
        //   3: Africa    ↔ Europe(2), Asia(4)
        //   4: Asia      ↔ Europe(2), Africa(3), Oceania(5)
        //   5: Oceania   ↔ Asia(4)                     ← refugium
        let mut regions = vec![
            Region {
                name: "North America".into(),
                population: 500_000_000,
                connections: vec![1, 2],
                infections: vec![],
                collapse_threshold: 0.55, // Fragile — collapses at 45% dead
                collapsed: false,
                collapsed_at_tick: None,
            },
            Region {
                name: "South America".into(),
                population: 430_000_000,
                connections: vec![0, 3],
                infections: vec![],
                collapse_threshold: 0.55, // Moderate resilience — 45% dead
                collapsed: false,
                collapsed_at_tick: None,
            },
            Region {
                name: "Europe".into(),
                population: 750_000_000,
                connections: vec![0, 3, 4],
                infections: vec![],
                collapse_threshold: 0.50, // Developed infrastructure — 50% dead
                collapsed: false,
                collapsed_at_tick: None,
            },
            Region {
                name: "Africa".into(),
                population: 1_400_000_000,
                connections: vec![1, 2, 4],
                infections: vec![],
                collapse_threshold: 0.50, // Resilient — 50% dead
                collapsed: false,
                collapsed_at_tick: None,
            },
            Region {
                name: "Asia".into(),
                population: 4_700_000_000,
                connections: vec![2, 3, 5],
                infections: vec![],
                collapse_threshold: 0.50, // Huge population — 50% dead
                collapsed: false,
                collapsed_at_tick: None,
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![4],
                infections: vec![],
                collapse_threshold: 0.50, // Small but developed — 50% dead
                collapsed: false,
                collapsed_at_tick: None,
            },
        ];

        // --- Initial disease ---
        // Start with a single disease so the player can learn the ropes.
        // Additional diseases emerge mid-game via spawn_disease().
        let available_types = vec![
            PathogenType::RnaVirus,
            PathogenType::RnaVirus,   // 2× weight
            PathogenType::DnaVirus,
            PathogenType::Bacterium,
            PathogenType::Bacterium,  // 2× weight
        ];
        let chosen_types = vec![available_types[rng.r#gen::<usize>() % available_types.len()]];

        let mut diseases = Vec::new();
        let mut used_names: Vec<String> = Vec::new();
        for pathogen_type in &chosen_types {
            let mut disease = Disease::generate(&mut rng, *pathogen_type, &used_names, false);
            disease.detected = true; // starting disease is already detected — player needs something to act on
            used_names.push(disease.name.clone());
            diseases.push(disease);
        }

        // --- Place initial outbreak ---
        // The starting disease has already been detected by global health systems.
        // We seed it well above the detection threshold so the player can immediately
        // see infections on the map and begin field research to identify it.
        let region_idx = rng.r#gen::<usize>() % regions.len();
        let infected = 1_000.0 + rng.r#gen::<f64>() * 2_000.0;
        let dead = infected * 0.01; // ~1% already dead when the player takes over
        regions[region_idx].infections.push(RegionDiseaseState {
            disease_idx: 0,
            infected,
            dead,
            immune: 0.0,
        });

        // --- Generate medicines to match diseases ---
        let mut medicines: Vec<Medicine> = diseases.iter().enumerate()
            .map(|(i, d)| Medicine::new_targeted(i, d.pathogen_type))
            .collect();

        // One broad-spectrum medicine targeting all diseases
        let all_disease_indices: Vec<usize> = (0..diseases.len()).collect();
        medicines.push(Medicine {
            name: "Broad-Spectrum".into(),
            therapy_type: TherapyType::BroadSpectrum,
            target_diseases: all_disease_indices,
            cost: 300.0,
            doses: 10_000_000.0,
            max_doses: 10_000_000.0,
            unlocked: false,
            tested_against: vec![],
            strain_generations: vec![],
        });

        Self {
            tick: 0,
            sim_state: SimState::Running,
            rng,
            resources: Resources {
                funding: 300.0,
                personnel: 20,
                political_power: 0.0,
                personnel_accum: 0.0,
                attrition_accum: 0.0,
            },
            policies: vec![RegionPolicy::default(); regions.len()],
            regions,
            diseases,
            medicines,
            field_research: None,
            applied_research: None,
            basic_research: None,
            unlocked_techs: vec![],
            outcome: GameOutcome::Playing,
            events: vec![],
            active_crisis: None,
            crisis_cooldowns: HashMap::new(),
            auto_resolve_crises: HashMap::new(),
            history: vec![],
            ui: UiState {
                open_panel: Panel::None,
                panel_selection: 0,
                medicine_ui: None,
                map_selection: 0,
                research_ui: None,
                policy_ui: None,
                status_message: None,
                crisis_selection: 0,
                crisis_auto_resolve: false,
                home_splash_done: false,
                speed_multiplier: 1,
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

    /// Total infected from detected diseases only (for UI display).
    pub fn total_infected_detected(&self) -> f64 {
        self.regions.iter()
            .flat_map(|r| &r.infections)
            .filter(|inf| self.diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.infected)
            .sum()
    }

    /// Total screened infections across all regions — what the player sees.
    /// Each region's infections are scaled by that region's screening visibility rate.
    pub fn total_infected_screened(&self) -> f64 {
        self.regions.iter().enumerate()
            .map(|(i, r)| {
                let vis = self.policies.get(i)
                    .map(|p| p.screening.visibility_rate())
                    .unwrap_or(ScreeningLevel::None.visibility_rate());
                r.screened_infected(&self.diseases, vis)
            })
            .sum()
    }

    /// Screening visibility rate for a specific region.
    pub fn screening_visibility(&self, region_idx: usize) -> f64 {
        self.policies.get(region_idx)
            .map(|p| p.screening.visibility_rate())
            .unwrap_or(ScreeningLevel::None.visibility_rate())
    }

    /// Best screening level across all regions — used for detection threshold.
    pub fn best_screening_level(&self) -> ScreeningLevel {
        self.policies.iter()
            .map(|p| p.screening)
            .max_by(|a, b| {
                a.visibility_rate().partial_cmp(&b.visibility_rate())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_default()
    }

    /// Total dead from detected diseases only (for UI display).
    pub fn total_dead_detected(&self) -> f64 {
        self.regions.iter()
            .flat_map(|r| &r.infections)
            .filter(|inf| self.diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.dead)
            .sum()
    }

    pub fn personnel_busy(&self) -> u32 {
        let field = self.field_research.as_ref().map_or(0, |p| p.personnel_assigned);
        let applied = self.applied_research.as_ref().map_or(0, |p| p.personnel_assigned);
        let basic = self.basic_research.as_ref().map_or(0, |p| p.personnel_assigned);
        let policy: u32 = self.policies.iter().map(|p| p.personnel_cost()).sum();
        field + applied + basic + policy
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
        // Political Power bonus: up to 50% more funding at full POL
        income *= 1.0 + self.resources.political_power * 0.5;
        income
    }

    /// Income lost per tick due to travel ban penalties (halved region contributions).
    /// Returns 0 if no travel bans are active.
    pub fn travel_ban_income_penalty(&self) -> f64 {
        let total_pop: f64 = self.regions.iter().map(|r| r.population as f64).sum();
        if total_pop <= 0.0 {
            return 0.0;
        }
        let mut penalty = 0.0;
        for (i, region) in self.regions.iter().enumerate() {
            if self.policies.get(i).is_some_and(|p| p.travel_ban) {
                let pop = region.population as f64;
                let dead: f64 = region.infections.iter().map(|inf| inf.dead).sum();
                let healthy_frac = (pop - dead).max(0.0) / pop;
                let region_share = pop / total_pop;
                // Penalty is the income lost: the portion that travel_ban_factor removes
                penalty += BASE_FUNDING_INCOME * region_share * healthy_frac * (1.0 - TRAVEL_BAN_INCOME_PENALTY);
            }
        }
        penalty
    }

    /// Per-tick cost to maintain all personnel on the roster.
    pub fn personnel_upkeep_rate(&self) -> f64 {
        self.resources.personnel as f64 * PERSONNEL_UPKEEP_COST
    }

    /// Total initial population across all regions (before any deaths).
    pub fn initial_population(&self) -> f64 {
        self.regions.iter().map(|r| r.population as f64).sum()
    }

    /// Effective POL threshold for a policy in a specific region.
    /// Regional severity (infection rate) reduces the threshold — a crisis
    /// in a region justifies action even with low global political will.
    pub fn effective_pol_threshold(&self, region_idx: usize, policy_idx: usize) -> f64 {
        let base = POLICY_POL_THRESHOLDS.get(policy_idx).copied().unwrap_or(1.0);
        let region = match self.regions.get(region_idx) {
            Some(r) => r,
            None => return base,
        };
        // severity = fraction of region population currently infected
        let pop = region.population as f64;
        if pop <= 0.0 { return base; }
        let infected: f64 = region.infections.iter().map(|i| i.infected).sum();
        let severity = (infected / pop).min(1.0);
        // High severity reduces threshold by up to 50%
        (base * (1.0 - severity * 0.5)).max(0.0)
    }

    /// Whether a policy can be activated given current POL and regional severity.
    pub fn policy_unlocked(&self, region_idx: usize, policy_idx: usize) -> bool {
        self.resources.political_power >= self.effective_pol_threshold(region_idx, policy_idx)
    }

    /// Spawn a new disease mid-game: generates a random disease, places an initial
    /// outbreak in a random region, and creates a matching targeted medicine.
    /// Returns `(disease_idx, region_idx)` if successful, or `None` if at the cap.
    /// Uses `self.rng` — caller must have extracted rng if borrowing mutably.
    pub fn spawn_disease(&mut self, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {


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

        let used_names: Vec<String> = self.diseases.iter().map(|d| d.name.clone()).collect();
        let disease_idx = self.diseases.len();
        let mut disease = Disease::generate(rng, pathogen_type, &used_names, true);
        disease.detected = false; // starts undetected
        self.diseases.push(disease);

        // Place initial outbreak in a random region
        let region_idx = rng.r#gen::<usize>() % self.regions.len();
        let initial_infected = 500.0 + rng.r#gen::<f64>() * 2_000.0;
        self.regions[region_idx].infections.push(RegionDiseaseState {
            disease_idx,
            infected: initial_infected,
            dead: 0.0,
            immune: 0.0,
        });

        self.medicines.push(Medicine::new_targeted(disease_idx, pathogen_type));

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

    /// Spawn a disease with stats scaled up based on current game day.
    /// Later diseases are tougher — simulating evolved superbugs.
    /// Scaling: +5% per day elapsed, capped at 3x base stats.
    pub fn spawn_disease_scaled(&mut self, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
        let day = self.tick as f64 / TICKS_PER_DAY;
        let scale = (1.0 + day * 0.05).min(3.0);

        let result = self.spawn_disease(rng)?;
        let (disease_idx, _) = result;

        // Boost the newly spawned disease's stats
        let d = &mut self.diseases[disease_idx];
        d.infectivity *= scale;
        d.lethality *= scale;
        d.cross_region_spread *= scale;
        // Don't scale recovery — harder diseases should be harder to recover from

        Some(result)
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
                "You developed medicines but never deployed them. Use [M] Medicines to protect or treat regions."
                    .to_string(),
            );
        } else if unlocked_meds == 0 && unidentified < self.diseases.len() {
            // Identified threats but never developed medicine
            if !self.unlocked_techs.contains(&BasicTech::TargetedDrugDesign) {
                tips.push(
                    "Research Targeted Drug Design in [R] Basic Research to unlock targeted medicine development."
                        .to_string(),
                );
            } else {
                tips.push(
                    "You identified threats but never developed a medicine. Use Applied Research to develop treatments."
                        .to_string(),
                );
            }
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
        // Identify Threat: diseases not fully known, sorted by knowledge ascending
        // (unknown diseases first, then partially identified)
        let mut identify_targets: Vec<(usize, f64)> = self.diseases.iter().enumerate()
            .filter(|(_, d)| d.detected && d.knowledge < KNOWLEDGE_FULL)
            .map(|(i, d)| (i, d.knowledge))
            .collect();
        identify_targets.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (i, _knowledge) in identify_targets {
            let kind = ResearchKind::IdentifyThreat { disease_idx: i };
            if active_kind != Some(&kind) {
                projects.push(kind);
            }
        }
        // Genomic Sequencing: fully identified diseases that still mutate
        for (i, disease) in self.diseases.iter().enumerate() {
            if disease.knowledge >= KNOWLEDGE_FULL
                && disease.pathogen_type.mutation_rate() > 0.00002
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

    /// Get the active research project for a given track.
    pub fn research_slot(&self, track: ResearchTrack) -> Option<&ResearchProject> {
        match track {
            ResearchTrack::Field => self.field_research.as_ref(),
            ResearchTrack::Applied => self.applied_research.as_ref(),
            ResearchTrack::Basic => self.basic_research.as_ref(),
        }
    }

    /// Available research projects for a given track (excludes currently active).
    pub fn available_projects(&self, track: ResearchTrack) -> Vec<ResearchKind> {
        match track {
            ResearchTrack::Field => self.available_field_projects(),
            ResearchTrack::Applied => self.available_applied_projects(),
            ResearchTrack::Basic => self.available_basic_projects(),
        }
    }

    /// Available basic research projects — techs whose prereqs are met and not yet unlocked.
    pub fn available_basic_projects(&self) -> Vec<ResearchKind> {
        let active_kind = self.basic_research.as_ref().map(|p| &p.kind);
        BasicTech::all()
            .iter()
            .filter(|tech| {
                !self.unlocked_techs.contains(tech)
                    && tech.prerequisites_met(self)
            })
            .map(|&tech| ResearchKind::BasicResearch { tech })
            .filter(|kind| active_kind != Some(kind))
            .collect()
    }

    /// Available applied research projects (excludes currently active).
    pub fn available_applied_projects(&self) -> Vec<ResearchKind> {
        let active_kind = self.applied_research.as_ref().map(|p| &p.kind);
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
            // Targeted medicines (Antiviral/Antibiotic) require TargetedDrugDesign.
            // BroadSpectrum medicines can be developed without basic research.
            let needs_tech = med.therapy_type != TherapyType::BroadSpectrum;
            let has_tech = !needs_tech
                || self.unlocked_techs.contains(&BasicTech::TargetedDrugDesign);
            if has_knowledge && has_tech {
                let kind = ResearchKind::DevelopMedicine { medicine_idx: i };
                if active_kind != Some(&kind) {
                    projects.push(kind);
                }
            }
        }
        // Train Personnel: always available as an applied project
        let kind = ResearchKind::TrainPersonnel;
        if active_kind != Some(&kind) {
            projects.push(kind);
        }
        projects
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
        assert!(state.total_dead() > 0.0);
        assert!(state.diseases[0].detected);
    }

    #[test]
    fn default_state_has_medicines() {
        let state = GameState::new_default(1);
        let disease_count = state.diseases.len();
        assert_eq!(disease_count, 1, "expected 1 starting disease, got {}", disease_count);
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
        assert!(tips.iter().any(|t| t.contains("develop") || t.contains("Applied")),
            "should suggest developing medicine: {:?}", tips);
    }
}
