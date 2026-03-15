# Disease Model Expansion Plan

Phased plan for building out the disease simulation and threats panel.

## Phase 1: SIR Model + Recovery

The foundation. Without recovery, every disease is 100% fatal given enough time.

### State changes (`src/state.rs`)

`Disease` gains:
- `recovery_rate: f64` — fraction of infected who recover per tick

`RegionInfection` gains:
- `recovered: f64` — people who had this disease and recovered (immune)

Both use `#[serde(default)]` for save file compatibility.

Add `total_recovered()` helpers on `Region` and `AppState`.

### Engine changes (`src/engine.rs`)

Susceptible calculation becomes: `pop - infected - recovered - dead`

Each tick, deaths and recoveries are concurrent processes on the infected pool:
```
new_deaths = lethality * infected * noise
new_recoveries = recovery_rate * infected * noise
// Cap so total outflow doesn't exceed infected
if new_deaths + new_recoveries > infected:
    scale = infected / (new_deaths + new_recoveries)
    new_deaths *= scale
    new_recoveries *= scale
infected += new_infections - new_deaths - new_recoveries
recovered += new_recoveries
dead += new_deaths
```

Key ratio: `recovery_rate / (recovery_rate + lethality)` = survival probability.
With `recovery_rate: 0.10, lethality: 0.02` → ~83% survive.

### Second disease

Add Strain Beta to `new_default()` in a different region. Different character:
slower within-region spread, less lethal, higher cross-region spread, slow recovery (chronic).

### Threats panel (`src/ui/threats.rs`)

- Show recovery rate in disease stats line
- When disease is selected, show per-region table: infected / recovered / dead
- Global totals for the selected disease

### Tests

- Population conservation: `infected + recovered + dead <= population`
- Recovery-specific: `total_recovered() > 0` after many ticks
- Update determinism test to assert on `total_recovered()`
- All 3 insta snapshots need review/update

---

## Phase 2: SEIR (Incubation)

### State changes

`Disease` gains:
- `incubation_period: f64` — average ticks before exposed become infectious (default 0.0 = instant, backward compat)

`RegionInfection` gains:
- `exposed: f64` — infected but not yet symptomatic/spreading

### Engine changes

Flow: susceptible → exposed → infected → recovered/dead

Transition rate from exposed to infected: `1.0 / incubation_period`. When `incubation_period` is 0, rate is 1.0 — exposed becomes infected instantly, matching Phase 1 behavior.

Cross-region spread seeds `exposed: 1.0` instead of `infected: 1.0`.

Use independent noise rolls per transition (exposure, incubation, death, recovery) instead of sharing one noise value.

### Gameplay implication

Diseases are invisible during incubation. Future hook for surveillance/early detection. For now we show exposed counts, but this could be hidden behind field research later.

### Tests

- `incubation_delays_infection`: disease with high incubation_period, verify infected builds slowly
- Update population conservation: `exposed + infected + recovered + dead <= population`
- Snapshot updates

---

## Phase 3: Transmission Modes

### State changes

New enum on `Disease`:
```rust
enum TransmissionMode { Airborne, Waterborne, Vector, Contact, Bodily }
```

Default: `Airborne` (backward compat with Strain Alpha).

### Engine changes

Multiplier on `cross_region_spread` during cross-region check:
- Airborne: 2.0x
- Waterborne: 1.5x
- Vector: 1.0x
- Contact: 0.5x
- Bodily: 0.3x

No other mechanical changes. The real payoff comes later when countermeasures key off transmission mode.

### UI

Display mode on disease stats line with per-mode color coding.

---

## Phase 4: Mutation & Drug Resistance

### State changes

`Disease` gains:
- `mutation_rate: f64` — probability of parameter drift per tick
- `drug_resistance: f64` — 0.0 to 1.0, reduces future treatment effectiveness
- `parent_idx: Option<usize>` — link to parent disease if this is a variant
- `generation: u32` — 0 = original, increments with each variant

### Engine changes

Two levels of mutation in `tick()`:

**Parameter drift:** With probability `mutation_rate`, small random walk on within-region spread, lethality, drug_resistance. Gradual evolution.

**Variant spawning:** Very rare (`mutation_rate * 0.01`), only when total infected > 10K. Creates a new Disease with modified params, `parent_idx` pointing back. Caps: no variants beyond generation 3, max ~10 total diseases.

Mutations happen on `new.diseases` while infection math uses `state.diseases` (the pre-tick snapshot). Mutations take effect next tick. This must be maintained.

### Design decision

Keep vec indices as disease identifiers. Never remove diseases from the vec — extinct diseases stay in the list with zero infected. This keeps indices stable without needing an ID system.

Drug resistance: treatment effectiveness = `base_effectiveness * (1.0 - drug_resistance)` (matters when medicines exist).

### Interaction with Phase 5 (waning immunity)

Variants raise a question: does recovery from the parent strain grant immunity to the variant? Options:
- **Full cross-immunity:** recovering from any strain in a lineage immunizes against all. Simple but unrealistic.
- **No cross-immunity:** each variant is treated as independent. Simple but makes variants extremely dangerous.
- **Partial cross-immunity via antigenic distance:** track how far a variant has drifted from its parent. Cross-immunity = `1.0 - drift`. Requires a `drift: f64` field on Disease and modifying the susceptible calculation to account for recovered counts from related strains.

Recommendation: start with full cross-immunity (simplest). Add antigenic distance only if gameplay needs it. The `parent_idx` lineage tracking makes it possible to retrofit later.

### Tests

- `mutation_drift_occurs`: high mutation_rate, many ticks, verify within-region spread changed
- `variant_spawns`: high mutation_rate + high infected, verify `diseases.len()` increases
- Determinism still holds (seeded RNG)

---

## Phase 5: Waning Immunity (SIRS)

### State changes

`Disease` gains:
- `immunity_waning_rate: Option<f64>` — `None` = permanent immunity, `Some(rate)` = fraction of recovered who lose immunity per tick

### Engine changes

Each tick: `recovered -= recovered * waning_rate`. Those people rejoin the susceptible pool implicitly (susceptible is computed, never stored).

Note: this is exponential decay, not a fixed duration. With `waning_rate: 0.02`, the half-life is ~35 ticks (`ln(2)/0.02`). The field is named "rate" not "duration" to avoid confusion about the decay curve.

### Gameplay

Creates epidemic waves. Diseases with short immunity cycle through the population, motivating permanent solutions (vaccines, eradication) over waiting for burnout.

### Tests

- `waning_immunity_reinfects`: short immunity_duration, many ticks, verify infections rise again after initial wave

---

## Phase 6: Severity

`severity` already exists on `Disease` but is unused.

### Engine changes

Resource drain: `funding -= severity * infected * cost_factor` per tick. Immediate mechanical impact without needing hospital capacity.

### Future hook

Hospital capacity: when `infected * severity` exceeds a region's capacity threshold, death rate increases. Not implemented here — just the data and display.

### UI

Show severity in threats panel. Derived "hospitalization load" per region.

---

## Cross-Cutting Concerns

**Save compatibility:** Every new field uses `#[serde(default)]` or `#[serde(default = "fn")]`. Add a test that deserializes a hardcoded JSON string from the pre-change format after each phase.

**Snapshot tests:** All 3 insta snapshots break with every phase. Run `cargo insta review` each time.

**Population conservation:** After Phase 2, the invariant is `exposed + infected + recovered + dead + susceptible == population`. Add debug assertion in `tick()`.

**`Region::alive()`** stays correct through all phases — it's `population - dead`, regardless of infection state.

**Performance:** Clone-and-mutate is fine with 6 regions and a handful of diseases. If variant spawning creates many diseases, revisit.
