use std::collections::{HashMap, VecDeque};

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
    /// Per-subsystem RNG streams, each seeded deterministically from the master
    /// seed. Splitting prevents one subsystem's draw count from shifting another's
    /// sequence, so e.g. disease #1 emerges on the same tick regardless of how
    /// many spread-noise draws occurred.
    pub rng_spread: ChaCha8Rng,
    pub rng_emergence: ChaCha8Rng,
    pub rng_crisis: ChaCha8Rng,
    pub rng_research: ChaCha8Rng,
    /// Catch-all for contracts, corporations, medicine adverse checks, operations.
    pub rng_misc: ChaCha8Rng,
    pub resources: Resources,
    pub regions: Vec<Region>,
    pub diseases: Vec<Disease>,
    #[serde(default)]
    pub medicines: Vec<Medicine>,
    /// All active research projects (flat list).
    /// No capacity limits — personnel and funding are the only constraints.
    #[serde(default)]
    pub active_research: Vec<ResearchProject>,
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
    /// Tracks which medicines have already fired an AutoDeployBlocked event
    /// this session, to avoid spamming the log every tick.
    #[serde(skip)]
    pub auto_deploy_blocked_notified: std::collections::HashSet<usize>,
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
    /// Per-project auto-repeat: when a repeatable project (TrainPersonnel,
    /// ManufactureDoses) completes, automatically restart it if its kind is in
    /// this set.
    #[serde(default)]
    pub auto_repeat_research: Vec<ResearchKind>,
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
    #[serde(default)]
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
    /// How many times each template_id has been declined (indexed by template_id).
    /// Used to escalate re-offer prices.
    #[serde(default)]
    pub contract_decline_counts: Vec<u8>,
    /// Regional corporations. 3 per region (18 total). Source of player income.
    #[serde(default)]
    pub corporations: Vec<Corporation>,
    /// Player's stock portfolio — shares owned per corporation index.
    #[serde(default)]
    pub portfolio: Vec<u32>,
    /// Total cost basis per corporation index (what the player actually paid).
    /// Used for accurate P/L display in the ledger.
    #[serde(default)]
    pub cost_basis: Vec<f64>,
    /// Named board members with individual satisfaction. Generated at game start
    /// from board-seat corporations and selected governors.
    #[serde(default)]
    pub board_members: Vec<BoardMember>,
    /// Tick when the next scheduled board meeting should fire.
    /// Board meetings are proactive, recurring events on a fixed schedule (~every 7-10 days).
    #[serde(default)]
    pub next_board_meeting_tick: u64,
    /// Fixed per-tick budget set by the board. Updated at board meetings based on
    /// overall satisfaction. Between meetings this is constant — income doesn't
    /// fluctuate with regional health. Contracts add on top of this.
    #[serde(default)]
    pub board_budget_per_tick: f64,
    /// The base board budget at game start (before GDP decline). Used as
    /// the reference point for satisfaction scaling so that the satisfaction
    /// multiplier operates on a stable base rather than a shrinking GDP-derived one.
    #[serde(default)]
    pub reference_base_budget_per_tick: f64,
    /// Tick when the Chairman's satisfaction first dropped below the hostile threshold (0.20).
    /// Reset to None when satisfaction recovers. Used to trigger Vote of No Confidence
    /// after ~3 consecutive days of hostility.
    #[serde(default)]
    pub chairman_hostile_since: Option<u64>,
    /// Monotonically increasing counter for assigning sequence group IDs to
    /// wave-coordinated diseases. Incremented each time a new group is created.
    #[serde(default)]
    pub next_sequence_group: u32,
    /// Active emergency loans. Interest accrues each day; hostile action fires if unpaid.
    #[serde(default)]
    pub loans: Vec<ActiveLoan>,
    /// Cumulative policy spending (sum of per-tick policy costs over the game).
    /// Used by embezzlement detection to compare against non-board stock positions.
    #[serde(default)]
    pub cumulative_policy_spending: f64,
    /// Whether the board has sent the formal embezzlement warning letter.
    #[serde(default)]
    pub embezzlement_warned: bool,
    pub ui: UiState,
}

/// A point-in-time snapshot for dashboard sparkline charts.
/// Values are player-visible estimates (screened/detected), not ground truth.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub tick: u64,
    /// Screened infected count (visibility depends on screening policy level).
    pub screened_infected: f64,
    /// Dead from detected diseases only (unidentified diseases not counted).
    pub detected_dead: f64,
}

/// Record a history snapshot every this many ticks (~1 hour of game time).
pub const HISTORY_INTERVAL: u64 = 5;
/// Maximum history entries to retain (covers ~4 days at 5-tick intervals).
pub const HISTORY_MAX: usize = 100;


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
/// Wave clustering window: a recent spawn within this many ticks boosts emergence.
/// ~3.3 days — long enough for 2–3 disease clusters, short enough to feel like waves.
pub const WAVE_CLUSTER_WINDOW_TICKS: u64 = (3.3 * TICKS_PER_DAY) as u64;

// Economy constants — single source of truth.
pub const BASE_FUNDING_INCOME: f64 = 5.4;
/// Per-tick cost for each personnel on the roster (busy or idle).
/// 20 personnel × 0.06 = $1.2/tick = $144/day upkeep vs ~$648/day base income.
/// With 2 contracts (~¥360/day), gross ~¥1008/day → ~¥864/day net.
/// History: 0.10 made training a trap (50% of income); 0.03 doubled income, trivializing economy.
pub const PERSONNEL_UPKEEP_COST: f64 = 0.06;
pub const TRAVEL_BAN_COST: f64 = 0.7;
pub const TRAVEL_BAN_PERSONNEL: u32 = 3;
pub const QUARANTINE_COST: f64 = 0.6;
pub const QUARANTINE_PERSONNEL: u32 = 3;
pub const DISCOURAGE_HOSP_COST: f64 = 0.0;
pub const DISCOURAGE_HOSP_PERSONNEL: u32 = 0;
/// Baseline hospital exposure increases within-region spread by 25%.
/// Discourage Hospitalization removes this penalty entirely.
pub const HOSPITAL_EXPOSURE_FACTOR: f64 = 1.25;
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
/// Delivery throughput reduction per collapsed neighbor (multiplicative).
/// Each collapsed neighbor reduces throughput by this fraction.
/// E.g., 0.15 means one collapsed neighbor → 85% throughput, two → 72%.
pub const COLLAPSE_THROUGHPUT_PENALTY_PER_NEIGHBOR: f64 = 0.15;
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
/// and reduces disease spread (higher tiers = greater reduction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum ScreeningLevel {
    #[default]
    None,
    /// Rough infected estimates. Cheap but inaccurate.
    Basic,
    /// Reveals infected + immune counts with moderate accuracy.
    Antigen,
    /// Near-complete data on infected/immune AND reduces disease spread.
    MassRapid,
}

// Emergency Decree constants — permanent, irreversible global decisions.
pub const DECREE_COUNT: usize = 6;
/// Number of standing orders shown in the Orders panel.
/// Must equal the length of the `standing_orders` array in `ui/operations.rs`.
/// Used by `ui::operations::selection_max()` to bound navigation — if you add
/// a standing order in operations.rs without updating this constant, the new
/// item will be silently unreachable via keyboard navigation.
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
/// Emergency Countermeasure: within-region spread multiplier applied to all diseases.
pub const COUNTERMEASURE_SPREAD_WITHIN_MULT: f64 = 0.50;
/// Emergency Countermeasure: cross-region spread multiplier applied to all diseases.
pub const COUNTERMEASURE_SPREAD_MULT: f64 = 0.25;
/// The degree to which the board leverages its economic and political influence
/// to enable the player to act in the world. Board members collectively represent
/// a significant chunk of the global economic elite; Authority reflects their
/// willingness to publicly and privately support the player's directives.
/// Authority is decided at board meetings, not per-tick — it can only change
/// by one level per meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Authority {
    Minimal,
    VeryLow,
    Low,
    Medium,
    High,
    Maximum,
}

impl Authority {
    /// Raise by one level (capped at Maximum).
    pub fn raise(self) -> Self {
        match self {
            Self::Minimal => Self::VeryLow,
            Self::VeryLow => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Maximum,
            Self::Maximum => Self::Maximum,
        }
    }

    /// Lower by one level (floored at Minimal).
    pub fn lower(self) -> Self {
        match self {
            Self::Minimal => Self::Minimal,
            Self::VeryLow => Self::Minimal,
            Self::Low => Self::VeryLow,
            Self::Medium => Self::Low,
            Self::High => Self::Medium,
            Self::Maximum => Self::High,
        }
    }

    /// Human-readable label for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimal => "Minimal",
            Self::VeryLow => "Very Low",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Maximum => "Maximum",
        }
    }

    /// Personnel trickle rate (per day) at each authority level.
    pub fn personnel_per_day(self) -> f64 {
        match self {
            Self::Minimal => 0.0,
            Self::VeryLow => 0.5,
            Self::Low => 1.0,
            Self::Medium => 2.0,
            Self::High => 3.0,
            Self::Maximum => 5.0,
        }
    }
}

impl Default for Authority {
    fn default() -> Self {
        Self::Minimal
    }
}

// ── Typed domain identifiers ────────────────────────────────────────────────
// These enums replace the raw `usize` indices previously used to identify
// policies, decrees, and standing orders throughout the codebase. Each enum
// is the single source of truth for its domain: display names, costs,
// authority requirements, and unlock conditions all live as methods here.

/// Typed identifier for each policy available per region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PolicyId {
    TravelBan,
    Quarantine,
    DiscourageHosp,
    BorderControls,
    WaterSanitation,
    BasicScreening,
    AntigenScreening,
    MassRapidScreen,
    MartialLaw,
    NuclearOption,
    FieldHospital,
    IntelStation,
}

impl PolicyId {
    /// All policies in index order (matching the historical 0–11 mapping).
    pub const ALL: [PolicyId; POLICY_COUNT] = [
        PolicyId::TravelBan,
        PolicyId::Quarantine,
        PolicyId::DiscourageHosp,
        PolicyId::BorderControls,
        PolicyId::WaterSanitation,
        PolicyId::BasicScreening,
        PolicyId::AntigenScreening,
        PolicyId::MassRapidScreen,
        PolicyId::MartialLaw,
        PolicyId::NuclearOption,
        PolicyId::FieldHospital,
        PolicyId::IntelStation,
    ];

    /// Display order — grouped by function for the policy panel UI.
    pub const DISPLAY_ORDER: [PolicyId; POLICY_COUNT] = [
        PolicyId::BasicScreening,
        PolicyId::AntigenScreening,
        PolicyId::MassRapidScreen,
        PolicyId::BorderControls,
        PolicyId::WaterSanitation,
        PolicyId::DiscourageHosp,
        PolicyId::TravelBan,
        PolicyId::Quarantine,
        PolicyId::IntelStation,
        PolicyId::FieldHospital,
        PolicyId::NuclearOption,
        PolicyId::MartialLaw,
    ];

    /// Short display name for UI and status messages.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::TravelBan => "Travel Ban",
            Self::Quarantine => "Quarantine",
            Self::DiscourageHosp => "Discourage Hospitalization",
            Self::BorderControls => "Border Controls",
            Self::WaterSanitation => "Water Sanitation",
            Self::BasicScreening => "Basic Screening",
            Self::AntigenScreening => "Antigen Screening",
            Self::MassRapidScreen => "Mass Rapid Screen",
            Self::MartialLaw => "Martial Law",
            Self::NuclearOption => "Nuclear Option",
            Self::FieldHospital => "Field Hospital",
            Self::IntelStation => "Intel Station",
        }
    }

    /// Minimum Authority level required to activate this policy.
    /// `None` means always available (no authority gate).
    pub fn authority_requirement(self) -> Option<Authority> {
        match self {
            Self::TravelBan => Some(Authority::Medium),
            Self::Quarantine => Some(Authority::Medium),
            Self::DiscourageHosp => Some(Authority::Low),
            Self::BorderControls => None,
            Self::WaterSanitation => None,
            Self::BasicScreening => None,
            Self::AntigenScreening => Some(Authority::Low),
            Self::MassRapidScreen => Some(Authority::Medium),
            Self::MartialLaw => Some(Authority::High),
            Self::NuclearOption => Some(Authority::High),
            Self::FieldHospital => Some(Authority::Medium),
            Self::IntelStation => None,
        }
    }

    /// BasicTech prerequisite for this policy, if any.
    pub fn research_prerequisite(self) -> Option<BasicTech> {
        match self {
            Self::AntigenScreening => Some(BasicTech::RapidSequencing),
            Self::MassRapidScreen => Some(BasicTech::MetagenomicSurveillance),
            _ => None,
        }
    }

    /// Whether this is a screening-tier policy (backed by the ScreeningLevel enum
    /// rather than an individual boolean field).
    pub fn is_screening(self) -> bool {
        matches!(self, Self::BasicScreening | Self::AntigenScreening | Self::MassRapidScreen)
    }

    /// The base screening tier index. Only valid for screening policies.
    pub const SCREENING_BASE: PolicyId = PolicyId::BasicScreening;
}

/// Typed identifier for each emergency decree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecreeId {
    ConscriptResearchers,
    AuthorizeHumanTrials,
    SacrificeRegion,
    SuspendRegionalAuthority,
    FortifyRegion,
    EmergencyCountermeasure,
}

impl DecreeId {
    /// All decrees in index order.
    pub const ALL: [DecreeId; DECREE_COUNT] = [
        DecreeId::ConscriptResearchers,
        DecreeId::AuthorizeHumanTrials,
        DecreeId::SacrificeRegion,
        DecreeId::SuspendRegionalAuthority,
        DecreeId::FortifyRegion,
        DecreeId::EmergencyCountermeasure,
    ];

    /// Short display name for UI and status messages.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ConscriptResearchers => "Conscript Researchers",
            Self::AuthorizeHumanTrials => "Authorize Human Trials",
            Self::SacrificeRegion => "Sacrifice Region",
            Self::SuspendRegionalAuthority => "Suspend Regional Authority",
            Self::FortifyRegion => "Fortify Region",
            Self::EmergencyCountermeasure => "Emergency Countermeasure",
        }
    }

    /// Chairman satisfaction cost when this decree is enacted.
    pub fn chairman_cost(self) -> f64 {
        match self {
            Self::ConscriptResearchers => -0.05,
            Self::AuthorizeHumanTrials => -0.10,
            Self::SacrificeRegion => -0.10,
            Self::SuspendRegionalAuthority => -0.15,
            Self::FortifyRegion => -0.10,
            Self::EmergencyCountermeasure => -0.20,
        }
    }

    /// Convert to array index (for backward compat during migration).
    pub fn as_index(self) -> usize {
        match self {
            Self::ConscriptResearchers => 0,
            Self::AuthorizeHumanTrials => 1,
            Self::SacrificeRegion => 2,
            Self::SuspendRegionalAuthority => 3,
            Self::FortifyRegion => 4,
            Self::EmergencyCountermeasure => 5,
        }
    }

    /// Convert from array index. Panics on out-of-range.
    pub fn from_index(idx: usize) -> Self {
        Self::ALL[idx]
    }
}

/// Typed identifier for standing orders (automation rules).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StandingOrderKind {
    AutoQuarantineAtHigh,
    AutoTravelBanAtCrit,
}

impl StandingOrderKind {
    pub const ALL: [StandingOrderKind; STANDING_ORDER_COUNT] = [
        StandingOrderKind::AutoQuarantineAtHigh,
        StandingOrderKind::AutoTravelBanAtCrit,
    ];
}

/// Typed discriminant for FundingCondition type-exclusivity grouping.
/// Contracts with the same category are mutually exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConditionCategory {
    ForbidPolicy,
    RequirePolicy,
    MaxDeaths,
    NoCollapse,
    ForbidDecree,
}


/// Single source of truth for decree unlock conditions.
/// All fields are OR'd — meeting ANY threshold unlocks the decree.
#[derive(Default)]
pub struct DecreeUnlockCondition {
    pub min_infected: Option<f64>,
    pub min_dead: Option<f64>,
    pub min_crit_regions: Option<usize>,
    pub min_collapsed_regions: Option<usize>,
}

impl DecreeUnlockCondition {
    /// Check whether this condition is met given the current game state.
    pub fn is_met(&self, state: &GameState) -> bool {
        if let Some(threshold) = self.min_infected {
            if state.total_infected() >= threshold { return true; }
        }
        if let Some(threshold) = self.min_dead {
            if state.total_dead() >= threshold { return true; }
        }
        if let Some(threshold) = self.min_crit_regions {
            let crit_count = state.regions.iter()
                .filter(|r| !r.collapsed && r.infections.iter().any(|i| i.infected > SEVERITY_CRIT_THRESHOLD))
                .count();
            if crit_count >= threshold { return true; }
        }
        if let Some(threshold) = self.min_collapsed_regions {
            let collapsed_count = state.regions.iter().filter(|r| r.collapsed).count();
            if collapsed_count >= threshold { return true; }
        }
        false
    }

    /// Human-readable description of the unlock condition.
    /// Region-based conditions are listed first (more actionable for the player),
    /// followed by raw population thresholds.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(v) = self.min_infected {
            parts.push(format!("{}+ infected", Self::format_threshold(v)));
        }
        if let Some(v) = self.min_crit_regions {
            if v == 1 {
                parts.push(format!(
                    "any region at CRITICAL ({}+ infected)",
                    Self::format_threshold(SEVERITY_CRIT_THRESHOLD)
                ));
            } else {
                parts.push(format!("{}+ regions at CRITICAL", v));
            }
        }
        if let Some(v) = self.min_collapsed_regions {
            parts.push(format!("{}+ regions collapsed", v));
        }
        if let Some(v) = self.min_dead {
            parts.push(format!("{}+ dead", Self::format_threshold(v)));
        }
        parts.join(" or ")
    }

    fn format_threshold(v: f64) -> String {
        if v >= 1_000_000_000.0 {
            format!("{}B", (v / 1_000_000_000.0) as u64)
        } else if v >= 1_000_000.0 {
            format!("{}M", (v / 1_000_000.0) as u64)
        } else if v >= 1_000.0 {
            format!("{}K", (v / 1_000.0) as u64)
        } else {
            format!("{}", v as u64)
        }
    }
}

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

    /// Whether this screening level detects exposed (incubating) individuals.
    /// Without antigen-level testing, exposed people show no symptoms and are invisible.
    pub fn shows_exposed(&self) -> bool {
        matches!(self, ScreeningLevel::Antigen | ScreeningLevel::MassRapid)
    }

    /// Spread reduction factor (1.0 = no reduction, lower = less spread).
    /// Any level of screening identifies and isolates cases, reducing transmission.
    pub fn spread_factor(&self) -> f64 {
        match self {
            ScreeningLevel::None => 1.0,
            ScreeningLevel::Basic => 0.90,      // 10% spread reduction
            ScreeningLevel::Antigen => 0.80,    // 20% spread reduction
            ScreeningLevel::MassRapid => 0.70,  // 30% spread reduction
        }
    }

    /// Medicine targeting efficiency at full screening progress.
    /// Without surveillance, doses are wasted on the wrong people.
    /// Returns the fraction of delivered doses that reach valid targets.
    pub fn targeting_efficiency(&self) -> f64 {
        match self {
            ScreeningLevel::None => 0.50,       // 50% waste — blind deployment
            ScreeningLevel::Basic => 0.75,      // 25% waste
            ScreeningLevel::Antigen => 0.90,    // 10% waste
            ScreeningLevel::MassRapid => 1.0,   // no waste — perfect targeting
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
    /// Discourages hospitalization: +50% lethality (no hospital care)
    /// but reduces spread by 20% (no hospital exposure).
    pub discourage_hosp: bool,
    /// Reduces cross-region spread by 30%, no income penalty.
    /// Cheaper alternative to travel ban. Superseded by travel ban if both active.
    #[serde(default)]
    pub border_controls: bool,
    /// Halves waterborne disease within-region spread. No effect on airborne/contact.
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
///   2 = Discourage Hosp.   7 = Mass Rapid Screen  10 = Field Hospital
///   3 = Border Controls   11 = Intel Station
///   4 = Water Sanitation
///
/// Display position is determined by `PolicyId::DISPLAY_ORDER` (grouped by function).
/// If you add a new policy, you must update:
///   - POLICY_COUNT and PolicyId enum (this file)
///   - get_bool/set_bool if it's a boolean policy (this file)
///   - toggle_policy and tick_enforce_costs (engine/policy.rs)
///   - render_manage policies vec (ui/policy.rs)
pub const POLICY_COUNT: usize = 12;

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


/// Policy display order — grouped by function for the policy panel UI.
/// Delegates to `PolicyId::DISPLAY_ORDER`.
pub fn policy_display_order() -> [PolicyId; POLICY_COUNT] {
    PolicyId::DISPLAY_ORDER
}

impl RegionPolicy {
    /// Per-policy funding costs for each active policy. Returns (PolicyId, cost)
    /// pairs, trait-adjusted. Used by both `funding_cost()` and `tick_enforce_costs()`
    /// to ensure a single source of truth for policy pricing.
    /// Delegates to `bool_policy_cost()` for boolean policies to avoid duplication.
    pub fn active_policy_costs(&self, traits: &[RegionTrait]) -> Vec<(PolicyId, f64)> {
        let mut costs = Vec::new();
        for id in [PolicyId::TravelBan, PolicyId::Quarantine, PolicyId::DiscourageHosp,
                   PolicyId::BorderControls, PolicyId::WaterSanitation, PolicyId::MartialLaw] {
            if self.get_bool(id) {
                costs.push((id, Self::bool_policy_cost(id, traits)));
            }
        }
        let scr_cost = self.screening.funding_cost();
        if scr_cost > 0.0 { costs.push((PolicyId::BasicScreening, scr_cost)); }
        costs
    }

    /// Per-tick funding cost of a single boolean policy, trait-adjusted.
    /// Used by toggle_policy to display the cost when enabling a policy.
    pub fn bool_policy_cost(policy: PolicyId, traits: &[RegionTrait]) -> f64 {
        let trade_dependent = traits.contains(&RegionTrait::TradeDependent);
        match policy {
            PolicyId::TravelBan => if trade_dependent { TRAVEL_BAN_COST * TRADE_DEPENDENT_TRAVEL_BAN_MULT } else { TRAVEL_BAN_COST },
            PolicyId::Quarantine => QUARANTINE_COST,
            PolicyId::DiscourageHosp => DISCOURAGE_HOSP_COST,
            PolicyId::BorderControls => BORDER_CONTROLS_COST,
            PolicyId::WaterSanitation => WATER_SANITATION_COST,
            PolicyId::MartialLaw => MARTIAL_LAW_COST,
            _ => 0.0,
        }
    }

    /// Funding cost adjusted for regional traits.
    /// TradeDependent: travel ban costs 1.5x.
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
        if self.discourage_hosp { cost += DISCOURAGE_HOSP_PERSONNEL; active_count += 1; }
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
        self.travel_ban || self.quarantine || self.discourage_hosp
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
        if self.discourage_hosp { n += 1; }
        if self.border_controls { n += 1; }
        if self.water_sanitation { n += 1; }
        if self.martial_law { n += 1; }
        if self.screening != ScreeningLevel::None { n += 1; }
        n
    }

    /// Whether a policy is currently active. Handles both boolean
    /// policies and screening tiers.
    pub fn is_active(&self, policy: PolicyId) -> bool {
        match policy {
            PolicyId::TravelBan | PolicyId::Quarantine | PolicyId::DiscourageHosp
            | PolicyId::BorderControls | PolicyId::WaterSanitation
            | PolicyId::MartialLaw | PolicyId::NuclearOption => self.get_bool(policy),
            PolicyId::BasicScreening => self.screening >= ScreeningLevel::Basic,
            PolicyId::AntigenScreening => self.screening >= ScreeningLevel::Antigen,
            PolicyId::MassRapidScreen => self.screening >= ScreeningLevel::MassRapid,
            PolicyId::FieldHospital | PolicyId::IntelStation => false,
        }
    }

    pub fn clear_all(&mut self) {
        self.travel_ban = false;
        self.quarantine = false;
        self.discourage_hosp = false;
        self.border_controls = false;
        self.water_sanitation = false;
        self.screening = ScreeningLevel::None;
        self.martial_law = false;
        // nuclear_annihilation is NOT cleared — it's permanent and post-collapse
    }

    /// Access a boolean policy field by typed ID.
    pub fn get_bool(&self, policy: PolicyId) -> bool {
        match policy {
            PolicyId::TravelBan => self.travel_ban,
            PolicyId::Quarantine => self.quarantine,
            PolicyId::DiscourageHosp => self.discourage_hosp,
            PolicyId::BorderControls => self.border_controls,
            PolicyId::WaterSanitation => self.water_sanitation,
            PolicyId::MartialLaw => self.martial_law,
            PolicyId::NuclearOption => self.nuclear_annihilation,
            _ => false,
        }
    }

    /// Set a boolean policy field by typed ID.
    pub fn set_bool(&mut self, policy: PolicyId, val: bool) {
        match policy {
            PolicyId::TravelBan => self.travel_ban = val,
            PolicyId::Quarantine => self.quarantine = val,
            PolicyId::DiscourageHosp => self.discourage_hosp = val,
            PolicyId::BorderControls => self.border_controls = val,
            PolicyId::WaterSanitation => self.water_sanitation = val,
            PolicyId::MartialLaw => self.martial_law = val,
            PolicyId::NuclearOption => self.nuclear_annihilation = val,
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
    /// Suspend Regional Authority: freeze all governor cooperation. No drift, no
    /// defiance, no cooperation bonuses. Board leverage overrides local governance.
    #[serde(default)]
    pub suspend_regional_authority: bool,
    /// Fortify Region: restore one region's infrastructure to 100%, all others
    /// lose 25% across all systems.
    #[serde(default)]
    pub fortified_region: Option<usize>,
    /// Emergency Countermeasure: reduce all disease within-region spread by 50% and
    /// cross-region spread by 75%. Kills 10% of alive population immediately.
    #[serde(default)]
    pub emergency_countermeasure: bool,
}

impl EnactedDecrees {
    pub fn is_enacted(&self, decree: DecreeId) -> bool {
        match decree {
            DecreeId::ConscriptResearchers => self.conscript_researchers,
            DecreeId::AuthorizeHumanTrials => self.authorize_human_trials,
            DecreeId::SacrificeRegion => self.sacrificed_region.is_some(),
            DecreeId::SuspendRegionalAuthority => self.suspend_regional_authority,
            DecreeId::FortifyRegion => self.fortified_region.is_some(),
            DecreeId::EmergencyCountermeasure => self.emergency_countermeasure,
        }
    }
}


/// A condition attached to a funding contract. Checked each tick.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum FundingCondition {
    /// A specific policy must NOT be active in any region.
    ForbidPolicy { policy: PolicyId },
    /// A specific policy must be active in at least one region.
    RequirePolicy { policy: PolicyId },
    /// Total global deaths must stay below this threshold.
    MaxDeaths { threshold: f64 },
    /// All regions must remain standing (revoked on first collapse).
    NoCollapse,
    /// A specific emergency decree must not be enacted while this contract is active.
    /// Once enacted, decrees are permanent. Violating this condition will gradually revoke the contract.
    ForbidDecree { decree: DecreeId },
}

impl FundingCondition {
    pub fn description(&self) -> String {
        match self {
            Self::ForbidPolicy { policy } => {
                format!("Do not use {}", policy.display_name())
            }
            Self::RequirePolicy { policy } => {
                format!("Maintain {} in at least one region", policy.display_name())
            }
            Self::MaxDeaths { threshold } => {
                format!("Global deaths below {}", format_large_number(*threshold))
            }
            Self::NoCollapse => "No region may collapse".to_string(),
            Self::ForbidDecree { decree } => {
                format!("Do not enact {}", decree.display_name())
            }
        }
    }

    /// Return a typed discriminant for type-exclusivity grouping.
    /// Contracts with the same category are mutually exclusive.
    pub fn category(&self) -> ConditionCategory {
        match self {
            Self::ForbidPolicy { .. } => ConditionCategory::ForbidPolicy,
            Self::RequirePolicy { .. } => ConditionCategory::RequirePolicy,
            Self::MaxDeaths { .. } => ConditionCategory::MaxDeaths,
            Self::NoCollapse => ConditionCategory::NoCollapse,
            Self::ForbidDecree { .. } => ConditionCategory::ForbidDecree,
        }
    }

    /// Check whether this condition is currently satisfied.
    pub fn is_met(&self, state: &GameState) -> bool {
        match self {
            Self::ForbidPolicy { policy } => {
                !state.policies.iter().any(|p| p.is_active(*policy))
            }
            Self::RequirePolicy { policy } => {
                state.policies.iter().any(|p| p.is_active(*policy))
            }
            Self::MaxDeaths { threshold } => {
                state.total_dead() < *threshold
            }
            Self::NoCollapse => {
                !state.regions.iter().any(|r| r.collapsed)
            }
            Self::ForbidDecree { decree } => {
                !state.enacted_decrees.is_enacted(*decree)
            }
        }
    }
}

fn default_satisfaction() -> f64 {
    1.0
}

/// A funding contract: income with strings attached, offered by a board member.
/// Accepting pleases the offering member but angers every other board member.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundingContract {
    pub name: String,
    /// Index into `GameState::board_members` identifying who offered this contract.
    #[serde(default)]
    pub board_member_idx: usize,
    /// Per-tick income while contract is active.
    pub income: f64,
    pub condition: FundingCondition,
    /// Unique template index (used to avoid duplicate offers).
    pub template_id: u8,
    /// Contract condition satisfaction (0.0–1.0). Degrades when condition violated, recovers when met.
    #[serde(default = "default_satisfaction")]
    pub satisfaction: f64,
    /// Whether the low-satisfaction warning has fired (resets when satisfaction recovers).
    #[serde(default)]
    pub warned: bool,
    /// Tick when last demand crisis was generated (cooldown tracking).
    #[serde(default)]
    pub last_demand_tick: u64,
    /// Tick when this contract was accepted (added to `contracts` vec).
    #[serde(default)]
    pub accepted_tick: u64,
    /// Whether a loyalty raise has already been offered for this contract.
    #[serde(default)]
    pub loyalty_raise_offered: bool,
    /// Tick when the last patron bonus was granted for this contract.
    #[serde(default)]
    pub last_bonus_tick: u64,
}

/// Contract condition satisfaction thresholds and rates.
pub const CONTRACT_CONDITION_WARN: f64 = 0.5;

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
pub const CONTRACT_CONDITION_REVOKE: f64 = 0.2;
/// Per-tick degradation when condition is violated (~0.05/day = 16 days from 1.0 to revocation).
pub const CONTRACT_DEGRADE_RATE: f64 = 0.05 / 120.0;
/// Per-tick recovery when condition is met (~0.02/day).
pub const CONTRACT_RECOVER_RATE: f64 = 0.02 / 120.0;
/// Minimum ticks between contract demand crises from the same contract (~5 days).
pub const CONTRACT_DEMAND_COOLDOWN: u64 = 600;
/// Loyalty raise multiplier — contract income increases by this fraction (e.g. 0.15 = 15% raise).
pub const LOYALTY_RAISE_FRACTION: f64 = 0.15;

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

    /// Exponent applied to regional `gdp_fraction` when computing a corporation's
    /// competitive capacity in its sector pool. When GDP = 1.0, the exponent has
    /// no effect (1.0^anything = 1.0). When GDP drops, exposed sectors amplify
    /// the damage nonlinearly. Replaces the old 60-value sensitivity matrix.
    pub fn crisis_exposure(&self) -> f64 {
        match self {
            Self::Energy => 0.8,      // Essential, somewhat insulated
            Self::Logistics => 1.4,   // Trade-dependent, amplifies damage
            Self::Biotech => 0.9,     // Essential during pandemic
            Self::Mining => 1.3,      // Labor-dependent, exposed
            Self::DataInfra => 0.6,   // Highly resilient
            Self::Automation => 0.7,  // Robots don't get sick
        }
    }

    /// Maximum bonus percentage this sector provides at full health.
    /// Used by both the engine (to apply the bonus) and UI (to display it).
    pub fn max_bonus_pct(&self) -> f64 {
        match self {
            Self::Energy => 15.0,     // Infrastructure drains reduced
            Self::Logistics => 25.0,  // Medicine delivery faster
            Self::Biotech => 10.0,    // Research speed increased
            Self::Mining => 50.0,     // Infrastructure recovery boosted
            Self::DataInfra => 20.0,  // Screening convergence faster
            Self::Automation => 10.0, // Policy costs reduced
        }
    }

    /// Short label describing what this sector's bonus does.
    pub fn bonus_label(&self) -> &'static str {
        match self {
            Self::Energy => "Infra drain",
            Self::Logistics => "Delivery",
            Self::Biotech => "Research",
            Self::Mining => "Infra recovery",
            Self::DataInfra => "Screening",
            Self::Automation => "Policy cost",
        }
    }

    /// Sign prefix for the bonus display (- for reductions, + for increases).
    fn bonus_sign(&self) -> &'static str {
        match self {
            Self::Energy | Self::Automation => "-",
            _ => "+",
        }
    }

    /// Formatted bonus text showing the effective bonus at the given strength (0.0–1.0).
    pub fn bonus_text(&self, strength: f64) -> String {
        format!("{} {}{:.0}%", self.bonus_label(), self.bonus_sign(), self.max_bonus_pct() * strength)
    }

    /// Returns the name of the policy this sector objects to most, if active.
    /// Returns None if no relevant policy is active or the sector doesn't complain.
    pub fn policy_grievance(&self, policy: &RegionPolicy) -> Option<&'static str> {
        match self {
            Self::Logistics => {
                if policy.travel_ban { Some("travel ban") }
                else if policy.border_controls { Some("border controls") }
                else { None }
            }
            Self::Mining => {
                if policy.quarantine { Some("quarantine") }
                else if policy.martial_law { Some("martial law") }
                else { None }
            }
            Self::Energy => {
                if policy.martial_law { Some("martial law") }
                else if policy.quarantine { Some("quarantine") }
                else { None }
            }
            Self::DataInfra => {
                if policy.martial_law { Some("martial law") }
                else { None }
            }
            Self::Automation => {
                if policy.quarantine { Some("quarantine") }
                else { None }
            }
            // Biotech benefits from pandemic response, rarely complains
            Self::Biotech => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Corporation {
    pub name: String,
    /// Surname of the corporation's director (for board member display).
    pub director_surname: String,
    pub sector: CorporationSector,
    pub region_idx: usize,
    /// Revenue per day at full health (before condition modifiers).
    pub base_revenue: f64,
    /// Current revenue per day (after all modifiers). Updated each tick.
    pub revenue: f64,
    /// Previous day's revenue. Used by stock price model to compute trending signal.
    #[serde(default)]
    pub prev_revenue: f64,
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
    /// Current share price. Derived from revenue performance and market sentiment.
    #[serde(default = "default_share_price")]
    pub share_price: f64,
    /// IPO price at game start. Used for fair value calculation and P/L display.
    #[serde(default = "default_share_price")]
    pub ipo_price: f64,
    /// Price history for sparkline display (last 30 data points, sampled daily).
    #[serde(default)]
    pub price_history: Vec<f64>,
    /// Tick when this corp last fired a CorporateDemand crisis. Per-corp cooldown.
    #[serde(default)]
    pub last_demand_tick: Option<u64>,
}

fn default_share_price() -> f64 {
    100.0
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

    /// Cost to bail out this corporation (refill reserves to max).
    /// Roughly 10 days of operating costs — expensive enough to be a real decision,
    /// cheap enough that saving a critical corporation is sometimes worth it.
    pub fn bailout_cost(&self) -> f64 {
        (self.operating_costs * 10.0).round()
    }

    /// Days until bankruptcy at current burn rate. None if not burning reserves.
    pub fn days_of_runway(&self) -> Option<f64> {
        if self.bankrupt { return None; }
        let profit = self.daily_profit();
        if profit >= 0.0 { return None; }
        Some(self.reserves / (-profit))
    }

    /// Previous day's share price from price history, or IPO price if no history.
    pub fn previous_price(&self) -> f64 {
        if self.price_history.len() >= 2 {
            self.price_history[self.price_history.len() - 2]
        } else {
            self.ipo_price
        }
    }

    /// Stock price change percentage vs previous day.
    pub fn price_change_pct(&self) -> f64 {
        let prev = self.previous_price();
        if prev <= 0.0 { return 0.0; }
        (self.share_price - prev) / prev * 100.0
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

/// Buffer above cumulative policy spending before embezzlement detection triggers.
/// Player can hold up to this much in non-board stocks without raising suspicion.
pub const EMBEZZLEMENT_BUFFER: f64 = 1000.0;

/// Funding income multiplier applied when the player continues investing in
/// non-board corps after receiving the embezzlement warning letter.
pub const EMBEZZLEMENT_FUNDING_PENALTY: f64 = 0.75;

// Board member system — named individuals who sit on the NWHO board.
// Each member has connections to existing game entities (corporations, regions,
// contracts) and individual satisfaction driven by those connections.

/// What role a board member plays, determining what drives their satisfaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardRole {
    /// Owns a board-seat corporation. Satisfaction tracks corp reserves.
    CorporateLeader { corp_idx: usize },
    /// Also a regional governor. Satisfaction tracks regional GDP.
    RegionGovernor { region_idx: usize },
}

/// Personality archetype for corporate board members.
/// Determines what the member cares about beyond pure stock performance.
/// Governor board members do NOT get a BoardPersonality — they have GovernorPersonality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardPersonality {
    /// Pure stock maximizer. The default corporate mindset, amplified.
    Profiteer,
    /// Values R&D and scientific progress over pure profit.
    Technocrat,
    /// Rare corporate leader who actually cares about lives saved.
    Humanitarian,
    /// Transactional. Wants the player to do business with their corporation.
    Dealmaker,
}

impl BoardPersonality {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Profiteer => "Profiteer",
            Self::Technocrat => "Technocrat",
            Self::Humanitarian => "Humanitarian",
            Self::Dealmaker => "Dealmaker",
        }
    }

    /// All variants, for random selection.
    pub const ALL: [BoardPersonality; 4] = [
        Self::Profiteer,
        Self::Technocrat,
        Self::Humanitarian,
        Self::Dealmaker,
    ];

    /// Short description of the chairman-specific power granted by this personality.
    pub fn chairman_effect_description(&self) -> &'static str {
        match self {
            Self::Profiteer => "Chairman effect: Budget swings ±15% (vs ±10%)",
            Self::Technocrat => "Chairman effect: Research costs -10%",
            Self::Humanitarian => "Chairman effect: Approval target +5%",
            Self::Dealmaker => "Chairman effect: Stock trade reactions 2x for all members",
        }
    }
}

/// Source/kind of a satisfaction modifier. Used for display labels and deduplication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModifierSource {
    /// Base disposition: +50% for everyone.
    Base,
    /// Stock price relative to IPO (corporate leaders).
    StockPerformance,
    /// Regional GDP health (governors).
    RegionalGdp,
    /// Research slot utilization (Technocrat personality).
    ResearchUtilization,
    /// Global population survival rate (Humanitarian personality).
    GlobalSurvival,
    /// Whether the player owns shares in the member's corp (Dealmaker personality).
    PlayerInvestment,
    /// Initial distrust of the player, decays over ~30 days.
    InitialSkepticism,
    /// Player bought shares in this member's corp.
    BoughtShares,
    /// Player sold shares of this member's corp.
    SoldShares,
    /// Player invested in a same-sector rival corp.
    RivalInvestment,
    /// GDP-hurting policy enacted in Profiteer's region.
    PolicyEnacted,
    /// Research completed (Technocrat bonus).
    ResearchCompleted,
    /// Contract accepted by player (offerer boost / others penalty).
    ContractAccepted,
    /// Contract refused by player.
    ContractRefused,
    /// Contract canceled by player.
    ContractCanceled,
    /// Crisis resolution effect.
    CrisisEffect,
    /// Contract loyalty bonus (held 10+ days).
    ContractLoyalty,
    /// Relative regional standing vs other regions (Hardliner governor personality).
    RegionalStanding,
    /// Restrictive policy count in governor's region (Blowhard governor personality).
    RestrictivePolicies,
    /// Active contract count (Operative governor personality).
    ActiveContracts,
    /// Funding reserves relative to daily income (Mobster governor personality).
    FundingReserves,
}

impl ModifierSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Base => "Base disposition",
            Self::StockPerformance => "Stock performance",
            Self::RegionalGdp => "Regional GDP",
            Self::ResearchUtilization => "Research utilization",
            Self::GlobalSurvival => "Global survival",
            Self::PlayerInvestment => "Player investment",
            Self::InitialSkepticism => "Initial skepticism",
            Self::BoughtShares => "Bought shares",
            Self::SoldShares => "Sold shares",
            Self::RivalInvestment => "Rival investment",
            Self::PolicyEnacted => "Policy enacted",
            Self::ResearchCompleted => "Research completed",
            Self::ContractAccepted => "Contract accepted",
            Self::ContractRefused => "Contract refused",
            Self::ContractCanceled => "Contract canceled",
            Self::CrisisEffect => "Crisis effect",
            Self::ContractLoyalty => "Contract loyalty",
            Self::RegionalStanding => "Regional standing",
            Self::RestrictivePolicies => "Policy restrictions",
            Self::ActiveContracts => "Active contracts",
            Self::FundingReserves => "Funding reserves",
        }
    }

    /// Whether this modifier is continuously recomputed each tick (not decaying).
    pub fn is_continuous(&self) -> bool {
        matches!(self,
            Self::Base | Self::StockPerformance | Self::RegionalGdp |
            Self::ResearchUtilization | Self::GlobalSurvival | Self::PlayerInvestment |
            Self::RegionalStanding | Self::RestrictivePolicies |
            Self::ActiveContracts | Self::FundingReserves
        )
    }
}

/// A named, visible satisfaction modifier on a board member.
/// Continuous modifiers are cleared and recomputed each tick.
/// Decaying modifiers persist and decay toward 0 over time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SatisfactionModifier {
    pub source: ModifierSource,
    pub value: f64,
}

/// A named individual on the NWHO board of directors.
/// Satisfaction is the clamped sum of all active modifiers — both continuously
/// recomputed ones (Base, Stock, GDP) and event-driven decaying ones (trades, contracts).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardMember {
    /// Display name (e.g., "Dir. Caldwell" or "Gov. Torres").
    pub name: String,
    /// Short title/role description for UI display.
    pub title: String,
    /// Primary role determining satisfaction driver.
    pub role: BoardRole,
    /// Corporation index this member owns (if any). Same as CorporateLeader.corp_idx
    /// when role is CorporateLeader, but governor-members may also own a corp.
    pub corp_idx: Option<usize>,
    /// Region index this member governs (if any). Same as RegionGovernor.region_idx
    /// when role is RegionGovernor, but corp-leaders may be linked to their corp's region.
    pub region_idx: Option<usize>,
    /// Individual satisfaction (0.0–1.0). Sum of all modifiers, clamped.
    pub satisfaction: f64,
    /// Named satisfaction modifiers. Each has a source label and value.
    /// Continuous modifiers are cleared and recomputed each tick.
    /// Event-driven modifiers decay toward 0 over time.
    #[serde(default)]
    pub modifiers: Vec<SatisfactionModifier>,
    /// Whether this member is the Chairman of the Board (2x satisfaction weight).
    #[serde(default)]
    pub is_chairman: bool,
    /// Personality archetype for corporate leaders. None for governor members.
    #[serde(default)]
    pub personality: Option<BoardPersonality>,
}

impl BoardMember {
    /// Add an event-driven modifier (will decay over time).
    pub fn add_modifier(&mut self, source: ModifierSource, value: f64) {
        self.modifiers.push(SatisfactionModifier { source, value });
    }

    /// Sum of all modifier values with the given source.
    pub fn modifier_total(&self, source: &ModifierSource) -> f64 {
        self.modifiers.iter()
            .filter(|m| &m.source == source)
            .map(|m| m.value)
            .sum()
    }
}

pub fn format_large_number(n: f64) -> String {
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
    /// The board's current grant of authority. Decided at board meetings —
    /// can only change by one level per meeting. Starts at Minimal.
    #[serde(default)]
    pub authority: Authority,
    /// Fractional accumulator for authority-based personnel gains.
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

    /// Description of what this priority level does, for the policy panel.
    pub fn hint(self) -> &'static str {
        match self {
            Self::High => "Auto-deploy serves this region first",
            Self::Normal => "Default auto-deploy order",
            Self::Low => "Auto-deploy serves this region last",
            Self::CutOff => "Auto-deploy skips; manual deploy only",
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
/// Each region has 1-2 traits that modify policy costs, spread rates, or collapse thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionTrait {
    /// Travel ban funding cost 1.5x, GDP penalty 30% instead of 20%.
    TradeDependent,
    /// Within-region spread rate +30% (crowded cities, public transit).
    DenseUrban,
    /// Cross-region inbound spread reduced 50% (natural isolation).
    IslandGeography,
    /// All policy personnel costs +1 (harder to staff programs).
    LowInfrastructure,
    /// Baseline lethality -15% (superior hospitals). Lost when Discourage Hospitalization is active.
    StrongPublicHealth,
    /// Collapse threshold -10pp (region endures more before collapsing).
    ResilientPopulation,
}

/// Travel ban cost multiplier for TradeDependent regions.
pub const TRADE_DEPENDENT_TRAVEL_BAN_MULT: f64 = 1.5;

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

    pub fn effect(&self) -> &'static str {
        match self {
            RegionTrait::TradeDependent => "Travel ban costs 1.5x, GDP -30%",
            RegionTrait::DenseUrban => "Within-region spread +30%",
            RegionTrait::IslandGeography => "Cross-region inbound spread -50%",
            RegionTrait::LowInfrastructure => "Policy personnel +1 each",
            RegionTrait::StrongPublicHealth => "Lethality -15% (lost if hospitals discouraged)",
            RegionTrait::ResilientPopulation => "Collapse threshold -10pp",
        }
    }
}

/// Unique per-region specialization that provides a local passive bonus.
/// Lost permanently when the region collapses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionSpecialization {
    /// Medicine deployment to this region costs 30% less.
    PharmaHub,
    /// Healthcare capacity in this region degrades 40% slower.
    TropicalMedicine,
    /// Policy funding costs in this region are 25% lower.
    RegulatoryApparatus,
    /// Civil order in this region degrades 40% slower.
    CommunityNetworks,
    /// Supply lines in this region degrade 40% slower.
    LogisticsHub,
    /// Screening convergence in this region is 50% faster.
    SurveillanceNetwork,
}

impl RegionSpecialization {
    /// Short label shown in the region detail panel.
    pub fn label(&self) -> &'static str {
        match self {
            Self::PharmaHub => "Pharma Hub: Deploy Cost -30%",
            Self::TropicalMedicine => "Tropical Medicine: Healthcare Drain -40%",
            Self::RegulatoryApparatus => "Regulatory Apparatus: Policy Cost -25%",
            Self::CommunityNetworks => "Community Networks: Civil Order Drain -40%",
            Self::LogisticsHub => "Logistics Hub: Supply Drain -40%",
            Self::SurveillanceNetwork => "Surveillance Network: Screening +50%",
        }
    }
}

/// PharmaHub: medicine deployment cost multiplier for the specialized region.
pub const PHARMA_HUB_DEPLOY_DISCOUNT: f64 = 0.7;
/// RegulatoryApparatus: policy funding cost multiplier for the specialized region.
pub const REGULATORY_APPARATUS_COST_MULT: f64 = 0.75;
/// TropicalMedicine: healthcare capacity drain multiplier (lower = slower degradation).
pub const TROPICAL_MEDICINE_HC_DRAIN_MULT: f64 = 0.6;
/// CommunityNetworks: civil order drain multiplier (lower = slower degradation).
pub const COMMUNITY_NETWORKS_CO_DRAIN_MULT: f64 = 0.6;
/// LogisticsHub: supply line drain multiplier (lower = slower degradation).
pub const LOGISTICS_HUB_SL_DRAIN_MULT: f64 = 0.6;
/// SurveillanceNetwork: screening convergence rate multiplier (higher = faster convergence).
pub const SURVEILLANCE_NETWORK_SCREENING_MULT: f64 = 1.5;

/// Governor personality — character archetypes that determine how governors
/// behave when loyal vs defiant. Each type requires a different player response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernorPersonality {
    /// Breaks things by accident. Defiance: randomly deactivates a policy or
    /// wastes funding. Bargain: cheap (public praise) but cooperation decays fast.
    Buffoon,
    /// All noise. Defiance: small funding drain + alarming event messages.
    /// The real danger is wasting resources appeasing someone who'd shut up on their own.
    /// Bargain: small cost, large cooperation gain.
    Blowhard,
    /// Absent. Defiance: doesn't sabotage — just stops enforcing. Policy effects
    /// reduced in the region. Bargain: costs personnel (you send someone to manage).
    Recluse,
    /// Zero-sum nationalist. Sees other regions as competitors, not allies.
    /// Less pliable generally. Defiance: unilaterally activates policies the player
    /// didn't set, costing unbudgeted personnel and funding. Pleased when competing
    /// regions suffer. Bargain: give them authority (high cost).
    Hardliner,
    /// Competent and helpful, always skimming. When loyal, policies more effective.
    /// When defiant, continuous funding drain that grows over time.
    /// Bargain: permanent cut of regional income.
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
/// Cooperation below 40 means defiance (policies less effective).
/// Cooperation above 80 means cooperation bonus (cheaper policies).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Governor {
    pub name: String,
    pub personality: GovernorPersonality,
    /// Cooperation 0-100. Starts at 60-80 depending on personality.
    pub cooperation: f64,
    /// Whether the defiance crisis has already fired for this governor.
    /// Reset when cooperation recovers above defiance threshold.
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
    /// Tick when the governor last had a sick crisis (cooldown tracking).
    #[serde(default)]
    pub last_sick_tick: Option<u64>,
    /// Whether this governor is dead (region becomes leaderless).
    #[serde(default)]
    pub dead: bool,
    /// Tick when a successor governor will arrive (None if governor is alive or no succession scheduled).
    #[serde(default)]
    pub succession_tick: Option<u64>,
}


/// Infection count thresholds for region severity levels.
/// Used by both the UI (status labels) and the engine (governor cooperation drift).
pub const SEVERITY_CRIT_THRESHOLD: f64 = 100_000.0;
pub const SEVERITY_HIGH_THRESHOLD: f64 = 10_000.0;
pub const SEVERITY_MOD_THRESHOLD: f64 = 1_000.0;

/// Cooperation threshold below which the governor becomes defiant.
pub const GOVERNOR_DEFIANCE_THRESHOLD: f64 = 40.0;
/// Cooperation threshold above which the governor provides cooperation bonuses.
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
/// Cooperation gain from appease action.
pub const APPEASE_COOPERATION_GAIN: f64 = 15.0;
/// Ticks between autonomous governor defiance actions (~2 days).
pub const GOVERNOR_ACTION_INTERVAL: u64 = 240;
/// Policy effectiveness when the region is leaderless (governor dead, no successor yet).
pub const LEADERLESS_EFFECTIVENESS: f64 = 0.5;
/// Days until a successor governor arrives after the previous one dies.
pub const GOVERNOR_SUCCESSION_DAYS: f64 = 12.0;
/// Starting cooperation for a successor governor (neutral).
pub const SUCCESSOR_COOPERATION: f64 = 50.0;

/// Bargain cooperation gains by personality.
pub const BARGAIN_COOPERATION_GAIN: f64 = 20.0;
/// Blowhard bargain: large cooperation gain (they're easy to please).
pub const BARGAIN_BLOWHARD_COOPERATION_GAIN: f64 = 30.0;
/// Buffoon bargain cost: small approval cost (public praise).
pub const BARGAIN_BUFFOON_APPROVAL_COST: f64 = 0.05;
/// Blowhard bargain cost: small funding cost.
pub const BARGAIN_BLOWHARD_FUNDING_COST: f64 = 100.0;
/// Recluse bargain cost: personnel (you send someone to physically manage).
pub const BARGAIN_RECLUSE_PERSONNEL_COST: u32 = 2;
/// Hardliner bargain cost: high funding (give them authority).
pub const BARGAIN_HARDLINER_FUNDING_COST: f64 = 400.0;
/// Operative bargain cost: fraction of regional income permanently skimmed.
pub const BARGAIN_OPERATIVE_INCOME_CUT: f64 = 0.10;
/// Maximum income skim an Operative governor can accumulate through bargains.
pub const MAX_OPERATIVE_INCOME_SKIM: f64 = 0.50;
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


impl Governor {
    /// Returns true if this governor is dead (region is leaderless).
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Returns true if this governor is defiant (cooperation below threshold).
    /// Dead governors are not defiant — they're absent entirely.
    pub fn is_defiant(&self) -> bool {
        !self.dead && self.cooperation < GOVERNOR_DEFIANCE_THRESHOLD
    }

    /// Returns true if this governor provides cooperation bonuses.
    pub fn is_cooperative(&self) -> bool {
        !self.dead && self.cooperation >= GOVERNOR_COOPERATION_THRESHOLD
    }

    /// Policy effectiveness multiplier based on governor state.
    /// 0.5 = leaderless (dead), 0.7 = defiant, 0.4 = defiant Recluse, 1.0 = normal.
    pub fn policy_effectiveness(&self) -> f64 {
        if self.dead {
            LEADERLESS_EFFECTIVENESS
        } else if self.cooperation < GOVERNOR_DEFIANCE_THRESHOLD {
            if self.personality == GovernorPersonality::Recluse {
                RECLUSE_DEFIANCE_EFFECTIVENESS
            } else {
                GOVERNOR_DEFIANCE_EFFECTIVENESS
            }
        } else {
            1.0
        }
    }

    /// Policy cost multiplier based on cooperation.
    /// 1.0 = normal/dead, 0.8 = cooperative.
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
    /// Unique regional specialization providing a local passive bonus.
    /// Lost when the region collapses.
    #[serde(default)]
    pub specialization: Option<RegionSpecialization>,
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
    /// Whether this region was voluntarily abandoned via the Sacrifice Region decree.
    /// Distinct from natural collapse (disease-driven). Shown as ABANDONED on defeat screen.
    #[serde(default)]
    pub abandoned: bool,
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
    #[serde(default)]
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
    /// Baseline hospitals restore some capacity. Discourage Hospitalization removes this.
    #[serde(default = "default_one")]
    pub healthcare_capacity: f64,
    /// Supply line integrity (0.0–1.0). Degrades when death rate is high or travel banned.
    /// Below 0.5: policy costs increase 50%. Below 0.25: medicine deployment takes 2x.
    /// At 0: no medicine deployment, no new policies.
    #[serde(default = "default_one")]
    pub supply_lines: f64,
    /// Civil order (0.0–1.0). Degrades when deaths mount and unpopular policies are active.
    /// At 0: spread rate +50% (anarchy). Also factors into GDP via infrastructure health.
    #[serde(default = "default_one")]
    pub civil_order: f64,
    /// Deployment priority for auto-deploy targeting. High regions are served
    /// first, CutOff regions are skipped entirely.
    #[serde(default)]
    pub deploy_priority: RegionPriority,
    /// Tick until which this region suffers network disruption from a neighboring collapse.
    /// While active: +50% medicine deployment costs (see DISRUPTION_MEDICINE_COST_MULT).
    /// Multiple collapses extend the duration (last-collapse-wins on end tick).
    #[serde(default)]
    pub disrupted_until: Option<u64>,
    /// Supply chain throughput penalty from collapsed neighbors (0.0–1.0 multiplier).
    /// 1.0 = no penalty, lower = fewer doses delivered. Each collapsed neighbor
    /// reduces throughput by COLLAPSE_THROUGHPUT_PENALTY_PER_NEIGHBOR (multiplicative).
    /// Updated each tick in tick_infrastructure.
    #[serde(default = "default_one")]
    pub collapse_supply_penalty: f64,
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
    /// Base GDP for this region (in abstract game units, displayed with "k" suffix).
    /// Reflects the region's economic scale — wealthier/larger economies have higher
    /// base GDP. This is the starting value; actual GDP fluctuates around it.
    #[serde(default = "default_one")]
    pub base_gdp: f64,
    /// Current regional GDP (in same units as base_gdp). Starts at base_gdp.
    /// Affected by population alive, disease burden, and active containment policies.
    /// Smoothed via exponential decay toward a computed target each tick.
    /// Governors care about the ratio gdp/base_gdp — it drives their board satisfaction.
    #[serde(default = "default_one")]
    pub gdp: f64,
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
        cooperation: 70.0,
        defiance_crisis_fired: false,
        last_action_tick: 0,
        bargain_count: 0,
        income_skim: 0.0,
        last_sick_tick: None,
        dead: false,
        succession_tick: None,
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

    /// True if this region has the given specialization and hasn't collapsed.
    pub fn has_specialization(&self, s: RegionSpecialization) -> bool {
        !self.collapsed && self.specialization == Some(s)
    }

    /// True if this region is currently experiencing network disruption.
    pub fn is_disrupted(&self, current_tick: u64) -> bool {
        self.disrupted_until.map_or(false, |t| t > current_tick)
    }

    pub fn alive(&self) -> f64 {
        (self.population as f64 - self.total_dead()).max(0.0)
    }

    /// GDP as a fraction of base (0.0–1.0). Used for governor satisfaction
    /// and status labels. Equivalent to the old 0.0–1.0 GDP field.
    pub fn gdp_fraction(&self) -> f64 {
        if self.base_gdp > 0.0 { (self.gdp / self.base_gdp).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Human-readable GDP status label based on GDP fraction and collapse state.
    /// Canonical source of truth for GDP classification — do NOT hardcode these
    /// thresholds elsewhere.
    pub fn gdp_status(&self) -> &'static str {
        if self.collapsed { "COLLAPSED" }
        else if self.gdp_fraction() < 0.40 { "Depression" }
        else if self.gdp_fraction() < 0.60 { "Recession" }
        else if self.gdp_fraction() < 0.80 { "Strained" }
        else { "Stable" }
    }

    /// Policy effectiveness multiplier based on governor cooperation and personality.
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
    /// Collapse supply penalty reduces throughput when neighboring regions have collapsed
    /// (the global supply chain narrows as logistics hubs go offline).
    /// These are independent sequential bottlenecks, so they multiply.
    pub fn delivery_efficiency(&self) -> f64 {
        self.supply_lines * self.healthcare_capacity * self.collapse_supply_penalty
    }

    /// True if ANY disease in this region has an active deploy cooldown.
    pub fn any_deploy_cooldown(&self, current_tick: u64) -> bool {
        self.last_deploy_tick.values().any(|&t| {
            let elapsed = current_tick.saturating_sub(t);
            DEPLOY_COOLDOWN_TICKS.saturating_sub(elapsed) > 0
        })
    }

    /// Total sick (exposed + infectious) across all diseases, capped at population.
    /// Includes the exposed/incubating compartment since those people have the disease
    /// even though they're not yet infectious. May double-count people with multiple
    /// diseases simultaneously, but the cap prevents exceeding population.
    pub fn total_infected(&self) -> f64 {
        let raw: f64 = self.infections.iter().map(|i| i.exposed + i.infected).sum();
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
                exposed: 0.0,
                infected: 0.0,
                dead: 0.0,
                immune: 0.0,
            });
            self.infections.last_mut().unwrap()
        }
    }

    /// Total infected from detected diseases only (for UI display).
    /// Includes both exposed and symptomatic — use `detected_symptomatic()` when
    /// exposed individuals should be hidden from the player.
    pub fn detected_infected(&self, diseases: &[Disease]) -> f64 {
        self.infections.iter()
            .filter(|inf| diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.exposed + inf.infected)
            .sum()
    }

    /// Total symptomatic infected from detected diseases only (excludes exposed/incubating).
    /// Use this for player-visible counts when screening doesn't reveal exposed individuals.
    pub fn detected_symptomatic(&self, diseases: &[Disease]) -> f64 {
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
    /// Exposed (latent) — infected but not yet infectious. Drains into `infected`
    /// at a rate determined by the disease's incubation period.
    pub exposed: f64,
    pub infected: f64,
    pub dead: f64,
    #[serde(default)]
    pub immune: f64,
}

/// Fundamental category of pathogen — determines behavior characteristics
/// and which therapy types are effective.
#[derive(Clone, Copy, Debug, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathogenType {
    /// Fast-mutating, high within-region spread, responds to antivirals
    RnaVirus,
    /// Slower-mutating, stable, responds to antivirals
    DnaVirus,
    /// Responds to antibiotics, can develop resistance
    Bacterium,
    /// Slow-growing, hard to treat, limited drug options
    Fungus,
    /// Extremely slow-spreading and completely untreatable — containment only
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

    /// Qualitative mutation risk for intel assessment display.
    /// Derived from mutation_rate() — kept colocated so changes stay in sync.
    pub fn mutation_risk_label(&self) -> &'static str {
        let rate = self.mutation_rate();
        if rate >= 0.0008 { "High" }
        else if rate >= 0.0002 { "Moderate" }
        else { "Low" }
    }

    /// Stat ranges tuned so total collapse occurs by day 90 without intervention.
    /// HARD REQUIREMENT: every seed must lose by day 90 with no player action.
    /// The enforcing test is `game_is_lost_within_90_days_without_intervention`.
    /// If it starts failing, increase within_region_spread — do NOT relax the test.
    ///
    /// Design principle: long infectious period (16–30 days) with near-zero natural
    /// recovery. This lets the epidemic sweep through each region's full population
    /// before burning out. A short infectious period (high per-tick lethality+recovery)
    /// causes epidemic burnout after infecting only a small fraction of the population.
    /// Do NOT copy the naive approach of increasing per-tick lethality to "speed up"
    /// deaths — it makes the overall death toll LOWER by shortening infectious period.
    ///
    /// IFR = lethality / (lethality + recovery) ≈ 85–95% across all types.
    /// R0 = within_region_spread / (lethality + recovery) ≈ 6–15 depending on type.
    /// Attack rate ≈ 99%+ given high R0. Total deaths ≈ 85–95% of population.
    fn stat_ranges(&self) -> DiseaseStatRanges {
        match self {
            // RNA viruses: very short incubation (0.5–1.5 days), fast spreader.
            // Within-region spread ~80% and cross-region ~2x vs pre-SEIR to compensate
            // for exposed compartment pipeline delay (SEIR reduces effective
            // growth rate at small population fractions).
            PathogenType::RnaVirus => DiseaseStatRanges {
                within_region_spread: (0.009, 0.013),
                lethality: (0.00040, 0.00070),
                recovery: (0.00006, 0.00015),
                cross_region: (0.012, 0.018),
                incubation_days: (0.5, 1.5),
            },
            // DNA viruses: short incubation (1–2 days).
            PathogenType::DnaVirus => DiseaseStatRanges {
                within_region_spread: (0.007, 0.011),
                lethality: (0.00035, 0.00065),
                recovery: (0.00004, 0.00010),
                cross_region: (0.010, 0.015),
                incubation_days: (1.0, 2.0),
            },
            // Bacteria: very short incubation (0.25–1.0 days).
            PathogenType::Bacterium => DiseaseStatRanges {
                within_region_spread: (0.007, 0.009),
                lethality: (0.00035, 0.00060),
                recovery: (0.00010, 0.00020),
                cross_region: (0.010, 0.013),
                incubation_days: (0.25, 1.0),
            },
            // Fungi: moderate incubation (1–3 days).
            PathogenType::Fungus => DiseaseStatRanges {
                within_region_spread: (0.006, 0.008),
                lethality: (0.00030, 0.00055),
                recovery: (0.00005, 0.00015),
                cross_region: (0.008, 0.012),
                incubation_days: (1.0, 3.0),
            },
            // Prions: long incubation (3–7 days) — silent spread before symptoms.
            PathogenType::Prion => DiseaseStatRanges {
                within_region_spread: (0.006, 0.009),
                lethality: (0.00045, 0.00090),
                recovery: (0.00003, 0.00006),
                cross_region: (0.008, 0.011),
                incubation_days: (3.0, 7.0),
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

    /// Whether this pathogen type can be treated with medicine at all.
    /// Prions are completely untreatable — containment is the only option.
    pub fn is_treatable(&self) -> bool {
        !matches!(self, PathogenType::Prion)
    }

    /// The therapy type that's most effective against this pathogen.
    /// Returns `None` for prions, which are completely untreatable.
    pub fn matched_therapy(&self) -> Option<TherapyType> {
        match self {
            PathogenType::RnaVirus | PathogenType::DnaVirus => Some(TherapyType::Antiviral),
            PathogenType::Bacterium => Some(TherapyType::Antibiotic),
            PathogenType::Fungus => Some(TherapyType::Antifungal),
            PathogenType::Prion => None,
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

    /// Within-region spread multiplier when quarantine is active in a region.
    /// Lower = quarantine is more effective at reducing spread.
    pub fn quarantine_factor(&self) -> f64 {
        match self {
            TransmissionVector::Airborne => 0.55,   // standard: 45% spread reduction
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

/// Stat ranges for procedural disease generation.
struct DiseaseStatRanges {
    within_region_spread: (f64, f64),
    lethality: (f64, f64),
    recovery: (f64, f64),
    cross_region: (f64, f64),
    /// Incubation period in days — time from exposure to becoming infectious.
    incubation_days: (f64, f64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disease {
    pub name: String,
    #[serde(default)]
    pub pathogen_type: PathogenType,
    #[serde(default)]
    pub transmission: TransmissionVector,
    pub within_region_spread: f64,
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
    /// Wave origin marker. Diseases that emerge in the same coordinated wave (post
    /// day 24 wave clustering) share a sequence_group ID. None = naturally independent.
    /// Visible in the Threats panel when Rapid Sequencing is unlocked and knowledge >= 0.66.
    #[serde(default)]
    pub sequence_group: Option<u32>,
    /// Incubation period in ticks — time from exposure to becoming infectious.
    /// Newly infected individuals enter the "exposed" compartment and transition
    /// to infectious at rate 1/incubation_ticks per tick.
    pub incubation_ticks: f64,
    /// Regions where this disease was first detected (indices into state.regions).
    /// Set once at detection time — records which regions had infections > 0
    /// at the moment detection triggered. Natural diseases show one region;
    /// engineered multi-seeded diseases show multiple (often non-adjacent).
    #[serde(default)]
    pub first_detected_regions: Vec<usize>,
    /// Day on which this disease was detected (tick / TICKS_PER_DAY).
    #[serde(default)]
    pub detected_day: f64,
    /// Previous day's observed (screened) infected estimate for this disease.
    /// Updated once per day in tick(). Used to compute observed Rt.
    #[serde(default)]
    pub prev_day_observed_infected: f64,
    /// Current accumulating observed infected estimate (snapshotted to prev at day boundary).
    #[serde(default)]
    pub current_day_observed_infected: f64,
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

    /// Estimated generation time in days: incubation period + half the mean infectious period.
    /// Used for Rt computation. Clamped to [0.5, 30.0] days.
    pub fn generation_time_days(&self) -> f64 {
        let mean_infectious_days = 0.5 / (self.lethality + self.recovery_rate) / TICKS_PER_DAY;
        ((self.incubation_ticks / TICKS_PER_DAY) + mean_infectious_days).clamp(0.5, 30.0)
    }

    /// Observed Rt from daily screened infection snapshots.
    /// Returns None if insufficient data (prev < 10 or curr = 0).
    pub fn observed_rt(&self) -> Option<f64> {
        let prev = self.prev_day_observed_infected;
        let curr = self.current_day_observed_infected;
        if prev > 10.0 && curr > 0.0 {
            let growth = curr / prev;
            Some(growth.powf(self.generation_time_days()))
        } else {
            None
        }
    }

    /// Effective mutation rate after genomic sequencing reductions.
    /// Each sequencing halves the rate: base_rate * 0.5^sequencing_count.
    pub fn effective_mutation_rate(&self) -> f64 {
        self.pathogen_type.mutation_rate() * 0.5_f64.powi(self.sequencing_count as i32)
    }

    /// Generate a random disease of the given pathogen type.
    ///
    /// If `toughness_bias` is true, within-region spread and lethality are biased toward
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
        let within_region_spread = stat(rng, ranges.within_region_spread, toughness_bias);
        let lethality = stat(rng, ranges.lethality, toughness_bias);
        let cross_region_spread = range_val(rng, ranges.cross_region);
        let recovery_rate = range_val(rng, ranges.recovery);
        let incubation_ticks = range_val(rng, ranges.incubation_days) * TICKS_PER_DAY;

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
            within_region_spread,
            lethality,
            cross_region_spread,
            recovery_rate,
            knowledge: 0.0,
            strain_generation: 0,
            sequencing_count: 0,
            detected: true, // callers override to false for new diseases
            spawned_at_tick: 0, // callers override to current tick when spawning
            mechanism_resistance: vec![],
            sequence_group: None,
            incubation_ticks,
            first_detected_regions: vec![],
            detected_day: 0.0,
            prev_day_observed_infected: 0.0,
            current_day_observed_infected: 0.0,
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
/// Minimum effective efficacy for auto-deploy to fire. Below this threshold,
/// deploying wastes doses on a near-useless medicine.
pub const AUTO_DEPLOY_MIN_EFFICACY: f64 = 0.04;
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
            // Broad-spectrum: weak efficacy against treatable pathogens, zero against prions.
            // A blunt bandaid — slows disease but can't stop it. Forces research investment.
            (TherapyType::BroadSpectrum, PathogenType::Prion) => 0.0,
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
    /// Whether this medicine is a vaccine or therapeutic. Determines deployment
    /// behavior: vaccines protect susceptible, therapeutics treat infected.
    #[serde(default)]
    pub mode: MedicineMode,
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
    /// Cumulative people treated (moved from infected to immune) across all deployments.
    #[serde(default)]
    pub total_treated: f64,
    /// Cumulative people protected (vaccinated) across all deployments.
    #[serde(default)]
    pub total_protected: f64,
    /// Index into `GameState::corporations` for this medicine's manufacturing partner.
    /// `None` for the starting broad-spectrum medicine (no specific manufacturer).
    /// When development completes and the manufacturer has a board seat, the board
    /// member's satisfaction increases (via a reserves boost to their corporation).
    #[serde(default)]
    pub manufacturer_corp_idx: Option<usize>,
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

/// Whether a medicine is a vaccine (protects susceptible) or therapeutic (treats infected).
/// Set at creation time — determines deployment behavior automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicineMode {
    /// Protects susceptible population against infection.
    Vaccine,
    /// Treats actively infected population.
    Therapeutic,
}

impl Default for MedicineMode {
    fn default() -> Self {
        MedicineMode::Therapeutic
    }
}

impl MedicineMode {
    pub fn label(&self) -> &'static str {
        match self {
            MedicineMode::Vaccine => "Vaccine",
            MedicineMode::Therapeutic => "Therapeutic",
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            MedicineMode::Vaccine => "Vax",
            MedicineMode::Therapeutic => "Trt",
        }
    }
}

/// What a medicine deployment targets: which disease in which mode.
/// The mode (vaccine vs therapeutic) is determined by the medicine itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployTarget {
    pub disease_idx: usize,
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
    /// Prions are completely untreatable — returns empty vec.
    pub fn targeted_medicines(disease_idx: usize, pathogen_type: PathogenType) -> Vec<Medicine> {
        let letter = (b'A' + disease_idx as u8) as char;

        let mechs: &[MechanismOfAction] = match pathogen_type {
            PathogenType::Bacterium => MechanismOfAction::bacterial_mechanisms(),
            PathogenType::Fungus => MechanismOfAction::fungal_mechanisms(),
            PathogenType::RnaVirus | PathogenType::DnaVirus => MechanismOfAction::viral_mechanisms(),
            PathogenType::Prion => {
                // Prions are completely untreatable — no medicines generated
                return vec![];
            }
        };
        // Safe to unwrap: prions return early above, all other types have a matched therapy.
        let therapy = pathogen_type.matched_therapy().unwrap();

        mechs.iter().flat_map(|&mech| {
            let doses = mech.base_doses();
            let base = |mode: MedicineMode| {
                let prefix = mode.short_label();
                Medicine {
                    name: format!("{}-{}-{}", prefix, mech.short_label(), letter),
                    therapy_type: therapy,
                    mode,
                    mechanism: Some(mech),
                    target_diseases: vec![disease_idx],
                    cost: mech.deploy_cost(),
                    doses: 0.0,
                    max_doses: doses,
                    unlocked: false,
                    tested_against: vec![],
                    strain_generations: vec![],
                    deployed_count: 0,
                    total_treated: 0.0,
                    total_protected: 0.0,
                    manufacturer_corp_idx: None,
                }
            };
            [base(MedicineMode::Therapeutic), base(MedicineMode::Vaccine)]
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

    /// How many strain generations behind this medicine is for a given disease.
    /// Returns 0 if not calibrated or disease not targeted.
    pub fn mutations_behind(&self, disease_idx: usize, diseases: &[Disease]) -> u32 {
        let pos = self.target_diseases.iter().position(|&d| d == disease_idx);
        match pos {
            Some(i) => {
                let med_gen = self.strain_generations.get(i).copied();
                match med_gen {
                    Some(mg) => {
                        let disease_gen = diseases.get(disease_idx)
                            .map_or(0, |d| d.strain_generation) as i32;
                        (disease_gen - mg).max(0) as u32
                    }
                    None => 0,
                }
            }
            None => 0,
        }
    }

    /// Set strain calibration for a disease to N generations behind the current
    /// strain. Used by TrialShortcut and EmergencySampleDelivery to fast-track
    /// testing with a built-in efficacy penalty.
    /// `generations_behind` is typically 2 (yielding ~0.70x efficacy from drift).
    pub fn set_strain_calibration_behind(
        &mut self,
        disease_idx: usize,
        diseases: &[Disease],
        generations_behind: i32,
    ) {
        if let Some(pos) = self.target_diseases.iter().position(|&d| d == disease_idx) {
            let current_gen = diseases.get(disease_idx)
                .map_or(0, |d| d.strain_generation) as i32;
            while self.strain_generations.len() <= pos {
                self.strain_generations.push(0);
            }
            self.strain_generations[pos] = current_gen - generations_behind;
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
        // Filter out untreatable pathogens (prions) — no medicine works on them
        result.retain(|&d_idx| {
            diseases.get(d_idx).map_or(true, |d| d.pathogen_type.is_treatable())
        });
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

/// Research category — purely for UI grouping and display.
/// Derived from `ResearchKind::category()`. Has no gameplay-mechanical effect;
/// personnel and funding are the only constraints on starting research.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ResearchCategory {
    Field,
    Applied,
    Basic,
}


impl ResearchCategory {
    pub fn name(self) -> &'static str {
        match self {
            ResearchCategory::Field => "Field",
            ResearchCategory::Applied => "Applied",
            ResearchCategory::Basic => "Basic",
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
    /// Pathogen suppression — permanently reduces a disease's within-region spread by ~20%.
    /// Requires the CompetitiveDisplacement basic tech to be unlocked.
    SuppressPathogen { disease_idx: usize },
    /// Directed attenuation — permanently reduces a disease's lethality by ~30%.
    /// In-situ modification of pathogen virulence factors.
    /// Requires the DirectedAttenuation basic tech to be unlocked.
    AttenuatePathogen { disease_idx: usize },
    /// Genomic interdiction — permanently eliminates a disease's cross-region spread.
    /// Disrupts pathogen transmission mechanisms at the genomic level.
    /// Requires the GeneDriveContainment basic tech to be unlocked.
    InterdictPathogen { disease_idx: usize },
    /// Field operations — send a team to stabilize degraded infrastructure in a region.
    /// Appears when any infrastructure system drops below INFRA_STRESSED (50%).
    /// Creates a mid-game phase shift: field research slots compete between disease
    /// work and keeping regions operational.
    FieldOperations { region_idx: usize, system: InfraSystem },
}

impl ResearchKind {
    /// Which category this research kind belongs to (UI grouping only).
    pub fn category(&self) -> ResearchCategory {
        match self {
            Self::IdentifyThreat { .. }
            | Self::ClinicalTrial { .. }
            | Self::GenomicSequencing { .. }
            | Self::SuppressPathogen { .. }
            | Self::AttenuatePathogen { .. }
            | Self::InterdictPathogen { .. }
            | Self::FieldOperations { .. } => ResearchCategory::Field,
            Self::DevelopMedicine { .. }
            | Self::ManufactureDoses { .. }
            | Self::TrainPersonnel => ResearchCategory::Applied,
            Self::BasicResearch { .. } => ResearchCategory::Basic,
        }
    }

    /// Whether this research kind supports auto-repeat.
    /// Repeatable: TrainPersonnel, ManufactureDoses.
    /// Non-repeatable: everything else (one-shot projects).
    pub fn is_repeatable(&self) -> bool {
        matches!(self, Self::TrainPersonnel | Self::ManufactureDoses { .. })
    }

    /// Human-readable label for this research kind, respecting disease knowledge level.
    /// Used by both UI (header status) and engine (event log messages).
    pub fn label(&self, state: &GameState) -> String {
        match self {
            Self::IdentifyThreat { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Identify {}", name)
            }
            Self::DevelopMedicine { medicine_idx } => {
                let name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Unknown");
                format!("Develop {}", name)
            }
            Self::ClinicalTrial { medicine_idx, .. } => {
                let name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Unknown");
                format!("Trial {}", name)
            }
            Self::ManufactureDoses { medicine_idx } => {
                let name = state.medicines.get(*medicine_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Unknown");
                format!("Manufacture {}", name)
            }
            Self::GenomicSequencing { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Sequence {}", name)
            }
            Self::TrainPersonnel => "Train Personnel".to_string(),
            Self::BasicResearch { tech } => format!("Research {}", tech.name()),
            Self::SuppressPathogen { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Suppress {}", name)
            }
            Self::AttenuatePathogen { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Attenuate {}", name)
            }
            Self::InterdictPathogen { disease_idx } => {
                let name = state.diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!("Interdict {}", name)
            }
            Self::FieldOperations { region_idx, system } => {
                let region = state.regions.get(*region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("Unknown");
                format!("Field Ops {} {}", system.label(), region)
            }
        }
    }
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
    /// Prereq: RapidSequencing (sequencing data guides field teams to high-value targets).
    MetagenomicSurveillance,
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
    /// Unlocks competitive displacement field research: release attenuated
    /// strains that outcompete virulent wild-type, reducing within-region spread.
    /// Prereq: VaccinePlatform + CombinationTherapy.
    CompetitiveDisplacement,
    /// Unlocks directed attenuation field research: permanently reduce
    /// a disease's lethality by modifying its virulence factors in situ.
    /// Prereq: CompetitiveDisplacement.
    DirectedAttenuation,
    /// Unlocks gene drive containment field research: self-propagating
    /// genetic modifications prevent pathogen establishment in new regions.
    /// Prereq: DirectedAttenuation.
    GeneDriveContainment,
    /// Reduces ManufactureDoses applied research duration by 35%.
    /// Prereq: at least one targeted medicine developed (mechanism.is_some() && unlocked).
    /// Note: when a Biotech corp is healthy, bonus could be increased to 50% (#1381).
    AutomatedSynthesis,
    /// Each ManufactureDoses run produces 25% more doses (thermostable formulations
    /// reduce cold-chain waste). Stacks multiplicatively with Europe's yield bonus.
    /// Prereq: AutomatedSynthesis.
    StabilizedFormulation,
    /// Disease-caused infrastructure degradation (HC/SL/CO) is 20% slower globally.
    /// Does NOT affect policy-triggered drains (travel ban, quarantine, etc.).
    /// Prereq: TargetedDrugDesign.
    ResilientGrids,
    /// Unlocks 20-day death projections in the Threats panel for each detected disease.
    /// Shows where the outbreak is heading, not just where it is.
    /// Prereq: RapidSequencing + ResistanceSurveillance.
    EpidemiologicalForecasting,
}

impl BasicTech {
    /// Human-readable name for display.
    pub fn name(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Targeted Drug Design",
            BasicTech::MonoclonalAntibodies => "Monoclonal Antibodies",
            BasicTech::PhageTherapy => "Phage Therapy",
            BasicTech::RapidSequencing => "Rapid Sequencing",
            BasicTech::MetagenomicSurveillance => "Metagenomic Surveillance",
            BasicTech::VaccinePlatform => "Vaccine Platform",
            BasicTech::ResistanceSurveillance => "Resistance Surveillance",
            BasicTech::CombinationTherapy => "Combination Therapy",
            BasicTech::CompetitiveDisplacement => "Competitive Displacement",
            BasicTech::DirectedAttenuation => "Directed Attenuation",
            BasicTech::GeneDriveContainment => "Gene Drive Containment",
            BasicTech::AutomatedSynthesis => "Automated Synthesis",
            BasicTech::StabilizedFormulation => "Stabilized Formulation",
            BasicTech::ResilientGrids => "Resilient Grids",
            BasicTech::EpidemiologicalForecasting => "Epidemiological Forecasting",
        }
    }

    /// Short description for the research panel.
    pub fn description(&self) -> &'static str {
        match self {
            BasicTech::TargetedDrugDesign => "Targeted antiviral and antibiotic development for identified pathogen classes.",
            BasicTech::MonoclonalAntibodies => "Engineered antibody therapies with high efficacy against identified viral strains.",
            BasicTech::PhageTherapy => "Bacteriophage-based treatment for bacterial pathogens. Low resistance development.",
            BasicTech::RapidSequencing => "50% faster sequencing. Reveals mutation drift rate and history.",
            BasicTech::MetagenomicSurveillance => "Environmental sample sequencing identifies pathogens without culture. Field research and clinical trials 25% faster.",
            BasicTech::VaccinePlatform => "3x effectiveness of preventive vaccination programs.",
            BasicTech::ResistanceSurveillance => "Tracks resistance levels and trends across all deployed medicines.",
            BasicTech::CombinationTherapy => "Multi-drug protocols reduce resistance accumulation from deployments by 50%.",
            BasicTech::CompetitiveDisplacement => "Release attenuated strains that outcompete virulent wild-type. Each project reduces within-region spread 20%.",
            BasicTech::DirectedAttenuation => "In-situ modification of pathogen virulence factors. Each project permanently reduces target lethality 30%.",
            BasicTech::GeneDriveContainment => "Self-propagating genetic modifications prevent pathogen establishment in new regions. Eliminates cross-region spread.",
            BasicTech::AutomatedSynthesis => "Standardized bioreactor protocols cut production cycle time by 35%.",
            BasicTech::StabilizedFormulation => "Thermostable formulations reduce cold-chain waste. Each manufacturing run yields 25% more usable doses.",
            BasicTech::ResilientGrids => "Hardened regional infrastructure protocols. Disease-caused infrastructure degradation 20% slower.",
            BasicTech::EpidemiologicalForecasting => "Predictive outbreak modeling. Threats panel shows projected deaths over 20 days for each active disease.",
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
            BasicTech::MetagenomicSurveillance => {
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
            BasicTech::CompetitiveDisplacement => {
                state.unlocked_techs.contains(&BasicTech::VaccinePlatform)
                    && state.unlocked_techs.contains(&BasicTech::CombinationTherapy)
            }
            BasicTech::DirectedAttenuation => {
                state.unlocked_techs.contains(&BasicTech::CompetitiveDisplacement)
            }
            BasicTech::GeneDriveContainment => {
                state.unlocked_techs.contains(&BasicTech::DirectedAttenuation)
            }
            BasicTech::AutomatedSynthesis => {
                // Prereq: at least one targeted medicine developed (not broad-spectrum)
                state.medicines.iter().any(|m| m.mechanism.is_some() && m.unlocked)
            }
            BasicTech::StabilizedFormulation => {
                state.unlocked_techs.contains(&BasicTech::AutomatedSynthesis)
            }
            BasicTech::ResilientGrids => {
                state.unlocked_techs.contains(&BasicTech::TargetedDrugDesign)
            }
            BasicTech::EpidemiologicalForecasting => {
                state.unlocked_techs.contains(&BasicTech::RapidSequencing)
                    && state.unlocked_techs.contains(&BasicTech::ResistanceSurveillance)
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
            BasicTech::MetagenomicSurveillance => "Rapid Sequencing",
            BasicTech::VaccinePlatform => "Monoclonal Antibodies or Phage Therapy",
            BasicTech::ResistanceSurveillance => "Rapid Sequencing",
            BasicTech::CombinationTherapy => "Deploy 2+ different medicines",
            BasicTech::CompetitiveDisplacement => "Vaccine Platform + Combination Therapy",
            BasicTech::DirectedAttenuation => "Competitive Displacement",
            BasicTech::GeneDriveContainment => "Directed Attenuation",
            BasicTech::AutomatedSynthesis => "Develop any targeted medicine",
            BasicTech::StabilizedFormulation => "Automated Synthesis",
            BasicTech::ResilientGrids => "Targeted Drug Design",
            BasicTech::EpidemiologicalForecasting => "Rapid Sequencing + Resistance Surveillance",
        }
    }

    /// All techs in display order.
    pub fn all() -> &'static [BasicTech] {
        &[
            BasicTech::TargetedDrugDesign,
            BasicTech::MonoclonalAntibodies,
            BasicTech::PhageTherapy,
            BasicTech::ResilientGrids,
            BasicTech::RapidSequencing,
            BasicTech::MetagenomicSurveillance,
            BasicTech::VaccinePlatform,
            BasicTech::ResistanceSurveillance,
            BasicTech::EpidemiologicalForecasting,
            BasicTech::CombinationTherapy,
            BasicTech::CompetitiveDisplacement,
            BasicTech::DirectedAttenuation,
            BasicTech::GeneDriveContainment,
            BasicTech::AutomatedSynthesis,
            BasicTech::StabilizedFormulation,
        ]
    }
}

impl ResearchKind {
    /// Project costs: (personnel, duration_ticks, funding).
    ///
    /// DevelopMedicine costs depend on mechanism of action: each mechanism has
    /// a dev_cost_multiplier that scales base costs (3 personnel, 200 ticks, $500).
    /// Broad-spectrum (multi-target, no mechanism) uses fixed high costs.
    /// These are BASE costs. Tech modifiers (RapidSequencing, MetagenomicSurveillance) are
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
                BasicTech::RapidSequencing => (4, 300.0, 350.0),
                BasicTech::MetagenomicSurveillance => (4, 280.0, 650.0),
                BasicTech::VaccinePlatform => (6, 360.0, 1000.0),
                BasicTech::ResistanceSurveillance => (3, 200.0, 500.0),
                BasicTech::CombinationTherapy => (4, 300.0, 800.0),
                BasicTech::CompetitiveDisplacement => (8, 480.0, 1200.0),
                BasicTech::DirectedAttenuation => (10, 600.0, 1500.0),
                BasicTech::GeneDriveContainment => (12, 720.0, 2000.0),
                BasicTech::AutomatedSynthesis => (4, 200.0, 500.0),
                BasicTech::StabilizedFormulation => (5, 280.0, 700.0),
                BasicTech::ResilientGrids => (3, 240.0, 550.0),
                BasicTech::EpidemiologicalForecasting => (2, 160.0, 300.0),
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
                let med = medicines.get(*medicine_idx);
                let med_name = med.map(|m| m.name.as_str()).unwrap_or("Unknown");
                let dis = diseases.get(*disease_idx)
                    .map(|d| d.display_name(*disease_idx))
                    .unwrap_or_else(|| "Unknown".to_string());
                let is_retrial = med.map_or(false, |m| m.tested_against.contains(disease_idx));
                if is_retrial {
                    format!("Re-trial: {} vs {}", med_name, dis)
                } else {
                    format!("Trial: {} vs {}", med_name, dis)
                }
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
        /// Within-region spread change factor (e.g., 1.1 = +10%). Only meaningful with RapidSequencing.
        spread_factor: f64,
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
    /// Advanced Intel completed analysis on a detected disease — reveals
    /// name and pathogen type immediately (knowledge boost to KNOWLEDGE_NAME).
    IntelAnalysis {
        disease_idx: usize,
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
    /// A region was voluntarily abandoned via the Sacrifice Region decree.
    RegionAbandoned {
        region_idx: usize,
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
    /// A board member is unhappy — contract condition satisfaction dropped to warning level.
    ContractWarning { member_name: String, reason: String },
    /// A contract was revoked because condition satisfaction bottomed out.
    ContractRevoked { name: String, reason: String },
    /// A satisfied patron granted a bonus (funding, personnel, research boost, or doses).
    PatronBonus { member_name: String, description: String },
    /// A corporation went bankrupt (permanent).
    CorporationBankrupt { corp_idx: usize, region_idx: usize },
    GameOver,
    /// A crisis event appeared and needs player attention.
    CrisisStarted,
    /// A crisis was auto-resolved based on player's saved preference.
    /// Carries the resolution outcome message for the event log.
    CrisisAutoResolved { message: String },
    /// A research project was auto-restarted because auto-repeat is on.
    ResearchAutoRestarted { kind: ResearchKind },
    /// A research completion in one category unlocked a project in another category.
    /// Notifies the player to start the next pipeline step manually.
    ResearchHandoff { message: String },
    /// Personnel left due to unpaid wages (funding at $0).
    PersonnelAttrition { count: u32 },
    /// Auto-deploy was blocked for a medicine because its effective efficacy
    /// against all tested diseases is below the deployment threshold.
    AutoDeployBlocked { medicine_idx: usize },
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
    /// An emergency decree became available due to escalating crisis severity.
    DecreeUnlocked {
        decree: DecreeId,
    },
    /// Suppression research complete — pathogen within-region spread permanently reduced.
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
    /// `efficiency` is the fraction of shipped doses that were usable (supply_lines × healthcare × collapse penalty).
    ShipmentDelivered {
        medicine_idx: usize,
        region_idx: usize,
        doses: f64,
        adverse: bool,
        efficiency: f64,
        /// Doses lost to poor targeting (no surveillance to identify who needs treatment).
        doses_wasted: f64,
        /// People actually treated (moved from infected to immune). 0 if vaccination.
        people_treated: f64,
        /// People actually protected (vaccinated from susceptible pool). 0 if treatment.
        people_protected: f64,
    },
    /// Emergency sample delivery sent to a region's governor.
    EmergencySampleDelivered {
        medicine_idx: usize,
        region_idx: usize,
        cooperation_change: f64,
        adverse: bool,
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
    /// A governor died from the pandemic. Region is now leaderless.
    GovernorDied {
        region_idx: usize,
        name: String,
    },
    /// A successor governor has arrived in a leaderless region.
    GovernorSucceeded {
        region_idx: usize,
        name: String,
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
    /// A crisis response team returned — personnel freed.
    CrisisTeamReturned {
        label: String,
        personnel: u32,
    },
    /// Board approval crossed a policy's threshold — that policy is now globally available.
    PolicyAuthorized {
        policy: PolicyId,
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
        project_idx: usize,
        double_personnel: bool,
    },
    TogglePolicy {
        region_idx: usize,
        policy: PolicyId,
    },
    /// Resolve the active crisis by choosing option A (0) or B (1).
    ResolveCrisis {
        choice: usize,
    },
    /// Enact an emergency decree. `region_idx` is only used for SacrificeRegion/FortifyRegion.
    EnactDecree {
        decree: DecreeId,
        region_idx: Option<usize>,
    },
    /// Spend funds to boost POL directly.
    /// Spend funds to boost a governor's cooperation.
    AppeaseGovernor { region_idx: usize },
    /// Personality-specific bargain with a defiant governor (non-monetary cost).
    BargainWithGovernor { region_idx: usize },
    /// Toggle a standing order on/off.
    ToggleStandingOrder { kind: StandingOrderKind },
    /// Toggle auto-deploy for a specific medicine.
    ToggleAutoDeploy { med_idx: usize },
    /// Toggle auto-repeat for a specific repeatable research project.
    ToggleAutoRepeat { kind: ResearchKind },
    /// Upgrade the global research lab (level 0→1 or 1→2). One-time funding cost.
    UpgradeLab,
    /// Cycle a region's deployment priority (High → Normal → Low → CutOff → High).
    CycleDeployPriority { region_idx: usize },
    /// Repay an outstanding loan in full. `loan_idx` indexes into `state.loans`.
    RepayLoan { loan_idx: usize },
    /// Buy shares in a corporation. Cost = share_price × quantity.
    BuyShares { corp_idx: usize, quantity: u32 },
    /// Sell shares in a corporation. Proceeds = share_price × quantity.
    SellShares { corp_idx: usize, quantity: u32 },
    /// Send experimental medicine samples to a specific region's governor.
    /// Bypasses full clinical trial pipeline — boosts governor cooperation
    /// but risks adverse reactions for untested medicines.
    EmergencySampleDelivery { medicine_idx: usize, region_idx: usize },
    /// Cancel an active contract by board member index. Frees the contract slot
    /// but applies a satisfaction penalty to the offering member.
    CancelContract { board_member_idx: usize },
    /// Inject funding into a corporation's reserves to prevent bankruptcy.
    BailoutCorporation { corp_idx: usize },
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
    /// If Some, personnel are tied up in a temporary operation for the specified
    /// duration and returned when it completes. If None, personnel are permanently deducted.
    #[serde(default)]
    pub operation: Option<OperationSpec>,
}

/// Specification for a temporary crisis operation that ties up personnel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationSpec {
    pub days: f64,
    pub label: String,
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
    /// Lab accident — lose applied or basic research, or spend resources to contain.
    LabAccident { targets_basic: bool },
    /// Political pressure — lift quarantine in a region or pay to resist.
    PoliticalPressure { region_idx: usize },
    /// Staff burnout — lose personnel or pay retention bonus.
    PersonnelCrisis { amount: u32 },
    /// Refugees flooding from collapsed region — accept (spread disease) or turn away (lose POL).
    /// `wave` counts how many regions have collapsed so far (1 on first collapse).
    RefugeeWave { from_region: usize, to_region: usize, #[serde(default = "default_one_u8")] wave: u8 },
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
    /// Board member's corporation deploys private security, demands operational control.
    CorporateSeizure { cooperate_loss: u32 },
    /// Cult blocks vaccination teams in a region.
    CultBlockade { region_idx: usize },
    /// Two corporations claim credit for your treatment breakthrough, threatening to cut contracts.
    VaccineDispute { neutral_loss: f64, credit_gain: f64, corp_a: String, corp_b: String },

    // --- Dark comedy events (personality and flavor) ---

    /// Quarterly performance review during the apocalypse.
    PerformanceReview,
    // --- Contract crises ---

    /// A board member offers a new contract. Interrupts gameplay so the player
    /// must accept or reject the terms. Replaces the old policy-panel-only flow.
    ContractOffer { template_id: u8 },
    /// Board member makes demands when contract condition satisfaction drops to warning zone.
    ContractDemand { template_id: u8 },

    // --- Governor defiance crises (fired when cooperation drops below threshold) ---

    /// Hardliner governor declares your directive illegitimate.
    GovernorHardliner { region_idx: usize },
    /// Blowhard governor makes noise — mostly hollow threats.
    GovernorBlowhard { region_idx: usize },
    /// Recluse governor stops responding — region drifts.
    GovernorRecluse { region_idx: usize },
    /// Operative governor starts skimming openly.
    GovernorOperative { region_idx: usize },
    /// Buffoon governor causes accidental damage.
    GovernorBuffoon { region_idx: usize },
    /// Mobster governor escalates demands.
    GovernorMobster { region_idx: usize },
    /// Governor falls ill during high infection levels. Personality determines the crisis.
    GovernorSick { region_idx: usize },
    /// Governor has died from the pandemic. Region becomes leaderless.
    GovernorDeath { region_idx: usize },

    // --- Detection alert types ---

    /// New unknown pathogen detected. Interrupts with alert offering immediate identification.
    /// Bypasses normal crisis cooldown — fires immediately on detection.
    NewPathogenDetected { disease_idx: usize },

    // --- Endgame crisis types ---

    /// Emergency consolidation: pull all resources into one surviving region.
    /// Fires when 2+ regions have collapsed.
    ArkProtocol { region_idx: usize },

    // --- Follow-up crisis types (spawned by earlier choices) ---

    /// Follow-up to BlackMarketMedicine (Allow): counterfeit drugs killing people.
    CounterfeitEpidemic { region_idx: usize },
    /// Follow-up to CorruptOfficial (Ignore): corruption has spread to a ring.
    EmbezzlementRing { stolen_per_day: f64 },
    /// Follow-up to CorporateSeizure (Cooperate): corporation claims your research as proprietary IP.
    CorporateOverreach,
    /// Follow-up to VaccineDispute (Credit one side): losing corp retaliates.
    SanctionsThreat { funding_loss: f64, corp_name: String },

    // --- Corporate crises ---

    /// Scheduled board meeting communiqué. Fires on a recurring timer (~every 10 days).
    /// Single-option event: the board informs the player of decisions made.
    /// Funding level is adjusted based on overall board satisfaction.
    BoardMeeting,
    /// Board sends a formal warning letter when non-board stock positions exceed
    /// cumulative policy spending + ¥1000 buffer. If the player continues, funding is cut.
    BoardEmbezzlementWarning,
    /// Chairman calls a Vote of No Confidence after sustained hostility (<0.20 satisfaction
    /// for ~3 consecutive days). Player must make concessions or stand firm.
    VoteOfNoConfidence,
    /// Board formally questions the player's inaction on research. Fires once around day 5
    /// if no identification research has been started for any disease.
    BoardResearchInquiry,

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
    /// Governor: cancels a policy in their region + cooperation drop.
    /// Corporation: personnel loss (intimidation) or POL penalty (smear campaign).
    LoanCallIn {
        lender_name: String,
        lender: LoanLender,
        outstanding: f64,
    },
    /// A board member offers to raise the price on a contract the player has held for 10+ days.
    LoyaltyRaise {
        template_id: u8,
    },

    // --- Corporate demand crises ---

    /// A corporation demands the player lift a policy that's crushing their revenue.
    /// Fires when a corp's sector is specifically hurt by an active policy and their
    /// revenue has dropped significantly. Per-corp cooldown prevents spam.
    CorporateDemand {
        corp_idx: usize,
    },
}

impl CrisisKind {
    /// Short tag identifying the crisis type (ignoring variant data).
    /// Used for cooldown tracking to prevent back-to-back repeats.
    pub fn tag(&self) -> &'static str {
        match self {
            CrisisKind::LabAccident { .. } => "lab",
            CrisisKind::PoliticalPressure { .. } => "political",
            CrisisKind::PersonnelCrisis { .. } => "personnel",
            CrisisKind::RefugeeWave { .. } => "refugee",
            CrisisKind::BlackMarketMedicine { .. } => "blackmarket",
            CrisisKind::QuarantineRiot { .. } => "riot",
            CrisisKind::MediaPanic => "media",
            CrisisKind::TrialShortcut { .. } => "trial",
            CrisisKind::VaccineHesitancy { .. } => "hesitancy",
            CrisisKind::CorruptOfficial { .. } => "corrupt",
            CrisisKind::ResourceDiversion { .. } => "diversion",
            CrisisKind::ExhaustionEpidemic { .. } => "exhaustion",
            CrisisKind::CorporateSeizure { .. } => "corporate_seizure",
            CrisisKind::CultBlockade { .. } => "cult",
            CrisisKind::VaccineDispute { .. } => "vaccine_dispute",
            CrisisKind::PerformanceReview => "performance_review",
            CrisisKind::ContractOffer { .. } => "contract_offer",
            CrisisKind::ContractDemand { .. } => "contract_demand",
            CrisisKind::GovernorHardliner { .. } => "gov_hardliner",
            CrisisKind::GovernorBlowhard { .. } => "gov_blowhard",
            CrisisKind::GovernorRecluse { .. } => "gov_recluse",
            CrisisKind::GovernorOperative { .. } => "gov_operative",
            CrisisKind::GovernorBuffoon { .. } => "gov_buffoon",
            CrisisKind::GovernorMobster { .. } => "gov_mobster",
            CrisisKind::GovernorSick { .. } => "gov_sick",
            CrisisKind::GovernorDeath { .. } => "gov_death",
            CrisisKind::NewPathogenDetected { .. } => "new_pathogen",
            CrisisKind::ArkProtocol { .. } => "ark_protocol",
            CrisisKind::CounterfeitEpidemic { .. } => "counterfeit",
            CrisisKind::EmbezzlementRing { .. } => "embezzlement",
            CrisisKind::CorporateOverreach => "corporate_overreach",
            CrisisKind::SanctionsThreat { .. } => "sanctions",
            CrisisKind::BoardMeeting => "board_meeting",
            CrisisKind::BoardEmbezzlementWarning => "board_embezzlement_warning",
            CrisisKind::VoteOfNoConfidence => "vote_no_confidence",
            CrisisKind::BoardResearchInquiry => "board_research_inquiry",
            CrisisKind::FieldTeamDetained { .. } => "field_team_detained",
            CrisisKind::FieldTeamDetainedAgain { .. } => "field_team_detained_again",
            CrisisKind::LoanOffer { .. } => "loan_offer",
            CrisisKind::LoanCallIn { .. } => "loan_call_in",
            CrisisKind::LoyaltyRaise { .. } => "loyalty_raise",
            CrisisKind::CorporateDemand { .. } => "corp_demand",
        }
    }

    /// Whether this crisis kind should fire immediately, bypassing CRISIS_MIN_GAP.
    /// Pathogen detections and governor crises are time-sensitive: the player needs
    /// to respond before the situation evolves further.
    pub fn bypasses_crisis_gap(&self) -> bool {
        matches!(self,
            CrisisKind::NewPathogenDetected { .. }
            | CrisisKind::GovernorHardliner { .. }
            | CrisisKind::GovernorBlowhard { .. }
            | CrisisKind::GovernorRecluse { .. }
            | CrisisKind::GovernorOperative { .. }
            | CrisisKind::GovernorBuffoon { .. }
            | CrisisKind::GovernorMobster { .. }
            | CrisisKind::GovernorSick { .. }
            | CrisisKind::GovernorDeath { .. }
        )
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
    Board,
    Ledger,
    Help,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MedicineUiState {
    BrowseMedicines,
    SelectRegion { medicine_idx: usize },
    /// Choose which disease to target (skipped for single-target medicines).
    SelectDisease { medicine_idx: usize, region_idx: usize },
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

/// Research panel UI state machine.
/// The research panel is a flat scrollable list with section headers (like the policy panel).
/// `BrowseAll` is the only browsing state — no intermediate category screen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResearchUiState {
    /// Flat list showing all research projects with section headers.
    BrowseAll,
    /// Confirming a project before starting it.
    /// `project_idx` indexes into `all_available_projects()`.
    ConfirmProject { project_idx: usize, double_personnel: bool },
    /// Confirming a lab upgrade before purchasing.
    ConfirmLabUpgrade,
}

/// A selectable item in the flat research panel list.
/// Built dynamically by `GameState::research_flat_items()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResearchFlatItem {
    /// An active research project (index into `active_research` vec).
    Active(usize),
    /// An available (startable) project. Index into `all_available_projects()`.
    Available(usize),
    /// The lab upgrade button.
    UpgradeLab,
}

impl ResearchFlatItem {
    /// The ResearchKind of this item, if it's an available project.
    pub fn available_kind(&self, state: &GameState) -> Option<ResearchKind> {
        match self {
            Self::Available(idx) => {
                state.all_available_projects().get(*idx).cloned()
            }
            _ => None,
        }
    }
}

/// Operations/Orders panel UI state machine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OpsUiState {
    /// Top level: browse decrees, standing orders, loans.
    BrowseOps,
    /// Confirm an emergency decree before enacting it.
    ConfirmDecree { decree: DecreeId },
    /// Select which region to sacrifice (for Sacrifice Region decree).
    SelectSacrificeRegion,
    /// Select which region to fortify (for Fortify Region decree).
    SelectFortifyRegion,
    /// Select which medicine to send as an emergency sample delivery.
    SelectEmergencyMedicine,
    /// Confirm emergency sample delivery to the currently selected map region.
    ConfirmEmergencyDelivery { medicine_idx: usize },
}

/// Board panel UI state machine. Information-only panel — no wizard steps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BoardUiState {
    /// Top level: browse board members.
    BrowseMembers,
}

/// Ledger (S.P.L.) panel UI state machine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LedgerUiState {
    /// Top level: browse all corporations and their share prices.
    BrowseStocks,
    /// Confirm a buy order for the selected corporation.
    ConfirmBuy { corp_idx: usize },
    /// Confirm a sell order for the selected corporation.
    ConfirmSell { corp_idx: usize },
    /// Confirm a bailout (reserve injection) for the selected corporation.
    ConfirmBailout { corp_idx: usize },
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
    /// - `Panel::Research / BrowseAll`         → index into `research_flat_items()` flat list
    /// - `Panel::Policy / ManagePolicies`     → display position (see MANAGE_* constants)
    /// - `Panel::Operations / BrowseOps`      → decrees, standing orders, loans
    ///
    /// Always bounded by `ui::panel_selection_max()` and reset to 0 on every wizard step transition.
    ///
    /// **Adding items to a panel list:** update the `selection_max()` function in the
    /// corresponding `ui/` module — the item-count logic lives alongside each renderer.
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
    /// Board panel state.
    #[serde(default)]
    pub board_ui: Option<BoardUiState>,
    /// Ledger panel state.
    #[serde(default)]
    pub ledger_ui: Option<LedgerUiState>,
    /// Whether the home splash animation has completed (or been skipped).
    /// Once true, the home panel renders fully without animation.
    #[serde(default)]
    pub home_splash_done: bool,
    /// Whether the typewriter animation has been fast-forwarded (all lines shown).
    /// First Enter press sets this; second Enter press sets `home_splash_done`.
    #[serde(default)]
    pub home_splash_revealed: bool,
    /// Game speed multiplier (1, 2, 4, 6). Affects real-time tick rate only.
    #[serde(default = "default_speed")]
    pub speed_multiplier: u8,
    /// Whether the player dismissed the "terminal too small" warning overlay.
    /// Transient — resets each session so the warning re-appears if still too small.
    #[serde(skip)]
    pub size_warning_dismissed: bool,
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
                Panel::Research => matches!(self.research_ui, Some(ResearchUiState::BrowseAll) | None),
                Panel::Policy => matches!(self.policy_ui, Some(PolicyUiState::ManagePolicies { .. }) | None),
                Panel::Operations => matches!(self.operations_ui, Some(OpsUiState::BrowseOps) | None),
                Panel::Board => matches!(self.board_ui, Some(BoardUiState::BrowseMembers) | None),
                Panel::Ledger => matches!(self.ledger_ui, Some(LedgerUiState::BrowseStocks) | None),
                Panel::None | Panel::Threats | Panel::Help => true,
            };
            if at_top {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                match panel {
                    Panel::Medicines => self.medicine_ui = None,
                    Panel::Research => self.research_ui = None,
                    Panel::Policy => self.policy_ui = None,
                    Panel::Operations => self.operations_ui = None,
                    Panel::Board => self.board_ui = None,
                    Panel::Ledger => self.ledger_ui = None,
                    Panel::None | Panel::Threats | Panel::Help => {}
                }
            } else {
                // Reset to top level of this panel
                self.panel_selection = 0;
                match panel {
                    Panel::Medicines => self.medicine_ui = Some(MedicineUiState::BrowseMedicines),
                    Panel::Research => self.research_ui = Some(ResearchUiState::BrowseAll),
                    Panel::Policy => {
                        self.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: self.map_selection });
                    }
                    Panel::Operations => self.operations_ui = Some(OpsUiState::BrowseOps),
                    Panel::Board => self.board_ui = Some(BoardUiState::BrowseMembers),
                    Panel::Ledger => self.ledger_ui = Some(LedgerUiState::BrowseStocks),
                    Panel::None | Panel::Threats | Panel::Help => {}
                }
            }
        } else {
            self.open_panel = panel;
            self.panel_selection = 0;
            // Once the player opens any panel, the home splash animation is done.
            self.home_splash_done = true;
            match panel {
                Panel::Medicines => self.medicine_ui = Some(MedicineUiState::BrowseMedicines),
                Panel::Research => self.research_ui = Some(ResearchUiState::BrowseAll),
                Panel::Policy => {
                    // Go directly to the policies for the currently selected region.
                    // Left/right map navigation (sync_panel_region) keeps this in sync.
                    self.policy_ui = Some(PolicyUiState::ManagePolicies { region_idx: self.map_selection });
                }
                Panel::Operations => {
                    self.operations_ui = Some(OpsUiState::BrowseOps);
                }
                Panel::Board => {
                    self.board_ui = Some(BoardUiState::BrowseMembers);
                }
                Panel::Ledger => {
                    self.ledger_ui = Some(LedgerUiState::BrowseStocks);
                }
                Panel::None | Panel::Threats | Panel::Help => {}
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
                    Some(MedicineUiState::ConfirmDeploy { medicine_idx, region_idx, .. }) => {
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
                    Some(MedicineUiState::BrowseMedicines) | None => {
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
                    Some(ResearchUiState::ConfirmProject { .. })
                    | Some(ResearchUiState::ConfirmLabUpgrade) => {
                        self.research_ui = Some(ResearchUiState::BrowseAll);
                        self.panel_selection = 0;
                    }
                    Some(ResearchUiState::BrowseAll) | None => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.research_ui = None;
                    }
                }
            }
            Panel::Operations => {
                match &self.operations_ui {
                    Some(OpsUiState::ConfirmDecree { .. })
                    | Some(OpsUiState::SelectSacrificeRegion)
                    | Some(OpsUiState::SelectFortifyRegion) => {
                        self.operations_ui = Some(OpsUiState::BrowseOps);
                        self.panel_selection = 0;
                    }
                    Some(OpsUiState::ConfirmEmergencyDelivery { .. }) => {
                        self.operations_ui = Some(OpsUiState::SelectEmergencyMedicine);
                        self.panel_selection = 0;
                    }
                    Some(OpsUiState::SelectEmergencyMedicine) => {
                        self.operations_ui = Some(OpsUiState::BrowseOps);
                        self.panel_selection = 0;
                    }
                    Some(OpsUiState::BrowseOps) | None => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.operations_ui = None;
                    }
                }
            }
            Panel::Board => {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                self.board_ui = None;
            }
            Panel::Ledger => {
                match &self.ledger_ui {
                    Some(LedgerUiState::ConfirmBuy { .. }) | Some(LedgerUiState::ConfirmSell { .. }) | Some(LedgerUiState::ConfirmBailout { .. }) => {
                        self.ledger_ui = Some(LedgerUiState::BrowseStocks);
                        self.panel_selection = 0;
                    }
                    Some(LedgerUiState::BrowseStocks) | None => {
                        self.open_panel = Panel::None;
                        self.panel_selection = 0;
                        self.ledger_ui = None;
                    }
                }
            }
            Panel::None | Panel::Threats | Panel::Help => {
                self.open_panel = Panel::None;
                self.panel_selection = 0;
                self.medicine_ui = None;
                self.research_ui = None;
                self.policy_ui = None;
                self.operations_ui = None;
                self.board_ui = None;
                self.ledger_ui = None;
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
        self.board_ui = None;
        self.ledger_ui = None;
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

        // Per-subsystem RNG streams. Emergence uses the raw seed so the
        // starting disease for a given seed is unchanged. Other streams use
        // wrapping_add offsets to produce independent sequences.
        let rng_spread = ChaCha8Rng::seed_from_u64(seed.wrapping_add(1));
        let mut rng_emergence = ChaCha8Rng::seed_from_u64(seed);
        let rng_crisis = ChaCha8Rng::seed_from_u64(seed.wrapping_add(3));
        let rng_research = ChaCha8Rng::seed_from_u64(seed.wrapping_add(4));
        let rng_misc = ChaCha8Rng::seed_from_u64(seed.wrapping_add(5));

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
                    cooperation: 65.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                    last_sick_tick: None,
                    dead: false,
                    succession_tick: None,
                },
                infections: vec![],
                traits: vec![RegionTrait::TradeDependent, RegionTrait::StrongPublicHealth],
                specialization: Some(RegionSpecialization::PharmaHub),
                collapse_threshold: 0.55, // Fragile — collapses at 45% dead
                dead: 0.0,
                collapsed: false,
                abandoned: false,
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
                disrupted_until: None,
                collapse_supply_penalty: 1.0,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
                base_gdp: 280.0,  // Large, wealthy economy
                gdp: 280.0,
            },
            Region {
                name: "South America".into(),
                population: 430_000_000,
                connections: vec![0, 3],
                governor: Governor {
                    name: "Gov. Vasquez".into(),
                    personality: GovernorPersonality::Blowhard,
                    cooperation: 70.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                    last_sick_tick: None,
                    dead: false,
                    succession_tick: None,
                },
                infections: vec![],
                traits: vec![RegionTrait::LowInfrastructure, RegionTrait::ResilientPopulation],
                specialization: Some(RegionSpecialization::TropicalMedicine),
                collapse_threshold: 0.55, // Moderate — collapses at 45% dead
                dead: 0.0,
                collapsed: false,
                abandoned: false,
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
                disrupted_until: None,
                collapse_supply_penalty: 1.0,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
                base_gdp: 45.0,   // Moderate economy
                gdp: 45.0,
            },
            Region {
                name: "Europe".into(),
                population: 750_000_000,
                connections: vec![0, 3, 4],
                governor: Governor {
                    name: "Gov. Lindqvist".into(),
                    personality: GovernorPersonality::Operative,
                    cooperation: 75.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                    last_sick_tick: None,
                    dead: false,
                    succession_tick: None,
                },
                infections: vec![],
                traits: vec![RegionTrait::TradeDependent, RegionTrait::DenseUrban],
                specialization: Some(RegionSpecialization::RegulatoryApparatus),
                collapse_threshold: 0.50, // Developed infrastructure — 50% dead
                dead: 0.0,
                collapsed: false,
                abandoned: false,
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
                disrupted_until: None,
                collapse_supply_penalty: 1.0,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
                base_gdp: 210.0,  // Strong, hub economy
                gdp: 210.0,
            },
            Region {
                name: "Africa".into(),
                population: 1_400_000_000,
                connections: vec![1, 2, 4],
                governor: Governor {
                    name: "Gov. Okonkwo".into(),
                    personality: GovernorPersonality::Buffoon,
                    cooperation: 60.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                    last_sick_tick: None,
                    dead: false,
                    succession_tick: None,
                },
                infections: vec![],
                traits: vec![RegionTrait::LowInfrastructure, RegionTrait::DenseUrban],
                specialization: Some(RegionSpecialization::CommunityNetworks),
                collapse_threshold: 0.50, // Resilient — 50% dead
                dead: 0.0,
                collapsed: false,
                abandoned: false,
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
                disrupted_until: None,
                collapse_supply_penalty: 1.0,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
                base_gdp: 30.0,   // Lower per-capita economy
                gdp: 30.0,
            },
            Region {
                name: "Asia".into(),
                population: 4_700_000_000,
                connections: vec![2, 3, 5],
                governor: Governor {
                    name: "Gov. Subramaniam".into(),
                    personality: GovernorPersonality::Recluse,
                    cooperation: 70.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                    last_sick_tick: None,
                    dead: false,
                    succession_tick: None,
                },
                infections: vec![],
                traits: vec![RegionTrait::DenseUrban, RegionTrait::ResilientPopulation],
                specialization: Some(RegionSpecialization::LogisticsHub),
                collapse_threshold: 0.50, // Huge population — 50% dead
                dead: 0.0,
                collapsed: false,
                abandoned: false,
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
                disrupted_until: None,
                collapse_supply_penalty: 1.0,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
                base_gdp: 380.0,  // Largest total economy
                gdp: 380.0,
            },
            Region {
                name: "Oceania".into(),
                population: 45_000_000,
                connections: vec![4],
                governor: Governor {
                    name: "Gov. Whitfield".into(),
                    personality: GovernorPersonality::Mobster,
                    cooperation: 75.0,
                    defiance_crisis_fired: false,
                    last_action_tick: 0,
                    bargain_count: 0,
                    income_skim: 0.0,
                    last_sick_tick: None,
                    dead: false,
                    succession_tick: None,
                },
                infections: vec![],
                traits: vec![RegionTrait::IslandGeography, RegionTrait::StrongPublicHealth],
                specialization: Some(RegionSpecialization::SurveillanceNetwork),
                collapse_threshold: 0.50, // Small but developed — 50% dead
                dead: 0.0,
                collapsed: false,
                abandoned: false,
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
                disrupted_until: None,
                collapse_supply_penalty: 1.0,
                estimated_infected: 0.0,
                screening_noise_bias: 0.0,
                base_gdp: 18.0,   // Small but developed economy
                gdp: 18.0,
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
        let chosen_types = vec![available_types[rng_emergence.r#gen::<usize>() % available_types.len()]];

        let mut diseases = Vec::new();
        let mut used_names: Vec<String> = Vec::new();
        for pathogen_type in &chosen_types {
            let mut disease = Disease::generate(&mut rng_emergence, *pathogen_type, &used_names, false);
            disease.detected = true; // starting disease is already detected — player needs something to act on
            used_names.push(disease.name.clone());
            diseases.push(disease);
        }

        // --- Place initial outbreak ---
        // The starting disease has already been detected by global health systems.
        // We seed it well above the detection threshold so the player can immediately
        // see infections on the map and begin field research to identify it.
        let region_idx = rng_emergence.r#gen::<usize>() % regions.len();
        let infected = 1_000.0 + rng_emergence.r#gen::<f64>() * 2_000.0;
        let dead = infected * 0.01; // ~1% already dead when the player takes over
        regions[region_idx].infections.push(RegionDiseaseState {
            disease_idx: 0,
            exposed: 0.0,
            infected,
            dead,
            immune: 0.0,
        });
        regions[region_idx].dead = dead;
        // Record first-detection region for the initial disease (Day 0, single region)
        diseases[0].first_detected_regions = vec![region_idx];
        diseases[0].detected_day = 0.0;
        // Seed the initial estimate — organic reporting catches roughly 15% of cases
        // at the time the player takes over. Without this, the first frame shows
        // "Infected: ~0" despite the briefing saying there's an active outbreak.
        regions[region_idx].estimated_infected = infected * ScreeningLevel::None.visibility_rate();

        // --- Generate medicines to match diseases ---
        // Two targeted medicines per non-prion disease (different mechanisms).
        // Prion diseases get no medicines — they are completely untreatable.
        let mut medicines: Vec<Medicine> = diseases.iter().enumerate()
            .flat_map(|(i, d)| Medicine::targeted_medicines(i, d.pathogen_type))
            .collect();

        // One broad-spectrum medicine targeting all diseases
        let all_disease_indices: Vec<usize> = (0..diseases.len()).collect();
        // Broad-spectrum therapeutic: starts unlocked at limited supply. A blunt bandaid
        // that slows early disease spread while the player develops targeted medicines.
        // 500K doses depletes within ~10 days as infections grow, forcing investment
        // in the research pipeline. Targeted medicines are 6–7x more effective.
        medicines.push(Medicine {
            name: "Broad-Spectrum".into(),
            therapy_type: TherapyType::BroadSpectrum,
            mode: MedicineMode::Therapeutic,
            mechanism: None,
            target_diseases: all_disease_indices.clone(),
            cost: 10.0,
            doses: 500_000.0,
            max_doses: 500_000.0,
            unlocked: true,
            tested_against: all_disease_indices.clone(),
            strain_generations: vec![],
            deployed_count: 0,
            total_treated: 0.0,
            total_protected: 0.0,
            manufacturer_corp_idx: None, // assigned in generate_corporations
        });

        let num_diseases = diseases.len();

        Self {
            tick: 0,
            sim_state: SimState::Running,
            rng_spread,
            rng_emergence,
            rng_crisis,
            rng_research,
            rng_misc,
            resources: Resources {
                funding: 500.0,
                personnel: 20,
                authority: Authority::Minimal,
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
            active_research: vec![],
            unlocked_techs: vec![],
            outcome: GameOutcome::Playing,
            events: vec![],
            auto_deploy_blocked_notified: std::collections::HashSet::new(),
            event_log: VecDeque::new(),
            active_crisis: None,
            crisis_cooldowns: HashMap::new(),
            pending_crises: vec![],
            last_crisis_resolved_tick: 0,
            auto_resolve_crises: HashMap::new(),
            history: vec![],
            auto_repeat_research: vec![],
            auto_deploy: vec![],
            standing_orders: StandingOrders::default(),
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
            contract_decline_counts: Vec::new(),
            corporations: Vec::new(),
            portfolio: Vec::new(),
            cost_basis: Vec::new(),
            board_members: Vec::new(),
            next_board_meeting_tick: 0, // initialized properly after RNG setup
            board_budget_per_tick: 0.0, // set properly after corporations are generated
            reference_base_budget_per_tick: 0.0,
            chairman_hostile_since: None,
            next_sequence_group: 0,
            loans: vec![],
            cumulative_policy_spending: 0.0,
            embezzlement_warned: false,
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
                board_ui: None,
                ledger_ui: None,
                home_splash_done: false,
                home_splash_revealed: false,
                speed_multiplier: 1,
                size_warning_dismissed: false,
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

    /// Whether a specific disease has any active infections (including exposed) globally.
    pub fn disease_has_infected(&self, disease_idx: usize) -> bool {
        self.regions.iter().any(|r| {
            r.disease_state(disease_idx).is_some_and(|inf| inf.exposed + inf.infected > 0.0)
        })
    }

    /// Total infected from detected diseases only (for UI display).
    pub fn total_infected_detected(&self) -> f64 {
        self.regions.iter()
            .flat_map(|r| &r.infections)
            .filter(|inf| self.diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .map(|inf| inf.exposed + inf.infected)
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

    /// Whether the screening level in a region reveals exposed (incubating) individuals.
    /// Requires both an Antigen+ tier AND meaningful ramp-up progress (>50%).
    pub fn screening_shows_exposed(&self, region_idx: usize) -> bool {
        self.policies.get(region_idx)
            .map(|p| p.screening.shows_exposed() && p.screening_progress > 0.5)
            .unwrap_or(false)
    }

    /// Medicine targeting efficiency for a region, accounting for screening_progress.
    /// Returns the fraction of delivered doses that reach valid targets (1.0 = perfect).
    /// Without screening, 50% of doses are wasted on the wrong people.
    pub fn targeting_efficiency(&self, region_idx: usize) -> f64 {
        let (level_eff, progress) = self.policies.get(region_idx)
            .map(|p| (p.screening.targeting_efficiency(), p.screening_progress))
            .unwrap_or((ScreeningLevel::None.targeting_efficiency(), 0.0));
        let base = ScreeningLevel::None.targeting_efficiency();
        base + (level_eff - base) * progress
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
        let research: u32 = self.active_research.iter().map(|p| p.personnel_assigned).sum();
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
        let crisis_ops: u32 = self.crisis_operations.iter().map(|op| op.personnel).sum();
        research + policy + hospitals + intel + crisis_ops
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
                // RegulatoryApparatus specialization: policy costs 25% lower
                let spec_mult = region.map(|r| {
                    if r.has_specialization(RegionSpecialization::RegulatoryApparatus) {
                        REGULATORY_APPARATUS_COST_MULT
                    } else {
                        1.0
                    }
                }).unwrap_or(1.0);
                // Automation sector bonus: policy costs lower
                let auto_bonus = self.sector_bonus(i, CorporationSector::Automation);
                let auto_mult = 1.0 - CorporationSector::Automation.max_bonus_pct() / 100.0 * auto_bonus;
                p.funding_cost(traits) * gov_mult * supply_mult * spec_mult * auto_mult
            })
            .sum()
    }

    /// Compute the GDP target for a region (actual value, not a fraction).
    /// GDP = base_gdp × alive_frac × infra_health × policy_factor × trade_factor.
    /// Deaths cause permanent economic shrinkage; infrastructure degradation
    /// (from infections) provides the dynamic damage path. Trade coupling
    /// means collapsing neighbors drag this region's GDP down too.
    pub fn gdp_target(&self, region_idx: usize) -> f64 {
        let region = &self.regions[region_idx];
        if region.collapsed {
            return 0.0;
        }
        let pop = region.population as f64;
        if pop <= 0.0 {
            return 0.0;
        }

        // Permanent shrinkage from deaths
        let alive_frac = (pop - region.total_dead()) / pop;

        // Infrastructure health: average of the three infrastructure metrics
        let infra_health = (region.healthcare_capacity + region.supply_lines + region.civil_order) / 3.0;

        // Trade coupling: neighbor economic health drags GDP via trade links.
        // Uses current smoothed gdp_fraction (not gdp_target) to avoid circularity.
        // When all neighbors healthy: trade_factor ≈ 1.0. When neighbors collapse: → 0.7.
        // TradeDependent regions feel a stronger hit (→ 0.5 instead of → 0.7).
        let trade_factor = if region.connections.is_empty() {
            1.0
        } else {
            let avg_neighbor_gdp: f64 = region.connections.iter()
                .map(|&c| self.regions[c].gdp_fraction())
                .sum::<f64>() / region.connections.len() as f64;
            let trade_dep = region.traits.contains(&RegionTrait::TradeDependent);
            let trade_weight = if trade_dep { 0.5 } else { 0.3 };
            (1.0 - trade_weight) + trade_weight * avg_neighbor_gdp
        };

        // Active containment policies reduce GDP — the core tension.
        let policy = match self.policies.get(region_idx) {
            Some(p) => p,
            None => return (region.base_gdp * alive_frac * infra_health * trade_factor).max(0.0),
        };
        let mut policy_factor = 1.0;
        if policy.quarantine {
            policy_factor *= 0.80; // 20% GDP hit — people can't work freely
        }
        if policy.travel_ban {
            let trade_dep = region.traits.contains(&RegionTrait::TradeDependent);
            if trade_dep {
                policy_factor *= 0.70; // 30% GDP hit — trade-dependent economy hit harder
            } else {
                policy_factor *= 0.80; // 20% GDP hit — trade disrupted
            }
        }
        if policy.border_controls && !policy.travel_ban {
            // Border controls are superseded by travel ban
            policy_factor *= 0.90; // 10% GDP hit — trade friction
        }
        if policy.martial_law {
            policy_factor *= 0.85; // 15% GDP hit — curfews, restricted movement
        }

        (region.base_gdp * alive_frac * infra_health * policy_factor * trade_factor).max(0.0)
    }

    /// Per-tick funding income: fixed board budget + contracts + decree modifiers.
    pub fn funding_income_rate(&self) -> f64 {
        let mut income = self.board_budget_per_tick;
        // Decree modifiers
        if self.enacted_decrees.sacrificed_region.is_some() {
            income *= SACRIFICE_INCOME_BONUS;
        }
        if self.enacted_decrees.conscript_researchers {
            income = (income - CONSCRIPT_INCOME_PENALTY).max(0.0);
        }
        // Embezzlement penalty — board pulls funding if warned and still over-investing.
        if self.embezzlement_warned && self.exceeds_embezzlement_threshold() {
            income *= EMBEZZLEMENT_FUNDING_PENALTY;
        }
        // Contract income — fixed, not affected by board budget
        let contract_income: f64 = self.contracts.iter().map(|c| c.income).sum();
        income + contract_income
    }

    /// Per-tick income from active contracts alone (for UI breakdown).
    pub fn contract_income_rate(&self) -> f64 {
        self.contracts.iter().map(|c| c.income).sum()
    }

    /// Compute the current GDP-derived board budget base from corporate tax revenue.
    /// This tracks the real economy — it shrinks as GDP declines. Used as the
    /// "current" input to compute_board_budget_per_tick (which dampens GDP decline
    /// via the stored reference_base_budget_per_tick) and for UI comparisons.
    pub fn base_board_budget_per_tick(&self) -> f64 {
        let mut total = 0.0;
        for region_idx in 0..self.regions.len() {
            let region = &self.regions[region_idx];
            if region.collapsed { continue; }
            let region_tax: f64 = self.corporations.iter()
                .filter(|c| c.region_idx == region_idx)
                .map(|c| c.tax_contribution())
                .sum();
            let skim_factor = (1.0 - region.governor.income_skim).max(0.0);
            total += region_tax * skim_factor;
        }
        total
    }

    /// Whether the player's non-board stock positions exceed the embezzlement threshold.
    pub fn exceeds_embezzlement_threshold(&self) -> bool {
        self.non_board_portfolio_value() > self.cumulative_policy_spending + EMBEZZLEMENT_BUFFER
    }

    /// Market value of player's shares in non-board-seat corporations.
    pub fn non_board_portfolio_value(&self) -> f64 {
        self.portfolio.iter().enumerate()
            .filter_map(|(i, &shares)| {
                if shares == 0 { return None; }
                let corp = self.corporations.get(i)?;
                if corp.board_seat { return None; }
                Some(shares as f64 * corp.share_price)
            })
            .sum()
    }

    /// Total outstanding debt across all active loans.
    pub fn total_debt(&self) -> f64 {
        self.loans.iter().map(|l| l.outstanding).sum()
    }

    /// Per-day interest cost on all active loans (for budget display).
    pub fn daily_debt_service(&self) -> f64 {
        self.loans.iter().map(|l| l.outstanding * l.daily_interest_rate).sum()
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

    /// Effective Authority requirement for a policy in a specific region.
    /// Regional severity (infection rate) can lower the requirement by one level —
    /// a crisis in a region justifies action even with low global authority.
    pub fn effective_authority_requirement(&self, region_idx: usize, policy: PolicyId) -> Option<Authority> {
        let base = match policy.authority_requirement() {
            Some(a) => a,
            None => return None, // Always available
        };
        let region = match self.regions.get(region_idx) {
            Some(r) => r,
            None => return Some(base),
        };
        let pop = region.population as f64;
        if pop <= 0.0 { return Some(base); }
        let infected: f64 = region.infections.iter().map(|i| i.infected).sum();
        let severity = (infected / pop).min(1.0);
        // High severity (>50% infected) lowers requirement by one level
        if severity > 0.5 {
            Some(base.lower())
        } else {
            Some(base)
        }
    }

    /// Whether a policy's research prerequisite is satisfied (or has none).
    pub fn policy_research_met(&self, policy: PolicyId) -> bool {
        match policy.research_prerequisite() {
            Some(tech) => self.unlocked_techs.contains(&tech),
            None => true,
        }
    }

    /// Whether a policy can be activated given current authority level, regional severity,
    /// and research prerequisites.
    pub fn policy_unlocked(&self, region_idx: usize, policy: PolicyId) -> bool {
        self.policy_research_met(policy)
            && match self.effective_authority_requirement(region_idx, policy) {
                Some(req) => self.resources.authority >= req,
                None => true,
            }
    }

    /// Single source of truth for decree unlock conditions.
    pub fn decree_unlock_condition(decree: DecreeId) -> DecreeUnlockCondition {
        match decree {
            DecreeId::ConscriptResearchers => DecreeUnlockCondition { min_infected: Some(500_000.0), min_dead: Some(100_000.0), ..Default::default() },
            DecreeId::AuthorizeHumanTrials => DecreeUnlockCondition { min_dead: Some(50_000_000.0), min_crit_regions: Some(2), ..Default::default() },
            DecreeId::SacrificeRegion => DecreeUnlockCondition { min_dead: Some(500_000_000.0), min_crit_regions: Some(1), ..Default::default() },
            DecreeId::SuspendRegionalAuthority => DecreeUnlockCondition { min_dead: Some(100_000_000.0), min_crit_regions: Some(3), ..Default::default() },
            DecreeId::FortifyRegion => DecreeUnlockCondition { min_dead: Some(200_000_000.0), min_collapsed_regions: Some(1), ..Default::default() },
            DecreeId::EmergencyCountermeasure => DecreeUnlockCondition { min_dead: Some(2_000_000_000.0), min_collapsed_regions: Some(3), ..Default::default() },
        }
    }

    /// Human-readable unlock condition for a decree, shown in the policy panel when locked.
    pub fn decree_unlock_hint(decree: DecreeId) -> String {
        format!("Unlocks: {}", Self::decree_unlock_condition(decree).describe())
    }

    /// Whether a decree is unlocked based on current crisis severity.
    pub fn decree_unlocked(&self, decree: DecreeId) -> bool {
        Self::decree_unlock_condition(decree).is_met(self)
    }

    /// Whether a personality-specific bargain is available for the given region.
    /// Requires: non-collapsed region, defiant governor, and personality-specific
    /// preconditions (Technocrat needs active applied research).
    pub fn bargain_available(&self, region_idx: usize) -> bool {
        let region = match self.regions.get(region_idx) {
            Some(r) => r,
            None => return false,
        };
        if region.collapsed || region.governor.is_dead() || !region.governor.is_defiant() {
            return false;
        }
        // All personality types can always bargain when defiant
        true
    }

    /// The personality of the current chairman, if any.
    /// Returns None if there's no chairman or the chairman is a governor (no personality).
    pub fn chairman_personality(&self) -> Option<BoardPersonality> {
        self.board_members.iter()
            .find(|m| m.is_chairman)
            .and_then(|m| m.personality)
    }

    /// Board satisfaction: average satisfaction across all board members (0.0–1.0).
    /// Corporate leaders track stock price relative to IPO; governors track population
    /// health. Stock price has natural lag via mean-reversion, avoiding the perverse
    /// incentive where good policies instantly tank satisfaction.
    pub fn board_satisfaction(&self) -> f64 {
        // Use individual board member satisfactions when available.
        if !self.board_members.is_empty() {
            let (weighted_total, weight_count) = self.board_members.iter().fold(
                (0.0_f64, 0.0_f64),
                |(total, count), m| {
                    let w = if m.is_chairman { 2.0 } else { 1.0 };
                    (total + m.satisfaction * w, count + w)
                },
            );
            return weighted_total / weight_count;
        }
        // Fallback for states without board members (old saves being loaded).
        let board_corps: Vec<&Corporation> =
            self.corporations.iter().filter(|c| c.board_seat).collect();
        if board_corps.is_empty() {
            return 0.0;
        }
        let total: f64 = board_corps.iter().map(|c| {
            if c.bankrupt { 0.0 } else {
                (c.share_price / c.ipo_price).clamp(0.0, 1.0)
            }
        }).sum();
        total / board_corps.len() as f64
    }

    /// Contract confidence: average condition-satisfaction across active contracts (0.0–1.0).
    /// Returns 0.0 when no contracts exist — no contracts means no contract-based
    /// authority contribution. This is neutral (not negative).
    pub fn contract_confidence(&self) -> f64 {
        if self.contracts.is_empty() {
            return 0.0;
        }
        let total: f64 = self.contracts.iter().map(|c| c.satisfaction).sum();
        total / self.contracts.len() as f64
    }

    /// Authority pressure score: how urgently the board should be granting authority.
    /// Computed from crisis severity, board satisfaction, and contract compliance.
    /// Returns (crisis_component, board_component, contract_component) that sum to pressure.
    /// Used by the dashboard to show a breakdown and by board meetings to decide authority.
    pub fn authority_pressure_components(&self) -> (f64, f64, f64) {
        let total_infected = self.total_infected_detected();
        let total_dead = self.total_dead_detected();
        let collapsed = self.regions.iter().filter(|r| r.collapsed).count() as f64;
        let total_regions = self.regions.len().max(1) as f64;

        // Crisis severity: the primary driver of authority (0.0–0.55).
        let infection_severity = (total_infected / 100_000.0).min(1.0).sqrt();
        let death_severity = (total_dead / 20_000.0).min(1.0).sqrt();
        let collapse_severity = (collapsed / total_regions).min(1.0);
        let crisis_component = (infection_severity * 0.25 + death_severity * 0.20 + collapse_severity * 0.10).min(0.55);

        // Board satisfaction: happy board members grant more authority (0.0–0.20)
        let humanitarian_bonus = if self.chairman_personality() == Some(BoardPersonality::Humanitarian) {
            0.05
        } else {
            0.0
        };
        let board_component = self.board_satisfaction() * 0.20 + humanitarian_bonus;

        // Contract confidence: contract compliance amplifies authority (0.0–0.20).
        let contract_component = self.contract_confidence() * 0.20;

        (crisis_component, board_component, contract_component)
    }

    /// Total authority pressure score (0.0–0.95).
    pub fn authority_pressure(&self) -> f64 {
        let (c, b, ct) = self.authority_pressure_components();
        (c + b + ct).clamp(0.0, 0.95)
    }

    /// The authority level the board would grant given current pressure.
    /// Board meetings use this to decide whether to raise or lower authority.
    pub fn suggested_authority(&self) -> Authority {
        let pressure = self.authority_pressure();
        if pressure >= 0.75 { Authority::Maximum }
        else if pressure >= 0.60 { Authority::High }
        else if pressure >= 0.45 { Authority::Medium }
        else if pressure >= 0.30 { Authority::Low }
        else if pressure >= 0.15 { Authority::VeryLow }
        else { Authority::Minimal }
    }

    /// Returns a reference to the board member's active satisfaction modifiers.
    /// Used by the UI to display the approval breakdown. Each modifier has a
    /// source label and value. The sum of all values (clamped 0–1) equals satisfaction.
    pub fn member_satisfaction_modifiers(&self, member_idx: usize) -> &[SatisfactionModifier] {
        &self.board_members[member_idx].modifiers
    }

    /// The next policy that would unlock at a higher authority level.
    /// Returns (name, required_authority) for the lowest-authority policy not yet
    /// globally available, or None if all are already unlocked.
    pub fn next_authority_unlock(&self) -> Option<(&'static str, Authority)> {
        let current = self.resources.authority;
        let mut best: Option<(&'static str, Authority)> = None;
        for &policy in &PolicyId::ALL {
            let required = match policy.authority_requirement() {
                Some(req) => req,
                None => continue, // Always available
            };
            if current >= required {
                continue; // Already unlocked
            }
            if best.is_none() || required < best.unwrap().1 {
                best = Some((policy.display_name(), required));
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
    pub(crate) fn available_field_projects(&self) -> Vec<ResearchKind> {
        let active_kinds: Vec<&ResearchKind> = self.active_research.iter()
            .filter(|p| p.kind.category() == ResearchCategory::Field)
            .map(|p| &p.kind).collect();
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
        // Competitive Displacement: fully known diseases, when tech is unlocked
        if self.unlocked_techs.contains(&BasicTech::CompetitiveDisplacement) {
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
        // Gene Drive Containment: fully known diseases with cross-region spread, when tech is unlocked
        if self.unlocked_techs.contains(&BasicTech::GeneDriveContainment) {
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

    /// Returns the indices of unlocked medicines in `self.medicines`, in iteration order.
    ///
    /// Used by both the medicines UI renderer and the confirm handler so both
    /// operate on the same ordered list without independently rebuilding it.
    /// `panel_selection` in `BrowseMedicines` indexes into this list.
    pub fn unlocked_medicine_indices(&self) -> Vec<usize> {
        self.medicines
            .iter()
            .enumerate()
            .filter(|(_, m)| m.unlocked)
            .map(|(i, _)| i)
            .collect()
    }

    /// All active projects in a given category.
    pub fn active_in_category(&self, cat: ResearchCategory) -> Vec<&ResearchProject> {
        self.active_research.iter()
            .filter(|p| p.kind.category() == cat)
            .collect()
    }

    /// All available research projects across all categories, concatenated.
    /// Field projects first, then Applied, then Basic.
    /// The index into this list is used by `ResearchFlatItem::Available` and
    /// `GameCommand::StartResearch`.
    pub fn all_available_projects(&self) -> Vec<ResearchKind> {
        let mut all = self.available_field_projects();
        all.extend(self.available_applied_projects());
        all.extend(self.available_basic_projects());
        all
    }

    /// Build the flat list of selectable items for the research panel.
    /// Used by both the renderer and the input handler.
    pub fn research_flat_items(&self) -> Vec<ResearchFlatItem> {
        let available = self.all_available_projects();
        let mut items = Vec::new();
        let mut claimed_active = vec![false; self.active_research.len()];

        // Build a stable canonical ordering by interleaving active projects
        // back into the available list at their natural position.
        // all_available_projects() excludes active projects, so we need to
        // merge them back in by category order (Field, Applied, Basic).
        for cat in [ResearchCategory::Field, ResearchCategory::Applied, ResearchCategory::Basic] {
            // Collect available items in this category with their index into
            // the full available list
            let avail_in_cat: Vec<(usize, &ResearchKind)> = available.iter()
                .enumerate()
                .filter(|(_, k)| k.category() == cat)
                .collect();
            // Collect active items in this category
            let active_in_cat: Vec<(usize, &ResearchProject)> = self.active_research.iter()
                .enumerate()
                .filter(|(_, p)| p.kind.category() == cat)
                .collect();

            // Active items first (they were started earlier, so lead their category),
            // then available items. This keeps active items in their category
            // group rather than jumping to a separate section.
            for (ai, _proj) in &active_in_cat {
                claimed_active[*ai] = true;
                items.push(ResearchFlatItem::Active(*ai));
            }
            for (avail_idx, _kind) in &avail_in_cat {
                items.push(ResearchFlatItem::Available(*avail_idx));
            }
        }

        // Append any unclaimed active projects (edge case: conditions changed)
        for (ai, _) in self.active_research.iter().enumerate() {
            if !claimed_active[ai] {
                items.push(ResearchFlatItem::Active(ai));
            }
        }

        if self.lab_level < 2 {
            items.push(ResearchFlatItem::UpgradeLab);
        }

        items
    }

    /// Project costs adjusted for unlocked technologies.
    /// - RapidSequencing halves GenomicSequencing duration.
    /// - MetagenomicSurveillance cuts IdentifyThreat, ClinicalTrial, and FieldOperations by 25%.
    ///   (Does not affect GenomicSequencing — already covered by RapidSequencing.)
    ///   Corp health modifier (25% → 35%) tracked in #1381.
    /// - AutomatedSynthesis cuts ManufactureDoses duration by 35%.
    pub fn effective_costs(&self, kind: &ResearchKind) -> (u32, f64, f64) {
        let (personnel, mut duration, mut funding) = kind.costs(&self.medicines);
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
        ) && self.unlocked_techs.contains(&BasicTech::MetagenomicSurveillance)
        {
            duration *= 0.75;
        }
        if matches!(kind, ResearchKind::ManufactureDoses { .. })
            && self.unlocked_techs.contains(&BasicTech::AutomatedSynthesis)
        {
            duration *= 0.65; // 35% faster
        }
        // ManufactureDoses: scale cost and time by how depleted the stockpile is.
        // At 0 doses you pay full price; at 90% full you pay 10%.
        if let ResearchKind::ManufactureDoses { medicine_idx } = kind {
            let depletion = self.manufacture_depletion_fraction(*medicine_idx);
            duration *= depletion;
            funding *= depletion;
        }
        // Technocrat chairman: 10% research funding discount
        if self.chairman_personality() == Some(BoardPersonality::Technocrat) {
            funding *= 0.9;
        }
        (personnel, duration, funding)
    }

    /// Fraction of stockpile that needs manufacturing (0.0 = full, 1.0 = empty).
    /// Used to scale ManufactureDoses cost and time proportionally.
    pub fn manufacture_depletion_fraction(&self, medicine_idx: usize) -> f64 {
        let target = self.medicines.get(medicine_idx)
            .map(|m| m.max_doses * self.manufacturing_yield_bonus())
            .unwrap_or(1.0);
        let current = self.medicines.get(medicine_idx)
            .map(|m| m.doses)
            .unwrap_or(0.0);
        ((target - current) / target).clamp(0.05, 1.0) // minimum 5% so it's never free
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

    /// True if the player has unlocked Epidemiological Forecasting (death projections).
    pub fn has_forecasting(&self) -> bool {
        self.unlocked_techs.contains(&BasicTech::EpidemiologicalForecasting)
    }

    /// True if any non-collapsed region with Advanced Intel (level 2) has active
    /// infections of the given disease. Used to show intel assessment data in the
    /// threats panel before full research knowledge is available.
    pub fn has_advanced_intel_on_disease(&self, disease_idx: usize) -> bool {
        self.regions.iter().any(|r| {
            !r.collapsed
                && r.intel_level >= 2
                && r.disease_state(disease_idx)
                    .is_some_and(|inf| inf.infected > 0.0)
        })
    }

    /// Projected deaths over the next `days` for a specific disease across all regions.
    /// Uses current infected * lethality as an instantaneous death rate estimate.
    /// Returns 0.0 if the disease has no active infections.
    pub fn projected_deaths(&self, disease_idx: usize, days: f64) -> f64 {
        let disease = match self.diseases.get(disease_idx) {
            Some(d) => d,
            None => return 0.0,
        };
        let deaths_per_tick: f64 = self.regions.iter()
            .filter_map(|r| r.disease_state(disease_idx))
            .map(|inf| inf.infected * disease.lethality)
            .sum();
        deaths_per_tick * TICKS_PER_DAY * days
    }

    /// Research speed multiplier from lab infrastructure (1.0 / 1.3 / 1.6).
    pub fn lab_speed_multiplier(&self) -> f64 {
        match self.lab_level {
            0 => 1.0,
            1 => 1.3,
            _ => 1.6,
        }
    }

    /// Combined research speed multiplier from lab upgrades and biotech sector bonus.
    /// Multiply by `ResearchProject::speed()` to get the effective per-tick rate.
    pub fn research_infra_multiplier(&self) -> f64 {
        let lab_mult = self.lab_speed_multiplier();
        let biotech_bonus = (0..self.regions.len())
            .map(|r| self.sector_bonus(r, CorporationSector::Biotech))
            .fold(0.0_f64, f64::max);
        let biotech_mult = 1.0 + CorporationSector::Biotech.max_bonus_pct() / 100.0 * biotech_bonus;
        lab_mult * biotech_mult
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

    /// Manufacturing yield bonus from StabilizedFormulation tech.
    pub fn manufacturing_yield_bonus(&self) -> f64 {
        if self.unlocked_techs.contains(&BasicTech::StabilizedFormulation) {
            1.25
        } else {
            1.0
        }
    }

    /// Actual medicine deploy cost for a specific (medicine, region) pair.
    /// Applies disruption multiplier and PharmaHub specialization discount.
    /// Use this for both UI affordability preview and engine-side validation
    /// so they can never drift apart.
    pub fn medicine_deploy_cost(&self, medicine_idx: usize, region_idx: usize) -> f64 {
        let base = self.medicines[medicine_idx].deploy_cost();
        let disruption_mult = if self.regions[region_idx].is_disrupted(self.tick) {
            DISRUPTION_MEDICINE_COST_MULT
        } else {
            1.0
        };
        let spec_mult = if self.regions[region_idx].has_specialization(RegionSpecialization::PharmaHub) {
            PHARMA_HUB_DEPLOY_DISCOUNT
        } else {
            1.0
        };
        base * disruption_mult * spec_mult
    }

    /// Approximate cross-region spread factor from `source` to `dest` (0.0–1.0).
    /// Accounts for travel bans, border controls, screening, island geography,
    /// governor effectiveness, nuclear annihilation, and source collapse.
    /// Uses a representative travel-ban factor (0.2) since this is disease-agnostic;
    /// the engine's per-disease spread uses exact per-transmission factors instead.
    pub fn cross_region_spread_factor(&self, source: usize, dest: usize) -> f64 {
        // Annihilated regions neither send nor receive spread
        if self.policies.get(source).is_some_and(|p| p.nuclear_annihilation)
            || self.policies.get(dest).is_some_and(|p| p.nuclear_annihilation)
        {
            return 0.0;
        }
        let mut factor = 1.0;

        let src_pol = self.policies.get(source);
        let dst_pol = self.policies.get(dest);

        let src_ban = src_pol.is_some_and(|p| p.travel_ban);
        let dst_ban = dst_pol.is_some_and(|p| p.travel_ban);
        let src_border = src_pol.is_some_and(|p| p.border_controls);
        let dst_border = dst_pol.is_some_and(|p| p.border_controls);

        // Governor effectiveness (min of both endpoints)
        let src_eff = self.regions.get(source).map(|r| r.policy_effectiveness()).unwrap_or(1.0);
        let dst_eff = self.regions.get(dest).map(|r| r.policy_effectiveness()).unwrap_or(1.0);
        let eff = src_eff.min(dst_eff);

        if src_ban || dst_ban {
            // Representative travel-ban factor (~0.2, averaging across transmission types)
            factor *= 1.0 - (1.0 - 0.2) * eff;
        } else if src_border || dst_border {
            factor *= 1.0 - (1.0 - 0.7) * eff;
        }

        // Screening at both endpoints
        let src_screening = src_pol.map(|p| p.screening.spread_factor()).unwrap_or(1.0);
        let dst_screening = dst_pol.map(|p| p.screening.spread_factor()).unwrap_or(1.0);
        factor *= 1.0 - (1.0 - src_screening.min(dst_screening)) * eff;

        // Island geography: 50% less inbound spread
        if self.regions[dest].has_trait(RegionTrait::IslandGeography) {
            factor *= 0.5;
        }

        // Collapsed source emits less spread
        if self.regions[source].collapsed {
            factor *= 0.3;
        }

        factor
    }

    /// Passive bonus strength for a sector in a region (0.0–1.0).
    /// Returns 0.0 if no non-bankrupt corp of that sector exists in the region.
    /// Scales with average revenue health (revenue / base_revenue) of matching corps.
    pub fn sector_bonus(&self, region_idx: usize, sector: CorporationSector) -> f64 {
        let mut total_health = 0.0;
        let mut count = 0;
        for corp in &self.corporations {
            if corp.region_idx == region_idx && corp.sector == sector && !corp.bankrupt {
                let health = if corp.base_revenue > 0.0 {
                    (corp.revenue / corp.base_revenue).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                total_health += health;
                count += 1;
            }
        }
        if count == 0 {
            return 0.0;
        }
        total_health / count as f64
    }

    /// All active sector bonuses for a region. Returns (sector, strength) pairs
    /// for sectors with at least one non-bankrupt corp.
    pub fn active_sector_bonuses(&self, region_idx: usize) -> Vec<(CorporationSector, f64)> {
        let mut bonuses = Vec::new();
        for &sector in &[
            CorporationSector::Energy,
            CorporationSector::Logistics,
            CorporationSector::Biotech,
            CorporationSector::Mining,
            CorporationSector::DataInfra,
            CorporationSector::Automation,
        ] {
            let strength = self.sector_bonus(region_idx, sector);
            if strength > 0.0 {
                bonuses.push((sector, strength));
            }
        }
        bonuses
    }

    /// Dose cost for an emergency sample delivery. Returns the number of doses consumed.
    pub fn emergency_delivery_dose_cost(&self, medicine_idx: usize) -> f64 {
        let med = &self.medicines[medicine_idx];
        (med.max_doses * 0.10).min(50.0).min(med.doses)
    }

    /// Funding cost for an emergency sample delivery (half of a normal deployment).
    pub fn emergency_delivery_funding_cost(&self, medicine_idx: usize, region_idx: usize) -> f64 {
        self.medicine_deploy_cost(medicine_idx, region_idx) * 0.5
    }

    /// Available basic research projects — techs whose prereqs are met and not yet unlocked.
    pub(crate) fn available_basic_projects(&self) -> Vec<ResearchKind> {
        let active_kinds: Vec<&ResearchKind> = self.active_research.iter()
            .filter(|p| p.kind.category() == ResearchCategory::Basic)
            .map(|p| &p.kind).collect();
        BasicTech::all()
            .iter()
            .filter(|tech| {
                !self.unlocked_techs.contains(tech)
                    && tech.prerequisites_met(self)
            })
            .map(|&tech| ResearchKind::BasicResearch { tech })
            .filter(|kind| !active_kinds.contains(&kind))
            .collect()
    }

    /// Available applied research projects (excludes currently active).
    pub(crate) fn available_applied_projects(&self) -> Vec<ResearchKind> {
        let active_kinds: Vec<&ResearchKind> = self.active_research.iter()
            .filter(|p| p.kind.category() == ResearchCategory::Applied)
            .map(|p| &p.kind).collect();
        let mut projects = Vec::new();
        for (i, med) in self.medicines.iter().enumerate() {
            if med.unlocked {
                // Unlocked medicines can be manufactured if doses are depleted
                if med.doses < med.max_doses {
                    let kind = ResearchKind::ManufactureDoses { medicine_idx: i };
                    if !active_kinds.contains(&&kind) {
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
                if !active_kinds.contains(&&kind) {
                    projects.push(kind);
                }
            }
        }
        // Train Personnel: always available
        let kind = ResearchKind::TrainPersonnel;
        if !active_kinds.contains(&&kind) {
            projects.push(kind);
        }
        projects
    }

    /// Diseases that are identified but cannot yet be developed in Applied Research, with reasons.
    /// Returns (disease_idx, reason_string) for each blocked disease.
    /// Used to show greyed-out "pending" entries in the Applied Research panel.
    pub fn blocked_medicine_developments(&self) -> Vec<(usize, String)> {
        let active_kinds: Vec<&ResearchKind> = self.active_research.iter()
            .filter(|p| p.kind.category() == ResearchCategory::Applied)
            .map(|p| &p.kind).collect();
        let has_targeted_drug_design = self.unlocked_techs.contains(&BasicTech::TargetedDrugDesign);

        // Collect disease indices already covered by an available or active targeted medicine
        // development option. (The global BroadSpectrum medicine is always unlocked from game
        // start and therefore appears only in ManufactureDoses, never in DevelopMedicine — it
        // cannot pollute this set. Prion diseases have no medicines at all.)
        let mut covered: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for kind in self.available_applied_projects() {
            if let ResearchKind::DevelopMedicine { medicine_idx } = kind {
                for &d_idx in &self.medicines[medicine_idx].target_diseases {
                    covered.insert(d_idx);
                }
            }
        }
        for kind in &active_kinds {
            if let ResearchKind::DevelopMedicine { medicine_idx } = kind {
                for &d_idx in &self.medicines[*medicine_idx].target_diseases {
                    covered.insert(d_idx);
                }
            }
        }

        let mut result = Vec::new();
        let mut seen_diseases: std::collections::HashSet<usize> = std::collections::HashSet::new();

        // Show identified prion diseases as untreatable (no medicines exist for them)
        for (i, disease) in self.diseases.iter().enumerate() {
            if disease.pathogen_type == PathogenType::Prion
                && disease.detected
                && disease.knowledge >= KNOWLEDGE_NAME
            {
                seen_diseases.insert(i);
                result.push((i, "Prion — no known therapeutic intervention".to_string()));
            }
        }

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
                    "{:.0}% knowledge · Targeted Drug Design required",
                    disease.knowledge * 100.0
                ),
                (false, true) => format!(
                    "{:.0}% knowledge · continue Field Research",
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
        assert_eq!(TherapyType::BroadSpectrum.efficacy(&PathogenType::Prion), 0.0);
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
        assert!((trade_cost - base_cost * 1.5).abs() < 0.01,
            "TradeDependent should multiply travel ban cost by 1.5x");
    }

    #[test]
    fn low_infrastructure_increases_personnel() {
        let mut policy = RegionPolicy::default();
        policy.quarantine = true;
        policy.discourage_hosp = true;
        let base = policy.personnel_cost(&[]);
        let low_infra = policy.personnel_cost(&[RegionTrait::LowInfrastructure]);
        // 2 active policies, each +1 = base + 2
        assert_eq!(low_infra, base + 2,
            "LowInfrastructure should add +1 per active policy");
    }

    #[test]
    fn metagenomic_surveillance_prereq_requires_rapid_sequencing() {
        let mut state = GameState::new_default(42);
        // Without RapidSequencing, prereq is not met
        assert!(!state.unlocked_techs.contains(&BasicTech::RapidSequencing));
        assert!(!BasicTech::MetagenomicSurveillance.prerequisites_met(&state));
        // After unlocking RapidSequencing, prereq is met
        state.unlocked_techs.push(BasicTech::RapidSequencing);
        assert!(BasicTech::MetagenomicSurveillance.prerequisites_met(&state));
    }

    #[test]
    fn metagenomic_surveillance_reduces_identify_threat_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::IdentifyThreat { disease_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.75).abs() < 0.01,
            "IdentifyThreat should be 25% faster: expected {}, got {}",
            base_duration * 0.75,
            fast_duration
        );
    }

    #[test]
    fn metagenomic_surveillance_reduces_clinical_trial_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::ClinicalTrial { medicine_idx: 0, disease_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.75).abs() < 0.01,
            "ClinicalTrial should be 25% faster: expected {}, got {}",
            base_duration * 0.75,
            fast_duration
        );
    }

    #[test]
    fn metagenomic_surveillance_reduces_field_operations_duration() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::FieldOperations { region_idx: 0, system: InfraSystem::Healthcare };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        let (_, fast_duration, _) = state.effective_costs(&kind);
        assert!(
            (fast_duration - base_duration * 0.75).abs() < 0.01,
            "FieldOperations should be 25% faster: expected {}, got {}",
            base_duration * 0.75,
            fast_duration
        );
    }

    #[test]
    fn metagenomic_surveillance_does_not_affect_genomic_sequencing() {
        let mut state = GameState::new_default(42);
        let kind = ResearchKind::GenomicSequencing { disease_idx: 0 };
        let (_, base_duration, _) = state.effective_costs(&kind);
        state.unlocked_techs.push(BasicTech::MetagenomicSurveillance);
        let (_, after_duration, _) = state.effective_costs(&kind);
        assert!(
            (base_duration - after_duration).abs() < 0.01,
            "GenomicSequencing should not be affected by MetagenomicSurveillance"
        );
    }

    #[test]
    fn metagenomic_surveillance_appears_in_all_after_rapid_sequencing() {
        let all = BasicTech::all();
        let rs_pos = all.iter().position(|t| *t == BasicTech::RapidSequencing).unwrap();
        let ps_pos = all.iter().position(|t| *t == BasicTech::MetagenomicSurveillance).unwrap();
        assert!(
            ps_pos == rs_pos + 1,
            "MetagenomicSurveillance should appear immediately after RapidSequencing in all()"
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
        assert!(!BasicTech::StabilizedFormulation.prerequisites_met(&state));
        state.unlocked_techs.push(BasicTech::AutomatedSynthesis);
        assert!(BasicTech::StabilizedFormulation.prerequisites_met(&state));
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
    fn stabilized_formulation_boosts_manufacturing_yield() {
        let mut state = GameState::new_default(42);
        let base = state.manufacturing_yield_bonus();
        assert!((base - 1.0).abs() < 0.001, "base yield should be 1.0 without tech");
        state.unlocked_techs.push(BasicTech::StabilizedFormulation);
        let with_tech = state.manufacturing_yield_bonus();
        assert!(
            (with_tech - 1.25).abs() < 0.001,
            "StabilizedFormulation should give 1.25x yield: expected 1.25, got {}",
            with_tech
        );
    }

    #[test]
    fn automated_synthesis_and_distributed_storage_appear_in_all() {
        let all = BasicTech::all();
        assert!(all.contains(&BasicTech::AutomatedSynthesis), "AutomatedSynthesis must be in all()");
        assert!(all.contains(&BasicTech::StabilizedFormulation), "StabilizedFormulation must be in all()");
        let as_pos = all.iter().position(|t| *t == BasicTech::AutomatedSynthesis).unwrap();
        let ds_pos = all.iter().position(|t| *t == BasicTech::StabilizedFormulation).unwrap();
        assert!(ds_pos > as_pos, "StabilizedFormulation should appear after AutomatedSynthesis");
    }

    #[test]
    fn board_budget_is_fixed_between_meetings() {
        // Board budget doesn't change when regions get infected — it's set at board meetings.
        let mut state = GameState::new_default(42);
        crate::engine::initialize_game(&mut state);

        let baseline = state.funding_income_rate();
        assert!(baseline > 0.0, "should have positive income from board budget");

        // Infect a region heavily — income should NOT change
        let pop0 = state.regions[0].population as f64;
        state.regions[0].infections.push(RegionDiseaseState {
            disease_idx: 0,
            exposed: 0.0,
            infected: pop0 * 0.30,
            dead: 0.0,
            immune: 0.0,
        });
        let after_infection = state.funding_income_rate();
        assert!(
            (after_infection - baseline).abs() < 0.001,
            "board budget should be fixed: baseline={baseline:.4}, after_infection={after_infection:.4}"
        );
    }
}
