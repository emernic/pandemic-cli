use std::collections::{HashMap, VecDeque};

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Deserializer, Serialize};

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

/// Deserialize field_research: accepts both old `Option<ResearchProject>` (single)
/// and new `Vec<ResearchProject>` (parallel) save formats.
fn deserialize_field_research<'de, D>(deserializer: D) -> Result<Vec<ResearchProject>, D::Error>
where D: Deserializer<'de> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FieldResearch {
        Vec(Vec<ResearchProject>),
        Option(Option<ResearchProject>),
    }
    match FieldResearch::deserialize(deserializer)? {
        FieldResearch::Vec(v) => Ok(v),
        FieldResearch::Option(Some(p)) => Ok(vec![p]),
        FieldResearch::Option(None) => Ok(vec![]),
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
    /// Active field research projects (Identify Threat, Clinical Trial, Genomic Sequencing).
    /// Multiple projects can run simultaneously, gated by personnel.
    #[serde(default, deserialize_with = "deserialize_field_research")]
    pub field_research: Vec<ResearchProject>,
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
    /// Emergency decrees — permanent, irreversible global decisions.
    #[serde(default)]
    pub enacted_decrees: EnactedDecrees,
    #[serde(default)]
    pub outcome: GameOutcome,
    /// Events from the most recent tick. Consumed by the UI layer for status
    /// messages. Cleared at the start of each tick.
    #[serde(skip)]
    pub events: Vec<GameEvent>,
    /// Persistent log of notable events with timestamps (day number, message).
    /// Populated by the UI layer from transient `events`. Capped at 50 entries.
    #[serde(default)]
    pub event_log: VecDeque<(f64, String)>,
    /// Active crisis event requiring player decision. Game pauses while active.
    #[serde(default)]
    pub active_crisis: Option<CrisisEvent>,
    /// Per-type cooldowns: crisis tag → tick when it last fired.
    /// Used to prevent the same crisis type repeating within CRISIS_TYPE_COOLDOWN ticks.
    #[serde(default)]
    pub crisis_cooldowns: HashMap<String, u64>,
    /// Scheduled follow-up crises from previous choices. Each entry is
    /// (fire_at_tick, crisis_kind). Checked every tick; fires when due.
    #[serde(default)]
    pub pending_crises: Vec<(u64, CrisisKind)>,
    /// Auto-resolve preferences: crisis tag → choice index (0 = A, 1 = B).
    /// When a crisis fires whose tag matches, it's resolved immediately without pausing.
    #[serde(default)]
    pub auto_resolve_crises: HashMap<String, usize>,
    /// Auto-research: when a project completes, automatically start the next
    /// highest-priority available project (if affordable). Per-track toggle.
    #[serde(default)]
    pub auto_research: [bool; 3],
    /// Historical snapshots for dashboard charts. Recorded every HISTORY_INTERVAL ticks.
    #[serde(default)]
    pub history: Vec<HistorySnapshot>,
    /// Consecutive ticks the player has had zero agency (no funds, no research, no doses).
    /// After MERCY_RULE_TICKS, the game ends.
    #[serde(default)]
    pub zero_agency_ticks: u64,
    /// True if defeat was triggered by the mercy rule (zero agency) rather than
    /// all regions collapsing. Used by the UI to show a distinct defeat message.
    #[serde(default)]
    pub mercy_rule: bool,
    /// Per-disease highest threat alert level already fired (0=none, 1=1M, 2=100M, 3=1B).
    /// Prevents repeat alerts for the same threshold.
    #[serde(default)]
    pub threat_alert_level: Vec<u8>,
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

/// Maximum concurrent field research projects. Personnel-gated: each project
/// requires dedicated staff, so the real limit is usually personnel, not slots.
pub const MAX_FIELD_RESEARCH: usize = 3;

// Medicine constants.
/// Fraction of infected treated per deployment (before efficacy modifiers).
/// Treatment is proportional — scales with infection size instead of fixed dose count.
pub const TREATMENT_FRACTION: f64 = 0.5;
/// Fraction of susceptible population vaccinated per deployment (before efficacy).
/// Vaccination is proportional like treatment — each deploy protects a meaningful
/// fraction, making repeated deployments build toward herd immunity.
pub const VACCINATION_FRACTION: f64 = 0.02;

/// Efficacy multiplier when deploying a medicine against a disease it wasn't
/// specifically developed for, but whose mechanism matches the pathogen type
/// (e.g., a CellWall inhibitor developed for Bacterium-A used against Bacterium-B).
pub const CROSS_REACTIVE_PENALTY: f64 = 0.5;

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
/// 20 personnel × 0.06 = $1.2/tick = $144/day upkeep vs $360/day gross income → ~$216/day net.
/// Training 5 more costs $36/day (17% of net) — meaningful but not a trap.
/// History: 0.10 made training a trap (50% of income); 0.03 doubled income, trivializing economy.
pub const PERSONNEL_UPKEEP_COST: f64 = 0.06;
/// Fraction of infected people who are too sick to contribute economically.
/// 70% are incapacitated (hospitalized, quarantined, bedridden); 30% are mild/asymptomatic.
pub const INFECTED_INCAPACITATION_RATE: f64 = 0.7;
pub const TRAVEL_BAN_INCOME_PENALTY: f64 = 0.5;
pub const TRAVEL_BAN_COST: f64 = 1.0;
pub const TRAVEL_BAN_PERSONNEL: u32 = 3;
pub const QUARANTINE_COST: f64 = 0.6;
pub const QUARANTINE_PERSONNEL: u32 = 3;
pub const HOSPITAL_SURGE_COST: f64 = 0.4;
pub const HOSPITAL_SURGE_PERSONNEL: u32 = 2;
/// Hospital Surge increases infectivity by this factor (1.25 = +25% spread).
pub const HOSPITAL_SURGE_SPREAD_FACTOR: f64 = 1.25;
pub const BORDER_CONTROLS_COST: f64 = 0.1;
pub const BORDER_CONTROLS_PERSONNEL: u32 = 1;
pub const WATER_SANITATION_COST: f64 = 0.3;
pub const WATER_SANITATION_PERSONNEL: u32 = 1;
pub const MARTIAL_LAW_COST: f64 = 1.5;
pub const MARTIAL_LAW_PERSONNEL: u32 = 4;
/// One-time funding cost for nuclear annihilation (no ongoing cost).
pub const NUCLEAR_ANNIHILATION_COST: f64 = 200.0;
/// One-time per-region cost to invest in healthcare infrastructure.
/// Permanently reduces lethality by 25% in the region. Competes with
/// research spending ($350-700) for early-game funding.
pub const HEALTHCARE_INVESTMENT_COST: f64 = 400.0;

/// Disease surveillance intensity. Each tier reveals different information
/// and only Mass Rapid screening actively reduces disease spread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScreeningLevel {
    #[default]
    None,
    /// Rough infected estimates. Cheap but inaccurate.
    #[serde(alias = "Low")]
    Basic,
    /// Reveals infected + immune counts with moderate accuracy.
    #[serde(alias = "Medium")]
    Antigen,
    /// Near-complete data on infected/immune AND reduces disease spread.
    #[serde(alias = "High")]
    MassRapid,
}

// Emergency Decree constants — permanent, irreversible global decisions.
pub const DECREE_COUNT: usize = 3;
/// Conscript Researchers: immediately gain personnel, permanent income penalty.
pub const CONSCRIPT_PERSONNEL_GAIN: u32 = 10;
/// Per-tick income penalty for Conscript Researchers ($50/day = 0.417/tick).
pub const CONSCRIPT_INCOME_PENALTY: f64 = 50.0 / 120.0;
/// Authorize Human Trials: clinical trial duration multiplier (0.5 = half duration).
pub const HUMAN_TRIALS_SPEED: f64 = 0.5;
/// Chance of adverse event killing infected when a human-trial clinical trial completes.
pub const HUMAN_TRIALS_ADVERSE_CHANCE: f64 = 0.30;
/// Fraction of infected killed in the adverse event.
pub const HUMAN_TRIALS_KILL_FRACTION: f64 = 0.05;
/// Sacrifice Region: income multiplier for remaining regions.
pub const SACRIFICE_INCOME_BONUS: f64 = 1.20;
/// Minimum POL required for each decree (indexed by decree position).
pub const DECREE_POL_THRESHOLDS: [f64; DECREE_COUNT] = [
    0.30, // Conscript Researchers — forcing citizens
    0.40, // Authorize Human Trials — ethical violation
    0.50, // Sacrifice Region — abandoning millions
];

/// Per-tick cost for each screening level.
pub const SCREENING_BASIC_COST: f64 = 0.2;
pub const SCREENING_ANTIGEN_COST: f64 = 0.5;
pub const SCREENING_MASS_RAPID_COST: f64 = 1.0;

impl ScreeningLevel {
    /// Fraction of actual infections visible to the player.
    /// Without screening, only ~15% of cases are reported organically.
    pub fn visibility_rate(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.15,
            ScreeningLevel::Basic => 0.40,
            ScreeningLevel::Antigen => 0.75,
            ScreeningLevel::MassRapid => 0.95,
        }
    }

    /// Multiplier on the detection threshold for hidden diseases.
    /// Lower = detect new threats sooner.
    pub fn detection_multiplier(&self) -> f64 {
        match self {
            ScreeningLevel::None => 1.0,
            ScreeningLevel::Basic => 0.7,
            ScreeningLevel::Antigen => 0.4,
            ScreeningLevel::MassRapid => 0.15,
        }
    }

    /// Per-tick funding cost for this screening level.
    pub fn funding_cost(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.0,
            ScreeningLevel::Basic => SCREENING_BASIC_COST,
            ScreeningLevel::Antigen => SCREENING_ANTIGEN_COST,
            ScreeningLevel::MassRapid => SCREENING_MASS_RAPID_COST,
        }
    }

    /// Personnel required for this screening level.
    pub fn personnel_cost(&self) -> u32 {
        match self {
            ScreeningLevel::None => 0,
            ScreeningLevel::Basic => 1,
            ScreeningLevel::Antigen => 2,
            ScreeningLevel::MassRapid => 4,
        }
    }

    /// Display name for the policy panel.
    pub fn label(&self) -> &'static str {
        match self {
            ScreeningLevel::None => "None",
            ScreeningLevel::Basic => "Basic",
            ScreeningLevel::Antigen => "Antigen",
            ScreeningLevel::MassRapid => "Mass Rapid",
        }
    }

    /// Whether this screening level reveals immune population counts.
    /// Only Antigen and Mass Rapid testing can identify immunity.
    pub fn shows_immune(&self) -> bool {
        matches!(self, ScreeningLevel::Antigen | ScreeningLevel::MassRapid)
    }

    /// Spread reduction factor (1.0 = no reduction, lower = less spread).
    /// Only Mass Rapid screening actively reduces disease transmission.
    pub fn spread_factor(&self) -> f64 {
        match self {
            ScreeningLevel::MassRapid => 0.75, // 25% spread reduction
            _ => 1.0,
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
    /// Reduces collapse threshold by 15 percentage points. Must be enacted
    /// before collapse — cleared when region collapses like all other policies.
    #[serde(default)]
    pub martial_law: bool,
    /// One-shot action for collapsed regions only. Kills 99% of remaining
    /// population, eliminating disease spread from the region. Irreversible.
    #[serde(default)]
    pub nuclear_annihilation: bool,
}

/// Total number of policy types available per region.
///
/// **Policy index mapping** (used across state.rs, engine/policy.rs, ui/policy.rs):
///   0 = Travel Ban        5 = Low Screening      8 = Martial Law
///   1 = Quarantine         6 = Medium Screening   9 = Nuclear Annihilation
///   2 = Hospital Surge     7 = High Screening    10 = Healthcare Investment
///   3 = Border Controls
///   4 = Water Sanitation
///
/// Display position equals policy_idx. If you add a new policy, you must update:
///   - POLICY_COUNT and POLICY_POL_THRESHOLDS (this file)
///   - get_bool/set_bool if it's a boolean policy (this file)
///   - toggle_policy and tick_enforce_costs (engine/policy.rs)
///   - render_manage policies vec (ui/policy.rs)
pub const POLICY_COUNT: usize = 11;

/// Minimum Political Power (0.0–1.0) required to activate each policy.
/// Indexed by policy_idx (see POLICY_COUNT doc for the mapping).
pub const POLICY_POL_THRESHOLDS: [f64; POLICY_COUNT] = [
    0.30, // Travel Ban — restricts citizens, needs political will
    0.25, // Quarantine — restricts movement, needs political will
    0.00, // Hospital Surge — defensive infrastructure, always available
    0.00, // Border Controls — checkpoint screening, always available
    0.00, // Water Sanitation — public health infrastructure, always available
    0.00, // Low Disease Screening — available immediately
    0.10, // Medium Disease Screening — mandatory testing, needs political will
    0.15, // High Disease Screening — mandatory mass testing, needs political will
    0.40, // Martial Law — drastic, needs high political will
    0.35, // Nuclear Annihilation — extreme, but collapsed regions raise urgency
    0.00, // Healthcare Investment — always available, encourages early spending
];

/// Short display name for each policy by index. Canonical source — used by
/// both engine (status messages) and UI (panel rendering).
pub fn policy_display_name(policy_idx: usize) -> &'static str {
    match policy_idx {
        0 => "Travel Ban",
        1 => "Quarantine",
        2 => "Hospital Surge",
        3 => "Border Controls",
        4 => "Water Sanitation",
        5 => "Basic Screening",
        6 => "Med Screening",
        7 => "Mass Screening",
        8 => "Martial Law",
        9 => "Nuclear Option",
        10 => "Healthcare",
        _ => "Unknown Policy",
    }
}

impl RegionPolicy {
    /// Funding cost adjusted for regional traits.
    /// TradeDependent: travel ban costs 2x.
    /// Always pass the region's traits — use `&[]` only when no region context exists.
    pub fn funding_cost(&self, traits: &[RegionTrait]) -> f64 {
        let trade_dependent = traits.contains(&RegionTrait::TradeDependent);
        let mut cost = 0.0;
        if self.travel_ban {
            cost += if trade_dependent { TRAVEL_BAN_COST * TRADE_DEPENDENT_TRAVEL_BAN_MULT } else { TRAVEL_BAN_COST };
        }
        if self.quarantine { cost += QUARANTINE_COST; }
        if self.hospital_surge { cost += HOSPITAL_SURGE_COST; }
        if self.border_controls { cost += BORDER_CONTROLS_COST; }
        if self.water_sanitation { cost += WATER_SANITATION_COST; }
        if self.martial_law { cost += MARTIAL_LAW_COST; }
        cost += self.screening.funding_cost();
        cost
    }

    /// Personnel cost adjusted for regional traits.
    /// LowInfrastructure: each active policy needs +1 personnel.
    /// Always pass the region's traits — use `&[]` only when no region context exists.
    pub fn personnel_cost(&self, traits: &[RegionTrait]) -> u32 {
        let low_infra = traits.contains(&RegionTrait::LowInfrastructure);
        let mut cost = 0u32;
        let mut active_count = 0u32;
        if self.travel_ban { cost += TRAVEL_BAN_PERSONNEL; active_count += 1; }
        if self.quarantine { cost += QUARANTINE_PERSONNEL; active_count += 1; }
        if self.hospital_surge { cost += HOSPITAL_SURGE_PERSONNEL; active_count += 1; }
        if self.border_controls { cost += BORDER_CONTROLS_PERSONNEL; active_count += 1; }
        if self.water_sanitation { cost += WATER_SANITATION_PERSONNEL; active_count += 1; }
        if self.martial_law { cost += MARTIAL_LAW_PERSONNEL; active_count += 1; }
        let screening_cost = self.screening.personnel_cost();
        cost += screening_cost;
        if screening_cost > 0 { active_count += 1; }
        if low_infra { cost += active_count; }
        cost
    }

    pub fn any_active(&self) -> bool {
        self.travel_ban || self.quarantine || self.hospital_surge
            || self.border_controls || self.water_sanitation
            || self.screening != ScreeningLevel::None
            || self.martial_law || self.nuclear_annihilation
    }

    /// Count of active policy toggles (each boolean that's true, plus
    /// screening levels above None count as 1).
    pub fn active_count(&self) -> u32 {
        let mut n = 0u32;
        if self.travel_ban { n += 1; }
        if self.quarantine { n += 1; }
        if self.hospital_surge { n += 1; }
        if self.border_controls { n += 1; }
        if self.water_sanitation { n += 1; }
        if self.martial_law { n += 1; }
        if self.screening != ScreeningLevel::None { n += 1; }
        n
    }

    pub fn clear_all(&mut self) {
        self.travel_ban = false;
        self.quarantine = false;
        self.hospital_surge = false;
        self.border_controls = false;
        self.water_sanitation = false;
        self.screening = ScreeningLevel::None;
        self.martial_law = false;
        // nuclear_annihilation is NOT cleared — it's permanent and post-collapse
    }

    /// Access a boolean policy field by index (0-4, 8-9).
    pub fn get_bool(&self, idx: usize) -> bool {
        match idx {
            0 => self.travel_ban,
            1 => self.quarantine,
            2 => self.hospital_surge,
            3 => self.border_controls,
            4 => self.water_sanitation,
            8 => self.martial_law,
            9 => self.nuclear_annihilation,
            _ => false,
        }
    }

    /// Set a boolean policy field by index (0-4, 8-9).
    pub fn set_bool(&mut self, idx: usize, val: bool) {
        match idx {
            0 => self.travel_ban = val,
            1 => self.quarantine = val,
            2 => self.hospital_surge = val,
            3 => self.border_controls = val,
            4 => self.water_sanitation = val,
            8 => self.martial_law = val,
            9 => self.nuclear_annihilation = val,
            _ => {}
        }
    }
}

/// Emergency decrees — permanent, irreversible global decisions with powerful
/// benefits and serious costs. Inspired by Frostpunk's "Book of Laws."
/// Once enacted, a decree cannot be undone.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnactedDecrees {
    /// Conscript Researchers: +10 personnel immediately, permanent income penalty.
    #[serde(default)]
    pub conscript_researchers: bool,
    /// Authorize Human Trials: clinical trials complete 50% faster,
    /// but 30% chance of adverse event (kills 5% of infected) on completion.
    #[serde(default)]
    pub authorize_human_trials: bool,
    /// Sacrifice Region: voluntarily collapse one region for +20% income from the rest.
    #[serde(default)]
    pub sacrificed_region: Option<usize>,
}

impl EnactedDecrees {
    pub fn is_enacted(&self, decree_idx: usize) -> bool {
        match decree_idx {
            0 => self.conscript_researchers,
            1 => self.authorize_human_trials,
            2 => self.sacrificed_region.is_some(),
            _ => false,
        }
    }

}

/// Display name for a decree by index.
pub fn decree_display_name(decree_idx: usize) -> &'static str {
    match decree_idx {
        0 => "Conscript Researchers",
        1 => "Authorize Human Trials",
        2 => "Sacrifice Region",
        _ => "Unknown Decree",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resources {
    pub funding: f64,
    pub personnel: u32,
    /// Political Power (0.0–1.0). Represents global willingness to act.
    /// Drifts toward a severity-based target (~30%/day). Crisis choices modify
    /// this directly, so POL hits take real time to recover from.
    #[serde(default)]
    pub political_power: f64,
    /// Legacy: old saves may include this field. Not used in game logic.
    #[serde(default, skip_serializing)]
    pub pol_crisis_modifier: f64,
    /// Fractional accumulator for POL-based personnel gains.
    #[serde(default)]
    pub personnel_accum: f64,
    /// Fractional accumulator for personnel attrition (when funding is $0).
    #[serde(default)]
    pub attrition_accum: f64,
    /// Tick when the player last rallied public support. Used for cooldown.
    #[serde(default)]
    pub last_rally_tick: Option<u64>,
}

impl Resources {
    /// Remaining cooldown ticks before another rally is possible. Returns 0 if ready.
    pub fn rally_cooldown_remaining(&self, current_tick: u64) -> u64 {
        match self.last_rally_tick {
            Some(t) => {
                let elapsed = current_tick.saturating_sub(t);
                RALLY_COOLDOWN_TICKS.saturating_sub(elapsed)
            }
            None => 0,
        }
    }
}

/// Regional traits that make each region play differently.
/// Each region has 1-2 traits that modify policy costs, spread rates, or resilience.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionTrait {
    /// Travel ban funding cost 2x, income penalty 75% instead of 50%.
    TradeDependent,
    /// Within-region spread rate +30% (crowded cities, public transit).
    DenseUrban,
    /// Cross-region inbound spread reduced 50% (natural isolation).
    IslandGeography,
    /// All policy personnel costs +1 (harder to staff programs).
    LowInfrastructure,
    /// Hospital surge lethality reduction 60% instead of 50%.
    StrongPublicHealth,
    /// Collapse threshold -10pp (region endures more before collapsing).
    ResilientPopulation,
}

/// Travel ban cost multiplier for TradeDependent regions.
pub const TRADE_DEPENDENT_TRAVEL_BAN_MULT: f64 = 2.0;
/// Travel ban income retained factor for TradeDependent regions (25% retained vs normal 50%).
pub const TRADE_DEPENDENT_INCOME_FACTOR: f64 = 0.25;

impl RegionTrait {
    pub fn label(&self) -> &'static str {
        match self {
            RegionTrait::TradeDependent => "Trade-Dependent",
            RegionTrait::DenseUrban => "Dense Urban",
            RegionTrait::IslandGeography => "Island Geography",
            RegionTrait::LowInfrastructure => "Low Infrastructure",
            RegionTrait::StrongPublicHealth => "Strong Public Health",
            RegionTrait::ResilientPopulation => "Resilient Population",
        }
    }

}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub population: u64,
    pub connections: Vec<usize>,
    pub infections: Vec<RegionDiseaseState>,
    /// Regional traits that affect policy costs and effectiveness.
    #[serde(default)]
    pub traits: Vec<RegionTrait>,
    /// Fraction of population that must remain alive to avoid collapse.
    /// E.g., 0.75 means the region collapses when alive drops below 75% of initial population.
    /// More developed regions have higher thresholds (fragile); less developed are more resilient.
    #[serde(default = "default_collapse_threshold")]
    pub collapse_threshold: f64,
    /// Total deaths across all diseases. This is the authoritative death count —
    /// used for susceptible calculations and population accounting. Per-disease
    /// `RegionDiseaseState.dead` tracks attribution for display only.
    #[serde(default)]
    pub dead: f64,
    /// Whether this region has collapsed. Collapsed regions lose all policies,
    /// block medicine deployment, and have reduced cross-region spread (0.3x).
    #[serde(default)]
    pub collapsed: bool,
    /// Tick when this region collapsed (None if still standing).
    #[serde(default)]
    pub collapsed_at_tick: Option<u64>,
    /// Permanent healthcare infrastructure investment. One-time purchase
    /// that reduces disease lethality by 25% in this region.
    #[serde(default)]
    pub healthcare_invested: bool,
    /// Per-capita income multiplier. Higher values mean this region
    /// contributes more funding per person. Default 1.0.
    #[serde(default = "default_one")]
    pub income_modifier: f64,
    /// Lethality multiplier from baseline healthcare quality. Lower = better
    /// healthcare = fewer deaths. Stacks with `healthcare_invested`. Default 1.0.
    #[serde(default = "default_one")]
    pub healthcare_modifier: f64,
    /// Tick when medicine was last deployed to this region. Used for deploy cooldown.
    #[serde(default)]
    pub last_deploy_tick: Option<u64>,
}

fn default_one() -> f64 {
    1.0
}

fn default_collapse_threshold() -> f64 {
    0.50
}

impl Region {
    pub fn has_trait(&self, t: RegionTrait) -> bool {
        self.traits.contains(&t)
    }

    pub fn alive(&self) -> f64 {
        (self.population as f64 - self.total_dead()).max(0.0)
    }

    /// Remaining cooldown ticks before this region can receive another deployment.
    /// Returns 0 if ready.
    pub fn deploy_cooldown_remaining(&self, current_tick: u64) -> u64 {
        match self.last_deploy_tick {
            Some(t) => {
                let elapsed = current_tick.saturating_sub(t);
                DEPLOY_COOLDOWN_TICKS.saturating_sub(elapsed)
            }
            None => 0,
        }
    }

    /// Total infected across all diseases, capped at population.
    /// May double-count people infected with multiple diseases simultaneously,
    /// but the cap prevents displaying more infected than the population.
    pub fn total_infected(&self) -> f64 {
        let raw: f64 = self.infections.iter().map(|i| i.infected).sum();
        raw.min(self.population as f64)
    }

    /// Total dead across all diseases. Uses the shared `dead` counter which
    /// is maintained by the simulation — no double-counting possible.
    pub fn total_dead(&self) -> f64 {
        self.dead
    }

    /// Total immune across all diseases, capped at population.
    /// May double-count people immune to multiple diseases simultaneously,
    /// but the cap prevents displaying more immune than the population.
    pub fn total_immune(&self) -> f64 {
        let raw: f64 = self.infections.iter().map(|i| i.immune).sum();
        raw.min(self.population as f64)
    }

    pub fn disease_state(&self, disease_idx: usize) -> Option<&RegionDiseaseState> {
        self.infections.iter().find(|i| i.disease_idx == disease_idx)
    }

    pub fn disease_state_mut(&mut self, disease_idx: usize) -> Option<&mut RegionDiseaseState> {
        self.infections.iter_mut().find(|i| i.disease_idx == disease_idx)
    }

    /// Get or create an infection entry for the given disease. Prevents duplicate
    /// entries for the same disease_idx, which would cause silent data corruption
    /// (only the first entry is visible to `disease_state()`).
    pub fn get_or_create_infection(&mut self, disease_idx: usize) -> &mut RegionDiseaseState {
        let pos = self.infections.iter().position(|i| i.disease_idx == disease_idx);
        if let Some(idx) = pos {
            &mut self.infections[idx]
        } else {
            self.infections.push(RegionDiseaseState {
                disease_idx,
                infected: 0.0,
                dead: 0.0,
                immune: 0.0,
            });
            self.infections.last_mut().unwrap()
        }
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
#[derive(Clone, Copy, Debug, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathogenType {
    /// Fast-mutating, high infectivity, responds to antivirals
    RnaVirus,
    /// Slower-mutating, stable, responds to antivirals
    DnaVirus,
    /// Responds to antibiotics, can develop resistance
    Bacterium,
    /// Slow-growing, hard to treat, limited drug options
    Fungus,
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
            PathogenType::Fungus => "Fungus",
            PathogenType::Prion => "Prion",
        }
    }

    /// Per-tick probability that this pathogen type mutates.
    /// Rates tuned so mutations are a real mid-game mechanic: RNA viruses
    /// mutate 3-4 times in a typical 30-day game, creating ongoing pressure
    /// to re-trial medicines. Slower types mutate less, making sequencing
    /// research most valuable against RNA threats.
    pub fn mutation_rate(&self) -> f64 {
        match self {
            PathogenType::RnaVirus => 0.001,     // ~1 mutation per 1000 ticks (~8 days)
            PathogenType::DnaVirus => 0.0002,    // ~1 per 5000 ticks (~42 days)
            PathogenType::Bacterium => 0.0004,   // ~1 per 2500 ticks (~21 days)
            PathogenType::Fungus => 0.0001,      // ~1 per 10000 ticks (~83 days)
            PathogenType::Prion => 0.00003,      // ~1 per 33333 ticks (~278 days)
        }
    }

    /// Stat ranges tuned so defeat occurs at day 25-50 without intervention.
    /// R0 = infectivity / (lethality + recovery) targets 2-4 for most types.
    /// First region collapse at ~day 12-15, total defeat at ~day 25-50
    /// depending on seed and pathogen type.
    fn stat_ranges(&self) -> DiseaseStatRanges {
        match self {
            // RNA viruses: fast spreader, moderate lethality
            // R0 ≈ 3.7, death fraction ≈ 47%.
            PathogenType::RnaVirus => DiseaseStatRanges {
                infectivity: (0.008, 0.014),
                lethality: (0.0008, 0.002),
                recovery: (0.0012, 0.002),
                cross_region: (0.003, 0.005),
            },
            // DNA viruses: moderate spread, higher lethality, slow recovery
            // R0 ≈ 2.6, death fraction ≈ 65%.
            PathogenType::DnaVirus => DiseaseStatRanges {
                infectivity: (0.006, 0.011),
                lethality: (0.0012, 0.003),
                recovery: (0.0008, 0.0015),
                cross_region: (0.002, 0.004),
            },
            // Bacteria: moderate all around
            // R0 ≈ 3.1, death fraction ≈ 41%.
            PathogenType::Bacterium => DiseaseStatRanges {
                infectivity: (0.006, 0.010),
                lethality: (0.0006, 0.0015),
                recovery: (0.001, 0.002),
                cross_region: (0.002, 0.004),
            },
            // Fungi: slow-growing, moderate lethality, very low natural recovery.
            // R0 ≈ 2.7, death fraction ≈ 68%. Hard to clear without antifungals.
            PathogenType::Fungus => DiseaseStatRanges {
                infectivity: (0.004, 0.008),
                lethality: (0.001, 0.002),
                recovery: (0.0004, 0.001),
                cross_region: (0.001, 0.003),
            },
            // Prions: slow but devastating, very high lethality, almost no recovery
            // R0 ≈ 1.7 (can dip below 1 at extremes), death fraction ≈ 87%.
            PathogenType::Prion => DiseaseStatRanges {
                infectivity: (0.004, 0.008),
                lethality: (0.002, 0.004),
                recovery: (0.0003, 0.0006),
                cross_region: (0.001, 0.003),
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
            PathogenType::Fungus => &[
                "Candida Omega", "Aspergillus Rex", "Cryptococcus Sigma",
                "Mucor-X", "Trichophyton Nova", "Coccidioides Tau",
                "Histoplasma Delta",
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
            // Fungi: spores (airborne) and hospital contact, no waterborne
            PathogenType::Fungus => {
                if roll < 0.45 { TransmissionVector::Airborne }
                else { TransmissionVector::Contact }
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
            PathogenType::Fungus => TherapyType::Antifungal,
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

    /// Infectivity multiplier when water sanitation is active.
    /// Only waterborne diseases are affected.
    pub fn water_sanitation_factor(&self) -> f64 {
        match self {
            TransmissionVector::Waterborne => 0.5,  // 50% reduction
            _ => 1.0,                                // no effect
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
    /// Per-mechanism resistance levels. When a medicine with mechanism X is deployed
    /// against this disease, resistance to mechanism X increases — affecting ALL drugs
    /// sharing that mechanism. Broad-spectrum drugs (mechanism=None) track separately.
    #[serde(default)]
    pub mechanism_resistance: Vec<ResistanceEntry>,
}

/// Resistance level for a specific mechanism of action against a disease.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResistanceEntry {
    pub mechanism: Option<MechanismOfAction>,
    pub level: f64,
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

    /// Get resistance level for a specific mechanism (0.0–1.0).
    pub fn get_resistance(&self, mechanism: Option<MechanismOfAction>) -> f64 {
        self.mechanism_resistance.iter()
            .find(|e| e.mechanism == mechanism)
            .map(|e| e.level)
            .unwrap_or(0.0)
    }

    /// Efficacy multiplier from mechanism resistance (0.2–1.0).
    pub fn resistance_factor(&self, mechanism: Option<MechanismOfAction>) -> f64 {
        (1.0 - self.get_resistance(mechanism)).max(0.2)
    }

    /// Add resistance for a mechanism, capping at 1.0.
    pub fn add_resistance(&mut self, mechanism: Option<MechanismOfAction>, amount: f64) {
        if let Some(entry) = self.mechanism_resistance.iter_mut().find(|e| e.mechanism == mechanism) {
            entry.level = (entry.level + amount).min(1.0);
        } else {
            self.mechanism_resistance.push(ResistanceEntry {
                mechanism,
                level: amount.min(1.0),
            });
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
            mechanism_resistance: vec![],
        }
    }
}

/// Knowledge thresholds for progressive disease revelation.
pub const KNOWLEDGE_NAME: f64 = 0.33;
pub const KNOWLEDGE_PARTIAL_STATS: f64 = 0.66;
pub const KNOWLEDGE_FULL: f64 = 1.0;
/// Minimum knowledge to develop broad-spectrum medicines targeting this disease.
/// One identification (0.50 knowledge) is enough for broad-spectrum.
pub const KNOWLEDGE_FOR_MEDICINE: f64 = 0.50;
/// Minimum knowledge to develop targeted medicines (Antiviral/Antibiotic).
/// Requires full study (knowledge 1.0) — creates a strategic choice between
/// rushing a broad-spectrum medicine now vs. studying for a more effective targeted one.
pub const KNOWLEDGE_FOR_TARGETED: f64 = 1.0;


/// Number of simulation ticks per in-game day. The UI displays days, not ticks.
/// 120 chosen so 5 ticks = 1 hour exactly (120 / 24 = 5).
pub const TICKS_PER_DAY: f64 = 120.0;
/// Mercy rule threshold: 5 days of zero player agency triggers defeat.
pub const MERCY_RULE_TICKS: u64 = 600;
/// Deploy cooldown per region in ticks (2 days). Healthcare systems need
/// time to distribute and administer doses.
pub const DEPLOY_COOLDOWN_TICKS: u64 = 240;
/// Cost to rally public support (boost POL).
pub const RALLY_COST: f64 = 300.0;
/// POL gain from a single rally (+5%).
pub const RALLY_POL_GAIN: f64 = 0.05;
/// Cooldown between rallies in ticks (2 days).
pub const RALLY_COOLDOWN_TICKS: u64 = 240;

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
    /// Targets fungal cell structures; effective against fungal pathogens.
    Antifungal,
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
            TherapyType::Antifungal => "Antifungal",
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
            (TherapyType::Antifungal, PathogenType::Fungus) => 1.0,
            // Broad-spectrum: partial efficacy against everything except prions
            (TherapyType::BroadSpectrum, PathogenType::Prion) => 0.1,
            (TherapyType::BroadSpectrum, _) => 0.5,
            // Prions resist everything
            (_, PathogenType::Prion) => 0.0,
            // Mismatched: nearly useless
            _ => 0.1,
        }
    }
}

/// Molecular mechanism by which a medicine acts. Targeted medicines have a specific
/// mechanism; broad-spectrum medicines do not. Future resistance systems will track
/// resistance per mechanism — overusing one mechanism builds resistance to all drugs
/// sharing it, creating pressure to diversify treatment strategies.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MechanismOfAction {
    // Bacterial mechanisms
    /// Beta-lactams: disrupt cell wall synthesis (e.g., penicillin, cephalosporins).
    CellWallInhibitor,
    /// Aminoglycosides: block bacterial protein synthesis at the ribosome.
    RibosomeInhibitor,
    /// Fluoroquinolones: inhibit DNA gyrase/topoisomerase, preventing replication.
    DnaGyraseInhibitor,
    /// Sulfonamides: block folate synthesis, starving bacteria of essential metabolites.
    MetabolicInhibitor,

    // Fungal mechanisms
    /// Azoles: block ergosterol synthesis, destabilizing fungal cell membranes.
    ErgosterolInhibitor,
    /// Polyenes (e.g., amphotericin B): bind ergosterol, punching holes in membranes.
    MembraneDisruptor,
    /// Echinocandins: block glucan synthesis, weakening the fungal cell wall.
    GlucanSynthaseInhibitor,

    // Viral mechanisms
    /// Nucleoside analogs: mimic building blocks to halt viral genome replication.
    PolymeraseInhibitor,
    /// Protease inhibitors: prevent viral polyprotein cleavage, blocking maturation.
    ProteaseInhibitor,
    /// Fusion/entry inhibitors: block viral attachment or membrane fusion.
    EntryInhibitor,
}

impl MechanismOfAction {
    /// Human-readable label for display in UI.
    pub fn label(&self) -> &'static str {
        match self {
            MechanismOfAction::CellWallInhibitor => "Cell Wall Inhibitor",
            MechanismOfAction::RibosomeInhibitor => "Ribosome Inhibitor",
            MechanismOfAction::DnaGyraseInhibitor => "DNA Gyrase Inhibitor",
            MechanismOfAction::MetabolicInhibitor => "Metabolic Inhibitor",
            MechanismOfAction::ErgosterolInhibitor => "Ergosterol Inhibitor",
            MechanismOfAction::MembraneDisruptor => "Membrane Disruptor",
            MechanismOfAction::GlucanSynthaseInhibitor => "Glucan Synthase Inhibitor",
            MechanismOfAction::PolymeraseInhibitor => "Polymerase Inhibitor",
            MechanismOfAction::ProteaseInhibitor => "Protease Inhibitor",
            MechanismOfAction::EntryInhibitor => "Entry Inhibitor",
        }
    }

    /// Short label for compact display (e.g., medicine list).
    pub fn short_label(&self) -> &'static str {
        match self {
            MechanismOfAction::CellWallInhibitor => "CellWall",
            MechanismOfAction::RibosomeInhibitor => "Ribosome",
            MechanismOfAction::DnaGyraseInhibitor => "Gyrase",
            MechanismOfAction::MetabolicInhibitor => "Metabolic",
            MechanismOfAction::ErgosterolInhibitor => "Ergosterol",
            MechanismOfAction::MembraneDisruptor => "Membrane",
            MechanismOfAction::GlucanSynthaseInhibitor => "Glucan",
            MechanismOfAction::PolymeraseInhibitor => "Polymerase",
            MechanismOfAction::ProteaseInhibitor => "Protease",
            MechanismOfAction::EntryInhibitor => "Entry",
        }
    }

    /// Mechanisms applicable to bacterial pathogens.
    pub fn bacterial_mechanisms() -> &'static [MechanismOfAction] {
        &[
            MechanismOfAction::CellWallInhibitor,
            MechanismOfAction::RibosomeInhibitor,
            MechanismOfAction::DnaGyraseInhibitor,
            MechanismOfAction::MetabolicInhibitor,
        ]
    }

    /// Mechanisms applicable to fungal pathogens.
    pub fn fungal_mechanisms() -> &'static [MechanismOfAction] {
        &[
            MechanismOfAction::ErgosterolInhibitor,
            MechanismOfAction::MembraneDisruptor,
            MechanismOfAction::GlucanSynthaseInhibitor,
        ]
    }

    /// Mechanisms applicable to viral pathogens.
    pub fn viral_mechanisms() -> &'static [MechanismOfAction] {
        &[
            MechanismOfAction::PolymeraseInhibitor,
            MechanismOfAction::ProteaseInhibitor,
            MechanismOfAction::EntryInhibitor,
        ]
    }

    /// Which pathogen types this mechanism works against.
    pub fn targets_pathogen(&self, pathogen: &PathogenType) -> bool {
        match pathogen {
            PathogenType::Bacterium => Self::bacterial_mechanisms().contains(self),
            PathogenType::Fungus => Self::fungal_mechanisms().contains(self),
            PathogenType::RnaVirus | PathogenType::DnaVirus => Self::viral_mechanisms().contains(self),
            PathogenType::Prion => false,
        }
    }

    /// Efficacy modifier for this mechanism (0.0–1.0). Multiplied with
    /// therapy-type efficacy to determine how well this drug works.
    /// Fast/cheap mechanisms are more potent initially; expensive ones less so.
    pub fn efficacy_modifier(&self) -> f64 {
        match self {
            // Bacterial: CellWall is potent but fragile, Metabolic is weak but durable
            MechanismOfAction::CellWallInhibitor => 0.95,
            MechanismOfAction::RibosomeInhibitor => 0.85,
            MechanismOfAction::DnaGyraseInhibitor => 0.80,
            MechanismOfAction::MetabolicInhibitor => 0.75,
            // Fungal: Ergosterol fast, Glucan durable
            MechanismOfAction::ErgosterolInhibitor => 0.90,
            MechanismOfAction::MembraneDisruptor => 0.85,
            MechanismOfAction::GlucanSynthaseInhibitor => 0.80,
            // Viral: Polymerase fast, Entry durable
            MechanismOfAction::PolymeraseInhibitor => 0.90,
            MechanismOfAction::ProteaseInhibitor => 0.85,
            MechanismOfAction::EntryInhibitor => 0.80,
        }
    }

    /// How fast resistance builds when deploying this mechanism.
    /// Cheap/fast mechanisms have high multipliers (resistance emerges quickly).
    /// Expensive/slow ones are harder for pathogens to adapt to.
    pub fn resistance_rate_multiplier(&self) -> f64 {
        match self {
            MechanismOfAction::CellWallInhibitor => 1.8,
            MechanismOfAction::RibosomeInhibitor => 1.0,
            MechanismOfAction::DnaGyraseInhibitor => 0.5,
            MechanismOfAction::MetabolicInhibitor => 0.3,

            MechanismOfAction::ErgosterolInhibitor => 1.6,
            MechanismOfAction::MembraneDisruptor => 1.0,
            MechanismOfAction::GlucanSynthaseInhibitor => 0.3,

            MechanismOfAction::PolymeraseInhibitor => 1.6,
            MechanismOfAction::ProteaseInhibitor => 1.0,
            MechanismOfAction::EntryInhibitor => 0.3,
        }
    }

    /// Development cost/time/personnel multiplier relative to base (3 personnel, 200 ticks, $500).
    /// Fast mechanisms are cheaper to develop; durable ones are expensive.
    pub fn dev_cost_multiplier(&self) -> f64 {
        match self {
            MechanismOfAction::CellWallInhibitor => 0.6,
            MechanismOfAction::RibosomeInhibitor => 1.0,
            MechanismOfAction::DnaGyraseInhibitor => 1.4,
            MechanismOfAction::MetabolicInhibitor => 1.8,

            MechanismOfAction::ErgosterolInhibitor => 0.6,
            MechanismOfAction::MembraneDisruptor => 1.0,
            MechanismOfAction::GlucanSynthaseInhibitor => 1.6,

            MechanismOfAction::PolymeraseInhibitor => 0.6,
            MechanismOfAction::ProteaseInhibitor => 1.0,
            MechanismOfAction::EntryInhibitor => 1.8,
        }
    }

    /// Short description of the mechanism's tradeoff profile.
    pub fn tradeoff_label(&self) -> &'static str {
        match self {
            MechanismOfAction::CellWallInhibitor
            | MechanismOfAction::ErgosterolInhibitor
            | MechanismOfAction::PolymeraseInhibitor => "Fast, resistance-prone",

            MechanismOfAction::RibosomeInhibitor
            | MechanismOfAction::MembraneDisruptor
            | MechanismOfAction::ProteaseInhibitor => "Balanced",

            MechanismOfAction::DnaGyraseInhibitor => "Slow, durable",

            MechanismOfAction::MetabolicInhibitor
            | MechanismOfAction::GlucanSynthaseInhibitor
            | MechanismOfAction::EntryInhibitor => "Expensive, very durable",
        }
    }

    /// Starting dose count for medicines using this mechanism.
    /// Fast/cheap mechanisms have fewer doses (need more manufacturing).
    pub fn base_doses(&self) -> f64 {
        let mult = self.dev_cost_multiplier();
        // Scale from 60M (mult=0.6) to 140M (mult=1.8)
        (60_000_000.0 + 80_000_000.0 * (mult - 0.6) / 1.2).round()
    }

    /// Deploy cost per use for this mechanism.
    /// Fast mechanisms cost more per deployment; expensive ones are cheaper per use.
    pub fn deploy_cost(&self) -> f64 {
        let mult = self.dev_cost_multiplier();
        // Scale from $65 (mult=0.6) to $35 (mult=1.8)
        (65.0 - 30.0 * (mult - 0.6) / 1.2).round()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Medicine {
    pub name: String,
    #[serde(default)]
    pub therapy_type: TherapyType,
    /// Molecular mechanism of action. `None` for broad-spectrum medicines.
    /// Targeted medicines get a specific mechanism assigned during development.
    #[serde(default)]
    pub mechanism: Option<MechanismOfAction>,
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
    /// Signed to allow negative values (fast-tracked medicines start behind gen 0).
    #[serde(default)]
    pub strain_generations: Vec<i32>,
    /// Number of times this medicine has been successfully deployed.
    #[serde(default)]
    pub deployed_count: u32,
    /// Legacy field, kept for save file compatibility. Previously distinguished
    /// rapid vs standard variants; now mechanism properties drive all costs.
    #[serde(default)]
    pub rapid: bool,
}

/// What a medicine deployment targets: protect susceptible (preventive) or treat infected (therapeutic).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeployTarget {
    Vaccinate { disease_idx: usize },
    Treat { disease_idx: usize },
}

impl Medicine {
    /// Deployment cost: base cost + $50 per billion population in the target region.
    pub fn deploy_cost(&self, region_population: u64) -> f64 {
        self.cost + region_population as f64 / 1_000_000_000.0 * 50.0
    }

    /// Generate targeted medicines for a disease. For non-prion pathogens, produces
    /// one medicine per mechanism of action (3-4 options depending on pathogen type).
    /// Each mechanism has distinct tradeoffs: fast/cheap mechanisms are potent but
    /// resistance-prone; expensive/slow ones are less potent but nearly resistance-proof.
    /// Prions produce a single medicine (no known molecular targets for mechanism branching).
    pub fn targeted_medicines(disease_idx: usize, pathogen_type: PathogenType) -> Vec<Medicine> {
        let therapy = pathogen_type.matched_therapy();
        let letter = (b'A' + disease_idx as u8) as char;

        let mechs: &[MechanismOfAction] = match pathogen_type {
            PathogenType::Bacterium => MechanismOfAction::bacterial_mechanisms(),
            PathogenType::Fungus => MechanismOfAction::fungal_mechanisms(),
            PathogenType::RnaVirus | PathogenType::DnaVirus => MechanismOfAction::viral_mechanisms(),
            PathogenType::Prion => {
                // Prions: single medicine, no mechanism
                return vec![Medicine {
                    name: format!("{}-{}", therapy.label(), letter),
                    therapy_type: therapy,
                    mechanism: None,
                    target_diseases: vec![disease_idx],
                    cost: 50.0,
                    doses: 100_000_000.0,
                    max_doses: 100_000_000.0,
                    unlocked: false,
                    tested_against: vec![],
                    strain_generations: vec![],
                    deployed_count: 0,
                    rapid: false,
                }];
            }
        };

        mechs.iter().map(|&mech| {
            let doses = mech.base_doses();
            Medicine {
                name: format!("{}-{}", mech.short_label(), letter),
                therapy_type: therapy,
                mechanism: Some(mech),
                target_diseases: vec![disease_idx],
                cost: mech.deploy_cost(),
                doses,
                max_doses: doses,
                unlocked: false,
                tested_against: vec![],
                strain_generations: vec![],
                deployed_count: 0,
                rapid: false,
            }
        }).collect()
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
                            .map_or(0, |d| d.strain_generation) as i32;
                        let behind = (disease_gen - mg).max(0);
                        (1.0 - behind as f64 * 0.15).max(0.1)
                    }
                    // Not yet calibrated (developed before mutation system) — full efficacy
                    None => 1.0,
                }
            }
            None => 1.0,
        }
    }

    /// Efficacy multiplier from mechanism resistance (0.2–1.0). Reads resistance
    /// from the disease based on this medicine's mechanism of action.
    pub fn resistance_factor(&self, disease_idx: usize, diseases: &[Disease]) -> f64 {
        diseases.get(disease_idx)
            .map(|d| d.resistance_factor(self.mechanism))
            .unwrap_or(1.0)
    }

    /// Estimate how many people a vaccination deployment would protect.
    /// Returns the number of doses that would be consumed (capped by available doses).
    /// `vax_multiplier` is 1.0 normally, 3.0 with VaccinePlatform tech.
    pub fn estimate_vaccination(&self, susceptible: f64, efficacy: f64, vax_multiplier: f64) -> f64 {
        let target = susceptible * VACCINATION_FRACTION * vax_multiplier * efficacy;
        target.min(self.doses)
    }

    /// Estimate how many people a treatment deployment would treat.
    /// Returns the number of doses that would be consumed (capped by available doses).
    pub fn estimate_treatment(&self, infected: f64, efficacy: f64) -> f64 {
        let target = infected * TREATMENT_FRACTION * efficacy;
        target.min(self.doses)
    }

    /// All diseases this medicine can be deployed against: primary targets first,
    /// then cross-reactive targets (same mechanism category, different disease).
    /// Cross-reactive targets get a 50% efficacy penalty during deployment.
    /// Only includes diseases whose pathogen type is known (identified to KNOWLEDGE_NAME).
    pub fn deployable_diseases(&self, diseases: &[Disease]) -> Vec<usize> {
        let mut result: Vec<usize> = self.target_diseases.clone();
        if let Some(mech) = self.mechanism {
            for (i, disease) in diseases.iter().enumerate() {
                if !result.contains(&i)
                    && disease.detected
                    && disease.knowledge >= KNOWLEDGE_NAME
                    && mech.targets_pathogen(&disease.pathogen_type)
                {
                    result.push(i);
                }
            }
        }
        result
    }

    /// Whether a disease is a cross-reactive target (not a primary target).
    pub fn is_cross_reactive(&self, disease_idx: usize) -> bool {
        !self.target_diseases.contains(&disease_idx)
    }

    /// Combined efficacy when deploying this medicine against a disease.
    /// Factors: therapy type × mechanism × strain calibration × cross-reactivity × resistance.
    pub fn effective_efficacy(&self, disease_idx: usize, diseases: &[Disease]) -> f64 {
        let therapy_efficacy = diseases.get(disease_idx)
            .map(|d| self.therapy_type.efficacy(&d.pathogen_type))
            .unwrap_or(0.0);
        let mechanism_eff = self.mechanism
            .map(|m| m.efficacy_modifier())
            .unwrap_or(1.0);
        let strain_eff = self.strain_efficacy(disease_idx, diseases);
        let cross_reactive = if self.is_cross_reactive(disease_idx) {
            CROSS_REACTIVE_PENALTY
        } else {
            1.0
        };
        let resistance = self.resistance_factor(disease_idx, diseases);
        therapy_efficacy * mechanism_eff * strain_eff * cross_reactive * resistance
    }

    /// Decode a UI selection index into a deploy target.
    /// Indices 0..n are vaccinate options, n..2n are treat options.
    /// Uses deployable_diseases (primary + cross-reactive) for the full target list.
    pub fn decode_deploy_target(&self, selection: usize, diseases: &[Disease]) -> Option<DeployTarget> {
        let targets = self.deployable_diseases(diseases);
        let n = targets.len();
        if selection < n {
            Some(DeployTarget::Vaccinate { disease_idx: targets[selection] })
        } else if selection < 2 * n {
            Some(DeployTarget::Treat { disease_idx: targets[selection - n] })
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

impl ResearchTrack {
    pub fn index(self) -> usize {
        match self {
            ResearchTrack::Field => 0,
            ResearchTrack::Applied => 1,
            ResearchTrack::Basic => 2,
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
}

impl ResearchProject {
    /// Whether this research project references the given disease index
    /// (directly or via a medicine that targets it).
    pub fn references_disease(&self, disease_idx: usize) -> bool {
        match &self.kind {
            ResearchKind::IdentifyThreat { disease_idx: d } => *d == disease_idx,
            ResearchKind::GenomicSequencing { disease_idx: d } => *d == disease_idx,
            ResearchKind::ClinicalTrial { disease_idx: d, .. } => *d == disease_idx,
            ResearchKind::DevelopMedicine { .. }
            | ResearchKind::ManufactureDoses { .. }
            | ResearchKind::TrainPersonnel
            | ResearchKind::BasicResearch { .. } => false,
        }
    }
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
/// Each unlocks or enhances capabilities across the game (drug development,
/// field research speed, vaccination effectiveness, etc.).
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
    /// Halves genomic sequencing duration and reveals mutation stat details.
    /// Prereq: completed at least one genomic sequencing project.
    RapidSequencing,
    /// Triples preventive vaccination effectiveness.
    /// Prereq: MonoclonalAntibodies or PhageTherapy (need advanced drug platform).
    VaccinePlatform,
    /// Reveals per-medicine resistance levels and trend indicators.
    /// Without this, players see efficacy dropping but don't know if it's
    /// strain drift (fixable by re-trial) or resistance (need new drug).
    /// Prereq: RapidSequencing.
    ResistanceSurveillance,
    /// Halves resistance buildup from all drug deployments. Multi-drug
    /// protocols make it harder for pathogens to evolve resistance.
    /// Prereq: deploy 2+ different medicines.
    CombinationTherapy,
}

impl BasicTech {
    /// Human-readable name for display.
    pub fn name(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Targeted Drug Design",
            BasicTech::MonoclonalAntibodies => "Monoclonal Antibodies",
            BasicTech::PhageTherapy => "Phage Therapy",
            BasicTech::RapidSequencing => "Rapid Sequencing",
            BasicTech::VaccinePlatform => "Vaccine Platform",
            BasicTech::ResistanceSurveillance => "Resistance Surveillance",
            BasicTech::CombinationTherapy => "Combination Therapy",
        }
    }

    /// Short description for the research panel.
    pub fn description(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Unlocks targeted Antiviral/Antibiotic development",
            BasicTech::MonoclonalAntibodies => "Unlocks high-efficacy mAb drugs for viruses",
            BasicTech::PhageTherapy => "Unlocks phage therapy drugs for bacteria",
            BasicTech::RapidSequencing => "Halves sequencing time, reveals mutation details",
            BasicTech::VaccinePlatform => "3x preventive vaccination effectiveness",
            BasicTech::ResistanceSurveillance => "Reveals drug resistance levels and trends",
            BasicTech::CombinationTherapy => "Halves resistance buildup from deployments",
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
            BasicTech::RapidSequencing => {
                // Prereq: completed at least one genomic sequencing on any disease
                state.diseases.iter().any(|d| d.sequencing_count > 0)
            }
            BasicTech::VaccinePlatform => {
                // Prereq: MonoclonalAntibodies or PhageTherapy (need advanced drug platform)
                state.unlocked_techs.contains(&BasicTech::MonoclonalAntibodies)
                    || state.unlocked_techs.contains(&BasicTech::PhageTherapy)
            }
            BasicTech::ResistanceSurveillance => {
                // Prereq: RapidSequencing (need sequencing infrastructure to monitor resistance)
                state.unlocked_techs.contains(&BasicTech::RapidSequencing)
            }
            BasicTech::CombinationTherapy => {
                // Prereq: deployed 2+ different medicines (any region/disease)
                let distinct_deployed = state.medicines.iter()
                    .filter(|m| m.deployed_count > 0)
                    .count();
                distinct_deployed >= 2
            }
        }
    }

    /// What prerequisites are needed (for display when locked).
    pub fn prereq_description(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Identify any pathogen",
            BasicTech::MonoclonalAntibodies => "Targeted Drug Design + study any virus",
            BasicTech::PhageTherapy => "Targeted Drug Design + study any bacterium",
            BasicTech::RapidSequencing => "Complete genomic sequencing on any pathogen",
            BasicTech::VaccinePlatform => "Monoclonal Antibodies or Phage Therapy",
            BasicTech::ResistanceSurveillance => "Rapid Sequencing",
            BasicTech::CombinationTherapy => "Deploy 2+ different medicines",
        }
    }

    /// All techs in display order.
    pub fn all() -> &'static [BasicTech] {
        &[
            BasicTech::TargetedDrugDesign,
            BasicTech::MonoclonalAntibodies,
            BasicTech::PhageTherapy,
            BasicTech::RapidSequencing,
            BasicTech::VaccinePlatform,
            BasicTech::ResistanceSurveillance,
            BasicTech::CombinationTherapy,
        ]
    }
}

impl ResearchKind {
    /// Project costs: (personnel, duration_ticks, funding).
    ///
    /// DevelopMedicine costs depend on mechanism of action: each mechanism has
    /// a dev_cost_multiplier that scales base costs (3 personnel, 200 ticks, $500).
    /// Broad-spectrum (multi-target, no mechanism) uses fixed high costs.
    /// RapidSequencing tech halves GenomicSequencing duration.
    pub fn costs(&self, medicines: &[Medicine]) -> (u32, f64, f64) {
        match self {
            ResearchKind::IdentifyThreat { .. } => (5, 160.0, 350.0),
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let med = medicines.get(*medicine_idx);
                let targets = med.map_or(1, |m| m.target_diseases.len());
                if targets > 1 {
                    (10, 400.0, 700.0)  // broad: slow and expensive, covers all
                } else if let Some(mech) = med.and_then(|m| m.mechanism) {
                    let mult = mech.dev_cost_multiplier();
                    let personnel = ((3.0 * mult).round() as u32).max(1);
                    let ticks = (200.0 * mult).round();
                    let funding = (500.0 * mult).round();
                    (personnel, ticks, funding)
                } else {
                    (4, 280.0, 700.0)   // fallback / prion
                }
            }
            ResearchKind::ClinicalTrial { .. } => (2, 60.0, 200.0),
            ResearchKind::ManufactureDoses { .. } => (3, 120.0, 250.0),
            ResearchKind::GenomicSequencing { .. } => (5, 200.0, 500.0),
            ResearchKind::TrainPersonnel => (1, 160.0, 150.0),
            ResearchKind::BasicResearch { tech } => match tech {
                BasicTech::TargetedDrugDesign => (3, 240.0, 600.0),
                BasicTech::MonoclonalAntibodies => (5, 360.0, 900.0),
                BasicTech::PhageTherapy => (5, 360.0, 900.0),
                BasicTech::RapidSequencing => (4, 300.0, 750.0),
                BasicTech::VaccinePlatform => (6, 360.0, 1000.0),
                BasicTech::ResistanceSurveillance => (3, 200.0, 500.0),
                BasicTech::CombinationTherapy => (4, 300.0, 800.0),
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
        /// Infectivity change factor (e.g., 1.1 = +10%). Only meaningful with RapidSequencing.
        infectivity_factor: f64,
        /// Lethality change factor (e.g., 0.9 = -10%). Only meaningful with RapidSequencing.
        lethality_factor: f64,
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
    /// A research project was auto-started because auto-research is on.
    ResearchAutoStarted { track: ResearchTrack },
    /// Personnel left due to unpaid wages (funding at $0).
    PersonnelAttrition { count: u32 },
    /// Bacterial horizontal gene transfer — broad-spectrum resistance spread
    /// from one bacterium to another.
    ResistanceTransferred {
        from_disease_idx: usize,
        to_disease_idx: usize,
    },
    /// A disease's death toll crossed a major threshold. Fires once per
    /// threshold per disease. Auto-pauses the game.
    ThreatEscalation {
        disease_idx: usize,
        deaths: f64,
        has_medicine: bool,
    },
    /// Human Trials decree caused an adverse event during a clinical trial.
    HumanTrialAdverseEvent {
        disease_idx: usize,
        deaths: f64,
    },
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
        slot_idx: usize,
    },
    RemoveResearchPersonnel {
        track: ResearchTrack,
        slot_idx: usize,
    },
    TogglePolicy {
        region_idx: usize,
        policy_idx: usize,
    },
    /// Resolve the active crisis by choosing option A (0) or B (1).
    ResolveCrisis {
        choice: usize,
    },
    /// Enact an emergency decree. `region_idx` is only used for SacrificeRegion.
    EnactDecree {
        decree_idx: usize,
        region_idx: Option<usize>,
    },
    /// Spend funds to boost POL directly.
    RallySupport,
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
    /// Lab accident — lose applied or basic research, or spend resources to contain.
    LabAccident { targets_basic: bool },
    /// Political pressure — lift quarantine in a region or pay to resist.
    PoliticalPressure { region_idx: usize },
    /// Staff burnout — lose personnel or pay retention bonus.
    PersonnelCrisis { amount: u32 },
    /// International aid offer — choose funding or personnel.
    InternationalAid { funding: f64, personnel: u32 },
    /// Mutation surge — pay to gain knowledge or let it drift.
    MutationSurge { disease_idx: usize },
    /// Refugees flooding from collapsed region — accept (spread disease) or turn away (lose POL).
    RefugeeWave { from_region: usize, to_region: usize },
    /// Research data leaked — go transparent or suppress.
    DataLeak,
    /// Untested drugs on the black market — confiscate or allow.
    BlackMarketMedicine { region_idx: usize },
    /// Riots in quarantined region — use force or negotiate.
    QuarantineRiot { region_idx: usize },
    /// Media causing panic — address it or ignore.
    MediaPanic,
    /// Pressure to skip clinical trials — fast-track marks medicine tested but with strain drift penalty.
    TrialShortcut { disease_idx: usize, medicine_idx: usize },
    /// Public refusing vaccines — education campaign or mandate.
    VaccineHesitancy { region_idx: usize },
    /// Corrupt official siphoning funds — amount locked at generation time.
    CorruptOfficial { stolen: f64 },
    /// Powerful nation wants your research data — share or refuse.
    ResourceDiversion { disease_idx: usize, share_reward: f64, refuse_cost: f64 },
    /// Hospital workers collapsing — reduce shifts or push through.
    ExhaustionEpidemic { region_idx: usize, personnel_loss: u32 },
    /// Whistleblower reports medicine side effects — halt or continue.
    WhistleblowerReport { medicine_idx: usize },
    /// Military threatens takeover of health agency.
    MilitaryTakeover { cooperate_loss: u32 },
    /// Cult blocks vaccination teams in a region.
    CultBlockade { region_idx: usize },
    /// Billionaire offers to fund everything — for a price.
    BillionaireOffer { reward: f64, personnel_loss: u32 },
    /// WHO headquarters loses power — relocate or improvise.
    WHOEvacuation { aid_loss: f64 },
    /// Warlord declares himself ruler of collapsed region, demands recognition.
    WarlordDemand { region_idx: usize },
    /// Two nations claim credit for your vaccine, threaten war.
    VaccineDispute { neutral_loss: f64, credit_gain: f64 },

    // --- Dark comedy events (personality and flavor) ---

    /// Quarterly performance review during the apocalypse.
    PerformanceReview,
    /// Pharmaceutical corp offers money to rename a disease.
    NamingRights { disease_idx: usize, payout: f64 },
    /// Unpaid intern claims a breakthrough. 50/50 gamble.
    InternDiscovery { cost: f64 },
    /// Congressional hearing about your handling of the crisis.
    CongressionalHearing,

    // --- Follow-up crisis types (spawned by earlier choices) ---

    /// Follow-up to CongressionalHearing (Send deputy): contempt charges.
    ContemptOfCongress { fine: f64 },
    /// Follow-up to BlackMarketMedicine (Allow): counterfeit drugs killing people.
    CounterfeitEpidemic { region_idx: usize },
    /// Follow-up to CorruptOfficial (Ignore): corruption has spread to a ring.
    EmbezzlementRing { stolen_per_day: f64 },
    /// Follow-up to MilitaryTakeover (Cooperate): military wants your research.
    MilitaryOverreach,
    /// Follow-up to DataLeak (Suppress): cover-up exposed, inquiry demanded.
    PublicInquiry,
}

impl CrisisKind {
    /// Short tag identifying the crisis type (ignoring variant data).
    /// Used for cooldown tracking to prevent back-to-back repeats.
    pub fn tag(&self) -> &'static str {
        match self {
            CrisisKind::SupplyDisruption { .. } => "supply",
            CrisisKind::LabAccident { .. } => "lab",
            CrisisKind::PoliticalPressure { .. } => "political",
            CrisisKind::PersonnelCrisis { .. } => "personnel",
            CrisisKind::InternationalAid { .. } => "aid",
            CrisisKind::MutationSurge { .. } => "mutation",
            CrisisKind::RefugeeWave { .. } => "refugee",
            CrisisKind::DataLeak => "dataleak",
            CrisisKind::BlackMarketMedicine { .. } => "blackmarket",
            CrisisKind::QuarantineRiot { .. } => "riot",
            CrisisKind::MediaPanic => "media",
            CrisisKind::TrialShortcut { .. } => "trial",
            CrisisKind::VaccineHesitancy { .. } => "hesitancy",
            CrisisKind::CorruptOfficial { .. } => "corrupt",
            CrisisKind::ResourceDiversion { .. } => "diversion",
            CrisisKind::ExhaustionEpidemic { .. } => "exhaustion",
            CrisisKind::WhistleblowerReport { .. } => "whistleblower",
            CrisisKind::MilitaryTakeover { .. } => "military",
            CrisisKind::CultBlockade { .. } => "cult",
            CrisisKind::BillionaireOffer { .. } => "billionaire",
            CrisisKind::WHOEvacuation { .. } => "who_evac",
            CrisisKind::WarlordDemand { .. } => "warlord",
            CrisisKind::VaccineDispute { .. } => "vaccine_dispute",
            CrisisKind::PerformanceReview => "performance_review",
            CrisisKind::NamingRights { .. } => "naming_rights",
            CrisisKind::InternDiscovery { .. } => "intern",
            CrisisKind::CongressionalHearing => "congress",
            CrisisKind::ContemptOfCongress { .. } => "contempt",
            CrisisKind::CounterfeitEpidemic { .. } => "counterfeit",
            CrisisKind::EmbezzlementRing { .. } => "embezzlement",
            CrisisKind::MilitaryOverreach => "military_overreach",
            CrisisKind::PublicInquiry => "public_inquiry",
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
    /// Choose which disease to target (skipped for single-target medicines).
    SelectDisease { medicine_idx: usize, region_idx: usize },
    /// Choose vaccinate (0) or treat (1) for the selected disease.
    SelectTarget { medicine_idx: usize, region_idx: usize, disease_idx: usize },
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
    /// Select which region to sacrifice (for Sacrifice Region decree).
    SelectSacrificeRegion,
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
    /// Viewing an active project. `slot_idx` selects which field project (0 for Applied/Basic).
    ViewActive { track: ResearchTrack, slot_idx: usize },
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
    /// Toggle a panel open/closed. If deep in a panel wizard, pressing the same
    /// panel key resets to the top level instead of closing. Only closes when
    /// already at the top level.
    pub fn toggle_panel(&mut self, panel: Panel, num_regions: usize) {
        if self.open_panel == panel {
            // Check if we're deeper than the top level — if so, reset to top
            let at_top = match panel {
                Panel::Medicines => matches!(self.medicine_ui, Some(MedicineUiState::BrowseMedicines) | None),
                Panel::Research => matches!(self.research_ui, Some(ResearchUiState::BrowseCategories) | None),
                Panel::Policy => matches!(self.policy_ui, Some(PolicyUiState::BrowseRegions) | None),
                _ => true,
            };
            if at_top {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                match panel {
                    Panel::Medicines => self.medicine_ui = None,
                    Panel::Research => self.research_ui = None,
                    Panel::Policy => self.policy_ui = None,
                    _ => {}
                }
            } else {
                // Reset to top level of this panel
                self.panel_selection = 0;
                match panel {
                    Panel::Medicines => self.medicine_ui = Some(MedicineUiState::BrowseMedicines),
                    Panel::Research => self.research_ui = Some(ResearchUiState::BrowseCategories),
                    Panel::Policy => self.policy_ui = Some(PolicyUiState::BrowseRegions),
                    _ => {}
                }
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
    pub fn close_panel(&mut self, medicines: &[Medicine], diseases: &[Disease]) {
        match self.open_panel {
            Panel::Medicines => {
                match self.medicine_ui.clone() {
                    Some(MedicineUiState::DeployResult { medicine_idx, .. }) => {
                        self.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                        self.panel_selection = 0;
                    }
                    Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, target_selection }) => {
                        let med = &medicines[medicine_idx];
                        // Reconstruct disease_idx and action from target_selection
                        let deployable = med.deployable_diseases(diseases);
                        let n = deployable.len();
                        let (disease_idx, action) = if target_selection < n {
                            (deployable[target_selection], 0)
                        } else {
                            (deployable[target_selection - n], 1)
                        };
                        self.medicine_ui = Some(MedicineUiState::SelectTarget {
                            medicine_idx,
                            region_idx,
                            disease_idx,
                        });
                        self.panel_selection = action;
                    }
                    Some(MedicineUiState::SelectTarget { medicine_idx, region_idx, .. }) => {
                        let med = &medicines[medicine_idx];
                        if med.deployable_diseases(diseases).len() == 1 {
                            self.medicine_ui = Some(MedicineUiState::SelectRegion { medicine_idx });
                        } else {
                            self.medicine_ui = Some(MedicineUiState::SelectDisease {
                                medicine_idx,
                                region_idx,
                            });
                        }
                        self.panel_selection = 0;
                    }
                    Some(MedicineUiState::SelectDisease { medicine_idx, .. }) => {
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
                    Some(PolicyUiState::ManagePolicies { .. })
                    | Some(PolicyUiState::SelectSacrificeRegion) => {
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
                    Some(ResearchUiState::ViewActive { track, .. }) => {
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
                Some(MedicineUiState::SelectDisease { medicine_idx, .. }) => {
                    state.medicines[*medicine_idx]
                        .deployable_diseases(&state.diseases).len()
                        .saturating_sub(1)
                }
                Some(MedicineUiState::SelectTarget { .. }) => {
                    1 // vaccinate (0) or treat (1)
                }
                Some(MedicineUiState::ConfirmDeploy { .. })
                | Some(MedicineUiState::DeployResult { .. })
                | None => 0,
            },
            Panel::Research => match &self.research_ui {
                Some(ResearchUiState::BrowseCategories) => 2, // Field, Applied, Basic
                Some(ResearchUiState::BrowseProjects { track }) => {
                    if *track == ResearchTrack::Field {
                        // Active projects + available projects (if capacity remains)
                        let n_active = state.field_research.len();
                        let n_available = if state.field_research_has_capacity() {
                            state.available_projects(*track).len()
                        } else {
                            0
                        };
                        (n_active + n_available).saturating_sub(1)
                    } else {
                        let active = state.research_slot(*track).is_some();
                        if active {
                            0
                        } else {
                            state.available_projects(*track).len().saturating_sub(1)
                        }
                    }
                }
                Some(ResearchUiState::ConfirmProject { .. }) => 0,
                Some(ResearchUiState::ViewActive { .. }) => 0,
                None => 0,
            },
            Panel::Policy => match &self.policy_ui {
                Some(PolicyUiState::BrowseRegions) => {
                    // Items: 0..regions-1 = regions, regions = rally, regions+1..regions+DECREE_COUNT = decrees
                    state.regions.len() + 1 + DECREE_COUNT - 1
                }
                Some(PolicyUiState::ManagePolicies { .. }) => POLICY_COUNT - 1,
                Some(PolicyUiState::SelectSacrificeRegion) => {
                    // Only non-collapsed regions are selectable
                    state.regions.iter().filter(|r| !r.collapsed).count().saturating_sub(1)
                }
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
            Some(MedicineUiState::SelectDisease { medicine_idx, region_idx }) => {
                if *region_idx != self.map_selection {
                    let med = *medicine_idx;
                    self.medicine_ui = Some(MedicineUiState::SelectDisease {
                        medicine_idx: med,
                        region_idx: self.map_selection,
                    });
                    self.panel_selection = 0;
                }
            }
            Some(MedicineUiState::SelectTarget { medicine_idx, disease_idx, region_idx }) => {
                if *region_idx != self.map_selection {
                    let (med, dis) = (*medicine_idx, *disease_idx);
                    self.medicine_ui = Some(MedicineUiState::SelectTarget {
                        medicine_idx: med,
                        region_idx: self.map_selection,
                        disease_idx: dis,
                    });
                    self.panel_selection = 0;
                }
            }
            Some(MedicineUiState::ConfirmDeploy { medicine_idx, .. }) => {
                // Regress to region selection — don't silently change region on confirm screen
                let med = *medicine_idx;
                self.medicine_ui = Some(MedicineUiState::SelectRegion {
                    medicine_idx: med,
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
                    let med = &state.medicines[medicine_idx];
                    let deployable = med.deployable_diseases(&state.diseases);
                    if deployable.len() == 1 {
                        // Single-target: skip disease selection
                        self.medicine_ui = Some(MedicineUiState::SelectTarget {
                            medicine_idx,
                            region_idx,
                            disease_idx: deployable[0],
                        });
                    } else {
                        self.medicine_ui = Some(MedicineUiState::SelectDisease {
                            medicine_idx,
                            region_idx,
                        });
                    }
                    self.panel_selection = 0;
                }
                None
            }
            Some(MedicineUiState::SelectDisease { medicine_idx, region_idx }) => {
                let med = &state.medicines[medicine_idx];
                let deployable = med.deployable_diseases(&state.diseases);
                if let Some(&disease_idx) = deployable.get(self.panel_selection) {
                    self.medicine_ui = Some(MedicineUiState::SelectTarget {
                        medicine_idx,
                        region_idx,
                        disease_idx,
                    });
                    self.panel_selection = 0;
                }
                None
            }
            Some(MedicineUiState::SelectTarget {
                medicine_idx,
                region_idx,
                disease_idx,
            }) => {
                let med = &state.medicines[medicine_idx];
                // panel_selection: 0 = vaccinate, 1 = treat
                let deployable = med.deployable_diseases(&state.diseases);
                let pos = deployable.iter().position(|&d| d == disease_idx);
                let target_selection = match pos {
                    Some(p) => p + self.panel_selection * deployable.len(),
                    None => return None,
                };
                if med.decode_deploy_target(target_selection, &state.diseases).is_some() {
                    let deploy_cost = med.deploy_cost(state.regions[region_idx].population);
                    if state.resources.funding < deploy_cost {
                        self.status_message = Some(
                            format!("Insufficient funds! Need ${:.0}, have ${:.0}",
                                deploy_cost, state.resources.funding),
                        );
                        None
                    } else {
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
                if track == ResearchTrack::Field {
                    // Field track: list shows active projects first, then available.
                    let n_active = state.field_research.len();
                    if self.panel_selection < n_active {
                        // Selected an active project → view it
                        self.research_ui = Some(ResearchUiState::ViewActive { track, slot_idx: self.panel_selection });
                        self.panel_selection = 0;
                    } else {
                        // Selected an available project
                        let project_idx = self.panel_selection - n_active;
                        let count = state.available_projects(track).len();
                        if project_idx < count && state.field_research_has_capacity() {
                            self.research_ui = Some(ResearchUiState::ConfirmProject {
                                track,
                                project_idx,
                                double_personnel: false,
                            });
                            self.panel_selection = 0;
                        }
                    }
                } else {
                    // Applied/Basic: single-slot behavior
                    if state.research_slot(track).is_some() {
                        self.research_ui = Some(ResearchUiState::ViewActive { track, slot_idx: 0 });
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
                }
                None
            }
            Some(ResearchUiState::ConfirmProject { track, project_idx, double_personnel }) => {
                Some(GameCommand::StartResearch { track, project_idx, double_personnel })
            }
            Some(ResearchUiState::ViewActive { track, .. }) => {
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
                let num_regions = state.regions.len();
                if self.panel_selection < num_regions {
                    // Region selected — manage its policies
                    let order = grid_reading_order(num_regions);
                    let region_idx = order.get(self.panel_selection).copied().unwrap_or(0);
                    if region_idx < num_regions {
                        self.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx });
                        self.panel_selection = 0;
                    }
                    None
                } else if self.panel_selection == num_regions {
                    // Rally Public Support
                    Some(GameCommand::RallySupport)
                } else {
                    // Decree selected (indices after rally)
                    let decree_idx = self.panel_selection - num_regions - 1;
                    if decree_idx == 2 && !state.enacted_decrees.is_enacted(2) {
                        // Sacrifice Region needs sub-selection
                        self.policy_ui = Some(PolicyUiState::SelectSacrificeRegion);
                        self.panel_selection = 0;
                        None
                    } else {
                        Some(GameCommand::EnactDecree { decree_idx, region_idx: None })
                    }
                }
            }
            Some(PolicyUiState::ManagePolicies { region_idx }) => {
                // panel_selection is display position; currently matches policy_idx
                // (see POLICY_COUNT doc for the index mapping)
                let policy_idx = self.panel_selection;
                Some(GameCommand::TogglePolicy {
                    region_idx,
                    policy_idx,
                })
            }
            Some(PolicyUiState::SelectSacrificeRegion) => {
                // Map display position to actual region index (skipping collapsed)
                let non_collapsed: Vec<usize> = state.regions.iter()
                    .enumerate()
                    .filter(|(_, r)| !r.collapsed)
                    .map(|(i, _)| i)
                    .collect();
                if let Some(&region_idx) = non_collapsed.get(self.panel_selection) {
                    Some(GameCommand::EnactDecree {
                        decree_idx: 2,
                        region_idx: Some(region_idx),
                    })
                } else {
                    None
                }
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
                traits: vec![RegionTrait::TradeDependent, RegionTrait::StrongPublicHealth],
                collapse_threshold: 0.55, // Fragile — collapses at 45% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                healthcare_invested: false,
                income_modifier: 1.8,     // Wealthy — major economic contributor
                healthcare_modifier: 0.85, // Good healthcare infrastructure
                last_deploy_tick: None,
            },
            Region {
                name: "South America".into(),
                population: 430_000_000,
                connections: vec![0, 3],
                infections: vec![],
                traits: vec![RegionTrait::LowInfrastructure, RegionTrait::ResilientPopulation],
                collapse_threshold: 0.55, // Moderate resilience — 45% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                healthcare_invested: false,
                income_modifier: 1.0,     // Moderate economy
                healthcare_modifier: 0.95, // Decent healthcare
                last_deploy_tick: None,
            },
            Region {
                name: "Europe".into(),
                population: 750_000_000,
                connections: vec![0, 3, 4],
                infections: vec![],
                traits: vec![RegionTrait::TradeDependent, RegionTrait::DenseUrban],
                collapse_threshold: 0.50, // Developed infrastructure — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                healthcare_invested: false,
                income_modifier: 1.5,     // Strong economy, hub region
                healthcare_modifier: 0.80, // Excellent healthcare
                last_deploy_tick: None,
            },
            Region {
                name: "Africa".into(),
                population: 1_400_000_000,
                connections: vec![1, 2, 4],
                infections: vec![],
                traits: vec![RegionTrait::LowInfrastructure, RegionTrait::DenseUrban],
                collapse_threshold: 0.50, // Resilient — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                healthcare_invested: false,
                income_modifier: 0.6,     // Lower per-capita income
                healthcare_modifier: 1.1,  // Strained healthcare — higher lethality
                last_deploy_tick: None,
            },
            Region {
                name: "Asia".into(),
                population: 4_700_000_000,
                connections: vec![2, 3, 5],
                infections: vec![],
                traits: vec![RegionTrait::DenseUrban, RegionTrait::ResilientPopulation],
                collapse_threshold: 0.50, // Huge population — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                healthcare_invested: false,
                income_modifier: 0.9,     // Large but moderate per-capita
                healthcare_modifier: 1.0,  // Baseline healthcare
                last_deploy_tick: None,
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![4],
                infections: vec![],
                traits: vec![RegionTrait::IslandGeography, RegionTrait::StrongPublicHealth],
                collapse_threshold: 0.50, // Small but developed — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                healthcare_invested: false,
                income_modifier: 2.5,     // Tiny but wealthy — high per-capita
                healthcare_modifier: 0.75, // Best healthcare infrastructure
                last_deploy_tick: None,
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
            PathogenType::Fungus,
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
        regions[region_idx].dead = dead;

        // --- Generate medicines to match diseases ---
        // Two targeted medicines per non-prion disease (different mechanisms),
        // one for prion diseases.
        let mut medicines: Vec<Medicine> = diseases.iter().enumerate()
            .flat_map(|(i, d)| Medicine::targeted_medicines(i, d.pathogen_type))
            .collect();

        // One broad-spectrum medicine targeting all diseases
        let all_disease_indices: Vec<usize> = (0..diseases.len()).collect();
        medicines.push(Medicine {
            name: "Broad-Spectrum".into(),
            therapy_type: TherapyType::BroadSpectrum,
            mechanism: None,
            target_diseases: all_disease_indices,
            cost: 100.0,
            doses: 200_000_000.0,
            max_doses: 200_000_000.0,
            unlocked: false,
            tested_against: vec![],
            strain_generations: vec![],
            deployed_count: 0,
            rapid: false,
        });

        let num_diseases = diseases.len();
        Self {
            tick: 0,
            sim_state: SimState::Running,
            rng,
            resources: Resources {
                funding: 500.0,
                personnel: 20,
                political_power: 0.0,
                pol_crisis_modifier: 0.0,
                personnel_accum: 0.0,
                attrition_accum: 0.0,
                last_rally_tick: None,
            },
            policies: vec![RegionPolicy::default(); regions.len()],
            enacted_decrees: EnactedDecrees::default(),
            regions,
            diseases,
            medicines,
            field_research: vec![],
            applied_research: None,
            basic_research: None,
            unlocked_techs: vec![],
            outcome: GameOutcome::Playing,
            events: vec![],
            event_log: VecDeque::new(),
            active_crisis: None,
            crisis_cooldowns: HashMap::new(),
            pending_crises: vec![],
            auto_resolve_crises: HashMap::new(),
            history: vec![],
            auto_research: [false; 3],
            zero_agency_ticks: 0,
            mercy_rule: false,
            threat_alert_level: vec![0; num_diseases],
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

    /// Infection trend: ratio of current infected to infected ~1 day ago.
    /// Returns None if not enough history. > 1.0 means growing, < 1.0 shrinking.
    pub fn infection_trend(&self) -> Option<f64> {
        // Look back ~1 day (120 ticks / HISTORY_INTERVAL = 24 entries)
        let lookback = (TICKS_PER_DAY as usize) / (HISTORY_INTERVAL as usize);
        if self.history.len() < lookback {
            return None;
        }
        let past = &self.history[self.history.len() - lookback];
        if past.total_infected < 100.0 {
            return None; // too few infections to show a meaningful trend
        }
        let current = self.total_infected();
        Some(current / past.total_infected)
    }

    /// Whether a specific disease has any active infections globally.
    pub fn disease_has_infected(&self, disease_idx: usize) -> bool {
        self.regions.iter().any(|r| {
            r.disease_state(disease_idx).is_some_and(|inf| inf.infected > 0.0)
        })
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

    /// Whether the screening level in a region reveals immune data.
    pub fn screening_shows_immune(&self, region_idx: usize) -> bool {
        self.policies.get(region_idx)
            .map(|p| p.screening.shows_immune())
            .unwrap_or(false)
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
        let field: u32 = self.field_research.iter().map(|p| p.personnel_assigned).sum();
        let applied = self.applied_research.as_ref().map_or(0, |p| p.personnel_assigned);
        let basic = self.basic_research.as_ref().map_or(0, |p| p.personnel_assigned);
        let policy: u32 = self.policies.iter().enumerate()
            .map(|(i, p)| {
                let traits = self.regions.get(i).map(|r| r.traits.as_slice()).unwrap_or(&[]);
                p.personnel_cost(traits)
            })
            .sum();
        field + applied + basic + policy
    }

    pub fn personnel_available(&self) -> u32 {
        self.resources.personnel.saturating_sub(self.personnel_busy())
    }

    pub fn total_policy_funding_cost(&self) -> f64 {
        self.policies.iter().enumerate()
            .map(|(i, p)| {
                let traits = self.regions.get(i).map(|r| r.traits.as_slice()).unwrap_or(&[]);
                p.funding_cost(traits)
            })
            .sum()
    }

    /// Per-region income contribution before travel ban modifier.
    /// `total_pop` must be > 0 (caller checks).
    fn region_base_income(region: &Region, total_pop: f64) -> f64 {
        let pop = region.population as f64;
        let infected: f64 = region.infections.iter().map(|inf| inf.infected).sum();
        let incapacitated = region.dead + infected * INFECTED_INCAPACITATION_RATE;
        let healthy_frac = (pop - incapacitated).max(0.0) / pop;
        let region_share = pop / total_pop;
        BASE_FUNDING_INCOME * region_share * healthy_frac * region.income_modifier
    }

    /// Estimated funding income per tick, based on current population health and policies.
    pub fn funding_income_rate(&self) -> f64 {
        let total_pop: f64 = self.regions.iter().map(|r| r.population as f64).sum();
        if total_pop <= 0.0 {
            return 0.0;
        }
        let mut income = 0.0;
        for (i, region) in self.regions.iter().enumerate() {
            // Collapsed regions contribute nothing — society has broken down
            if region.collapsed {
                continue;
            }
            let base = Self::region_base_income(region, total_pop);
            let travel_ban_factor = if self.policies.get(i).is_some_and(|p| p.travel_ban) {
                if region.has_trait(RegionTrait::TradeDependent) {
                    TRADE_DEPENDENT_INCOME_FACTOR
                } else {
                    TRAVEL_BAN_INCOME_PENALTY
                }
            } else {
                1.0
            };
            income += base * travel_ban_factor;
        }
        // Decree modifiers
        if self.enacted_decrees.sacrificed_region.is_some() {
            income *= SACRIFICE_INCOME_BONUS;
        }
        if self.enacted_decrees.conscript_researchers {
            income = (income - CONSCRIPT_INCOME_PENALTY).max(0.0);
        }
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
                let factor = if region.has_trait(RegionTrait::TradeDependent) { TRADE_DEPENDENT_INCOME_FACTOR } else { TRAVEL_BAN_INCOME_PENALTY };
                penalty += Self::region_base_income(region, total_pop) * (1.0 - factor);
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

    /// Current POL drift target based on severity, time, and active policies.
    /// POL drifts toward this value at ~30%/day. Called by engine::tick().
    pub fn pol_target(&self) -> f64 {
        let initial_pop = self.initial_population();
        let death_frac = if initial_pop > 0.0 { self.total_dead() / initial_pop } else { 0.0 };
        let infected_frac = if initial_pop > 0.0 { self.total_infected() / initial_pop } else { 0.0 };
        let time_frac = self.tick as f64 / (30.0 * TICKS_PER_DAY);
        let severity = death_frac.sqrt() + infected_frac.sqrt() * 0.4;
        let active_policies: u32 = self.policies.iter().map(|p| p.active_count()).sum();
        let policy_drain = active_policies as f64 * 0.02;
        (severity + time_frac * 0.1 - policy_drain).clamp(0.0, 0.90)
    }

    /// The next policy that would unlock with more POL. Returns (name, threshold)
    /// for the lowest-threshold policy not yet globally available, or None if all
    /// are already unlocked at current POL.
    pub fn next_pol_unlock(&self) -> Option<(&'static str, f64)> {
        let pol = self.resources.political_power;
        let mut best: Option<(&'static str, f64)> = None;
        for idx in 0..POLICY_COUNT {
            let threshold = POLICY_POL_THRESHOLDS[idx];
            if threshold <= 0.0 {
                continue; // Always available, skip
            }
            if pol >= threshold {
                continue; // Already unlocked
            }
            if best.is_none() || threshold < best.unwrap().1 {
                best = Some((policy_display_name(idx), threshold));
            }
        }
        best
    }

    /// Spawn a new disease mid-game: generates a random disease, places an initial
    /// outbreak in a random region, and creates a matching targeted medicine.
    /// Returns `(disease_idx, region_idx)` if successful, or `None` if at the cap.
    /// Uses `self.rng` — caller must have extracted rng if borrowing mutably.
    pub fn spawn_disease(&mut self, rng: &mut ChaCha8Rng) -> Option<(usize, usize)> {
        // If at capacity, try to recycle a burned-out disease slot.
        let recycle_idx = if self.diseases.len() >= MAX_DISEASES {
            self.find_burned_out_disease()
        } else {
            None
        };

        if self.diseases.len() >= MAX_DISEASES && recycle_idx.is_none() {
            return None;
        }

        // Pick a pathogen type (weighted: prions rare).
        // Enforce diversity: no type appears more than twice among active diseases.
        let mut type_counts = HashMap::new();
        for (i, d) in self.diseases.iter().enumerate() {
            // Skip the slot being recycled — it's about to be replaced
            if recycle_idx == Some(i) { continue; }
            *type_counts.entry(d.pathogen_type).or_insert(0usize) += 1;
        }
        let mut types = vec![
            PathogenType::RnaVirus,
            PathogenType::RnaVirus,
            PathogenType::DnaVirus,
            PathogenType::Bacterium,
            PathogenType::Bacterium,
            PathogenType::Fungus,
        ];
        if rng.r#gen::<f64>() < 0.15 {
            types.push(PathogenType::Prion);
        }
        types.retain(|t| type_counts.get(t).copied().unwrap_or(0) < 2);
        // Fallback: if all types are saturated, allow any type
        if types.is_empty() {
            types = vec![
                PathogenType::RnaVirus,
                PathogenType::DnaVirus,
                PathogenType::Bacterium,
                PathogenType::Fungus,
                PathogenType::Prion,
            ];
        }
        let pathogen_type = types[rng.r#gen::<usize>() % types.len()];

        let used_names: Vec<String> = self.diseases.iter().map(|d| d.name.clone()).collect();

        let disease_idx = if let Some(idx) = recycle_idx {
            // Recycle: replace the burned-out disease and clean up its traces.
            let mut disease = Disease::generate(rng, pathogen_type, &used_names, true);
            disease.detected = false;
            self.diseases[idx] = disease;

            // Remove all infection entries for the old disease in all regions.
            for region in &mut self.regions {
                region.infections.retain(|inf| inf.disease_idx != idx);
            }

            // Cancel any active field research targeting the recycled disease.
            self.field_research.retain(|r| !r.references_disease(idx));
            if self.applied_research.as_ref().is_some_and(|r| r.references_disease(idx)) {
                self.applied_research = None;
            }

            // Remove old medicines targeting the recycled disease (excluding broad-spectrum).
            self.medicines.retain(|m| {
                m.therapy_type == TherapyType::BroadSpectrum
                    || !(m.target_diseases.len() == 1 && m.target_diseases[0] == idx)
            });
            // Add new medicines for the replacement disease.
            self.medicines.extend(Medicine::targeted_medicines(idx, pathogen_type));

            idx
        } else {
            // Normal path: append new disease.
            let idx = self.diseases.len();
            let mut disease = Disease::generate(rng, pathogen_type, &used_names, true);
            disease.detected = false;
            self.diseases.push(disease);
            self.medicines.extend(Medicine::targeted_medicines(idx, pathogen_type));

            // Update broad-spectrum medicine to also target new disease
            for med in &mut self.medicines {
                if med.therapy_type == TherapyType::BroadSpectrum
                    && !med.target_diseases.contains(&idx)
                {
                    med.target_diseases.push(idx);
                }
            }

            idx
        };

        // Place initial outbreak in a random non-collapsed region (prefer viable targets)
        let viable: Vec<usize> = self.regions.iter().enumerate()
            .filter(|(_, r)| !r.collapsed)
            .map(|(i, _)| i)
            .collect();
        let region_idx = if viable.is_empty() {
            rng.r#gen::<usize>() % self.regions.len()
        } else {
            viable[rng.r#gen::<usize>() % viable.len()]
        };
        let initial_infected = 500.0 + rng.r#gen::<f64>() * 2_000.0;
        self.regions[region_idx].infections.push(RegionDiseaseState {
            disease_idx,
            infected: initial_infected,
            dead: 0.0,
            immune: 0.0,
        });

        Some((disease_idx, region_idx))
    }

    /// Find a disease with zero infected across all regions (fully burned out).
    fn find_burned_out_disease(&self) -> Option<usize> {
        for (d_idx, _disease) in self.diseases.iter().enumerate() {
            let total_infected: f64 = self.regions.iter()
                .filter_map(|r| r.disease_state(d_idx))
                .map(|inf| inf.infected)
                .sum();
            if total_infected < 1.0 {
                return Some(d_idx);
            }
        }
        None
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

    /// True if any deployed medicine targeting this disease has significant resistance (≥30%).
    pub fn has_resistant_medicine(&self, disease_idx: usize) -> bool {
        self.medicines.iter().any(|m| {
            m.target_diseases.contains(&disease_idx)
                && m.deployed_count > 0
                && m.resistance_factor(disease_idx, &self.diseases) < 0.7
        })
    }

    /// Check if the player has zero agency — no meaningful actions available.
    pub fn has_zero_agency(&self) -> bool {
        let upkeep = self.personnel_upkeep_rate();
        let policy_cost = self.total_policy_funding_cost();
        let income = self.funding_income_rate();
        let net_income = income - upkeep - policy_cost;

        // Must have very low funds AND negative/zero income (can't recover)
        let broke = self.resources.funding < 100.0 && net_income <= 0.0;
        // No active research of any kind
        let no_research = self.field_research.is_empty()
            && self.applied_research.is_none()
            && self.basic_research.is_none();
        // No medicine doses to deploy
        let no_doses = self.medicines.iter().all(|m| m.doses <= 0.0);

        broke && no_research && no_doses
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
        let deployed_any = self.medicines.iter().any(|m| m.unlocked && m.deployed_count > 0);
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
        let any_policy_active = self.policies.iter().any(|p| p.any_active());
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
        // Migrate shared death counter from per-disease attribution (pre-shared-death saves).
        for region in &mut self.regions {
            if region.dead == 0.0 {
                let attributed: f64 = region.infections.iter().map(|i| i.dead).sum();
                if attributed > 0.0 {
                    region.dead = attributed.min(region.population as f64);
                }
            }
        }
    }

    /// Available field research projects (excludes currently active).
    pub fn available_field_projects(&self) -> Vec<ResearchKind> {
        let active_kinds: Vec<&ResearchKind> = self.field_research.iter().map(|p| &p.kind).collect();
        let mut projects = Vec::new();
        // Identify Threat: diseases not fully known, sorted by knowledge ascending
        // (unknown diseases first, then partially identified)
        let mut identify_targets: Vec<(usize, f64)> = self.diseases.iter().enumerate()
            .filter(|(i, d)| d.detected && d.knowledge < KNOWLEDGE_FULL && self.disease_has_infected(*i))
            .map(|(i, d)| (i, d.knowledge))
            .collect();
        identify_targets.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (i, _knowledge) in identify_targets {
            let kind = ResearchKind::IdentifyThreat { disease_idx: i };
            if !active_kinds.contains(&&kind) {
                projects.push(kind);
            }
        }
        // Genomic Sequencing: fully identified diseases that still mutate and are active
        for (i, disease) in self.diseases.iter().enumerate() {
            if disease.knowledge >= KNOWLEDGE_FULL
                && disease.effective_mutation_rate() > 0.0001
                && self.disease_has_infected(i)
            {
                let kind = ResearchKind::GenomicSequencing { disease_idx: i };
                if !active_kinds.contains(&&kind) {
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
                if !self.disease_has_infected(d_idx) {
                    continue;
                }
                let needs_trial = if !med.tested_against.contains(&d_idx) {
                    true // Never tested
                } else {
                    // Tested, but check if strain has drifted
                    let med_gen = med.strain_generations.get(target_pos).copied().unwrap_or(0);
                    let disease_gen = self.diseases.get(d_idx)
                        .map_or(0, |d| d.strain_generation) as i32;
                    disease_gen > med_gen
                };
                if needs_trial {
                    let kind = ResearchKind::ClinicalTrial {
                        medicine_idx: i,
                        disease_idx: d_idx,
                    };
                    if !active_kinds.contains(&&kind) {
                        projects.push(kind);
                    }
                }
            }
        }
        projects
    }

    /// Get the active research project for a given track.
    /// For Field track (which supports multiple projects), returns the first.
    pub fn research_slot(&self, track: ResearchTrack) -> Option<&ResearchProject> {
        match track {
            ResearchTrack::Field => self.field_research.first(),
            ResearchTrack::Applied => self.applied_research.as_ref(),
            ResearchTrack::Basic => self.basic_research.as_ref(),
        }
    }

    /// Whether field research has capacity for another project.
    pub fn field_research_has_capacity(&self) -> bool {
        self.field_research.len() < MAX_FIELD_RESEARCH
    }

    /// Available research projects for a given track (excludes currently active).
    pub fn available_projects(&self, track: ResearchTrack) -> Vec<ResearchKind> {
        match track {
            ResearchTrack::Field => self.available_field_projects(),
            ResearchTrack::Applied => self.available_applied_projects(),
            ResearchTrack::Basic => self.available_basic_projects(),
        }
    }

    /// Project costs adjusted for unlocked technologies.
    /// Currently: RapidSequencing halves GenomicSequencing duration.
    pub fn effective_costs(&self, kind: &ResearchKind) -> (u32, f64, f64) {
        let (personnel, mut duration, funding) = kind.costs(&self.medicines);
        if matches!(kind, ResearchKind::GenomicSequencing { .. })
            && self.unlocked_techs.contains(&BasicTech::RapidSequencing)
        {
            duration *= 0.5;
        }
        (personnel, duration, funding)
    }

    /// Vaccination effectiveness multiplier. VaccinePlatform tech triples it.
    pub fn vaccination_multiplier(&self) -> f64 {
        if self.unlocked_techs.contains(&BasicTech::VaccinePlatform) {
            3.0
        } else {
            1.0
        }
    }

    /// True if the player has unlocked Resistance Surveillance (can see resistance levels).
    pub fn has_resistance_surveillance(&self) -> bool {
        self.unlocked_techs.contains(&BasicTech::ResistanceSurveillance)
    }

    /// Resistance buildup multiplier. CombinationTherapy tech halves it.
    pub fn resistance_multiplier(&self) -> f64 {
        if self.unlocked_techs.contains(&BasicTech::CombinationTherapy) {
            0.5
        } else {
            1.0
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
            // Targeted medicines (Antiviral/Antibiotic) require full study (knowledge 1.0)
            // plus TargetedDrugDesign tech. Broad-spectrum only needs identification (0.5).
            let is_targeted = med.therapy_type != TherapyType::BroadSpectrum;
            let knowledge_threshold = if is_targeted { KNOWLEDGE_FOR_TARGETED } else { KNOWLEDGE_FOR_MEDICINE };
            let has_knowledge = med.target_diseases.iter().any(|&d_idx| {
                self.diseases.get(d_idx).map_or(false, |d| d.knowledge >= knowledge_threshold)
            });
            let has_tech = !is_targeted
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
        // Two targeted medicines per non-prion disease + one broad-spectrum
        let targeted_count: usize = state.medicines.iter()
            .filter(|m| m.target_diseases.len() == 1)
            .count();
        assert!(targeted_count >= disease_count * 2,
            "expected at least 2 targeted medicines per disease, got {targeted_count}");
        assert_eq!(state.medicines.last().unwrap().therapy_type, TherapyType::BroadSpectrum);
        // Medicines start locked — must be developed via research
        assert!(state.medicines.iter().all(|m| !m.unlocked));
        // Each disease should have at least one targeted medicine
        for i in 0..disease_count {
            assert!(state.medicines.iter().any(|m| m.target_diseases == vec![i]),
                "disease {i} should have a targeted medicine");
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

    #[test]
    fn targeted_medicine_requires_full_study() {
        let mut state = GameState::new_default(42);
        // Give identification-level knowledge (0.5) — enough for broad-spectrum
        for d in &mut state.diseases {
            d.knowledge = KNOWLEDGE_FOR_MEDICINE;
        }
        // Unlock TargetedDrugDesign so the tech gate isn't the blocker
        state.unlocked_techs.push(BasicTech::TargetedDrugDesign);

        let projects = state.available_applied_projects();
        // Broad-spectrum should be available (only needs knowledge 0.5)
        let has_broad = projects.iter().any(|k| match k {
            ResearchKind::DevelopMedicine { medicine_idx } =>
                state.medicines[*medicine_idx].therapy_type == TherapyType::BroadSpectrum,
            _ => false,
        });
        assert!(has_broad, "broad-spectrum should be available at knowledge 0.5");

        // Targeted should NOT be available (needs knowledge 1.0)
        let has_targeted = projects.iter().any(|k| match k {
            ResearchKind::DevelopMedicine { medicine_idx } =>
                state.medicines[*medicine_idx].therapy_type != TherapyType::BroadSpectrum,
            _ => false,
        });
        assert!(!has_targeted, "targeted medicine should NOT be available at knowledge 0.5");

        // Now give full study knowledge (1.0) — targeted should unlock
        for d in &mut state.diseases {
            d.knowledge = KNOWLEDGE_FULL;
        }
        let projects = state.available_applied_projects();
        let has_targeted = projects.iter().any(|k| match k {
            ResearchKind::DevelopMedicine { medicine_idx } =>
                state.medicines[*medicine_idx].therapy_type != TherapyType::BroadSpectrum,
            _ => false,
        });
        assert!(has_targeted, "targeted medicine should be available at knowledge 1.0");
    }

    #[test]
    fn mechanism_branching_shows_all_variants() {
        let mut state = GameState::new_default(42);
        // Full knowledge + TargetedDrugDesign unlocks all mechanism variants
        for d in &mut state.diseases {
            d.knowledge = KNOWLEDGE_FULL;
        }
        state.unlocked_techs.push(BasicTech::TargetedDrugDesign);

        let projects = state.available_applied_projects();
        let develop_projects: Vec<_> = projects.iter()
            .filter(|k| matches!(k, ResearchKind::DevelopMedicine { .. }))
            .collect();
        // Disease 0 (Bacterium in default seed) should have 4 targeted + 1 broad = 5
        // Or 3 targeted + 1 broad = 4 if virus/fungus
        // At minimum, we should see more than 2 develop options
        assert!(develop_projects.len() >= 4,
            "should have 4+ develop options (all mechanisms + broad), got {}: {:?}",
            develop_projects.len(),
            develop_projects.iter().map(|k| match k {
                ResearchKind::DevelopMedicine { medicine_idx } =>
                    state.medicines[*medicine_idx].name.clone(),
                _ => "?".to_string(),
            }).collect::<Vec<_>>());

        // Each mechanism variant should have different cost multipliers
        let costs: Vec<_> = develop_projects.iter()
            .map(|k| match k {
                ResearchKind::DevelopMedicine { .. } =>
                    k.costs(&state.medicines),
                _ => (0, 0.0, 0.0),
            })
            .collect();
        // Should have different costs (not all the same)
        let unique_costs: std::collections::HashSet<_> = costs.iter()
            .map(|(p, _, _)| *p)
            .collect();
        assert!(unique_costs.len() >= 2,
            "mechanism variants should have different development costs");
    }

    #[test]
    fn mechanism_efficacy_affects_deployment() {
        let state = GameState::new_default(42);
        // Disease 0 is always a Bacterium in seed 42 — both mechanisms must exist
        let fast_idx = state.medicines.iter().position(|m|
            m.mechanism == Some(MechanismOfAction::CellWallInhibitor)
        ).expect("CellWallInhibitor medicine should exist for Bacterium");
        let slow_idx = state.medicines.iter().position(|m|
            m.mechanism == Some(MechanismOfAction::MetabolicInhibitor)
        ).expect("MetabolicInhibitor medicine should exist for Bacterium");
        let fast_eff = state.medicines[fast_idx].effective_efficacy(0, &state.diseases);
        let slow_eff = state.medicines[slow_idx].effective_efficacy(0, &state.diseases);
        // Fast mechanism should have higher initial efficacy
        assert!(fast_eff > slow_eff,
            "fast mechanism should have higher efficacy: {} vs {}",
            fast_eff, slow_eff);
    }

    #[test]
    fn defeat_tips_no_false_never_deployed_after_deploy() {
        let mut state = GameState::new_default(42);
        state.medicines[0].unlocked = true;
        state.medicines[0].deployed_count = 3;
        // Even if doses are back at max (re-manufactured), deployed_count tracks it
        state.medicines[0].doses = state.medicines[0].max_doses;
        let tips = state.defeat_tips();
        assert!(!tips.iter().any(|t| t.contains("never deployed")),
            "should not claim 'never deployed' when deployed_count > 0: {:?}", tips);
    }

    #[test]
    fn all_regions_have_traits() {
        let state = GameState::new_default(42);
        for region in &state.regions {
            assert!(!region.traits.is_empty(),
                "{} should have at least one trait", region.name);
        }
    }

    #[test]
    fn trade_dependent_travel_ban_costs_more() {
        let mut policy = RegionPolicy::default();
        policy.travel_ban = true;
        let base_cost = policy.funding_cost(&[]);
        let trade_cost = policy.funding_cost(&[RegionTrait::TradeDependent]);
        assert!(trade_cost > base_cost,
            "TradeDependent should increase travel ban cost: {} vs {}", trade_cost, base_cost);
        assert!((trade_cost - base_cost * 2.0).abs() < 0.01,
            "TradeDependent should double travel ban cost");
    }

    #[test]
    fn low_infrastructure_increases_personnel() {
        let mut policy = RegionPolicy::default();
        policy.quarantine = true;
        policy.hospital_surge = true;
        let base = policy.personnel_cost(&[]);
        let low_infra = policy.personnel_cost(&[RegionTrait::LowInfrastructure]);
        // 2 active policies, each +1 = base + 2
        assert_eq!(low_infra, base + 2,
            "LowInfrastructure should add +1 per active policy");
    }
}
