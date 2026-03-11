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

/// Deserialize hospital_level: accepts old `healthcare_invested: bool` saves
/// (true → 1, false → 0) and new `hospital_level: u8` saves.
fn deserialize_hospital_level<'de, D>(deserializer: D) -> Result<u8, D::Error>
where D: Deserializer<'de> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HospitalLevel {
        Level(u8),
        OldBool(bool),
    }
    match HospitalLevel::deserialize(deserializer)? {
        HospitalLevel::Level(n) => Ok(n),
        HospitalLevel::OldBool(true) => Ok(1),
        HospitalLevel::OldBool(false) => Ok(0),
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
    /// Tick when the last crisis was resolved. Used to enforce a minimum gap
    /// between consecutive crises so they don't become white noise.
    #[serde(default)]
    pub last_crisis_resolved_tick: u64,
    /// Auto-resolve preferences: crisis tag → choice index (0 = A, 1 = B).
    /// When a crisis fires whose tag matches, it's resolved immediately without pausing.
    #[serde(default)]
    pub auto_resolve_crises: HashMap<String, usize>,
    /// Auto-research: when a project completes, automatically start the next
    /// highest-priority available project (if affordable). Per-track toggle.
    #[serde(default)]
    pub auto_research: [bool; 3],
    /// Auto-deploy: automatically deploy tested medicines to the worst-affected
    /// region each tick cycle. Per-medicine toggle, indexed by medicine index.
    #[serde(default)]
    pub auto_deploy: Vec<bool>,
    /// Standing orders: automation rules that fire during tick when conditions are met.
    #[serde(default)]
    pub standing_orders: StandingOrders,
    /// Medicine shipments in transit. Created on deploy; effects apply on arrival.
    #[serde(default)]
    pub pending_shipments: Vec<Shipment>,
    /// Historical snapshots for dashboard charts. Recorded every HISTORY_INTERVAL ticks.
    #[serde(default)]
    pub history: Vec<HistorySnapshot>,
    /// Per-disease highest death milestone tier already notified (0=none, 1=1M, 2=100M, 3=1B).
    /// Prevents repeat alerts for the same threshold.
    #[serde(default, alias = "threat_alert_level")]
    pub death_milestone_tier: Vec<u8>,
    /// Per-disease flag: whether the pre-detection intel briefing has fired for this disease.
    /// Advanced Intel stations generate a warning before full detection when local infections
    /// cross 500. This vec is grown alongside `diseases` to track which diseases have been
    /// briefed already.
    #[serde(default)]
    pub intel_pre_detection_briefed: Vec<bool>,
    /// Emergency consolidation: when activated, all resources concentrate on one region.
    /// Contains the index of the consolidated HQ region, or None if not active.
    #[serde(default)]
    pub ark_protocol: Option<usize>,
    /// Cumulative doses deployed across all medicines and regions.
    #[serde(default)]
    pub total_doses_deployed: f64,
    /// Active field operations (Recon, Emergency Response, Infrastructure Survey).
    /// These cost personnel and time, not money.
    #[serde(default)]
    pub field_operations: Vec<FieldOperation>,
    /// Crisis response operations: temporary personnel commitments from crisis resolutions.
    /// Personnel are returned automatically when the operation completes.
    #[serde(default)]
    pub crisis_operations: Vec<CrisisOperation>,
    /// Count of pathogen suppression operations completed (field research).
    #[serde(default)]
    pub pathogens_suppressed: u32,
    /// Count of pathogen attenuation operations completed (field research).
    #[serde(default)]
    pub pathogens_attenuated: u32,
    /// Count of pathogen interdiction operations completed (field research).
    #[serde(default)]
    pub pathogens_interdicted: u32,
    /// Global research lab level (0=Standard, 1=Enhanced Sequencing, 2=Advanced Genomics).
    /// Built via the Research panel. Each level multiplies all research progress rates.
    #[serde(default)]
    pub lab_level: u8,
    /// Active funding contracts providing conditional income.
    #[serde(default)]
    pub contracts: Vec<FundingContract>,
    /// Pending contract offer (accept/reject via Policy panel). None if no offer.
    #[serde(default)]
    pub contract_offer: Option<FundingContract>,
    /// Tick when the last contract was offered (for spacing offers).
    #[serde(default)]
    pub last_contract_offer_tick: u64,
    /// Regional corporations. 3 per region (18 total). Source of player income.
    #[serde(default)]
    pub corporations: Vec<Corporation>,
    /// Tick when the last board demand crisis fired (cooldown tracking).
    #[serde(default)]
    pub last_board_demand_tick: u64,
    /// Monotonically increasing counter for assigning sequence group IDs to
    /// wave-coordinated diseases. Incremented each time a new group is created.
    #[serde(default)]
    pub next_sequence_group: u32,
    /// Active emergency loans. Interest accrues each day; hostile action fires if unpaid.
    #[serde(default)]
    pub loans: Vec<ActiveLoan>,
    pub ui: UiState,
}

/// A point-in-time snapshot for dashboard sparkline charts.
/// Values are player-visible estimates (screened/detected), not ground truth.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub tick: u64,
    /// Screened infected count (visibility depends on screening policy level).
    #[serde(alias = "total_infected")]
    pub screened_infected: f64,
    /// Dead from detected diseases only (unidentified diseases not counted).
    #[serde(alias = "total_dead")]
    pub detected_dead: f64,
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
/// At 1.0, a single deployment of a perfectly matched medicine treats all infected.
/// Broad-spectrum (0.15 efficacy) treats ~15% per deploy — a bandaid, not a cure.
pub const TREATMENT_FRACTION: f64 = 1.0;
/// Fraction of susceptible population vaccinated per deployment (before efficacy).
/// Vaccination is proportional like treatment. At 0.15, a targeted vaccine (eff 1.0)
/// covers 15% of susceptible per deploy. Five deployments cover ~56%, ten cover ~80%.
/// This creates compound returns: each deploy shrinks the susceptible pool, slowing
/// future infection growth. VaccinePlatform tech (3x) makes this very powerful.
pub const VACCINATION_FRACTION: f64 = 0.15;

/// Efficacy multiplier when deploying a medicine against a disease it wasn't
/// specifically developed for, but whose mechanism matches the pathogen type
/// (e.g., a CellWall inhibitor developed for Bacterium-A used against Bacterium-B).
pub const CROSS_REACTIVE_PENALTY: f64 = 0.5;

// Disease emergence constants.
/// First new disease can emerge after this many ticks (~day 5).
/// Gives the player time to identify disease 0 and start the research pipeline.
pub const EMERGENCE_MIN_TICK: u64 = (5.0 * TICKS_PER_DAY) as u64;
/// Per-tick probability of a new disease emerging (after min tick).
/// ~1 new disease every 14 days → steady pressure without overwhelming the
/// research pipeline (which takes ~14-20 days per disease).
pub const EMERGENCE_CHANCE_PER_TICK: f64 = 0.0012;
/// Maximum number of simultaneous diseases.
pub const MAX_DISEASES: usize = 5;

// Economy constants — single source of truth.
pub const BASE_FUNDING_INCOME: f64 = 5.4;
/// Per-tick cost for each personnel on the roster (busy or idle).
/// 20 personnel × 0.06 = $1.2/tick = $144/day upkeep vs ~$648/day base income.
/// With 2 contracts (~¥360/day), gross ~¥1008/day → ~¥864/day net.
/// History: 0.10 made training a trap (50% of income); 0.03 doubled income, trivializing economy.
pub const PERSONNEL_UPKEEP_COST: f64 = 0.06;
/// Fraction of infected people who are too sick to contribute economically.
/// 70% are incapacitated (hospitalized, quarantined, bedridden); 30% are mild/asymptomatic.
pub const INFECTED_INCAPACITATION_RATE: f64 = 0.7;
pub const TRAVEL_BAN_INCOME_PENALTY: f64 = 0.5;
/// Fraction of regional income that depends on connected neighbors' economic health.
/// Domestic = (1 - TRADE_INCOME_FRACTION), Trade = TRADE_INCOME_FRACTION × avg(neighbor health).
/// Set to 0.25: a region with all neighbors healthy gets full income, but if all neighbors
/// collapse, it loses 25% of income even if healthy itself.
pub const TRADE_INCOME_FRACTION: f64 = 0.25;
pub const TRAVEL_BAN_COST: f64 = 1.0;
pub const TRAVEL_BAN_PERSONNEL: u32 = 3;
pub const QUARANTINE_COST: f64 = 0.6;
pub const QUARANTINE_PERSONNEL: u32 = 3;
pub const HOSPITAL_SURGE_COST: f64 = 0.4;
pub const HOSPITAL_SURGE_PERSONNEL: u32 = 2;
/// Hospital Surge increases infectivity by this factor (1.25 = +25% spread).
pub const HOSPITAL_SURGE_SPREAD_FACTOR: f64 = 1.25;
/// Co-infection lethality multiplier per additional active disease in the same
/// region. With 2 diseases: +25% lethality; 3 diseases: +50%.
/// Threshold: a disease counts as "co-infecting" when it has >= 1000 infected.
pub const COINFECTION_LETHALITY_PER_DISEASE: f64 = 0.25;
pub const COINFECTION_THRESHOLD: f64 = 1000.0;
pub const BORDER_CONTROLS_COST: f64 = 0.1;
pub const BORDER_CONTROLS_PERSONNEL: u32 = 1;
pub const WATER_SANITATION_COST: f64 = 0.3;
pub const WATER_SANITATION_PERSONNEL: u32 = 1;
pub const MARTIAL_LAW_COST: f64 = 1.5;
pub const MARTIAL_LAW_PERSONNEL: u32 = 4;
/// One-time funding cost for nuclear annihilation (no ongoing cost).
pub const NUCLEAR_ANNIHILATION_COST: f64 = 200.0;
/// Field Hospital (Level 1): one-time build cost per region.
/// Reduces lethality by 25%, requires 1 ongoing personnel.
pub const FIELD_HOSPITAL_COST: f64 = 500.0;
/// Field Hospital ongoing personnel requirement.
pub const FIELD_HOSPITAL_PERSONNEL: u32 = 1;
/// Medical Center (Level 2): upgrade cost on top of Level 1.
/// Total lethality reduction 40%, +25% medicine efficacy, requires 2 ongoing personnel.
pub const MEDICAL_CENTER_COST: f64 = 800.0;
/// Medical Center ongoing personnel requirement (replaces Level 1 cost).
pub const MEDICAL_CENTER_PERSONNEL: u32 = 2;
/// Medicine deployment effectiveness bonus in regions with Medical Center.
pub const MEDICAL_CENTER_EFFICACY_BONUS: f64 = 0.25;

/// Intel Station (Level 1): one-time build cost per region.
/// Detects new diseases at 3,000 local infections instead of 10,000. Requires 1 ongoing personnel.
pub const INTEL_STATION_COST: f64 = 75.0;
/// Intel Station ongoing personnel requirement (Level 1 and Level 2).
pub const INTEL_STATION_PERSONNEL: u32 = 1;
/// Advanced Intel (Level 2): upgrade cost on top of Level 1.
/// Detects at 1,000 local infections. Generates intelligence briefings. Requires 2 ongoing personnel.
pub const ADVANCED_INTEL_COST: f64 = 150.0;
/// Advanced Intel ongoing personnel requirement (replaces Level 1 cost).
pub const ADVANCED_INTEL_PERSONNEL: u32 = 2;

/// Ticks a neighboring-collapse disruption lasts (10 days).
pub const COLLAPSE_DISRUPTION_TICKS: u64 = (10.0 * TICKS_PER_DAY) as u64;
/// Medicine deployment cost multiplier for regions disrupted by a neighboring collapse.
pub const DISRUPTION_MEDICINE_COST_MULT: f64 = 1.5;
/// Post-collapse secondary death rate: fraction of alive population lost per day
/// to starvation, violence, and infrastructure breakdown.
pub const COLLAPSE_DEATH_RATE: f64 = 0.05;
/// Subsistence floor: collapse deaths stop when population falls to this fraction
/// of the original. Represents survivors who can sustain themselves without
/// modern infrastructure.
pub const COLLAPSE_SUBSISTENCE_FLOOR: f64 = 0.02;

/// Maximum active funding contracts.
pub const MAX_CONTRACTS: usize = 3;
/// Ticks between contract offers (~5 days).
pub const CONTRACT_OFFER_INTERVAL: u64 = (5.0 * TICKS_PER_DAY) as u64;
/// Tick when the first contract offer appears (~1 day).
pub const CONTRACT_FIRST_OFFER_TICK: u64 = TICKS_PER_DAY as u64;

/// Research Lab upgrade costs (one-time, no ongoing personnel cost).
/// Level 1 (Enhanced Sequencing Lab): +30% research speed.
/// Level 2 (Advanced Genomics Center): +60% research speed.
pub const LAB_LEVEL_1_COST: f64 = 150.0;
pub const LAB_LEVEL_2_COST: f64 = 300.0;

/// Disease surveillance intensity. Each tier reveals different information
/// and only Mass Rapid screening actively reduces disease spread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
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
pub const DECREE_COUNT: usize = 6;
/// Number of standing orders shown in the Orders panel.
/// Must equal the length of the `standing_orders` array in `ui/policy.rs`.
/// Used by `panel_selection_max()` to bound navigation — if you add a standing
/// order in policy.rs without updating this constant, the new item will be
/// silently unreachable via keyboard navigation.
pub const STANDING_ORDER_COUNT: usize = 2;
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
/// Fortify Region: infrastructure penalty applied to all OTHER non-collapsed regions.
pub const FORTIFY_INFRA_PENALTY: f64 = 0.25;
/// Emergency Countermeasure: fraction of alive population killed immediately.
pub const COUNTERMEASURE_KILL_FRACTION: f64 = 0.10;
/// Emergency Countermeasure: infectivity multiplier applied to all diseases.
pub const COUNTERMEASURE_INFECTIVITY_MULT: f64 = 0.50;
/// Emergency Countermeasure: cross-region spread multiplier applied to all diseases.
pub const COUNTERMEASURE_SPREAD_MULT: f64 = 0.25;
/// Per-tick cost for each screening level (halved from original — screening
/// is now genuinely useful since it provides real fog-of-war rather than
/// a transparent multiplier).
pub const SCREENING_BASIC_COST: f64 = 0.1;
pub const SCREENING_ANTIGEN_COST: f64 = 0.25;
pub const SCREENING_MASS_RAPID_COST: f64 = 0.5;

/// Screening ramp-up rate per tick. At ~0.004/tick, full progress takes
/// ~250 ticks ≈ 4.2 days (TICKS_PER_DAY=60). This prevents the toggle-peek exploit.
pub const SCREENING_RAMP_RATE: f64 = 0.004;
/// Screening decay rate when disabled. Decays ~2x faster than build-up.
pub const SCREENING_DECAY_RATE: f64 = 0.008;

impl ScreeningLevel {
    /// Theoretical accuracy at full screening progress. Used for:
    ///   1. UI indicators (`~` prefix) and tier descriptions.
    ///   2. Noise suppression scaling in `tick_screening()` — higher visibility
    ///      means the per-region systematic bias is suppressed more aggressively,
    ///      so upgrading screening reveals more *accurate* data, not just faster data.
    ///
    /// NOT used as a direct multiplier on infected counts (displayed values come
    /// from the convergence-based `estimated_infected` system).
    pub fn visibility_rate(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.15,
            ScreeningLevel::Basic => 0.40,
            ScreeningLevel::Antigen => 0.75,
            ScreeningLevel::MassRapid => 0.95,
        }
    }

    /// Per-tick convergence rate for the infected estimate toward ground truth.
    /// Higher = faster tracking. At None, the estimate is always significantly
    /// behind reality (creating genuine fog of war). At MassRapid, near real-time.
    ///
    /// Steady-state accuracy vs a disease doubling every 3 days:
    ///   None:      ~25% of real (very foggy)
    ///   Basic:     ~60% of real (rough idea)
    ///   Antigen:   ~90% of real (good tracking)
    ///   MassRapid: ~99% of real (near-perfect)
    pub fn convergence_rate(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.0007,     // Estimate lags badly behind exponential growth
            ScreeningLevel::Basic => 0.003,      // Rough tracking — still significantly behind
            ScreeningLevel::Antigen => 0.02,     // Good tracking, slight lag on fast growth
            ScreeningLevel::MassRapid => 0.15,   // Near real-time
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
    /// Reduces infection rate within the region (20–65% depending on transmission vector).
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
    /// Progress of screening infrastructure build-up (0.0 to 1.0).
    /// Ramps up over ~2 days when screening is active; decays when off.
    /// Prevents toggle-peek exploit: instant on/off gives no benefit.
    #[serde(default)]
    pub screening_progress: f64,
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
///   0 = Travel Ban        5 = Basic Screening     8 = Martial Law
///   1 = Quarantine         6 = Antigen Screening   9 = Nuclear Annihilation
///   2 = Hospital Surge     7 = Mass Rapid Screen  10 = Field Hospital
///   3 = Border Controls   11 = Intel Station
///   4 = Water Sanitation
///
/// Display position is determined by `policy_display_order()` (grouped by function).
/// If you add a new policy, you must update:
///   - POLICY_COUNT and POLICY_POL_THRESHOLDS (this file)
///   - get_bool/set_bool if it's a boolean policy (this file)
///   - toggle_policy and tick_enforce_costs (engine/policy.rs)
///   - render_manage policies vec (ui/policy.rs)
pub const POLICY_COUNT: usize = 12;

/// Policy index for Martial Law. Included in active_policy_costs alongside 0-4
/// because it has a recurring funding cost, unlike Nuclear (9) which is one-shot.
pub const POLICY_IDX_MARTIAL_LAW: usize = 8;

/// Policy index for Nuclear Annihilation. Special-cased in UI (☢ icon) and engine
/// (allowed in collapsed regions when all other policies are unavailable).
pub const POLICY_IDX_NUCLEAR: usize = 9;

/// Policy index for the first (Basic) screening tier. The three screening tiers occupy
/// indices POLICY_IDX_SCREENING_BASE through POLICY_IDX_SCREENING_BASE + 2
/// (Basic / Med / Mass Screening), all backed by the single `screening` enum field.
pub const POLICY_IDX_SCREENING_BASE: usize = 5;

/// Minimum Political Power (0.0–1.0) required to activate each policy.
/// Indexed by policy_idx (see POLICY_COUNT doc for the mapping).
pub const POLICY_POL_THRESHOLDS: [f64; POLICY_COUNT] = [
    0.15, // Travel Ban — basic containment, available early
    0.20, // Quarantine — restricts movement, moderate political will
    0.10, // Hospital Surge — medical infrastructure, needs some authority
    0.00, // Border Controls — basic containment, always available (costs personnel + funding)
    0.00, // Water Sanitation — basic public health, always available (costs personnel + funding)
    0.00, // Basic Screening — disease reporting, always available (costs personnel + funding)
    0.10, // Antigen Screening — mandatory testing, needs political will
    0.15, // Mass Rapid Screening — mandatory mass testing, needs political will
    0.40, // Martial Law — drastic, needs high political will
    0.35, // Nuclear Annihilation — extreme, but collapsed regions raise urgency
    0.15, // Field Hospital — institutional build, needs political authority
    0.00, // Intel Station — always available, encourages early investment
];

/// Panel selection positions for the ManagePolicies subpanel.
///
/// Layout: [0..POLICY_COUNT) = policy toggles in display order,
///         MANAGE_PRIORITY_POS = Deployment Priority cycle,
///         MANAGE_APPEASE_POS = Appease Governor,
///         MANAGE_BARGAIN_POS = Bargain (only when governor is defiant).
///
/// Both `ui/policy.rs` (render_manage) and `state.rs` (handle_policy_confirm) use
/// these constants so the two sites stay in sync automatically.
pub const MANAGE_PRIORITY_POS: usize = POLICY_COUNT;
pub const MANAGE_APPEASE_POS: usize = MANAGE_PRIORITY_POS + 1;
pub const MANAGE_BARGAIN_POS: usize = MANAGE_APPEASE_POS + 1;

/// Policy display order — grouped by function, cheapest/earliest to most
/// expensive/latest within each group:
///
///   Detection:    Intel Station (11), Basic Screening (5), Antigen (6), Mass Rapid (7)
///   Containment:  Border Controls (3), Travel Ban (0), Quarantine (1)
///   Medical:      Water Sanitation (4), Hospital Surge (2), Field Hospital (10)
///   Extreme:      Nuclear Option (9), Martial Law (8)
///
/// This is the canonical display ordering — both the policy renderer and the confirm
/// handler use this to map display position → policy_idx.
pub fn policy_display_order() -> [usize; POLICY_COUNT] {
    [11, 5, 6, 7, 3, 0, 1, 4, 2, 10, 9, 8]
}

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
        6 => "Antigen Screening",
        7 => "Mass Rapid Screen",
        8 => "Martial Law",
        9 => "Nuclear Option",
        10 => "Field Hospital",
        11 => "Intel Station",
        _ => "Unknown Policy",
    }
}

impl RegionPolicy {
    /// Per-policy funding costs for each active policy. Returns (policy_idx, cost)
    /// pairs, trait-adjusted. Used by both `funding_cost()` and `tick_enforce_costs()`
    /// to ensure a single source of truth for policy pricing.
    /// Delegates to `bool_policy_cost()` for boolean policies to avoid duplication.
    pub fn active_policy_costs(&self, traits: &[RegionTrait]) -> Vec<(usize, f64)> {
        let mut costs = Vec::new();
        for idx in [0, 1, 2, 3, 4, POLICY_IDX_MARTIAL_LAW] {
            if self.get_bool(idx) {
                costs.push((idx, Self::bool_policy_cost(idx, traits)));
            }
        }
        let scr_cost = self.screening.funding_cost();
        if scr_cost > 0.0 { costs.push((POLICY_IDX_SCREENING_BASE, scr_cost)); }
        costs
    }

    /// Per-tick funding cost of a single boolean policy by index, trait-adjusted.
    /// Used by toggle_policy to display the cost when enabling a policy.
    pub fn bool_policy_cost(policy_idx: usize, traits: &[RegionTrait]) -> f64 {
        let trade_dependent = traits.contains(&RegionTrait::TradeDependent);
        match policy_idx {
            0 => if trade_dependent { TRAVEL_BAN_COST * TRADE_DEPENDENT_TRAVEL_BAN_MULT } else { TRAVEL_BAN_COST },
            1 => QUARANTINE_COST,
            2 => HOSPITAL_SURGE_COST,
            3 => BORDER_CONTROLS_COST,
            4 => WATER_SANITATION_COST,
            8 => MARTIAL_LAW_COST,
            _ => 0.0,
        }
    }

    /// Funding cost adjusted for regional traits.
    /// TradeDependent: travel ban costs 2x.
    /// Always pass the region's traits — use `&[]` only when no region context exists.
    pub fn funding_cost(&self, traits: &[RegionTrait]) -> f64 {
        self.active_policy_costs(traits).iter().map(|(_, c)| c).sum()
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

    /// Whether a policy (by index) is currently active. Handles both boolean
    /// policies and screening tiers.
    pub fn is_active(&self, policy_idx: usize) -> bool {
        match policy_idx {
            0..=4 | 8 | 9 => self.get_bool(policy_idx),
            5 => self.screening >= ScreeningLevel::Basic,
            6 => self.screening >= ScreeningLevel::Antigen,
            7 => self.screening >= ScreeningLevel::MassRapid,
            _ => false,
        }
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
    /// Suspend Regional Authority: freeze all governor loyalty. No drift, no
    /// defiance, no cooperation bonuses. Central command overrides local governance.
    #[serde(default)]
    pub suspend_regional_authority: bool,
    /// Fortify Region: restore one region's infrastructure to 100%, all others
    /// lose 25% across all systems.
    #[serde(default)]
    pub fortified_region: Option<usize>,
    /// Emergency Countermeasure: reduce all disease infectivity by 50% and
    /// cross-region spread by 75%. Kills 10% of alive population immediately.
    #[serde(default)]
    pub emergency_countermeasure: bool,
}

impl EnactedDecrees {
    pub fn is_enacted(&self, decree_idx: usize) -> bool {
        match decree_idx {
            0 => self.conscript_researchers,
            1 => self.authorize_human_trials,
            2 => self.sacrificed_region.is_some(),
            3 => self.suspend_regional_authority,
            4 => self.fortified_region.is_some(),
            5 => self.emergency_countermeasure,
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
        3 => "Suspend Regional Authority",
        4 => "Fortify Region",
        5 => "Emergency Countermeasure",
        _ => "Unknown Decree",
    }
}

/// A condition attached to a funding contract. Checked each tick.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum FundingCondition {
    /// A specific policy must NOT be active in any region.
    ForbidPolicy { policy_idx: usize },
    /// A specific policy must be active in at least one region.
    RequirePolicy { policy_idx: usize },
    /// Total global deaths must stay below this threshold.
    MaxDeaths { threshold: f64 },
    /// All regions must remain standing (revoked on first collapse).
    NoCollapse,
    /// A specific emergency decree must not be enacted while this contract is active.
    /// Once enacted, decrees are permanent. Violating this condition will gradually revoke the contract.
    ForbidDecree { decree_idx: usize },
}

impl FundingCondition {
    pub fn description(&self) -> String {
        match self {
            Self::ForbidPolicy { policy_idx } => {
                format!("Do not use {}", policy_display_name(*policy_idx))
            }
            Self::RequirePolicy { policy_idx } => {
                format!("Maintain {} in at least one region", policy_display_name(*policy_idx))
            }
            Self::MaxDeaths { threshold } => {
                format!("Global deaths below {}", format_large_number(*threshold))
            }
            Self::NoCollapse => "No region may collapse".to_string(),
            Self::ForbidDecree { decree_idx } => {
                format!("Do not enact {}", decree_display_name(*decree_idx))
            }
        }
    }

    /// Check whether this condition is currently satisfied.
    pub fn is_met(&self, state: &GameState) -> bool {
        match self {
            Self::ForbidPolicy { policy_idx } => {
                !state.policies.iter().any(|p| p.is_active(*policy_idx))
            }
            Self::RequirePolicy { policy_idx } => {
                state.policies.iter().any(|p| p.is_active(*policy_idx))
            }
            Self::MaxDeaths { threshold } => {
                state.total_dead() < *threshold
            }
            Self::NoCollapse => {
                !state.regions.iter().any(|r| r.collapsed)
            }
            Self::ForbidDecree { decree_idx } => {
                !state.enacted_decrees.is_enacted(*decree_idx)
            }
        }
    }
}

fn default_satisfaction() -> f64 {
    1.0
}

/// A funding contract: external income with strings attached.
/// Each contract is backed by a named patron NPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundingContract {
    pub name: String,
    /// Named NPC behind this contract (e.g., "Elena Vasquez, Logistics Magnate").
    #[serde(default)]
    pub patron: String,
    /// Per-tick income while contract is active.
    pub income: f64,
    pub condition: FundingCondition,
    /// Patron's personality-driven explanation for the deal.
    pub source: String,
    /// Unique template index (used to avoid duplicate offers).
    pub template_id: u8,
    /// Patron satisfaction (0.0–1.0). Degrades when condition violated, recovers when met.
    #[serde(default = "default_satisfaction")]
    pub satisfaction: f64,
    /// Whether the low-satisfaction warning has fired (resets when satisfaction recovers).
    #[serde(default)]
    pub warned: bool,
    /// Tick when last patron demand crisis was generated (cooldown tracking).
    #[serde(default)]
    pub last_demand_tick: u64,
}

/// Satisfaction thresholds and rates for the patron system.
pub const PATRON_SATISFACTION_WARN: f64 = 0.5;

// Emergency loan system — governors and corporations offer loans when the player is broke.
// If unpaid by the due date, the lender takes hostile action.

/// Who offered (and holds) an emergency loan.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum LoanLender {
    Governor { region_idx: usize },
    Corporation { corp_idx: usize },
}

/// An active emergency loan with outstanding balance and due date.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveLoan {
    pub lender_name: String,
    pub lender: LoanLender,
    /// Original amount borrowed.
    pub principal: f64,
    /// Current outstanding balance (accrues interest each day).
    pub outstanding: f64,
    /// Daily interest rate (e.g., 0.10 = 10% per day on outstanding).
    pub daily_interest_rate: f64,
    /// Game day when the loan is due.
    pub due_day: f64,
    /// Whether the hostile follow-up crisis has already been queued to prevent duplicates.
    pub hostile_queued: bool,
}

impl ActiveLoan {
    /// Per-tick interest amount to accrue.
    pub fn interest_per_tick(&self) -> f64 {
        self.outstanding * self.daily_interest_rate / TICKS_PER_DAY
    }
}

/// Per-day interest rate for loans from governors (slightly lower — political relationship).
pub const LOAN_GOVERNOR_INTEREST_RATE: f64 = 0.08;
/// Per-day interest rate for loans from corporations (higher — business terms).
pub const LOAN_CORP_INTEREST_RATE: f64 = 0.12;
/// Loan due after this many days.
pub const LOAN_DUE_DAYS: f64 = 10.0;
/// Don't offer a new loan if player already has this many outstanding loans.
pub const LOAN_MAX_SIMULTANEOUS: usize = 2;
/// Minimum ticks between loan offer crises.
pub const LOAN_OFFER_COOLDOWN: u64 = 240; // ~2 days
pub const PATRON_SATISFACTION_REVOKE: f64 = 0.2;
/// Per-tick degradation when condition is violated (~0.05/day = 16 days from 1.0 to revocation).
pub const PATRON_DEGRADE_RATE: f64 = 0.05 / 120.0;
/// Per-tick recovery when condition is met (~0.02/day).
pub const PATRON_RECOVER_RATE: f64 = 0.02 / 120.0;
/// Minimum ticks between patron demand crises from the same patron (~5 days).
pub const PATRON_DEMAND_COOLDOWN: u64 = 600;

// Corporation system — regional economic entities.
// Each region hosts 3 corporations from different sectors. Their financial health
// depends on workforce availability, infrastructure, and player policy choices.
// Corporate tax revenue is the primary source of player income.

/// Economic sector determining a corporation's sensitivities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorporationSector {
    /// Power generation, grid infrastructure. Hurt by infrastructure collapse.
    Energy,
    /// Shipping, autonomous freight. Hurt by travel restrictions.
    Logistics,
    /// Pharmaceuticals, genomics. Benefits from healthcare spending.
    Biotech,
    /// Resource extraction, materials. Workforce-dependent.
    Mining,
    /// Communications, data centers. Hurt by civil order collapse.
    DataInfra,
    /// Robotics, manufacturing, AI systems. Partially pandemic-resistant.
    Automation,
}

impl CorporationSector {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Energy => "Energy",
            Self::Logistics => "Logistics",
            Self::Biotech => "Biotech",
            Self::Mining => "Mining",
            Self::DataInfra => "Data/Infra",
            Self::Automation => "Automation",
        }
    }

    /// How much workforce loss (from infection/death) reduces revenue.
    /// 0.0 = immune to workforce loss, 1.0 = fully proportional.
    pub fn workforce_sensitivity(&self) -> f64 {
        match self {
            Self::Energy => 0.4,      // Automated, fewer workers needed
            Self::Logistics => 0.5,   // Partially automated freight
            Self::Biotech => 0.7,     // Skilled workers hard to replace
            Self::Mining => 0.8,      // Labor-intensive
            Self::DataInfra => 0.3,   // Mostly automated
            Self::Automation => 0.2,  // Robots don't get sick
        }
    }

    /// Revenue multiplier when travel ban is active in the region.
    pub fn travel_ban_factor(&self) -> f64 {
        match self {
            Self::Energy => 0.85,     // Local operations, mild impact
            Self::Logistics => 0.30,  // Devastating — core business is movement
            Self::Biotech => 0.70,    // Supply chain disruption
            Self::Mining => 0.60,     // Can't ship product
            Self::DataInfra => 0.90,  // Data travels on wires
            Self::Automation => 0.75, // Parts supply affected
        }
    }

    /// Revenue multiplier when quarantine is active in the region.
    pub fn quarantine_factor(&self) -> f64 {
        match self {
            Self::Energy => 0.90,     // Essential service, keeps running
            Self::Logistics => 0.60,  // Restricted movement hurts
            Self::Biotech => 0.85,    // Labs still operate
            Self::Mining => 0.70,     // Workers can't get to sites
            Self::DataInfra => 0.95,  // Remote operations fine
            Self::Automation => 0.85, // Factories still run
        }
    }

    /// Revenue multiplier when border controls are active.
    pub fn border_controls_factor(&self) -> f64 {
        match self {
            Self::Energy => 0.95,
            Self::Logistics => 0.70,  // International shipping slowed
            Self::Biotech => 0.90,
            Self::Mining => 0.80,     // Export friction
            Self::DataInfra => 0.95,
            Self::Automation => 0.85,
        }
    }

    /// Revenue multiplier when hospital surge is active (some sectors benefit).
    pub fn hospital_surge_factor(&self) -> f64 {
        match self {
            Self::Energy => 1.0,
            Self::Logistics => 1.05,  // Medical supply contracts
            Self::Biotech => 1.15,    // Increased demand for their products
            Self::Mining => 1.0,
            Self::DataInfra => 1.05,  // Health data infrastructure
            Self::Automation => 1.0,
        }
    }

    /// How much healthcare_capacity degradation affects this sector.
    /// Applied as: 1.0 - (1.0 - hc) * sensitivity
    pub fn healthcare_sensitivity(&self) -> f64 {
        match self {
            Self::Energy => 0.3,
            Self::Logistics => 0.4,
            Self::Biotech => 0.8,     // Depends on healthcare infrastructure
            Self::Mining => 0.5,
            Self::DataInfra => 0.2,
            Self::Automation => 0.2,
        }
    }

    /// How much supply_lines degradation affects this sector.
    pub fn supply_line_sensitivity(&self) -> f64 {
        match self {
            Self::Energy => 0.6,      // Needs fuel/parts
            Self::Logistics => 0.9,   // IS the supply line
            Self::Biotech => 0.5,     // Lab supplies
            Self::Mining => 0.7,      // Equipment, export routes
            Self::DataInfra => 0.3,   // Needs some physical infra
            Self::Automation => 0.5,  // Parts supply
        }
    }

    /// How much civil_order degradation affects this sector.
    pub fn civil_order_sensitivity(&self) -> f64 {
        match self {
            Self::Energy => 0.5,      // Infrastructure target
            Self::Logistics => 0.4,   // Route safety
            Self::Biotech => 0.3,     // Secure facilities
            Self::Mining => 0.6,      // Remote sites vulnerable
            Self::DataInfra => 0.7,   // Cables get cut, towers get looted
            Self::Automation => 0.4,  // Factories targeted
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Corporation {
    pub name: String,
    pub sector: CorporationSector,
    pub region_idx: usize,
    /// Revenue per day at full health (before condition modifiers).
    pub base_revenue: f64,
    /// Current revenue per day (after all modifiers). Updated each tick.
    pub revenue: f64,
    /// Fixed operating costs per day.
    pub operating_costs: f64,
    /// Cash reserves. Depleted when revenue < costs.
    pub reserves: f64,
    /// Starting reserves (for display as percentage).
    pub max_reserves: f64,
    /// Permanently failed. Revenue goes to 0, never recovers.
    pub bankrupt: bool,
    /// Tick when bankruptcy occurred.
    pub bankrupt_at_tick: Option<u64>,
    /// Whether this corporation's leader sits on the NWHO board.
    pub board_seat: bool,
}

impl Corporation {
    /// Current daily profit (revenue minus costs). Negative means burning reserves.
    pub fn daily_profit(&self) -> f64 {
        if self.bankrupt { return 0.0; }
        self.revenue - self.operating_costs
    }

    /// Reserves as a fraction of max (0.0 to 1.0).
    pub fn reserves_fraction(&self) -> f64 {
        if self.max_reserves <= 0.0 { return 0.0; }
        (self.reserves / self.max_reserves).clamp(0.0, 1.0)
    }

    /// Per-tick tax contribution to the player's income.
    pub fn tax_contribution(&self) -> f64 {
        if self.bankrupt { return 0.0; }
        self.revenue / TICKS_PER_DAY * CORPORATE_TAX_RATE
    }

    /// Days until bankruptcy at current burn rate. None if not burning reserves.
    pub fn days_of_runway(&self) -> Option<f64> {
        if self.bankrupt { return None; }
        let profit = self.daily_profit();
        if profit >= 0.0 { return None; }
        Some(self.reserves / (-profit))
    }
}

/// Fraction of corporate revenue collected as tax (player income).
/// Calibrated so total corporate tax ≈ old BASE_FUNDING_INCOME at full health.
pub const CORPORATE_TAX_RATE: f64 = 0.15;

/// Days of operating costs a corporation starts with in reserves.
/// At 30 days, a corp with zero revenue survives ~1 month.
pub const CORP_STARTING_RESERVE_DAYS: f64 = 30.0;

/// Operating costs as a fraction of base revenue.
/// At 0.65, corps need ~35% revenue to break even.
pub const CORP_COST_RATIO: f64 = 0.65;

fn format_large_number(n: f64) -> String {
    if n >= 1_000_000_000.0 {
        format!("{:.1}B", n / 1_000_000_000.0)
    } else if n >= 1_000_000.0 {
        format!("{:.0}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.0}K", n / 1_000.0)
    } else {
        format!("{:.0}", n)
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
    /// Fractional accumulator for POL-based personnel gains.
    #[serde(default)]
    pub personnel_accum: f64,
    /// Fractional accumulator for personnel attrition (when funding is $0).
    #[serde(default)]
    pub attrition_accum: f64,
    /// Tick when a FundingWarning event was last emitted. Prevents log spam.
    #[serde(default)]
    pub last_funding_warning_tick: u64,
    /// Tick when a loan offer crisis was last triggered. Rate-limits loan offers.
    #[serde(default)]
    pub last_loan_offer_tick: u64,
}


/// Deployment priority for a region. Controls auto-deploy targeting order
/// and is visible in the policy panel for strategic allocation decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionPriority {
    /// Auto-deploy serves this region before Normal/Low regions.
    High,
    /// Default priority.
    #[default]
    Normal,
    /// Auto-deploy serves this region after High/Normal regions.
    Low,
    /// Auto-deploy skips this region entirely. Manual deploy still works.
    CutOff,
}

impl RegionPriority {
    /// Numeric rank for sorting (lower = higher priority).
    pub fn rank(self) -> u8 {
        match self {
            Self::High => 0,
            Self::Normal => 1,
            Self::Low => 2,
            Self::CutOff => 3,
        }
    }

    /// Short label for UI display.
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "HIGH",
            Self::Normal => "NORMAL",
            Self::Low => "LOW",
            Self::CutOff => "CUT OFF",
        }
    }

    /// Cycle to next priority level.
    pub fn next(self) -> Self {
        match self {
            Self::High => Self::Normal,
            Self::Normal => Self::Low,
            Self::Low => Self::CutOff,
            Self::CutOff => Self::High,
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

/// Governor personality — character archetypes that determine how governors
/// behave when loyal vs defiant. Each type requires a different player response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernorPersonality {
    /// Breaks things by accident. Defiance: randomly deactivates a policy or
    /// wastes funding. Bargain: cheap (public praise) but loyalty decays fast.
    Buffoon,
    /// All noise. Defiance: small funding drain + alarming event messages.
    /// The real danger is wasting resources appeasing someone who'd shut up on their own.
    /// Bargain: small cost, large loyalty gain.
    #[serde(alias = "Populist")]
    Blowhard,
    /// Absent. Defiance: doesn't sabotage — just stops enforcing. Policy effects
    /// reduced in the region. Bargain: costs personnel (you send someone to manage).
    #[serde(alias = "Technocrat")]
    Recluse,
    /// Does too much. Defiance: unilaterally activates policies the player didn't set,
    /// costing unbudgeted personnel and funding. Bargain: give them authority (high cost).
    #[serde(alias = "Nationalist")]
    Hardliner,
    /// Competent and helpful, always skimming. When loyal, policies more effective.
    /// When defiant, continuous funding drain that grows over time.
    /// Bargain: permanent cut of regional income.
    #[serde(alias = "Cooperative")]
    Operative,
    /// Everything escalates. Defiance: periodic lump-sum demands that increase each time.
    /// Bargain: pure money, most expensive, gets worse over time.
    Mobster,
}

impl GovernorPersonality {
    pub fn label(&self) -> &'static str {
        match self {
            GovernorPersonality::Buffoon => "Buffoon",
            GovernorPersonality::Blowhard => "Blowhard",
            GovernorPersonality::Recluse => "Recluse",
            GovernorPersonality::Hardliner => "Hardliner",
            GovernorPersonality::Operative => "Operative",
            GovernorPersonality::Mobster => "Mobster",
        }
    }
}

/// A regional governor who reacts to player decisions.
/// Loyalty below 40 means defiance (policies less effective).
/// Loyalty above 80 means cooperation bonus (cheaper policies).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Governor {
    pub name: String,
    pub personality: GovernorPersonality,
    /// Loyalty 0-100. Starts at 60-80 depending on personality.
    pub loyalty: f64,
    /// Whether the defiance crisis has already fired for this governor.
    /// Reset when loyalty recovers above defiance threshold.
    #[serde(default)]
    pub defiance_crisis_fired: bool,
    /// Tick when the governor last took an autonomous defiance action.
    #[serde(default)]
    pub last_action_tick: u64,
    /// Mobster: how many times the player has bargained (escalates cost).
    #[serde(default)]
    pub bargain_count: u32,
    /// Operative: fraction of regional income being skimmed (accumulates with bargains).
    #[serde(default)]
    pub income_skim: f64,
}


/// Infection count thresholds for region severity levels.
/// Used by both the UI (status labels) and the engine (governor loyalty drift).
pub const SEVERITY_CRIT_THRESHOLD: f64 = 100_000.0;
pub const SEVERITY_HIGH_THRESHOLD: f64 = 10_000.0;
pub const SEVERITY_MOD_THRESHOLD: f64 = 1_000.0;

/// Loyalty threshold below which the governor becomes defiant.
pub const GOVERNOR_DEFIANCE_THRESHOLD: f64 = 40.0;
/// Loyalty threshold above which the governor provides cooperation bonuses.
pub const GOVERNOR_COOPERATION_THRESHOLD: f64 = 80.0;
/// Policy effectiveness multiplier when governor is defiant (most personalities).
pub const GOVERNOR_DEFIANCE_EFFECTIVENESS: f64 = 0.7;
/// Policy effectiveness for a defiant Recluse — worse than other personalities
/// because the governor has completely checked out.
pub const RECLUSE_DEFIANCE_EFFECTIVENESS: f64 = 0.4;
/// Policy cost multiplier when governor is cooperative.
pub const GOVERNOR_COOPERATION_COST_MULT: f64 = 0.8;
/// Cost to appease a governor.
pub const APPEASE_COST: f64 = 200.0;
/// Loyalty gain from appease action.
pub const APPEASE_LOYALTY_GAIN: f64 = 15.0;
/// Ticks between autonomous governor defiance actions (~2 days).
pub const GOVERNOR_ACTION_INTERVAL: u64 = 240;

/// Bargain loyalty gains by personality.
pub const BARGAIN_LOYALTY_GAIN: f64 = 20.0;
/// Blowhard bargain: large loyalty gain (they're easy to please).
pub const BARGAIN_BLOWHARD_LOYALTY_GAIN: f64 = 30.0;
/// Buffoon bargain cost: small POL cost (public praise).
pub const BARGAIN_BUFFOON_POL_COST: f64 = 0.05;
/// Blowhard bargain cost: small funding cost.
pub const BARGAIN_BLOWHARD_FUNDING_COST: f64 = 100.0;
/// Recluse bargain cost: personnel (you send someone to physically manage).
pub const BARGAIN_RECLUSE_PERSONNEL_COST: u32 = 2;
/// Hardliner bargain cost: high funding (give them authority).
pub const BARGAIN_HARDLINER_FUNDING_COST: f64 = 400.0;
/// Operative bargain cost: fraction of regional income permanently skimmed.
pub const BARGAIN_OPERATIVE_INCOME_CUT: f64 = 0.10;
/// Mobster bargain base cost: pure funding, escalates each time.
pub const BARGAIN_MOBSTER_BASE_COST: f64 = 200.0;

// --- Infrastructure constants ---

/// Which infrastructure system within a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InfraSystem {
    Healthcare,
    SupplyLines,
    CivilOrder,
}

impl InfraSystem {
    pub fn label(self) -> &'static str {
        match self {
            InfraSystem::Healthcare => "Healthcare",
            InfraSystem::SupplyLines => "Supply Lines",
            InfraSystem::CivilOrder => "Civil Order",
        }
    }

}

/// Infrastructure breakpoint: stressed. Effects start.
pub const INFRA_STRESSED: f64 = 0.50;
/// Infrastructure breakpoint: critical. Severe effects.
pub const INFRA_CRITICAL: f64 = 0.25;

/// How much infrastructure a completed field operations project restores (0.30 = 30%).
pub const FIELD_OPS_RESTORE: f64 = 0.30;

/// Healthcare: lethality multiplier when stressed (below 50%).
pub const HEALTHCARE_STRESSED_LETHALITY: f64 = 2.0;
/// Healthcare: lethality multiplier when critical (below 25%).
pub const HEALTHCARE_CRITICAL_LETHALITY: f64 = 4.0;

/// Supply lines: policy cost multiplier when stressed.
pub const SUPPLY_STRESSED_COST_MULT: f64 = 1.5;

/// Civil order: spread multiplier when at zero (anarchy).
pub const CIVIL_ORDER_ANARCHY_SPREAD: f64 = 1.5;

// --- Field Operations constants ---

/// Recon Mission: reveals pathogen type without full identification cost.
pub const OP_RECON_PERSONNEL: u32 = 2;
pub const OP_RECON_TICKS: f64 = 180.0; // 1.5 days
pub const OP_RECON_KNOWLEDGE: f64 = 0.25;

/// Emergency Response: temporary lethality reduction in a region.
pub const OP_EMERGENCY_PERSONNEL: u32 = 3;
pub const OP_EMERGENCY_TICKS: f64 = 120.0; // 1 day to deploy
pub const OP_EMERGENCY_EFFECT_TICKS: u64 = 360; // effect lasts 3 days
pub const OP_EMERGENCY_LETHALITY_MULT: f64 = 0.75; // 25% lethality reduction

/// Infrastructure Survey: free but slow infrastructure repair.
pub const OP_SURVEY_PERSONNEL: u32 = 2;
pub const OP_SURVEY_TICKS: f64 = 240.0; // 2 days
pub const OP_SURVEY_REPAIR: f64 = 0.15; // restores 15%

/// Supply Chain Reinforcement: funded investment to bolster supply lines in a region.
/// Restores supply lines AND adds permanent resilience (reduces degradation rate).
pub const OP_SUPPLY_PERSONNEL: u32 = 2;
pub const OP_SUPPLY_TICKS: f64 = 360.0; // 3 days
pub const OP_SUPPLY_COST: f64 = 800.0;
pub const OP_SUPPLY_RESTORE: f64 = 0.20; // restores 20%
pub const OP_SUPPLY_RESILIENCE: f64 = 0.25; // +25% degradation resistance per deployment

/// Civil Order Stabilization: funded operation to shore up civil order in a region.
/// Restores civil order AND adds permanent resilience (reduces degradation rate).
pub const OP_CIVIL_PERSONNEL: u32 = 1;
pub const OP_CIVIL_TICKS: f64 = 240.0; // 2 days
pub const OP_CIVIL_COST: f64 = 600.0;
pub const OP_CIVIL_RESTORE: f64 = 0.15; // restores 15%
pub const OP_CIVIL_RESILIENCE: f64 = 0.25; // +25% degradation resistance per deployment

/// Maximum resilience bonus from infrastructure investment (caps stacking).
pub const MAX_INFRA_RESILIENCE: f64 = 0.75; // 75% max degradation reduction

/// Evacuation Corridor: moves susceptible population out of a struggling region.
/// Evacuees travel to a destination region — bringing disease risk with them.
/// Reduces susceptible pool in source (slows future deaths), but may seed destination.
pub const OP_EVAC_PERSONNEL: u32 = 2;
pub const OP_EVAC_TICKS: f64 = 120.0; // 1 day to coordinate
pub const OP_EVAC_COST: f64 = 600.0;
/// Fraction of source susceptibles moved to destination.
pub const OP_EVAC_FRACTION: f64 = 0.10;
/// Each point of source infection rate adds this much to seeding probability (cap at 0.80).
pub const OP_EVAC_SEED_RATE_FACTOR: f64 = 3.0;

/// Number of deployable field operation types (Recon, Emergency, Survey, Supply, Civil, Evac).
/// Must equal the number of match arms in `handle_operations_confirm()` (lib.rs) and
/// the length of the `ops` array in `ui/operations.rs`. If you add a new op type,
/// update this constant — otherwise `panel_selection_max()` will make the new type unreachable.
pub const FIELD_OP_TYPE_COUNT: usize = 6;

/// What kind of field operation is being conducted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldOpKind {
    /// Partial identification of an unidentified pathogen.
    Recon { disease_idx: usize },
    /// Emergency medical response in a region.
    EmergencyResponse { region_idx: usize },
    /// Infrastructure repair via engineering survey.
    InfraSurvey { region_idx: usize },
    /// Bolster supply lines in a region (funded).
    SupplyChainReinforcement { region_idx: usize },
    /// Shore up civil order in a region (funded).
    CivilOrderStabilization { region_idx: usize },
    /// Move susceptible population from a struggling region to a safer one (funded).
    /// Reduces source susceptibles (fewer future deaths), but may seed destination.
    EvacuationCorridor { source_idx: usize, dest_idx: usize },
}

impl FieldOpKind {
    pub fn personnel(&self) -> u32 {
        match self {
            FieldOpKind::Recon { .. } => OP_RECON_PERSONNEL,
            FieldOpKind::EmergencyResponse { .. } => OP_EMERGENCY_PERSONNEL,
            FieldOpKind::InfraSurvey { .. } => OP_SURVEY_PERSONNEL,
            FieldOpKind::SupplyChainReinforcement { .. } => OP_SUPPLY_PERSONNEL,
            FieldOpKind::CivilOrderStabilization { .. } => OP_CIVIL_PERSONNEL,
            FieldOpKind::EvacuationCorridor { .. } => OP_EVAC_PERSONNEL,
        }
    }

    pub fn duration_ticks(&self) -> f64 {
        match self {
            FieldOpKind::Recon { .. } => OP_RECON_TICKS,
            FieldOpKind::EmergencyResponse { .. } => OP_EMERGENCY_TICKS,
            FieldOpKind::InfraSurvey { .. } => OP_SURVEY_TICKS,
            FieldOpKind::SupplyChainReinforcement { .. } => OP_SUPPLY_TICKS,
            FieldOpKind::CivilOrderStabilization { .. } => OP_CIVIL_TICKS,
            FieldOpKind::EvacuationCorridor { .. } => OP_EVAC_TICKS,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FieldOpKind::Recon { .. } => "Recon Mission",
            FieldOpKind::EmergencyResponse { .. } => "Emergency Response",
            FieldOpKind::InfraSurvey { .. } => "Infrastructure Survey",
            FieldOpKind::SupplyChainReinforcement { .. } => "Supply Reinforcement",
            FieldOpKind::CivilOrderStabilization { .. } => "Civil Stabilization",
            FieldOpKind::EvacuationCorridor { .. } => "Evacuation Corridor",
        }
    }

    /// Funding cost to start this operation, if any. Free ops return None.
    pub fn cost(&self) -> Option<f64> {
        match self {
            FieldOpKind::SupplyChainReinforcement { .. } => Some(OP_SUPPLY_COST),
            FieldOpKind::CivilOrderStabilization { .. } => Some(OP_CIVIL_COST),
            FieldOpKind::EvacuationCorridor { .. } => Some(OP_EVAC_COST),
            _ => None,
        }
    }
}

/// An active field operation in progress.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldOperation {
    pub kind: FieldOpKind,
    pub personnel: u32,
    pub ticks_remaining: f64,
    pub total_ticks: f64,
}

impl Governor {
    /// Returns true if this governor is defiant (loyalty below threshold).
    pub fn is_defiant(&self) -> bool {
        self.loyalty < GOVERNOR_DEFIANCE_THRESHOLD
    }

    /// Returns true if this governor provides cooperation bonuses.
    pub fn is_cooperative(&self) -> bool {
        self.loyalty >= GOVERNOR_COOPERATION_THRESHOLD
    }

    /// Policy effectiveness multiplier based on loyalty and personality.
    /// 1.0 = normal, 0.7 = defiant, 0.4 = defiant Recluse.
    pub fn policy_effectiveness(&self) -> f64 {
        if self.is_defiant() {
            if self.personality == GovernorPersonality::Recluse {
                RECLUSE_DEFIANCE_EFFECTIVENESS
            } else {
                GOVERNOR_DEFIANCE_EFFECTIVENESS
            }
        } else {
            1.0
        }
    }

    /// Policy cost multiplier based on loyalty.
    /// 1.0 = normal, 0.8 = cooperative.
    pub fn cost_multiplier(&self) -> f64 {
        if self.is_cooperative() {
            GOVERNOR_COOPERATION_COST_MULT
        } else {
            1.0
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub population: u64,
    pub connections: Vec<usize>,
    pub infections: Vec<RegionDiseaseState>,
    /// Regional governor who reacts to player decisions.
    #[serde(default = "default_governor")]
    pub governor: Governor,
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
    /// Cumulative deaths from post-collapse secondary causes (starvation, violence,
    /// lack of medical care). These are included in `dead` but not in any
    /// `RegionDiseaseState.dead`, making the gap visible to the player.
    #[serde(default)]
    pub collapse_deaths: f64,
    /// Field hospital level: 0 = none, 1 = Field Hospital (25% lethality reduction),
    /// 2 = Medical Center (40% lethality reduction + 25% medicine efficacy bonus).
    /// Destroyed (reset to 0) when region collapses.
    #[serde(default, alias = "healthcare_invested", deserialize_with = "deserialize_hospital_level")]
    pub hospital_level: u8,
    /// Intelligence station level: 0 = none, 1 = Intel Station (detects at 3k local infections),
    /// 2 = Advanced Intel (detects at 1k infections, generates briefings).
    /// Destroyed (reset to 0) when region collapses.
    #[serde(default)]
    pub intel_level: u8,
    /// Per-capita income multiplier. Higher values mean this region
    /// contributes more funding per person. Default 1.0.
    #[serde(default = "default_one")]
    pub income_modifier: f64,
    /// Lethality multiplier from baseline healthcare quality. Lower = better
    /// healthcare = fewer deaths. Stacks with hospital_level. Default 1.0.
    #[serde(default = "default_one")]
    pub healthcare_modifier: f64,
    /// Tick when medicine was last deployed to this region for each disease.
    /// Keyed by disease_idx. Per-disease cooldown lets the player treat multiple
    /// diseases in the same region without one blocking the others.
    #[serde(default)]
    pub last_deploy_tick: HashMap<usize, u64>,
    /// Healthcare capacity (0.0–1.0). Degrades as infection load grows.
    /// Below 0.5: lethality increases. Below 0.25: lethality increases more + research slows.
    /// At 0: maximum lethality, no field research possible here.
    /// Hospital Surge restores some capacity while active.
    #[serde(default = "default_one")]
    pub healthcare_capacity: f64,
    /// Supply line integrity (0.0–1.0). Degrades when death rate is high or travel banned.
    /// Below 0.5: policy costs increase 50%. Below 0.25: medicine deployment takes 2x.
    /// At 0: no medicine deployment, no new policies.
    #[serde(default = "default_one")]
    pub supply_lines: f64,
    /// Civil order (0.0–1.0). Degrades when deaths mount and unpopular policies are active.
    /// Below 0.5: screening effectiveness halved. Below 0.25: policies randomly deactivate.
    /// At 0: all policies disabled, spread rate +50%.
    #[serde(default = "default_one")]
    pub civil_order: f64,
    /// Deployment priority for auto-deploy targeting. High regions are served
    /// first, CutOff regions are skipped entirely.
    #[serde(default)]
    pub deploy_priority: RegionPriority,
    /// Permanent resilience bonus for supply lines (0.0-0.75). Reduces supply line
    /// degradation rate. Stacks from Supply Reinforcement operations.
    #[serde(default)]
    pub supply_resilience: f64,
    /// Permanent resilience bonus for civil order (0.0-0.75). Reduces civil order
    /// degradation rate. Stacks from Civil Stabilization operations.
    #[serde(default)]
    pub civil_resilience: f64,
    /// Tick at which an emergency response effect expires. While active,
    /// lethality in this region is reduced by OP_EMERGENCY_LETHALITY_MULT.
    #[serde(default)]
    pub emergency_response_until: Option<u64>,
    /// Tick until which this region suffers network disruption from a neighboring collapse.
    /// While active: +50% medicine deployment costs (see DISRUPTION_MEDICINE_COST_MULT).
    /// Multiple collapses extend the duration (last-collapse-wins on end tick).
    #[serde(default)]
    pub disrupted_until: Option<u64>,
    /// Estimated total infected (from detected diseases) visible to the player.
    /// This is a lagged estimate — not a simple multiplier of real values.
    /// Updated each tick by convergence toward reality; convergence rate depends
    /// on screening level and screening_progress. Creates genuine fog of war.
    #[serde(default)]
    pub estimated_infected: f64,
    /// Per-region systematic reporting bias, derived from the game seed at start.
    /// Range roughly [-0.3, 0.3]: positive = this region tends to over-report,
    /// negative = under-report. Suppressed at high screening levels (near zero at
    /// MassRapid). Makes unscreened data genuinely wrong, not just late.
    #[serde(default)]
    pub screening_noise_bias: f64,
    /// Cached recent death rate (deaths per day), updated once per day
    /// by the tick function. Used for the time-to-collapse estimate.
    #[serde(skip)]
    pub cached_deaths_per_day: f64,
    /// Death count at the previous rate-sampling point.
    #[serde(skip)]
    pub prev_dead: f64,
    /// Tick at which `prev_dead` was last sampled.
    #[serde(skip)]
    pub prev_dead_tick: u64,
}

fn default_governor() -> Governor {
    Governor {
        name: "Unknown".into(),
        personality: GovernorPersonality::Operative,
        loyalty: 70.0,
        defiance_crisis_fired: false,
        last_action_tick: 0,
        bargain_count: 0,
        income_skim: 0.0,
    }
}

fn default_one() -> f64 {
    1.0
}

fn default_one_u8() -> u8 {
    1
}

fn default_collapse_threshold() -> f64 {
    0.50
}

impl Region {
    pub fn has_trait(&self, t: RegionTrait) -> bool {
        self.traits.contains(&t)
    }

    /// True if this region is currently experiencing network disruption.
    pub fn is_disrupted(&self, current_tick: u64) -> bool {
        self.disrupted_until.map_or(false, |t| t > current_tick)
    }

    pub fn alive(&self) -> f64 {
        (self.population as f64 - self.total_dead()).max(0.0)
    }

    /// Policy effectiveness multiplier based on governor loyalty and personality.
    /// 1.0 when normal/cooperative, 0.7 when defiant, 0.4 when defiant Recluse.
    pub fn policy_effectiveness(&self) -> f64 {
        self.governor.policy_effectiveness()
    }

    /// Effective collapse threshold accounting for traits and martial law.
    /// The region collapses when alive < population * threshold.
    pub fn effective_collapse_threshold(&self, martial_law: bool) -> f64 {
        let mut threshold = self.collapse_threshold;
        if self.has_trait(RegionTrait::ResilientPopulation) {
            threshold -= 0.10;
        }
        if martial_law {
            threshold -= 0.15;
        }
        threshold.max(0.10)
    }

    /// Estimated days until this region collapses at the current death rate.
    /// Returns None if the death rate is negligible or the region has already collapsed.
    pub fn days_to_collapse(&self, martial_law: bool) -> Option<f64> {
        if self.collapsed || self.cached_deaths_per_day < 1.0 {
            return None;
        }
        let pop = self.population as f64;
        let collapse_dead = pop * (1.0 - self.effective_collapse_threshold(martial_law));
        let deaths_remaining = collapse_dead - self.total_dead();
        if deaths_remaining <= 0.0 {
            return Some(0.0);
        }
        Some(deaths_remaining / self.cached_deaths_per_day)
    }

    /// Remaining cooldown ticks before this region can receive a deployment
    /// targeting the given disease. Returns 0 if ready.
    pub fn deploy_cooldown_remaining(&self, current_tick: u64, disease_idx: usize) -> u64 {
        match self.last_deploy_tick.get(&disease_idx) {
            Some(&t) => {
                let elapsed = current_tick.saturating_sub(t);
                DEPLOY_COOLDOWN_TICKS.saturating_sub(elapsed)
            }
            None => 0,
        }
    }

    /// Fraction of shipped doses that are effectively delivered and administered.
    /// Supply lines determine how many doses physically arrive (logistics).
    /// Healthcare capacity determines how many arriving doses can be administered (staff/facilities).
    /// These are independent sequential bottlenecks, so they multiply.
    pub fn delivery_efficiency(&self) -> f64 {
        self.supply_lines * self.healthcare_capacity
    }

    /// True if ANY disease in this region has an active deploy cooldown.
    pub fn any_deploy_cooldown(&self, current_tick: u64) -> bool {
        self.last_deploy_tick.values().any(|&t| {
            let elapsed = current_tick.saturating_sub(t);
            DEPLOY_COOLDOWN_TICKS.saturating_sub(elapsed) > 0
        })
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

    /// Screened infection count: the player's estimated total infected for this region.
    /// This is a lagged, convergence-based estimate — NOT a simple multiplier.
    /// The estimate is maintained by tick_screening() in the engine.
    pub fn screened_infected(&self) -> f64 {
        self.estimated_infected
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

    /// Stat ranges tuned so total collapse occurs by day 90 without intervention.
    /// HARD REQUIREMENT: every seed must lose by day 90 with no player action.
    /// The enforcing test is `game_is_lost_within_90_days_without_intervention`.
    /// If it starts failing, increase infectivity — do NOT relax the test.
    ///
    /// Design principle: long infectious period (16–30 days) with near-zero natural
    /// recovery. This lets the epidemic sweep through each region's full population
    /// before burning out. A short infectious period (high per-tick lethality+recovery)
    /// causes epidemic burnout after infecting only a small fraction of the population.
    /// Do NOT copy the naive approach of increasing per-tick lethality to "speed up"
    /// deaths — it makes the overall death toll LOWER by shortening infectious period.
    ///
    /// IFR = lethality / (lethality + recovery) ≈ 85–95% across all types.
    /// R0 = infectivity / (lethality + recovery) ≈ 6–15 depending on type.
    /// Attack rate ≈ 99%+ given high R0. Total deaths ≈ 85–95% of population.
    fn stat_ranges(&self) -> DiseaseStatRanges {
        match self {
            // RNA viruses: fast spreader, high lethality.
            // Base doubling time ~1.3 days (unscaled). Scaling accelerates later.
            PathogenType::RnaVirus => DiseaseStatRanges {
                infectivity: (0.004, 0.007),
                lethality: (0.00040, 0.00070),
                recovery: (0.00006, 0.00015),
                cross_region: (0.004, 0.008),
            },
            // DNA viruses: moderate spread, high lethality.
            PathogenType::DnaVirus => DiseaseStatRanges {
                infectivity: (0.003, 0.006),
                lethality: (0.00035, 0.00065),
                recovery: (0.00004, 0.00010),
                cross_region: (0.003, 0.007),
            },
            // Bacteria: broad reach, moderate lethality, some recovery.
            PathogenType::Bacterium => DiseaseStatRanges {
                infectivity: (0.003, 0.005),
                lethality: (0.00035, 0.00060),
                recovery: (0.00010, 0.00020),
                cross_region: (0.003, 0.006),
            },
            // Fungi: slower growth, high lethality, almost no natural recovery.
            PathogenType::Fungus => DiseaseStatRanges {
                infectivity: (0.002, 0.004),
                lethality: (0.00030, 0.00055),
                recovery: (0.00005, 0.00015),
                cross_region: (0.002, 0.005),
            },
            // Prions: slowest spread, near-certain death once infected.
            PathogenType::Prion => DiseaseStatRanges {
                infectivity: (0.002, 0.005),
                lethality: (0.00045, 0.00090),
                recovery: (0.00003, 0.00006),
                cross_region: (0.002, 0.004),
            },
        }
    }

    /// Name pools for procedural disease name generation.
    /// Generate a procedurally constructed name for a pathogen of this type.
    /// Uses a single u64 split across component indices, giving hundreds of unique
    /// combinations per pathogen type without consuming extra RNG calls.
    /// Each pathogen type has a distinct naming convention.
    pub fn generate_name(&self, seed: u64) -> String {
        // Split the 64-bit seed into independent parts for each component.
        // High 32 bits → first component, low 32 bits → second component,
        // low 16 bits → optional third component (prions only).
        let hi = (seed >> 32) as usize;
        let lo = (seed & 0xFFFF_FFFF) as usize;
        let lo16 = (seed & 0xFFFF) as usize;
        match self {
            PathogenType::RnaVirus => {
                const FAMILIES: &[&str] = &[
                    "Corvid", "Nipah", "Marburg", "Lassa", "Hanta", "Dengue", "MERS",
                    "RSV", "CCHF", "Ebola", "Metapneumo", "Enterovirus", "Rotavirus",
                    "Norovirus", "West Nile", "Chikungunya", "Zika", "Rift Valley",
                    "Mayaro", "Oropouche", "Junin", "Machupo", "Sabia", "Guanarito",
                ];
                const STRAINS: &[&str] = &[
                    "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta",
                    "Theta", "Iota", "Kappa", "Lambda", "Mu", "Sigma", "Tau", "Phi",
                    "Chi", "Psi", "Omega", "Rho", "X7", "R5", "N9", "C3", "T4",
                ];
                format!("{}-{}", FAMILIES[hi % FAMILIES.len()], STRAINS[lo % STRAINS.len()])
            }
            PathogenType::DnaVirus => {
                const FAMILIES: &[&str] = &[
                    "Variola", "Monkeypox", "Adeno", "Herpes", "Papilloma", "Mimivirus",
                    "Cytomegalo", "Parvovirus", "Kaposi", "Molluscum", "Orthopox",
                    "Iridovirus", "Polyomavirus", "Bocavirus", "Gyrovirus",
                    "Circovirus", "Torque-Teno", "Anellovirus", "Amdoparvovirus", "Densovirus",
                ];
                const CODES: &[&str] = &[
                    "X47", "R3", "N9", "T7", "A4", "B6", "D2", "E8", "F1", "G5",
                    "H3", "K9", "M7", "P4", "Q2", "S6", "T3", "V8", "W5", "Z1",
                ];
                format!("{}-{}", FAMILIES[hi % FAMILIES.len()], CODES[lo % CODES.len()])
            }
            PathogenType::Bacterium => {
                const GENERA: &[&str] = &[
                    "Yersinia", "Vibrio", "Mycobacterium", "Burkholderia", "Clostridium",
                    "Rickettsia", "Streptococcus", "Klebsiella", "Pseudomonas", "Acinetobacter",
                    "Enterococcus", "Salmonella", "Shigella", "Listeria", "Brucella",
                    "Legionella", "Francisella", "Helicobacter", "Campylobacter", "Borrelia",
                    "Treponema", "Bartonella", "Coxiella", "Ehrlichia", "Leptospira",
                ];
                const DESIGNATORS: &[&str] = &[
                    "Omega", "Sigma", "Alpha", "Phi", "Delta", "Beta", "Gamma",
                    "Tau", "Rho", "Psi", "Zeta", "Epsilon", "Kappa", "Lambda",
                    "Mu", "Nu", "Xi", "Pi", "Upsilon", "Chi",
                ];
                format!("{}-{}", GENERA[hi % GENERA.len()], DESIGNATORS[lo % DESIGNATORS.len()])
            }
            PathogenType::Fungus => {
                const GENERA: &[&str] = &[
                    "Candida", "Aspergillus", "Cryptococcus", "Mucor", "Trichophyton",
                    "Coccidioides", "Histoplasma", "Fusarium", "Sporothrix", "Paracoccidioides",
                    "Blastomyces", "Pneumocystis", "Alternaria", "Exserohilum", "Scedosporium",
                    "Saksenaea", "Cunninghamella", "Rhizopus", "Lomentospora", "Apophysomyces",
                ];
                const MODIFIERS: &[&str] = &[
                    "Omega", "Rex", "Nova", "Sigma", "Alpha", "Beta", "Delta", "Phi",
                    "Tau", "Rho", "Prime", "Variant-C", "Auris", "Tropicalis", "Glabrata",
                ];
                format!("{} {}", GENERA[hi % GENERA.len()], MODIFIERS[lo % MODIFIERS.len()])
            }
            PathogenType::Prion => {
                const TYPES: &[&str] = &[
                    "PrP", "TSE", "CJD", "BSE", "GSS", "FFI", "CWD", "Scrapie",
                    "sCJD", "vCJD", "gCJD", "fCJD", "sMM", "sMV",
                ];
                const DESIGNATORS: &[&str] = &[
                    "Sigma", "Tau", "Delta", "Omega", "Alpha", "Beta",
                    "Epsilon", "Rho", "Xi", "Psi", "Lambda", "Kappa",
                ];
                const NOUNS: &[&str] = &[
                    "Fold", "Variant", "Strain", "Form", "Type", "Isoform",
                ];
                format!("{}-{} {}",
                    TYPES[hi % TYPES.len()],
                    DESIGNATORS[lo % DESIGNATORS.len()],
                    NOUNS[lo16 % NOUNS.len()])
            }
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
            TransmissionVector::Airborne => 0.55,   // standard: 45% infectivity reduction
            TransmissionVector::Waterborne => 0.80,  // weak: only 20% reduction
            TransmissionVector::Contact => 0.35,     // strong: 65% reduction
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

/// Controls how a disease mutates over time.
/// Most diseases follow a normal random walk; late-game engineered pathogens
/// may show anomalous patterns that a careful player can detect from the data.
/// No UI commentary — the anomaly is only visible in the numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationMode {
    /// Standard ±10% random walk on infectivity and lethality.
    #[default]
    Normal,
    /// Disease does not mutate. Strain generation stays fixed.
    Locked,
    /// Lethality increases on every mutation; infectivity stays fixed.
    DirectedLethality,
    /// Infectivity increases on every mutation; lethality stays fixed.
    DirectedInfectivity,
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
    /// Undetected diseases spread silently — the player sees "Unknown pathogen (undetected)" in the threats panel.
    #[serde(default = "default_true")]
    pub detected: bool,
    /// Tick at which this disease was spawned. Used to compute silent spread duration at detection.
    #[serde(default)]
    pub spawned_at_tick: u64,
    /// Per-mechanism resistance levels. When a medicine with mechanism X is deployed
    /// against this disease, resistance to mechanism X increases — affecting ALL drugs
    /// sharing that mechanism. Broad-spectrum drugs (mechanism=None) track separately.
    #[serde(default)]
    pub mechanism_resistance: Vec<ResistanceEntry>,
    /// How much this disease has adapted to containment measures (quarantine, travel bans).
    /// 0.0 = no adaptation, 1.0 = fully adapted (containment half as effective).
    /// Builds when disease has active infections in contained regions; decays when
    /// containment is lifted. Creates pressure to rotate strategies rather than
    /// relying on quarantine forever.
    #[serde(default)]
    pub containment_adaptation: f64,
    /// Mutation behavior pattern. Normal diseases random-walk; anomalous late-game
    /// pathogens may be locked (no mutation) or directed (one-way drift).
    #[serde(default)]
    pub mutation_mode: MutationMode,
    /// Wave origin marker. Diseases that emerge in the same coordinated wave (post
    /// day 24 wave clustering) share a sequence_group ID. None = naturally independent.
    /// Visible in the Threats panel when Rapid Sequencing is unlocked and knowledge >= 0.66.
    #[serde(default)]
    pub sequence_group: Option<u32>,
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
    /// Locked diseases always return 0.0 regardless of sequencing.
    pub fn effective_mutation_rate(&self) -> f64 {
        if self.mutation_mode == MutationMode::Locked {
            return 0.0;
        }
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
        // Generate name seed FIRST — consumes exactly 1 RNG call, same position as the
        // old name_pool() index draw. Stats use subsequent RNG values, preserving alignment.
        let name_seed: u64 = rng.r#gen();

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

        let transmission = pathogen_type.random_transmission(rng);
        let infectivity = stat(rng, ranges.infectivity, toughness_bias);
        let lethality = stat(rng, ranges.lethality, toughness_bias);
        let cross_region_spread = range_val(rng, ranges.cross_region);
        let recovery_rate = range_val(rng, ranges.recovery);

        // Use pre-generated seed to build name — no additional RNG calls.
        // Combine seed with attempt index for variation across retries.
        // Combination space is ~300–1000 per pathogen type, so collisions are rare.
        let name = (0u64..20)
            .map(|i| pathogen_type.generate_name(name_seed.wrapping_add(i)))
            .find(|n| !used_names.contains(n))
            .unwrap_or_else(|| format!("Pathogen-{}", used_names.len() + 1));

        Disease {
            name,
            pathogen_type,
            transmission,
            infectivity,
            lethality,
            cross_region_spread,
            recovery_rate,
            knowledge: 0.0,
            strain_generation: 0,
            sequencing_count: 0,
            detected: true, // callers override to false for new diseases
            spawned_at_tick: 0, // callers override to current tick when spawning
            mechanism_resistance: vec![],
            containment_adaptation: 0.0,
            mutation_mode: MutationMode::Normal,
            sequence_group: None,
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
/// 60 ticks/day = each tick represents ~24 minutes of game time.
pub const TICKS_PER_DAY: f64 = 60.0;
/// Personnel added per completed TrainPersonnel project.
pub const TRAIN_PERSONNEL_BATCH: u32 = 5;
/// Deploy cooldown per disease per region in ticks (half a day).
/// Per-disease cooldown means treating disease A doesn't block treating disease B.
pub const DEPLOY_COOLDOWN_TICKS: u64 = (TICKS_PER_DAY / 2.0) as u64;
/// Shipping delay in ticks (half a day). Medicine effects apply on delivery.
pub const SHIPPING_TICKS: u64 = (TICKS_PER_DAY / 2.0) as u64;

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
            // Broad-spectrum: weak efficacy against everything except prions.
            // A blunt bandaid — slows disease but can't stop it. Forces research investment.
            (TherapyType::BroadSpectrum, PathogenType::Prion) => 0.05,
            (TherapyType::BroadSpectrum, _) => 0.15,
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
#[derive(Clone, Copy, Debug, Hash, Serialize, Deserialize, PartialEq, Eq)]
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
        // Scale from 600M (mult=0.6) to 1.4B (mult=1.8)
        (600_000_000.0 + 800_000_000.0 * (mult - 0.6) / 1.2).round()
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
}

//// A medicine deployment in transit to a region. Created when the player
/// dispatches doses; takes effect when `arrive_tick` is reached.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shipment {
    pub medicine_idx: usize,
    pub region_idx: usize,
    pub target: DeployTarget,
    /// Physical doses on the truck (deducted from inventory at dispatch).
    pub doses: f64,
    /// Funding cost already paid at dispatch.
    #[serde(default)]
    pub cost: f64,
    /// Tick when this shipment next attempts delivery.
    pub arrive_tick: u64,
}

// What a medicine deployment targets: protect susceptible (preventive) or treat infected (therapeutic).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployTarget {
    Vaccinate { disease_idx: usize },
    Treat { disease_idx: usize },
}

impl Medicine {
    /// Deployment cost based on the medicine's base cost. Deployment is
    /// limited primarily by doses and cooldown, not funding. This keeps the
    /// core gameplay loop (research -> deploy) affordable.
    pub fn deploy_cost(&self) -> f64 {
        self.cost
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
                    doses: 0.0,
                    max_doses: 1_000_000_000.0,
                    unlocked: false,
                    tested_against: vec![],
                    strain_generations: vec![],
                    deployed_count: 0,
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
                doses: 0.0,
                max_doses: doses,
                unlocked: false,
                tested_against: vec![],
                strain_generations: vec![],
                deployed_count: 0,
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

}

/// Which research track a project belongs to.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResearchTrack {
    Field,
    #[serde(alias = "Bench")]
    Applied,
    Basic,
}

/// Number of research track categories (Field, Applied, Basic).
/// The "Upgrade Lab" item in BrowseCategories is always rendered AFTER these tracks
/// at index `RESEARCH_TRACK_COUNT`. Both `panel_selection_max()` (state.rs) and
/// the BrowseCategories renderer (ui/research.rs) use this constant — if you add a
/// fourth track, update this constant and both dependent sites together.
pub const RESEARCH_TRACK_COUNT: usize = 3;

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
            ResearchKind::SuppressPathogen { disease_idx: d } => *d == disease_idx,
            ResearchKind::AttenuatePathogen { disease_idx: d } => *d == disease_idx,
            ResearchKind::InterdictPathogen { disease_idx: d } => *d == disease_idx,
            ResearchKind::DevelopMedicine { .. }
            | ResearchKind::ManufactureDoses { .. }
            | ResearchKind::TrainPersonnel
            | ResearchKind::BasicResearch { .. }
            | ResearchKind::FieldOperations { .. } => false,
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
    /// Pathogen suppression — permanently reduces a disease's infectivity by ~20%.
    /// Requires the PathogenSuppression basic tech to be unlocked.
    SuppressPathogen { disease_idx: usize },
    /// Directed attenuation — permanently reduces a disease's lethality by ~30%.
    /// In-situ modification of pathogen virulence factors.
    /// Requires the DirectedAttenuation basic tech to be unlocked.
    AttenuatePathogen { disease_idx: usize },
    /// Genomic interdiction — permanently eliminates a disease's cross-region spread.
    /// Disrupts pathogen transmission mechanisms at the genomic level.
    /// Requires the GenomicInterdiction basic tech to be unlocked.
    InterdictPathogen { disease_idx: usize },
    /// Field operations — send a team to stabilize degraded infrastructure in a region.
    /// Appears when any infrastructure system drops below INFRA_STRESSED (50%).
    /// Creates a mid-game phase shift: field research slots compete between disease
    /// work and keeping regions operational.
    FieldOperations { region_idx: usize, system: InfraSystem },
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
    /// All field research (IdentifyThreat, ClinicalTrial, FieldOperations) completes 25% faster.
    /// Prereq: RapidSequencing (logical progression: fast sequencing → predictive field response).
    PredictiveSurveillance,
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
    /// Unlocks pathogen suppression field research: permanently reduce
    /// a disease's infectivity by modifying its evolutionary trajectory.
    /// Prereq: VaccinePlatform + CombinationTherapy.
    PathogenSuppression,
    /// Unlocks directed attenuation field research: permanently reduce
    /// a disease's lethality by modifying its virulence factors in situ.
    /// Prereq: PathogenSuppression.
    DirectedAttenuation,
    /// Unlocks genomic interdiction field research: permanently eliminate
    /// a disease's ability to spread between regions.
    /// Prereq: DirectedAttenuation.
    GenomicInterdiction,
    /// Reduces ManufactureDoses applied research duration by 35%.
    /// Prereq: at least one targeted medicine developed (mechanism.is_some() && unlocked).
    /// Note: when a Biotech corp is healthy, bonus could be increased to 50% (#1381).
    AutomatedSynthesis,
    /// Each ManufactureDoses run produces 25% more doses. Stacks multiplicatively
    /// with Europe's manufacturing yield bonus.
    /// Prereq: AutomatedSynthesis.
    DistributedStorage,
}

impl BasicTech {
    /// Human-readable name for display.
    pub fn name(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Targeted Drug Design",
            BasicTech::MonoclonalAntibodies => "Monoclonal Antibodies",
            BasicTech::PhageTherapy => "Phage Therapy",
            BasicTech::RapidSequencing => "Rapid Sequencing",
            BasicTech::PredictiveSurveillance => "Predictive Surveillance",
            BasicTech::VaccinePlatform => "Vaccine Platform",
            BasicTech::ResistanceSurveillance => "Resistance Surveillance",
            BasicTech::CombinationTherapy => "Combination Therapy",
            BasicTech::PathogenSuppression => "Pathogen Suppression",
            BasicTech::DirectedAttenuation => "Directed Attenuation",
            BasicTech::GenomicInterdiction => "Genomic Interdiction",
            BasicTech::AutomatedSynthesis => "Automated Synthesis",
            BasicTech::DistributedStorage => "Distributed Storage",
        }
    }

    /// Short description for the research panel.
    pub fn description(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Targeted antiviral and antibiotic development for identified pathogen classes.",
            BasicTech::MonoclonalAntibodies => "Engineered antibody therapies with high efficacy against identified viral strains.",
            BasicTech::PhageTherapy => "Bacteriophage-based treatment for bacterial pathogens. Low resistance development.",
            BasicTech::RapidSequencing => "Cuts sequencing time in half. Reveals mutation drift rate and history.",
            BasicTech::PredictiveSurveillance => "Integrated genomic surveillance network. Field identification and clinical trials 25% faster.",
            BasicTech::VaccinePlatform => "Triples effectiveness of preventive vaccination programs.",
            BasicTech::ResistanceSurveillance => "Tracks resistance levels and trends across all deployed medicines.",
            BasicTech::CombinationTherapy => "Multi-drug protocols reduce resistance accumulation from deployments by 50%.",
            BasicTech::PathogenSuppression => "Field research to suppress pathogen spread. Each project reduces infectivity ~20%.",
            BasicTech::DirectedAttenuation => "In-situ modification of pathogen virulence factors. Each project permanently reduces target lethality.",
            BasicTech::GenomicInterdiction => "Disrupt pathogen transmission mechanisms at the genomic level. Eliminates cross-region spread.",
            BasicTech::AutomatedSynthesis => "Standardized bioreactor protocols cut production cycle time by 35%.",
            BasicTech::DistributedStorage => "Distributed cold storage increases yield per manufacturing run by 25%.",
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
            BasicTech::PredictiveSurveillance => {
                state.unlocked_techs.contains(&BasicTech::RapidSequencing)
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
            BasicTech::PathogenSuppression => {
                state.unlocked_techs.contains(&BasicTech::VaccinePlatform)
                    && state.unlocked_techs.contains(&BasicTech::CombinationTherapy)
            }
            BasicTech::DirectedAttenuation => {
                state.unlocked_techs.contains(&BasicTech::PathogenSuppression)
            }
            BasicTech::GenomicInterdiction => {
                state.unlocked_techs.contains(&BasicTech::DirectedAttenuation)
            }
            BasicTech::AutomatedSynthesis => {
                // Prereq: at least one targeted medicine developed (not broad-spectrum)
                state.medicines.iter().any(|m| m.mechanism.is_some() && m.unlocked)
            }
            BasicTech::DistributedStorage => {
                state.unlocked_techs.contains(&BasicTech::AutomatedSynthesis)
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
            BasicTech::PredictiveSurveillance => "Rapid Sequencing",
            BasicTech::VaccinePlatform => "Monoclonal Antibodies or Phage Therapy",
            BasicTech::ResistanceSurveillance => "Rapid Sequencing",
            BasicTech::CombinationTherapy => "Deploy 2+ different medicines",
            BasicTech::PathogenSuppression => "Vaccine Platform + Combination Therapy",
            BasicTech::DirectedAttenuation => "Pathogen Suppression",
            BasicTech::GenomicInterdiction => "Directed Attenuation",
            BasicTech::AutomatedSynthesis => "Develop any targeted medicine",
            BasicTech::DistributedStorage => "Automated Synthesis",
        }
    }

    /// All techs in display order.
    pub fn all() -> &'static [BasicTech] {
        &[
            BasicTech::TargetedDrugDesign,
            BasicTech::MonoclonalAntibodies,
            BasicTech::PhageTherapy,
            BasicTech::RapidSequencing,
            BasicTech::PredictiveSurveillance,
            BasicTech::VaccinePlatform,
            BasicTech::ResistanceSurveillance,
            BasicTech::CombinationTherapy,
            BasicTech::PathogenSuppression,
            BasicTech::DirectedAttenuation,
            BasicTech::GenomicInterdiction,
            BasicTech::AutomatedSynthesis,
            BasicTech::DistributedStorage,
        ]
    }
}

impl ResearchKind {
    /// Project costs: (personnel, duration_ticks, funding).
    ///
    /// DevelopMedicine costs depend on mechanism of action: each mechanism has
    /// a dev_cost_multiplier that scales base costs (3 personnel, 200 ticks, $500).
    /// Broad-spectrum (multi-target, no mechanism) uses fixed high costs.
    /// These are BASE costs. Tech modifiers (RapidSequencing, PredictiveSurveillance) are
    /// applied in GameState::effective_costs(), not here.
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
                BasicTech::PredictiveSurveillance => (4, 280.0, 650.0),
                BasicTech::VaccinePlatform => (6, 360.0, 1000.0),
                BasicTech::ResistanceSurveillance => (3, 200.0, 500.0),
                BasicTech::CombinationTherapy => (4, 300.0, 800.0),
                BasicTech::PathogenSuppression => (8, 480.0, 1200.0),
                BasicTech::DirectedAttenuation => (10, 600.0, 1500.0),
                BasicTech::GenomicInterdiction => (12, 720.0, 2000.0),
                BasicTech::AutomatedSynthesis => (4, 200.0, 500.0),
                BasicTech::DistributedStorage => (5, 280.0, 700.0),
            },
            ResearchKind::FieldOperations { .. } => (3, 240.0, 300.0),
            ResearchKind::SuppressPathogen { .. } => (8, 600.0, 500.0),
            ResearchKind::AttenuatePathogen { .. } => (8, 600.0, 800.0),
            ResearchKind::InterdictPathogen { .. } => (10, 800.0, 1200.0),
        }
    }

    /// Short display label for a research project (used in the research panel).
    pub fn display_label(&self, diseases: &[Disease], medicines: &[Medicine], regions: &[Region]) -> String {
        match self {
            ResearchKind::IdentifyThreat { disease_idx } => {
                let disease = diseases.get(*disease_idx);
                let name = disease
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                let verb = if disease.is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME) {
                    "Study"
                } else {
                    "Identify"
                };
                format!("{}: {}", verb, name)
            }
            ResearchKind::DevelopMedicine { medicine_idx } => {
                let name = medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Unknown");
                format!("Develop: {}", name)
            }
            ResearchKind::ClinicalTrial { medicine_idx, disease_idx } => {
                let med = medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Unknown");
                let dis = diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Trial: {} vs {}", med, dis)
            }
            ResearchKind::ManufactureDoses { medicine_idx } => {
                let name = medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Unknown");
                format!("Manufacture: {}", name)
            }
            ResearchKind::GenomicSequencing { disease_idx } => {
                let name = diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Sequence: {}", name)
            }
            ResearchKind::TrainPersonnel => format!("Train Personnel (+{})", TRAIN_PERSONNEL_BATCH),
            ResearchKind::BasicResearch { tech } => tech.name().to_string(),
            ResearchKind::SuppressPathogen { disease_idx } => {
                let name = diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Suppress: {}", name)
            }
            ResearchKind::AttenuatePathogen { disease_idx } => {
                let name = diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Attenuate: {}", name)
            }
            ResearchKind::InterdictPathogen { disease_idx } => {
                let name = diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Interdict: {}", name)
            }
            ResearchKind::FieldOperations { region_idx, system } => {
                let region_name = regions.get(*region_idx)
                    .map(|r| r.name.as_str())
                    .unwrap_or("Unknown");
                format!("Stabilize {}: {}", system.label(), region_name)
            }
        }
    }
}

impl ResearchProject {
    /// Create a ResearchProject for tests.
    #[cfg(test)]
    pub fn test(kind: ResearchKind, required_ticks: f64, personnel_assigned: u32) -> Self {
        Self {
            kind,
            progress: 0.0,
            required_ticks,
            personnel_assigned,
        }
    }

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
        /// Days the disease was spreading silently before detection.
        #[serde(default)]
        silent_days: f64,
    },
    /// An intelligence briefing from an Advanced Intel station.
    IntelBriefing {
        message: String,
    },
    /// A disease spread to a new region via cross-region transmission.
    DiseaseSpreadToRegion {
        disease_idx: usize,
        region_idx: usize,
    },
    /// A region's society has collapsed — too many deaths.
    RegionCollapsed {
        region_idx: usize,
        /// Number of personnel lost due to the collapse (0 if none were available).
        personnel_lost: u32,
    },
    /// Post-collapse secondary deaths (starvation, violence, infrastructure breakdown).
    /// Fires once per day per collapsed region so the event log isn't spammed.
    CollapseSecondaryDeaths {
        region_idx: usize,
        deaths: f64,
    },
    /// A non-collapsed region is now suffering network disruption from a neighboring collapse.
    /// Policy costs +30%, medicine deployment costs +50% for 10 days.
    NetworkDisruption {
        disrupted_region_idx: usize,
        collapsed_region_idx: usize,
    },
    /// The game just ended (defeat). UI should pause and close panels.
    /// The actual outcome is on `GameState::outcome`; this just signals the transition.
    /// A new funding contract offer is available.
    ContractOffered { name: String },
    /// A patron is unhappy — satisfaction dropped to warning level.
    ContractWarning { patron: String, reason: String },
    /// A contract was revoked because patron satisfaction bottomed out.
    ContractRevoked { name: String, reason: String },
    /// A corporation went bankrupt (permanent).
    CorporationBankrupt { corp_idx: usize, region_idx: usize },
    GameOver,
    /// A crisis event appeared and needs player attention.
    CrisisStarted,
    /// A crisis was auto-resolved based on player's saved preference.
    /// Carries the resolution outcome message for the event log.
    CrisisAutoResolved { message: String },
    /// A research project was auto-started because auto-research is on.
    ResearchAutoStarted { track: ResearchTrack },
    /// A research completion on one track unlocked a project on another track.
    /// Notifies the player to start the next pipeline step manually.
    ResearchHandoff { message: String },
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
    /// A pathogen was identified through field research — name and type revealed.
    PathogenIdentified {
        disease_idx: usize,
    },
    /// A medicine was developed through applied research — ready for trials.
    MedicineDeveloped {
        medicine_idx: usize,
    },
    /// A clinical trial completed — medicine tested against a disease.
    TrialCompleted {
        medicine_idx: usize,
        disease_idx: usize,
    },
    /// A basic research tech was unlocked.
    TechUnlocked {
        tech: BasicTech,
    },
    /// Suppression research complete — pathogen infectivity permanently reduced.
    PathogenSuppressed {
        disease_idx: usize,
    },
    /// Attenuation research complete — pathogen lethality permanently reduced.
    PathogenAttenuated {
        disease_idx: usize,
    },
    /// Interdiction research complete — cross-region transmission eliminated.
    PathogenInterdicted {
        disease_idx: usize,
    },
    /// Field operations completed: infrastructure system stabilized in a region.
    InfrastructureStabilized {
        region_idx: usize,
        system: InfraSystem,
    },
    /// A medicine shipment was dispatched and is in transit.
    MedicineShipped {
        medicine_idx: usize,
        region_idx: usize,
        doses: f64,
    },
    /// A shipment delivered and doses took effect.
    /// `efficiency` is the fraction of shipped doses that were usable (supply_lines × healthcare).
    ShipmentDelivered {
        medicine_idx: usize,
        region_idx: usize,
        doses: f64,
        adverse: bool,
        efficiency: f64,
    },
    /// A disease has adapted to containment measures — quarantine/travel ban less effective.
    ContainmentAdaptation {
        disease_idx: usize,
        /// Adaptation level (0.0–1.0) at the time of the event.
        level: f64,
    },
    /// Emergency consolidation activated — all resources consolidated into one region.
    ArkProtocolActivated {
        region_idx: usize,
    },
    /// A defiant governor took an autonomous action.
    GovernorAction {
        region_idx: usize,
        description: String,
    },
    /// Infrastructure dropped below a breakpoint threshold.
    InfrastructureBreakpoint {
        region_idx: usize,
        system: InfraSystem,
        /// The breakpoint crossed: 0.50 (stressed) or 0.25 (critical) or 0.0 (failed)
        threshold: f64,
    },
    /// A standing order automatically activated a policy.
    PolicyAutoActivated {
        region_idx: usize,
        policy_name: String,
    },
    /// A field operation completed.
    FieldOpCompleted {
        label: String,
        result: String,
    },
    /// A crisis response team returned — personnel freed.
    CrisisTeamReturned {
        label: String,
        personnel: u32,
    },
}

/// Automation rules that fire during tick when conditions are met.
/// Each field is a global toggle; when enabled, the rule applies to all regions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StandingOrders {
    /// Auto-enable Quarantine when a region's infections exceed the HIGH threshold (10K).
    pub auto_quarantine_at_high: bool,
    /// Auto-enable Travel Ban when a region's infections exceed the CRIT threshold (100K).
    pub auto_travel_ban_at_crit: bool,
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
        target: DeployTarget,
    },
    StartResearch {
        track: ResearchTrack,
        project_idx: usize,
        double_personnel: bool,
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
    /// Spend funds to boost a governor's loyalty.
    AppeaseGovernor { region_idx: usize },
    /// Personality-specific bargain with a defiant governor (non-monetary cost).
    BargainWithGovernor { region_idx: usize },
    /// Toggle a standing order. Kind: 0=auto_quarantine_at_high, 1=auto_travel_ban_at_crit.
    ToggleStandingOrder { kind: usize },
    /// Toggle auto-deploy for a specific medicine.
    ToggleAutoDeploy { med_idx: usize },
    /// Toggle auto-research for a specific track.
    ToggleAutoResearch { track: ResearchTrack },
    /// Start a field operation (costs personnel and time, not money).
    StartFieldOp { kind: FieldOpKind },
    /// Upgrade the global research lab (level 0→1 or 1→2). One-time funding cost.
    UpgradeLab,
    /// Cycle a region's deployment priority (High → Normal → Low → CutOff → High).
    CycleDeployPriority { region_idx: usize },
    /// Repay an outstanding loan in full. `loan_idx` indexes into `state.loans`.
    RepayLoan { loan_idx: usize },
}

/// A crisis event that pauses the game and requires a player decision.
/// Crises create ongoing strategic choices throughout the game — the player
/// must pick one of two options, each with trade-offs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrisisEvent {
    pub kind: CrisisKind,
    pub title: String,
    pub description: String,
    pub options: Vec<CrisisOption>,
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CrisisCost {
    #[serde(default)]
    pub funding: f64,
    #[serde(default)]
    pub personnel: u32,
    /// If Some, personnel are tied up in a temporary operation for this many days
    /// and returned when it completes. If None, personnel are permanently deducted.
    #[serde(default)]
    pub operation_days: Option<f64>,
    /// Label shown in the event log when the temporary operation completes.
    #[serde(default)]
    pub operation_label: Option<String>,
}

/// A temporary crisis response operation that ties up personnel for a set duration.
/// When the timer expires, the personnel are automatically returned.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrisisOperation {
    pub label: String,
    pub personnel: u32,
    pub ticks_remaining: f64,
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
    /// `wave` counts how many regions have collapsed so far (1 on first collapse).
    RefugeeWave { from_region: usize, to_region: usize, #[serde(default = "default_one_u8")] wave: u8 },
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
    /// Two corporations claim credit for your treatment breakthrough, threatening to cut contracts.
    VaccineDispute { neutral_loss: f64, credit_gain: f64, corp_a: String, corp_b: String },

    // --- Dark comedy events (personality and flavor) ---

    /// Quarterly performance review during the apocalypse.
    PerformanceReview,
    /// Pharmaceutical corp offers money to rename a disease.
    NamingRights { disease_idx: usize, payout: f64 },
    /// Unpaid intern claims a breakthrough. 50/50 gamble.
    InternDiscovery { cost: f64 },
    /// Congressional hearing about your handling of the crisis.
    CongressionalHearing,

    // --- Patron/contract crises ---

    /// A funding patron offers a new contract. Interrupts gameplay so the player
    /// must accept or reject the terms. Replaces the old policy-panel-only flow.
    ContractOffer { template_id: u8 },
    /// Funding patron makes demands when satisfaction drops to warning zone.
    PatronDemand { template_id: u8 },

    // --- Governor defiance crises (fired when loyalty drops below threshold) ---

    /// Hardliner governor declares your mandate unconstitutional.
    #[serde(alias = "GovernorNationalist")]
    GovernorHardliner { region_idx: usize },
    /// Blowhard governor makes noise — mostly hollow threats.
    #[serde(alias = "GovernorPopulist")]
    GovernorBlowhard { region_idx: usize },
    /// Recluse governor stops responding — region drifts.
    #[serde(alias = "GovernorTechnocrat")]
    GovernorRecluse { region_idx: usize },
    /// Operative governor starts skimming openly.
    #[serde(alias = "GovernorCooperative")]
    GovernorOperative { region_idx: usize },
    /// Buffoon governor causes accidental damage.
    GovernorBuffoon { region_idx: usize },
    /// Mobster governor escalates demands.
    GovernorMobster { region_idx: usize },

    // --- Endgame crisis types ---

    /// Emergency consolidation: pull all resources into one surviving region.
    /// Fires when 2+ regions have collapsed.
    ArkProtocol { region_idx: usize },

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
    /// Follow-up to MediaPanic (Ignore): misinformation degrades screening.
    Infodemic { region_idx: usize },
    /// Follow-up to VaccineDispute (Credit one side): losing corp retaliates.
    SanctionsThreat { funding_loss: f64, corp_name: String },

    // --- Corporate crises ---

    /// Board of directors demands action when board satisfaction drops too low.
    /// `severity` 0 = warning demand (satisfaction < 0.5), 1 = ultimatum (< 0.3).
    BoardDemand { severity: u8 },

    // --- Corporate detention crises ---

    /// Field team detained by a private corporation in a collapsed region.
    /// Pay the resolution fee, escalate through official channels (slow, uncertain),
    /// or write them off entirely. If paid, a repeat detention may follow.
    FieldTeamDetained {
        region_idx: usize,
        corp_idx: usize,
        fee: f64,
        team_size: u32,
    },
    /// Follow-up to FieldTeamDetained when the player paid the first fee.
    /// The corporation has learned what the market will bear.
    FieldTeamDetainedAgain {
        region_idx: usize,
        corp_idx: usize,
        fee: f64,
        team_size: u32,
    },

    // --- Emergency loan crises ---

    /// A governor or corporation offers an emergency loan when the player's policies
    /// are being suspended due to insufficient funds.
    LoanOffer {
        lender_name: String,
        lender: LoanLender,
        amount: f64,
        daily_interest_rate: f64,
    },
    /// A lender calls in an overdue loan. Player must repay or face hostile action.
    /// Governor: cancels a policy in their region + loyalty drop.
    /// Corporation: personnel loss (intimidation) or POL penalty (smear campaign).
    LoanCallIn {
        lender_name: String,
        lender: LoanLender,
        outstanding: f64,
    },
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
            CrisisKind::ContractOffer { .. } => "contract_offer",
            CrisisKind::PatronDemand { .. } => "patron_demand",
            CrisisKind::GovernorHardliner { .. } => "gov_hardliner",
            CrisisKind::GovernorBlowhard { .. } => "gov_blowhard",
            CrisisKind::GovernorRecluse { .. } => "gov_recluse",
            CrisisKind::GovernorOperative { .. } => "gov_operative",
            CrisisKind::GovernorBuffoon { .. } => "gov_buffoon",
            CrisisKind::GovernorMobster { .. } => "gov_mobster",
            CrisisKind::ArkProtocol { .. } => "ark_protocol",
            CrisisKind::ContemptOfCongress { .. } => "contempt",
            CrisisKind::CounterfeitEpidemic { .. } => "counterfeit",
            CrisisKind::EmbezzlementRing { .. } => "embezzlement",
            CrisisKind::MilitaryOverreach => "military_overreach",
            CrisisKind::PublicInquiry => "public_inquiry",
            CrisisKind::Infodemic { .. } => "infodemic",
            CrisisKind::SanctionsThreat { .. } => "sanctions",
            CrisisKind::BoardDemand { .. } => "board_demand",
            CrisisKind::FieldTeamDetained { .. } => "field_team_detained",
            CrisisKind::FieldTeamDetainedAgain { .. } => "field_team_detained_again",
            CrisisKind::LoanOffer { .. } => "loan_offer",
            CrisisKind::LoanCallIn { .. } => "loan_call_in",
        }
    }
}

/// Crisis events start appearing after this many ticks (~3 days).
pub const CRISIS_MIN_TICK: u64 = (3.0 * TICKS_PER_DAY) as u64;
/// Average ticks between crises (~7 days).
pub const CRISIS_INTERVAL: u64 = (7.0 * TICKS_PER_DAY) as u64;
/// Minimum ticks before the same crisis type can repeat (~15 days).
pub const CRISIS_TYPE_COOLDOWN: u64 = (15.0 * TICKS_PER_DAY) as u64;
/// Minimum ticks between any two consecutive crises (~1.5 days).
/// Prevents crisis spam during collapse cascades and late-game.
pub const CRISIS_MIN_GAP: u64 = (1.5 * TICKS_PER_DAY) as u64;

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
    Operations,
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
    ConfirmDeploy { medicine_idx: usize, region_idx: usize, target: DeployTarget },
    /// Shown after a deployment completes, displaying the result prominently.
    DeployResult { medicine_idx: usize, message: String },
}

/// Policy panel UI state machine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyUiState {
    /// Manage policies for a specific region (the only state — no overview page).
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
    /// Viewing an active project. `slot_idx` selects which field project (0 for Applied/Basic).
    ViewActive { track: ResearchTrack, slot_idx: usize },
}

/// Operations/Orders panel UI state machine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OpsUiState {
    /// Top level: browse active ops, available operation types, decrees, standing orders, loans.
    BrowseOps,
    /// Pick a target disease (for Recon).
    SelectReconTarget,
    /// Pick a target region (for Emergency Response).
    SelectEmergencyTarget,
    /// Pick a target region (for Infra Survey).
    SelectSurveyTarget,
    /// Pick a target region (for Supply Chain Reinforcement).
    SelectSupplyTarget,
    /// Pick a target region (for Civil Order Stabilization).
    SelectCivilOrderTarget,
    /// Step 1: pick the source region to evacuate FROM.
    SelectEvacSource,
    /// Step 2: pick the destination region to evacuate TO. source_idx is the chosen source.
    SelectEvacDest { source_idx: usize },
    /// Confirm an emergency decree before enacting it.
    ConfirmDecree { decree_idx: usize },
    /// Select which region to sacrifice (for Sacrifice Region decree).
    SelectSacrificeRegion,
    /// Select which region to fortify (for Fortify Region decree).
    SelectFortifyRegion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiState {
    pub open_panel: Panel,
    /// Generic list cursor — index of the selected item in the current panel view.
    /// Semantics depend on `open_panel` + the active sub-state (medicine_ui, research_ui, etc.):
    ///
    /// - `Panel::Threats`                     → index into display_order (diseases sorted by deaths desc)
    /// - `Panel::Medicines / BrowseMedicines` → index into unlocked medicines
    /// - `Panel::Medicines / SelectRegion`    → index into grid_reading_order(regions)
    /// - `Panel::Medicines / SelectDisease`   → index into deployable_diseases list
    /// - `Panel::Medicines / SelectTarget`    → 0 = Vaccinate, 1 = Treat
    /// - `Panel::Research / BrowseCategories` → 0..RESEARCH_TRACK_COUNT-1 = track, RESEARCH_TRACK_COUNT = UpgradeLab
    /// - `Panel::Research / BrowseProjects`   → index into [active projects, then available projects]
    /// - `Panel::Policy / ManagePolicies`     → display position (see MANAGE_* constants)
    /// - `Panel::Operations / BrowseOps`      → 0..n_active = active op, then 0..FIELD_OP_TYPE_COUNT = op types
    ///
    /// Always bounded by `panel_selection_max()` and reset to 0 on every wizard step transition.
    ///
    /// **Adding items to a panel list:** update the corresponding `panel_selection_max()` branch.
    /// Named constants (RESEARCH_TRACK_COUNT, STANDING_ORDER_COUNT, FIELD_OP_TYPE_COUNT, MANAGE_*)
    /// tie the max calculation to the renderer so changes propagate correctly.
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
    /// Only set by command responses (deploy, research, etc.) — NOT by game events.
    #[serde(default)]
    pub status_message: Option<String>,
    /// Latest event notification shown in the top-right of the status bar.
    /// Set by process_events() when a game event fires; persists until replaced.
    /// Distinct from status_message — events go here, command feedback goes there.
    #[serde(default)]
    pub event_notification: Option<String>,
    /// Which crisis option is selected (0 = A, 1 = B).
    #[serde(default)]
    pub crisis_selection: usize,
    /// Whether the [X] auto-resolve toggle is active for the current crisis popup.
    #[serde(default)]
    pub crisis_auto_resolve: bool,
    /// Operations panel wizard state.
    #[serde(default)]
    pub operations_ui: Option<OpsUiState>,
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
    pub fn toggle_panel(&mut self, panel: Panel, _num_regions: usize) {
        if self.open_panel == panel {
            // Check if we're deeper than the top level — if so, reset to top
            let at_top = match panel {
                Panel::Medicines => matches!(self.medicine_ui, Some(MedicineUiState::BrowseMedicines) | None),
                Panel::Research => matches!(self.research_ui, Some(ResearchUiState::BrowseCategories) | None),
                Panel::Policy => matches!(self.policy_ui, Some(PolicyUiState::ManagePolicies { .. }) | None),
                Panel::Operations => matches!(self.operations_ui, Some(OpsUiState::BrowseOps) | None),
                _ => true,
            };
            if at_top {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                match panel {
                    Panel::Medicines => self.medicine_ui = None,
                    Panel::Research => self.research_ui = None,
                    Panel::Policy => self.policy_ui = None,
                    Panel::Operations => self.operations_ui = None,
                    _ => {}
                }
            } else {
                // Reset to top level of this panel
                self.panel_selection = 0;
                match panel {
                    Panel::Medicines => self.medicine_ui = Some(MedicineUiState::BrowseMedicines),
                    Panel::Research => self.research_ui = Some(ResearchUiState::BrowseCategories),
                    Panel::Policy => {
                        self.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: self.map_selection });
                    }
                    Panel::Operations => self.operations_ui = Some(OpsUiState::BrowseOps),
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
                    // Go directly to the policies for the currently selected region.
                    // Left/right map navigation (sync_panel_region) keeps this in sync.
                    self.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: self.map_selection });
                }
                Panel::Operations => {
                    self.operations_ui = Some(OpsUiState::BrowseOps);
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
                    Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, target }) => {
                        let (disease_idx, action) = match target {
                            DeployTarget::Vaccinate { disease_idx } => (disease_idx, 0),
                            DeployTarget::Treat { disease_idx } => (disease_idx, 1),
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
                // ManagePolicies is the top level — Esc always closes the panel.
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                self.policy_ui = None;
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
            Panel::Operations => {
                match &self.operations_ui {
                    Some(OpsUiState::SelectReconTarget)
                    | Some(OpsUiState::SelectEmergencyTarget)
                    | Some(OpsUiState::SelectSurveyTarget)
                    | Some(OpsUiState::SelectSupplyTarget)
                    | Some(OpsUiState::SelectCivilOrderTarget)
                    | Some(OpsUiState::SelectEvacSource)
                    | Some(OpsUiState::ConfirmDecree { .. })
                    | Some(OpsUiState::SelectSacrificeRegion)
                    | Some(OpsUiState::SelectFortifyRegion) => {
                        self.operations_ui = Some(OpsUiState::BrowseOps);
                        self.panel_selection = 0;
                    }
                    Some(OpsUiState::SelectEvacDest { .. }) => {
                        // Step 2 → back to step 1
                        self.operations_ui = Some(OpsUiState::SelectEvacSource);
                        self.panel_selection = 0;
                    }
                    _ => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.operations_ui = None;
                    }
                }
            }
            _ => {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                self.medicine_ui = None;
                self.research_ui = None;
                self.policy_ui = None;
                self.operations_ui = None;
            }
        }
    }


    /// Close all panels and return to the main dashboard.
    /// Unlike `close_panel` (Esc), which goes back one step in a wizard,
    /// this resets all panel state immediately regardless of depth.
    pub fn go_home(&mut self) {
        self.open_panel = Panel::None;
        self.panel_selection = 0;
        self.medicine_ui = None;
        self.research_ui = None;
        self.policy_ui = None;
        self.operations_ui = None;
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
                Some(ResearchUiState::BrowseCategories) => RESEARCH_TRACK_COUNT, // Field(0), Applied(1), Basic(2), UpgradeLab(RESEARCH_TRACK_COUNT)
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
                // Repair/Appease/Bargain hidden for collapsed regions.
                Some(PolicyUiState::ManagePolicies { region_idx }) => {
                    if state.regions.get(*region_idx).is_some_and(|r| r.collapsed) {
                        POLICY_COUNT - 1
                    } else if state.bargain_available(*region_idx) {
                        MANAGE_BARGAIN_POS
                    } else {
                        MANAGE_APPEASE_POS
                    }
                }
                None => 0,
            },
            Panel::Operations => match &self.operations_ui {
                Some(OpsUiState::BrowseOps) => {
                    // Active ops + op types + decrees + standing orders + loans
                    (state.field_operations.len() + FIELD_OP_TYPE_COUNT
                        + DECREE_COUNT + STANDING_ORDER_COUNT + state.loans.len())
                        .saturating_sub(1)
                }
                Some(OpsUiState::SelectReconTarget) => {
                    // Unidentified diseases
                    state.diseases.iter()
                        .filter(|d| d.detected && d.knowledge < KNOWLEDGE_NAME)
                        .count()
                        .saturating_sub(1)
                }
                Some(OpsUiState::SelectEmergencyTarget)
                | Some(OpsUiState::SelectSurveyTarget)
                | Some(OpsUiState::SelectSupplyTarget)
                | Some(OpsUiState::SelectCivilOrderTarget)
                | Some(OpsUiState::SelectEvacSource)
                | Some(OpsUiState::SelectEvacDest { .. })
                | Some(OpsUiState::SelectSacrificeRegion)
                | Some(OpsUiState::SelectFortifyRegion) => {
                    // Non-collapsed regions
                    state.regions.iter().filter(|r| !r.collapsed).count().saturating_sub(1)
                }
                Some(OpsUiState::ConfirmDecree { .. }) => 0,
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
        } else {
            self.panel_selection = 0;
        }
    }

    /// Navigate up (in map) or to the previous item (in a panel).
    pub fn select_prev(&mut self, num_regions: usize, panel_max: usize) {
        if self.open_panel == Panel::None {
            self.map_selection = map_navigate(
                self.map_selection,
                MapDirection::Up,
                num_regions,
            );
        } else if self.panel_selection > 0 {
            self.panel_selection -= 1;
        } else {
            self.panel_selection = panel_max;
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
        self.sync_panel_region(num_regions);
    }

    /// Navigate right on the map (always, regardless of open panel).
    /// Left/right are reserved for region navigation — panels use up/down only.
    pub fn select_right(&mut self, num_regions: usize) {
        self.map_selection = map_navigate(
            self.map_selection,
            MapDirection::Right,
            num_regions,
        );
        self.sync_panel_region(num_regions);
    }

    /// Keep region-specific panel views in sync with the map selection.
    fn sync_panel_region(&mut self, num_regions: usize) {
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
            Some(MedicineUiState::SelectRegion { .. }) => {
                // Keep list cursor in sync with map — left/right should move the
                // deploy target, consistent with how deeper wizard steps work.
                let order = grid_reading_order(num_regions);
                self.panel_selection = order.iter()
                    .position(|&r| r == self.map_selection)
                    .unwrap_or(0);
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
                governor: Governor {
                    name: "Gov. Torres".into(),
                    personality: GovernorPersonality::Hardliner,
                    loyalty: 65.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                },
                infections: vec![],
                traits: vec![RegionTrait::TradeDependent, RegionTrait::StrongPublicHealth],
                collapse_threshold: 0.55, // Fragile — collapses at 45% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                collapse_deaths: 0.0,
                hospital_level: 0,
                intel_level: 0,
                income_modifier: 1.8,     // Wealthy — major economic contributor
                healthcare_modifier: 0.85, // Good healthcare infrastructure
                last_deploy_tick: HashMap::new(),
                cached_deaths_per_day: 0.0,
                prev_dead: 0.0,
                prev_dead_tick: 0,
                healthcare_capacity: 1.0,
                supply_lines: 1.0,
                civil_order: 1.0,
                deploy_priority: RegionPriority::Normal,
                supply_resilience: 0.0,
                civil_resilience: 0.0,
                emergency_response_until: None,
                disrupted_until: None,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
            },
            Region {
                name: "South America".into(),
                population: 430_000_000,
                connections: vec![0, 3],
                governor: Governor {
                    name: "Gov. Vasquez".into(),
                    personality: GovernorPersonality::Blowhard,
                    loyalty: 70.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                },
                infections: vec![],
                traits: vec![RegionTrait::LowInfrastructure, RegionTrait::ResilientPopulation],
                collapse_threshold: 0.55, // Moderate resilience — 45% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                collapse_deaths: 0.0,
                hospital_level: 0,
                intel_level: 0,
                income_modifier: 1.0,     // Moderate economy
                healthcare_modifier: 0.95, // Decent healthcare
                last_deploy_tick: HashMap::new(),
                cached_deaths_per_day: 0.0,
                prev_dead: 0.0,
                prev_dead_tick: 0,
                healthcare_capacity: 1.0,
                supply_lines: 1.0,
                civil_order: 1.0,
                deploy_priority: RegionPriority::Normal,
                supply_resilience: 0.0,
                civil_resilience: 0.0,
                emergency_response_until: None,
                disrupted_until: None,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
            },
            Region {
                name: "Europe".into(),
                population: 750_000_000,
                connections: vec![0, 3, 4],
                governor: Governor {
                    name: "Gov. Lindqvist".into(),
                    personality: GovernorPersonality::Operative,
                    loyalty: 75.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                },
                infections: vec![],
                traits: vec![RegionTrait::TradeDependent, RegionTrait::DenseUrban],
                collapse_threshold: 0.50, // Developed infrastructure — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                collapse_deaths: 0.0,
                hospital_level: 0,
                intel_level: 0,
                income_modifier: 1.5,     // Strong economy, hub region
                healthcare_modifier: 0.80, // Excellent healthcare
                last_deploy_tick: HashMap::new(),
                cached_deaths_per_day: 0.0,
                prev_dead: 0.0,
                prev_dead_tick: 0,
                healthcare_capacity: 1.0,
                supply_lines: 1.0,
                civil_order: 1.0,
                deploy_priority: RegionPriority::Normal,
                supply_resilience: 0.0,
                civil_resilience: 0.0,
                emergency_response_until: None,
                disrupted_until: None,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
            },
            Region {
                name: "Africa".into(),
                population: 1_400_000_000,
                connections: vec![1, 2, 4],
                governor: Governor {
                    name: "Gov. Okonkwo".into(),
                    personality: GovernorPersonality::Buffoon,
                    loyalty: 60.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                },
                infections: vec![],
                traits: vec![RegionTrait::LowInfrastructure, RegionTrait::DenseUrban],
                collapse_threshold: 0.50, // Resilient — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                collapse_deaths: 0.0,
                hospital_level: 0,
                intel_level: 0,
                income_modifier: 0.6,     // Lower per-capita income
                healthcare_modifier: 1.1,  // Strained healthcare — higher lethality
                last_deploy_tick: HashMap::new(),
                cached_deaths_per_day: 0.0,
                prev_dead: 0.0,
                prev_dead_tick: 0,
                healthcare_capacity: 1.0,
                supply_lines: 1.0,
                civil_order: 1.0,
                deploy_priority: RegionPriority::Normal,
                supply_resilience: 0.0,
                civil_resilience: 0.0,
                emergency_response_until: None,
                disrupted_until: None,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
            },
            Region {
                name: "Asia".into(),
                population: 4_700_000_000,
                connections: vec![2, 3, 5],
                governor: Governor {
                    name: "Gov. Chen".into(),
                    personality: GovernorPersonality::Recluse,
                    loyalty: 70.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                },
                infections: vec![],
                traits: vec![RegionTrait::DenseUrban, RegionTrait::ResilientPopulation],
                collapse_threshold: 0.50, // Huge population — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                collapse_deaths: 0.0,
                hospital_level: 0,
                intel_level: 0,
                income_modifier: 0.9,     // Large but moderate per-capita
                healthcare_modifier: 1.0,  // Baseline healthcare
                last_deploy_tick: HashMap::new(),
                cached_deaths_per_day: 0.0,
                prev_dead: 0.0,
                prev_dead_tick: 0,
                healthcare_capacity: 1.0,
                supply_lines: 1.0,
                civil_order: 1.0,
                deploy_priority: RegionPriority::Normal,
                supply_resilience: 0.0,
                civil_resilience: 0.0,
                emergency_response_until: None,
                disrupted_until: None,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![4],
                governor: Governor {
                    name: "Gov. Whitfield".into(),
                    personality: GovernorPersonality::Mobster,
                    loyalty: 75.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                },
                infections: vec![],
                traits: vec![RegionTrait::IslandGeography, RegionTrait::StrongPublicHealth],
                collapse_threshold: 0.50, // Small but developed — 50% dead
                dead: 0.0,
                collapsed: false,
                collapsed_at_tick: None,
                collapse_deaths: 0.0,
                hospital_level: 0,
                intel_level: 0,
                income_modifier: 2.5,     // Tiny but wealthy — high per-capita
                healthcare_modifier: 0.75, // Best healthcare infrastructure
                last_deploy_tick: HashMap::new(),
                cached_deaths_per_day: 0.0,
                prev_dead: 0.0,
                prev_dead_tick: 0,
                healthcare_capacity: 1.0,
                supply_lines: 1.0,
                civil_order: 1.0,
                deploy_priority: RegionPriority::Normal,
                supply_resilience: 0.0,
                civil_resilience: 0.0,
                emergency_response_until: None,
                disrupted_until: None,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
            },
        ];

        // Assign per-region systematic noise biases so unscreened data is genuinely
        // wrong (not just stale). Each region over- or under-reports consistently,
        // varying by seed. Derived from a separate RNG stream keyed on the game seed
        // + region index so we don't shift the main RNG sequence (which would change
        // disease generation and break seeded reproducibility).
        // Suppressed at higher screening levels in tick_screening().
        for (i, region) in regions.iter_mut().enumerate() {
            let mut bias_rng = ChaCha8Rng::seed_from_u64(seed ^ (i as u64).wrapping_mul(0x9e3779b97f4a7c15));
            region.screening_noise_bias = bias_rng.r#gen::<f64>() * 0.6 - 0.3; // [-0.3, 0.3]
        }

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
        // Seed the initial estimate — organic reporting catches roughly 15% of cases
        // at the time the player takes over. Without this, the first frame shows
        // "Infected: ~0" despite the briefing saying there's an active outbreak.
        regions[region_idx].estimated_infected = infected * ScreeningLevel::None.visibility_rate();

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
            target_diseases: all_disease_indices.clone(),
            cost: 10.0,
            doses: 25_000_000.0,
            max_doses: 25_000_000.0,
            unlocked: true,
            // Broad-spectrum starts unlocked at limited supply: a blunt bandaid
            // that slows early disease spread while the player develops targeted medicines.
            // 25M doses depletes quickly in multi-region outbreaks, forcing investment
            // in the research pipeline. Targeted medicines are 6–7x more effective.
            tested_against: all_disease_indices.clone(),
            strain_generations: vec![],
            deployed_count: 0,
        });

        let num_diseases = diseases.len();

        Self {
            tick: 0,
            sim_state: SimState::Running,
            rng,
            resources: Resources {
                funding: 500.0,
                personnel: 20,
                political_power: 0.20,
                personnel_accum: 0.0,
                attrition_accum: 0.0,
                last_funding_warning_tick: 0,
                last_loan_offer_tick: 0,
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
            last_crisis_resolved_tick: 0,
            auto_resolve_crises: HashMap::new(),
            history: vec![],
            auto_research: [false; 3],
            auto_deploy: vec![],
            standing_orders: StandingOrders::default(),
            field_operations: vec![],
            crisis_operations: vec![],
            pending_shipments: vec![],
            death_milestone_tier: vec![0; num_diseases],
            intel_pre_detection_briefed: vec![false; num_diseases],
            ark_protocol: None,
            total_doses_deployed: 0.0,
            pathogens_suppressed: 0,
            pathogens_attenuated: 0,
            pathogens_interdicted: 0,
            lab_level: 0,
            contracts: Vec::new(),
            contract_offer: None,
            last_contract_offer_tick: 0,
            corporations: Vec::new(),
            last_board_demand_tick: 0,
            next_sequence_group: 0,
            loans: vec![],
            ui: UiState {
                open_panel: Panel::None,
                panel_selection: 0,
                medicine_ui: None,
                map_selection: 0,
                research_ui: None,
                policy_ui: None,
                status_message: None,
                event_notification: None,
                crisis_selection: 0,
                crisis_auto_resolve: false,
                operations_ui: None,
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

    /// Get corporations headquartered in a given region.
    pub fn region_corporations(&self, region_idx: usize) -> Vec<&Corporation> {
        self.corporations.iter().filter(|c| c.region_idx == region_idx).collect()
    }


    /// Infection trend: ratio of current screened infected to ~1 day ago.
    /// Returns None if not enough history. > 1.0 means growing, < 1.0 shrinking.
    /// Uses screened values consistently (both current and historical are
    /// player-visible estimates, not ground truth).
    pub fn infection_trend(&self) -> Option<f64> {
        let lookback = self.trend_lookback()?;
        let past = &self.history[self.history.len() - lookback];
        if past.screened_infected < 100.0 {
            return None; // too few infections to show a meaningful trend
        }
        let current = self.total_infected_screened();
        Some(current / past.screened_infected)
    }

    /// Death trend: new deaths in last day vs the day before that.
    /// Returns None if not enough history. > 1.0 means accelerating, < 1.0 decelerating.
    pub fn death_trend(&self) -> Option<f64> {
        let lookback = self.trend_lookback()?;
        if self.history.len() < lookback * 2 {
            return None;
        }
        let now_dead = self.total_dead_detected();
        let one_day_ago = &self.history[self.history.len() - lookback];
        let two_days_ago = &self.history[self.history.len() - lookback * 2];
        let recent_deaths = now_dead - one_day_ago.detected_dead;
        let prior_deaths = one_day_ago.detected_dead - two_days_ago.detected_dead;
        if prior_deaths < 100.0 {
            return None;
        }
        Some(recent_deaths / prior_deaths)
    }

    /// Lookback entries for ~1 day of history. None if insufficient data.
    fn trend_lookback(&self) -> Option<usize> {
        let lookback = (TICKS_PER_DAY as usize) / (HISTORY_INTERVAL as usize);
        if self.history.len() < lookback {
            None
        } else {
            Some(lookback)
        }
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
    /// Uses the convergence-based estimated_infected per region.
    pub fn total_infected_screened(&self) -> f64 {
        self.regions.iter()
            .map(|r| r.estimated_infected)
            .sum()
    }

    /// Effective screening visibility for a specific region, accounting for
    /// screening_progress ramp-up. Returns 0.15 (base) when no screening or
    /// screening just enabled, up to the tier's max at full progress.
    pub fn screening_visibility(&self, region_idx: usize) -> f64 {
        let (level_vis, progress) = self.policies.get(region_idx)
            .map(|p| (p.screening.visibility_rate(), p.screening_progress))
            .unwrap_or((0.15, 0.0));
        let base = ScreeningLevel::None.visibility_rate();
        base + (level_vis - base) * progress
    }

    /// Whether the screening level in a region reveals immune data.
    /// Requires both an Antigen+ tier AND meaningful ramp-up progress (>50%).
    pub fn screening_shows_immune(&self, region_idx: usize) -> bool {
        self.policies.get(region_idx)
            .map(|p| p.screening.shows_immune() && p.screening_progress > 0.5)
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

    /// Effective detection multiplier accounting for screening progress.
    /// Blends between None (1.0) and the best tier's multiplier based on progress.
    /// Lower value = detects earlier.
    pub fn effective_detection_multiplier(&self) -> f64 {
        let base = ScreeningLevel::None.detection_multiplier(); // 1.0
        self.policies.iter()
            .map(|p| {
                let level = p.screening.detection_multiplier();
                // Interpolate: 1.0 at progress=0, level_mult at progress=1
                base + (level - base) * p.screening_progress
            })
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(base)
    }

    /// Total dead from detected diseases plus post-collapse secondary deaths (for UI display).
    /// Excludes deaths from undetected diseases (player doesn't know about those yet).
    pub fn total_dead_detected(&self) -> f64 {
        let disease_dead: f64 = self.regions.iter()
            .flat_map(|r| &r.infections)
            .filter(|inf| self.diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.dead)
            .sum();
        let collapse_dead: f64 = self.regions.iter()
            .map(|r| r.collapse_deaths)
            .sum();
        disease_dead + collapse_dead
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
        let hospitals: u32 = self.regions.iter().map(|r| match r.hospital_level {
            2 => MEDICAL_CENTER_PERSONNEL,
            1 => FIELD_HOSPITAL_PERSONNEL,
            _ => 0,
        }).sum();
        let intel: u32 = self.regions.iter().map(|r| match r.intel_level {
            2 => ADVANCED_INTEL_PERSONNEL,
            1 => INTEL_STATION_PERSONNEL,
            _ => 0,
        }).sum();
        let ops: u32 = self.field_operations.iter().map(|op| op.personnel).sum();
        let crisis_ops: u32 = self.crisis_operations.iter().map(|op| op.personnel).sum();
        field + applied + basic + policy + hospitals + intel + ops + crisis_ops
    }

    pub fn personnel_available(&self) -> u32 {
        self.resources.personnel.saturating_sub(self.personnel_busy())
    }

    /// Total cost of all pending medicine shipments (already paid, in transit).
    pub fn pending_shipment_cost(&self) -> f64 {
        self.pending_shipments.iter().map(|s| s.cost).sum()
    }

    pub fn total_policy_funding_cost(&self) -> f64 {
        self.policies.iter().enumerate()
            .map(|(i, p)| {
                let region = self.regions.get(i);
                let traits = region.map(|r| r.traits.as_slice()).unwrap_or(&[]);
                let gov_mult = region.map(|r| r.governor.cost_multiplier()).unwrap_or(1.0);
                // Supply line degradation increases policy costs
                let supply_mult = region.map(|r| {
                    if r.supply_lines < INFRA_STRESSED { SUPPLY_STRESSED_COST_MULT } else { 1.0 }
                }).unwrap_or(1.0);
                p.funding_cost(traits) * gov_mult * supply_mult
            })
            .sum()
    }

    /// Per-region income contribution before travel ban modifier.
    /// `total_pop` must be > 0 (caller checks).
    /// Travel ban income factor for a region (1.0 = no ban, 0.3 or 0.5 = ban active).
    fn region_travel_ban_factor(&self, region_idx: usize, region: &Region) -> f64 {
        if self.policies.get(region_idx).is_some_and(|p| p.travel_ban) {
            if region.has_trait(RegionTrait::TradeDependent) {
                TRADE_DEPENDENT_INCOME_FACTOR
            } else {
                TRAVEL_BAN_INCOME_PENALTY
            }
        } else {
            1.0
        }
    }

    fn region_base_income(region: &Region, total_pop: f64) -> f64 {
        let pop = region.population as f64;
        let infected: f64 = region.infections.iter().map(|inf| inf.infected).sum();
        let incapacitated = region.dead + infected * INFECTED_INCAPACITATION_RATE;
        let healthy_frac = (pop - incapacitated).max(0.0) / pop;
        let region_share = pop / total_pop;
        BASE_FUNDING_INCOME * region_share * healthy_frac * region.income_modifier
    }

    /// Economic health factor for a region (0.0 = collapsed, up to 1.0 = fully healthy).
    /// Used by neighbors to compute trade income. Accounts for both active infections
    /// and cumulative deaths — a region that lost 30% of its population is economically
    /// devastated even if current infections are low.
    fn region_economic_health(region: &Region) -> f64 {
        if region.collapsed {
            return 0.0;
        }
        let pop = region.population as f64;
        if pop <= 0.0 {
            return 0.0;
        }
        let infected: f64 = region.infections.iter().map(|inf| inf.infected).sum();
        let infected_frac = infected / pop;
        let death_frac = region.dead / pop;
        // Active infections are weighted more heavily (immediate economic disruption)
        // Deaths reflect permanent economic damage
        let damage = infected_frac * 3.0 + death_frac * 2.0;
        (1.0 - damage).clamp(0.1, 1.0)
    }

    /// Average economic health of a region's connected neighbors (0.0 to 1.0).
    fn neighbor_trade_health(&self, region_idx: usize) -> f64 {
        let connections = &self.regions[region_idx].connections;
        if connections.is_empty() {
            return 1.0; // No neighbors = no trade dependency
        }
        let sum: f64 = connections
            .iter()
            .map(|&n| Self::region_economic_health(&self.regions[n]))
            .sum();
        sum / connections.len() as f64
    }

    /// Estimated funding income per tick, based on corporate tax and contracts.
    ///
    /// Corporate tax revenue from each region's corporations (already accounts for
    /// workforce health, infrastructure, and policy effects). Governor skim and
    /// inter-region trade modifiers apply on top.
    ///
    /// Falls back to the pre-corporation formula if no corporations exist (backwards compat).
    /// Per-region raw income before decree modifiers (per tick).
    /// Handles both corporate (normal) and legacy (pre-corporation save) paths.
    /// Collapsed regions return 0. Called by both `funding_income_rate()` and
    /// `per_region_income_breakdown()` to avoid duplicating the formula.
    fn region_raw_income_pre_decree(&self, i: usize, region: &Region, total_pop: f64) -> f64 {
        if region.collapsed { return 0.0; }
        if self.corporations.is_empty() {
            // Fallback: old abstract formula (for saves without corporations)
            let base = Self::region_base_income(region, total_pop);
            let after_ban = base * self.region_travel_ban_factor(i, region);
            let skim_factor = 1.0 - region.governor.income_skim;
            let after_skim = after_ban * skim_factor;
            let domestic = after_skim * (1.0 - TRADE_INCOME_FRACTION);
            let trade = after_skim * TRADE_INCOME_FRACTION * self.neighbor_trade_health(i);
            domestic + trade
        } else {
            // Corporate income: sum tax contributions per region, apply skim + trade
            let region_corp_tax: f64 = self.corporations.iter()
                .filter(|c| c.region_idx == i)
                .map(|c| c.tax_contribution())
                .sum();
            let skim_factor = 1.0 - region.governor.income_skim;
            let after_skim = region_corp_tax * skim_factor;
            let domestic = after_skim * (1.0 - TRADE_INCOME_FRACTION);
            let trade = after_skim * TRADE_INCOME_FRACTION * self.neighbor_trade_health(i);
            domestic + trade
        }
    }

    pub fn funding_income_rate(&self) -> f64 {
        let total_pop: f64 = self.regions.iter().map(|r| r.population as f64).sum();
        if total_pop <= 0.0 {
            return 0.0;
        }
        let mut income: f64 = self.regions.iter().enumerate()
            .map(|(i, region)| self.region_raw_income_pre_decree(i, region, total_pop))
            .sum();
        // Decree modifiers
        if self.enacted_decrees.sacrificed_region.is_some() {
            income *= SACRIFICE_INCOME_BONUS;
        }
        if self.enacted_decrees.conscript_researchers {
            income = (income - CONSCRIPT_INCOME_PENALTY).max(0.0);
        }
        // Contract income — fixed, not affected by population health
        let contract_income: f64 = self.contracts.iter().map(|c| c.income).sum();
        income + contract_income
    }

    /// Per-tick income from active contracts alone (for UI breakdown).
    pub fn contract_income_rate(&self) -> f64 {
        self.contracts.iter().map(|c| c.income).sum()
    }

    /// Total outstanding debt across all active loans.
    pub fn total_debt(&self) -> f64 {
        self.loans.iter().map(|l| l.outstanding).sum()
    }

    /// Per-day interest cost on all active loans (for budget display).
    pub fn daily_debt_service(&self) -> f64 {
        self.loans.iter().map(|l| l.outstanding * l.daily_interest_rate).sum()
    }


    /// Per-region income contribution per day (after all modifiers including decrees).
    /// Returns a vec of (region_idx, income_per_day) for all regions in order.
    /// Collapsed regions produce 0. Decree modifiers are applied proportionally.
    pub fn per_region_income_breakdown(&self) -> Vec<(usize, f64)> {
        let total_pop: f64 = self.regions.iter().map(|r| r.population as f64).sum();
        if total_pop <= 0.0 {
            return self.regions.iter().enumerate().map(|(i, _)| (i, 0.0)).collect();
        }
        let per_region_raw: Vec<f64> = self.regions.iter().enumerate()
            .map(|(i, region)| self.region_raw_income_pre_decree(i, region, total_pop))
            .collect();
        let pre_decree_total: f64 = per_region_raw.iter().sum();
        // Compute decree multiplier so totals stay consistent with funding_income_rate()
        let decree_factor = if pre_decree_total > 0.0 {
            self.funding_income_rate() / pre_decree_total
        } else {
            1.0
        };
        per_region_raw.into_iter().enumerate()
            .map(|(i, raw)| (i, raw * decree_factor * TICKS_PER_DAY))
            .collect()
    }

    /// Trade income lost per tick due to neighbor health degradation.
    /// Returns the difference between max possible trade income and actual trade income.
    pub fn trade_income_penalty(&self) -> f64 {
        let total_pop: f64 = self.regions.iter().map(|r| r.population as f64).sum();
        if total_pop <= 0.0 {
            return 0.0;
        }
        let mut penalty = 0.0;
        for (i, region) in self.regions.iter().enumerate() {
            if region.collapsed {
                continue;
            }
            let base = Self::region_base_income(region, total_pop);
            let after_ban = base * self.region_travel_ban_factor(i, region);
            let trade_loss = after_ban * TRADE_INCOME_FRACTION * (1.0 - self.neighbor_trade_health(i));
            penalty += trade_loss;
        }
        penalty
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
                let factor = self.region_travel_ban_factor(i, region);
                penalty += Self::region_base_income(region, total_pop) * (1.0 - factor);
            }
        }
        penalty
    }

    /// Measure of player's technological capability (0.0+).
    /// Each unlocked tech and deployed medicine increases disease emergence pressure.
    /// Used by spawn_disease_scaled to make the arms race bidirectional.
    pub fn tech_pressure(&self) -> f64 {
        let tech_count = self.unlocked_techs.len() as f64;
        let deployed_medicines = self.medicines.iter()
            .filter(|m| m.unlocked && m.deployed_count > 0)
            .count() as f64;
        // Each tech adds 0.15 emergence pressure (8 techs max = 1.2)
        // Each deployed medicine adds 0.05 (soft scaling with active capability)
        // Total caps at 2.0 (meaning 3x base emergence rate at maximum player capability)
        (tech_count * 0.15 + deployed_medicines * 0.05).min(2.0)
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

    /// Whether a decree is unlocked based on current crisis severity.
    /// Decrees become available when conditions are dire enough to justify them.
    /// Human-readable unlock condition for a decree, shown in the policy panel when locked.
    /// Must stay in sync with `decree_unlocked`.
    pub fn decree_unlock_hint(decree_idx: usize) -> &'static str {
        match decree_idx {
            0 => "Unlocks: 500K+ infected or 100K+ dead",
            1 => "Unlocks: 50M+ dead or 2+ regions at CRITICAL",
            2 => "Unlocks: any region collapsed or 500M+ dead",
            3 => "Unlocks: 50M+ dead or 2+ regions at CRITICAL",
            4 => "Unlocks: any region collapsed or 500M+ dead",
            5 => "Unlocks: 3+ regions collapsed or 2B+ dead",
            _ => "",
        }
    }

    pub fn decree_unlocked(&self, decree_idx: usize) -> bool {
        let total_infected = self.total_infected();
        let total_dead = self.total_dead();
        let collapsed_count = self.regions.iter().filter(|r| r.collapsed).count();
        let crit_count = self.regions.iter()
            .filter(|r| !r.collapsed && r.infections.iter().any(|i| i.infected > SEVERITY_CRIT_THRESHOLD))
            .count();

        match decree_idx {
            // Conscript Researchers: 500K+ infected OR 100K+ dead
            0 => total_infected >= 500_000.0 || total_dead >= 100_000.0,
            // Authorize Human Trials: 50M+ dead OR 2+ regions at CRIT severity
            1 => total_dead >= 50_000_000.0 || crit_count >= 2,
            // Sacrifice Region: any region collapsed OR 500M+ dead
            2 => collapsed_count >= 1 || total_dead >= 500_000_000.0,
            // Suspend Regional Authority: 50M+ dead OR 2+ regions at CRIT severity
            3 => total_dead >= 50_000_000.0 || crit_count >= 2,
            // Fortify Region: any region collapsed OR 500M+ dead
            4 => collapsed_count >= 1 || total_dead >= 500_000_000.0,
            // Emergency Countermeasure: 3+ regions collapsed OR 2B+ dead
            5 => collapsed_count >= 3 || total_dead >= 2_000_000_000.0,
            _ => false,
        }
    }

    /// Whether a personality-specific bargain is available for the given region.
    /// Requires: non-collapsed region, defiant governor, and personality-specific
    /// preconditions (Technocrat needs active applied research).
    pub fn bargain_available(&self, region_idx: usize) -> bool {
        let region = match self.regions.get(region_idx) {
            Some(r) => r,
            None => return false,
        };
        if region.collapsed || !region.governor.is_defiant() {
            return false;
        }
        // All personality types can always bargain when defiant
        true
    }

    /// Current POL drift target based on severity, time, and active policies.
    /// POL drifts toward this value at ~30%/day. Called by engine::tick().
    /// Returns (baseline, death_component, infection_component) that sum to pol_target().
    /// `death_component` = sqrt(death_frac), `infection_component` = 0.4 * sqrt(infected_frac).
    /// Used by the dashboard to show a breakdown without duplicating the formula.
    ///
    /// Uses OBSERVED figures (detected deaths, screened infections) — not ground truth.
    /// This means poor screening suppresses political pressure, creating a strategic
    /// trade-off: neglect screening to slow POL growth, or invest for better intel.
    pub fn pol_target_components(&self) -> (f64, f64, f64) {
        let initial_pop = self.initial_population();
        let death_frac = if initial_pop > 0.0 { self.total_dead_detected() / initial_pop } else { 0.0 };
        let infected_frac = if initial_pop > 0.0 { self.total_infected_screened() / initial_pop } else { 0.0 };
        let baseline = 0.20_f64;
        let death_component = death_frac.sqrt();
        let infection_component = infected_frac.sqrt() * 0.4;
        (baseline, death_component, infection_component)
    }

    pub fn pol_target(&self) -> f64 {
        let (baseline, death_component, infection_component) = self.pol_target_components();
        // Baseline: 20% institutional mandate even before the crisis escalates.
        // POL grows naturally as crisis severity worsens — the worse things get,
        // the more emergency authority is granted. Removed per-policy drain (which
        // was perverse: spending political capital shouldn't reduce future mandate).
        (baseline + death_component + infection_component).clamp(0.0, 0.90)
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
    /// Returns true if the Ark Protocol is active and this region is not the Ark
    /// and not already collapsed. These regions have been abandoned.
    pub fn is_abandoned(&self, region_idx: usize) -> bool {
        self.ark_protocol.is_some_and(|ark| ark != region_idx)
            && !self.regions.get(region_idx).is_some_and(|r| r.collapsed)
    }

    /// Generate debrief-style tips referencing what actually happened in this run.
    /// Returns up to 2 tips, most impactful first.
    pub fn defeat_tips(&self) -> Vec<String> {
        let mut tips = Vec::new();

        let unidentified = self.diseases.iter()
            .filter(|d| d.knowledge < KNOWLEDGE_NAME)
            .count();
        let total_diseases = self.diseases.len();

        // Unidentified diseases — reference the count
        if unidentified == total_diseases {
            tips.push(format!(
                "{total_diseases} pathogen{} active, none identified. Can't develop medicine for what you don't understand.",
                if total_diseases == 1 { "" } else { "s" }
            ));
        } else if unidentified > 0 {
            tips.push(format!(
                "{unidentified} of {total_diseases} pathogens never identified. Unidentified threats can't be treated."
            ));
        }

        // Developed but never deployed — reference the specific medicine
        let undeployed: Vec<&Medicine> = self.medicines.iter()
            .filter(|m| m.unlocked && m.deployed_count == 0)
            .collect();
        if !undeployed.is_empty() && tips.len() < 2 {
            let name = &undeployed[0].name;
            tips.push(format!(
                "{name} was developed but never deployed."
            ));
        } else if self.medicines.iter().all(|m| !m.unlocked) && unidentified < total_diseases && tips.len() < 2 {
            // Identified but never developed any medicine
            tips.push("Identified threats but never developed a medicine. The research pipeline stalled.".to_string());
        }

        // No policies used — reference the worst-hit region
        let any_policy_active = self.policies.iter().any(|p| p.any_active());
        if !any_policy_active && tips.len() < 2 {
            let worst = self.regions.iter()
                .max_by(|a, b| a.total_dead().partial_cmp(&b.total_dead()).unwrap());
            if let Some(region) = worst {
                if region.total_dead() > 0.0 {
                    let dead = region.total_dead();
                    let dead_str = format_number(dead);
                    tips.push(format!(
                        "{} lost {dead_str}. Containment policies were never activated.",
                        region.name
                    ));
                }
            }
        }

        // First collapse timing — useful when player made it reasonably far
        if tips.len() < 2 {
            let first_collapse = self.regions.iter()
                .filter_map(|r| r.collapsed_at_tick)
                .min();
            if let Some(tick) = first_collapse {
                let day = tick as f64 / TICKS_PER_DAY;
                let first_region = self.regions.iter()
                    .find(|r| r.collapsed_at_tick == Some(tick));
                if let Some(region) = first_region {
                    tips.push(format!(
                        "{} collapsed first on day {day:.1}. Earlier intervention there might have bought time.",
                        region.name
                    ));
                }
            }
        }

        // Fallback — reference something specific about the run
        if tips.is_empty() {
            let days = self.tick as f64 / TICKS_PER_DAY;
            tips.push(format!("Lasted {days:.1} days. Faster research and deployment is key."));
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
        // Pathogen Suppression: fully known diseases, when tech is unlocked
        if self.unlocked_techs.contains(&BasicTech::PathogenSuppression) {
            for (i, disease) in self.diseases.iter().enumerate() {
                if disease.knowledge >= KNOWLEDGE_FULL && self.disease_has_infected(i) {
                    let kind = ResearchKind::SuppressPathogen { disease_idx: i };
                    if !active_kinds.contains(&&kind) {
                        projects.push(kind);
                    }
                }
            }
        }
        // Directed Attenuation: fully known diseases, when tech is unlocked
        if self.unlocked_techs.contains(&BasicTech::DirectedAttenuation) {
            for (i, disease) in self.diseases.iter().enumerate() {
                if disease.knowledge >= KNOWLEDGE_FULL && self.disease_has_infected(i) {
                    let kind = ResearchKind::AttenuatePathogen { disease_idx: i };
                    if !active_kinds.contains(&&kind) {
                        projects.push(kind);
                    }
                }
            }
        }
        // Genomic Interdiction: fully known diseases with cross-region spread, when tech is unlocked
        if self.unlocked_techs.contains(&BasicTech::GenomicInterdiction) {
            for (i, disease) in self.diseases.iter().enumerate() {
                if disease.knowledge >= KNOWLEDGE_FULL
                    && self.disease_has_infected(i)
                    && disease.cross_region_spread > 0.0
                {
                    let kind = ResearchKind::InterdictPathogen { disease_idx: i };
                    if !active_kinds.contains(&&kind) {
                        projects.push(kind);
                    }
                }
            }
        }
        // Field Operations: send teams to stabilize degraded infrastructure.
        // Appears when any infrastructure system drops below INFRA_STRESSED.
        // Only one field ops per region+system pair at a time.
        for (r_idx, region) in self.regions.iter().enumerate() {
            if region.collapsed || self.is_abandoned(r_idx) { continue; }
            for system in [InfraSystem::Healthcare, InfraSystem::SupplyLines, InfraSystem::CivilOrder] {
                let level = match system {
                    InfraSystem::Healthcare => region.healthcare_capacity,
                    InfraSystem::SupplyLines => region.supply_lines,
                    InfraSystem::CivilOrder => region.civil_order,
                };
                if level < INFRA_STRESSED {
                    let kind = ResearchKind::FieldOperations { region_idx: r_idx, system };
                    if !active_kinds.contains(&&kind) {
                        projects.push(kind);
                    }
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
    /// - RapidSequencing halves GenomicSequencing duration.
    /// - PredictiveSurveillance cuts IdentifyThreat, ClinicalTrial, and FieldOperations by 25%.
    ///   (Does not affect GenomicSequencing — already covered by RapidSequencing.)
    ///   Corp health modifier (25% → 35%) tracked in #1381.
    /// - AutomatedSynthesis cuts ManufactureDoses duration by 35%.
    pub fn effective_costs(&self, kind: &ResearchKind) -> (u32, f64, f64) {
        let (personnel, mut duration, funding) = kind.costs(&self.medicines);
        if matches!(kind, ResearchKind::GenomicSequencing { .. })
            && self.unlocked_techs.contains(&BasicTech::RapidSequencing)
        {
            duration *= 0.5;
        }
        if matches!(
            kind,
            ResearchKind::IdentifyThreat { .. }
                | ResearchKind::ClinicalTrial { .. }
                | ResearchKind::FieldOperations { .. }
        ) && self.unlocked_techs.contains(&BasicTech::PredictiveSurveillance)
        {
            duration *= 0.75;
        }
        if matches!(kind, ResearchKind::ManufactureDoses { .. })
            && self.unlocked_techs.contains(&BasicTech::AutomatedSynthesis)
        {
            duration *= 0.65; // 35% faster
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

    /// Research speed multiplier from lab infrastructure (1.0 / 1.3 / 1.6).
    pub fn lab_speed_multiplier(&self) -> f64 {
        match self.lab_level {
            0 => 1.0,
            1 => 1.3,
            _ => 1.6,
        }
    }

    /// Human-readable name for the current lab level.
    pub fn lab_level_name(&self) -> &'static str {
        match self.lab_level {
            0 => "Standard Lab",
            1 => "Enhanced Sequencing",
            _ => "Advanced Genomics",
        }
    }

    /// Resistance buildup multiplier. CombinationTherapy tech halves it.
    pub fn resistance_multiplier(&self) -> f64 {
        if self.unlocked_techs.contains(&BasicTech::CombinationTherapy) {
            0.5
        } else {
            1.0
        }
    }

    // -- Regional specialization bonuses --
    // Each region provides a unique passive bonus while it hasn't collapsed.
    // Losing a region means losing its specialization permanently.

    /// North America: Applied research hub. +20% applied research speed.
    pub fn applied_research_bonus(&self) -> f64 {
        if !self.regions[0].collapsed { 1.2 } else { 1.0 }
    }

    /// South America: Field research expertise. +20% field research speed.
    pub fn field_research_bonus(&self) -> f64 {
        if !self.regions[1].collapsed { 1.2 } else { 1.0 }
    }

    /// Europe: Manufacturing capacity. +20% bonus doses from manufacturing.
    /// DistributedStorage tech adds an additional 25% multiplier (stacks multiplicatively).
    pub fn manufacturing_yield_bonus(&self) -> f64 {
        let base = if !self.regions[2].collapsed { 1.2 } else { 1.0 };
        let tech_bonus = if self.unlocked_techs.contains(&BasicTech::DistributedStorage) {
            1.25
        } else {
            1.0
        };
        base * tech_bonus
    }

    /// Africa: Basic research networks. +20% basic research speed.
    pub fn basic_research_bonus(&self) -> f64 {
        if !self.regions[3].collapsed { 1.2 } else { 1.0 }
    }

    /// Asia: Supply chain efficiency. 20% cheaper medicine deployment.
    pub fn deployment_cost_bonus(&self) -> f64 {
        if !self.regions[4].collapsed { 0.8 } else { 1.0 }
    }

    /// Actual medicine deploy cost for a specific (medicine, region) pair.
    /// Applies disruption multiplier and regional deployment cost bonus.
    /// Use this for both UI affordability preview and engine-side validation
    /// so they can never drift apart.
    pub fn medicine_deploy_cost(&self, medicine_idx: usize, region_idx: usize) -> f64 {
        let base = self.medicines[medicine_idx].deploy_cost();
        let disruption_mult = if self.regions[region_idx].is_disrupted(self.tick) {
            DISRUPTION_MEDICINE_COST_MULT
        } else {
            1.0
        };
        base * disruption_mult * self.deployment_cost_bonus()
    }

    /// Oceania: Clinical trial infrastructure. +25% faster clinical trials.
    pub fn clinical_trial_bonus(&self) -> f64 {
        if !self.regions[5].collapsed { 0.75 } else { 1.0 }
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

    /// Diseases that are identified but cannot yet be developed in Applied Research, with reasons.
    /// Returns (disease_idx, reason_string) for each blocked disease.
    /// Used to show greyed-out "pending" entries in the Applied Research panel.
    pub fn blocked_medicine_developments(&self) -> Vec<(usize, String)> {
        let active_kind = self.applied_research.as_ref().map(|p| &p.kind);
        let has_targeted_drug_design = self.unlocked_techs.contains(&BasicTech::TargetedDrugDesign);

        // Collect disease indices already covered by an available or active targeted medicine
        // development option. (The global BroadSpectrum medicine is always unlocked from game
        // start and therefore appears only in ManufactureDoses, never in DevelopMedicine — it
        // cannot pollute this set. Prion medicines have therapy_type BroadSpectrum but target
        // only one disease and are skipped in the main loop below, so they have no effect.)
        let mut covered: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for kind in self.available_applied_projects() {
            if let ResearchKind::DevelopMedicine { medicine_idx } = kind {
                for &d_idx in &self.medicines[medicine_idx].target_diseases {
                    covered.insert(d_idx);
                }
            }
        }
        if let Some(ResearchKind::DevelopMedicine { medicine_idx }) = active_kind {
            for &d_idx in &self.medicines[*medicine_idx].target_diseases {
                covered.insert(d_idx);
            }
        }

        let mut result = Vec::new();
        let mut seen_diseases: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for med in &self.medicines {
            if med.unlocked || med.therapy_type == TherapyType::BroadSpectrum {
                continue;
            }
            let disease_idx = match med.target_diseases.first().copied() {
                Some(d) => d,
                None => continue,
            };
            if seen_diseases.contains(&disease_idx) || covered.contains(&disease_idx) {
                continue;
            }
            let disease = match self.diseases.get(disease_idx) {
                Some(d) => d,
                None => continue,
            };
            if disease.knowledge <= 0.0 {
                continue; // Not identified at all — no entry until player starts Field Research
            }
            seen_diseases.insert(disease_idx);

            let has_full_knowledge = disease.knowledge >= KNOWLEDGE_FOR_TARGETED;
            let reason = match (has_full_knowledge, has_targeted_drug_design) {
                (true, false) => "Targeted Drug Design required [Basic Research]".to_string(),
                (false, false) => format!(
                    "study {:.0}% complete · Targeted Drug Design required",
                    disease.knowledge * 100.0
                ),
                (false, true) => format!(
                    "study {:.0}% complete · continue Field Research",
                    disease.knowledge * 100.0
                ),
                (true, true) => continue, // Should be in covered — skip defensively
            };

            result.push((disease_idx, reason));
        }
        result
    }

}

/// Format a number for display: 1234 → "1.2K", 1234567 → "1.2M", etc.
pub fn format_number(n: f64) -> String {
    let abs = n.abs();
    if abs < 0.5 {
        return "0".to_string();
    }
    if abs >= 999_999_500.0 {
        format!("{:.1}B", n / 1_000_000_000.0)
    } else if abs >= 999_950.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if abs >= 999.5 {
        format!("{:.1}K", n / 1_000.0)
    } else {
        format!("{:.0}", n)
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
        // Broad-spectrum starts unlocked; all others start locked
        let broad = state.medicines.last().unwrap();
        assert!(broad.unlocked, "broad-spectrum should start unlocked");
        assert!(state.medicines.iter()
            .filter(|m| m.therapy_type != TherapyType::BroadSpectrum)
            .all(|m| !m.unlocked),
            "targeted medicines should start locked");
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

        // Broad-spectrum: weak efficacy — a bandaid, not a cure
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::RnaVirus), 0.15);
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::Bacterium), 0.15);

        // Prions resist everything
        assert_eq!(TherapyType::Antiviral.efficacy(&PathogenType::Prion), 0.0);
        assert_eq!(TherapyType::Antibiotic.efficacy(&PathogenType::Prion), 0.0);
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::Prion), 0.05);
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
        // Reset broad-spectrum to locked so we can test the knowledge-gate logic
        for med in &mut state.medicines {
            if med.therapy_type == TherapyType::BroadSpectrum {
                med.unlocked = false;
                med.doses = 0.0;
            }
        }
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
        // Broad-spectrum starts unlocked — mark it as deployed too so it doesn't
        // trigger the "never deployed" tip
        for med in &mut state.medicines {
            if med.therapy_type == TherapyType::BroadSpectrum {
                med.deployed_count = 1;
            }
        }
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

    #[test]
    fn predictive_surveillance_prereq_requires_rapid_sequencing() {
        let mut state = GameState::new_default(42);
        // Without RapidSequencing, prereq is not met
        assert!(!state.unlocked_techs.contains(&BasicTech::RapidSequencing));
        assert!(!BasicTech::PredictiveSurveillance.prerequisites_met(&state));
        // After unlocking RapidSequencing, prereq is met
        state.unlocked_techs.push(BasicTech::RapidSequencing);
        assert!(BasicTech::PredictiveSurveillance.prerequisites_met(&state));
    }

    #[test]
    fn predictive_surveillance_reduces_identify_threat_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::IdentifyThreat { disease_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::PredictiveSurveillance);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.75).abs() < 0.01,
            "IdentifyThreat should be 25% faster: expected {}, got {}",
            base_duration * 0.75,
            fast_duration
        );
    }

    #[test]
    fn predictive_surveillance_reduces_clinical_trial_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::PredictiveSurveillance);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.75).abs() < 0.01,
            "ClinicalTrial should be 25% faster: expected {}, got {}",
            base_duration * 0.75,
            fast_duration
        );
    }

    #[test]
    fn predictive_surveillance_reduces_field_operations_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::FieldOperations { region_idx: 0, system: InfraSystem::Healthcare };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::PredictiveSurveillance);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.75).abs() < 0.01,
            "FieldOperations should be 25% faster: expected {}, got {}",
            base_duration * 0.75,
            fast_duration
        );
    }

    #[test]
    fn predictive_surveillance_does_not_affect_genomic_sequencing() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::GenomicSequencing { disease_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::PredictiveSurveillance);
        let (_, after_duration, _) = state.effective_costs(&kind);
        assert!(
            (base_duration - after_duration).abs() < 0.01,
            "GenomicSequencing should not be affected by PredictiveSurveillance"
        );
    }

    #[test]
    fn predictive_surveillance_appears_in_all_after_rapid_sequencing() {
        let all = BasicTech::all();
        let rs_pos = all.iter().position(|t| *t == BasicTech::RapidSequencing).unwrap();
        let ps_pos = all.iter().position(|t| *t == BasicTech::PredictiveSurveillance).unwrap();
        assert!(
            ps_pos == rs_pos + 1,
            "PredictiveSurveillance should appear immediately after RapidSequencing in all()"
        );
    }

    #[test]
    fn automated_synthesis_prereq_requires_developed_medicine() {
        let mut state = GameState::new_default(42);
        // No targeted medicines unlocked yet (broad-spectrum starts unlocked but has no mechanism)
        assert!(state.medicines.iter().all(|m| m.mechanism.is_none() || !m.unlocked));
        assert!(!BasicTech::AutomatedSynthesis.prerequisites_met(&state));
        // After a targeted medicine is developed, prereq is met
        // Find first medicine with a mechanism
        let idx = state.medicines.iter().position(|m| m.mechanism.is_some()).unwrap();
        state.medicines[idx].unlocked = true;
        assert!(BasicTech::AutomatedSynthesis.prerequisites_met(&state));
    }

    #[test]
    fn distributed_storage_prereq_requires_automated_synthesis() {
        let mut state = GameState::new_default(42);
        assert!(!BasicTech::DistributedStorage.prerequisites_met(&state));
        state.unlocked_techs.push(BasicTech::AutomatedSynthesis);
        assert!(BasicTech::DistributedStorage.prerequisites_met(&state));
    }

    #[test]
    fn automated_synthesis_reduces_manufacture_doses_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::ManufactureDoses { medicine_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::AutomatedSynthesis);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.65).abs() < 0.01,
            "ManufactureDoses should be 35% faster with AutomatedSynthesis: expected {}, got {}",
            base_duration * 0.65,
            fast_duration
        );
    }

    #[test]
    fn automated_synthesis_does_not_affect_develop_medicine_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::DevelopMedicine { medicine_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::AutomatedSynthesis);
        let (_, after_duration, _) = state.effective_costs(&kind);
        assert!(
            (base_duration - after_duration).abs() < 0.01,
            "DevelopMedicine should not be affected by AutomatedSynthesis"
        );
    }

    #[test]
    fn distributed_storage_boosts_manufacturing_yield() {
        let mut state = GameState::new_default(42);
        // Base: Europe alive = 1.2
        let base = state.manufacturing_yield_bonus();
        assert!((base - 1.2).abs() < 0.001, "base yield should be 1.2 with Europe alive");
        state.unlocked_techs.push(BasicTech::DistributedStorage);
        let with_tech = state.manufacturing_yield_bonus();
        assert!(
            (with_tech - 1.2 * 1.25).abs() < 0.001,
            "DistributedStorage should stack multiplicatively with Europe: expected {}, got {}",
            1.2 * 1.25,
            with_tech
        );
    }

    #[test]
    fn distributed_storage_boost_applies_without_europe() {
        let mut state = GameState::new_default(42);
        state.regions[2].collapsed = true; // collapse Europe
        state.unlocked_techs.push(BasicTech::DistributedStorage);
        let bonus = state.manufacturing_yield_bonus();
        assert!(
            (bonus - 1.25).abs() < 0.001,
            "DistributedStorage alone should give 1.25x yield: got {}",
            bonus
        );
    }

    #[test]
    fn automated_synthesis_and_distributed_storage_appear_in_all() {
        let all = BasicTech::all();
        assert!(all.contains(&BasicTech::AutomatedSynthesis), "AutomatedSynthesis must be in all()");
        assert!(all.contains(&BasicTech::DistributedStorage), "DistributedStorage must be in all()");
        let as_pos = all.iter().position(|t| *t == BasicTech::AutomatedSynthesis).unwrap();
        let ds_pos = all.iter().position(|t| *t == BasicTech::DistributedStorage).unwrap();
        assert!(ds_pos > as_pos, "DistributedStorage should appear after AutomatedSynthesis");
    }
}
