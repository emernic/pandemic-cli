# Architecture

## Core Principle

All game state (simulation + UI) lives in a single JSON-serializable struct. Each simulation step is two phases: `tick()` advances game logic and produces ephemeral `GameEvent`s, then `process_events()` translates those events into UI presentation (status messages, event log). Both phases are deterministic; events are transient (`#[serde(skip)]`) and always empty at save boundaries. See `docs/target-architecture.md` for the full event system documentation.

## Layers

```
┌─────────────────────────────────────┐
│  main.rs / snapshot.rs              │  I/O boundary
│  Terminal, files, CLI args          │
├─────────────────────────────────────┤
│  lib.rs                             │  Coordination
│  apply_action() routes input:       │
│    UI actions → UiState methods     │
│    Game commands → engine           │
├─────────────────────────────────────┤
│  ui/                                │  Rendering + UI state machines
│  Reads state, renders it.           │
│  Owns panel navigation, wizards,    │
│  selection indices.                 │
├─────────────────────────────────────┤
│  engine/                            │  Game logic only
│  tick() advances simulation.        │
│  execute_command() handles player   │
│  commands. No UI knowledge.         │
├─────────────────────────────────────┤
│  state.rs                           │  Pure data
│  GameState, all structs/enums,      │
│  constants, query methods.          │
└─────────────────────────────────────┘
```

Dependencies flow downward only. UI never imports from engine. Engine never touches UiState.

## Engine Structure

The engine is a module directory. `mod.rs` is the orchestrator — it owns `tick()` and `execute_command()`, which sequence and dispatch to subsystem modules:

```
engine/
  mod.rs       — tick() orchestrator, execute_command() dispatcher, defeat/collapse checks
  research.rs  — start_research(), add/remove_personnel(), tick_research()
  medicine.rs  — deploy_medicine(), tick_shipments(), try_auto_deploy()
  policy.rs    — toggle_policy(), tick_enforce_costs(), tick_governor_loyalty()
  crisis.rs    — generate_crisis(), activate_crisis(), resolve_crisis()
  spread.rs    — tick_spread_within(), tick_spread_cross_region(), tick_mutation()
  disease.rs   — spawn_disease_scaled() (mid-game new threat emergence)
  personnel.rs — scientist assignment, burnout, recovery tick
```

Each subsystem module exposes `pub(super)` functions in two categories:

- **Tick helpers** — called from `tick()` each simulation step (e.g., `tick_research()`, `tick_enforce_costs()`). These advance ongoing processes.
- **Command handlers** — called from `execute_command()` when the player acts (e.g., `start_research()`, `deploy_medicine()`). These handle one-shot player decisions.

Subsystem modules depend only on `state.rs`, never on each other or on UI.

### How to add a new game system

1. Create `engine/newsystem.rs`
2. Add tick helper(s) if it has per-tick behavior → call from `tick()` in mod.rs
3. Add command handler(s) if the player interacts with it → add a `GameCommand` variant in state.rs, dispatch in `execute_command()`
4. Add a `GameEvent` variant if tick events need UI feedback → handle in `ui::process_events()`
5. Keep it `pub(super)` — only mod.rs calls into subsystem modules

### Input flow

```
keypress
  → action.rs: key_to_action() → Action
  → lib.rs: apply_action()
      UI actions (navigate, open panel) → UiState methods
      Confirm → UiState::handle_confirm() → Option<GameCommand>
        → engine::execute_command() → CommandResult { message, success }
        → apply_action maps result to UI navigation (inline)
```

### Tick flow

```
tick() in engine/mod.rs:
  1.  spread::tick_spread_within()     — within-region disease transmission
  2.  spread::tick_spread_cross_region() — inter-region spread via connections
  3.  spread::tick_mutation()          — disease strain evolution
  4.  research::tick_research()        — advance/complete research projects
  5.  personnel::tick_personnel()      — scientist burnout and recovery
  6.  medicine::try_auto_deploy()      — auto-deploy triggered by trial completions
  7.  medicine::tick_shipments()       — deliver in-transit medicine shipments
  8.  policy::tick_enforce_costs()     — suspend unaffordable policies, deduct costs
  9.  policy::tick_governor_loyalty()  — governor loyalty drift
  10. policy::tick_governor_actions()  — defiant governor consequences
  11. Resource income, personnel upkeep, political power drift
  12. Disease detection, threat escalation
  13. Scheduled follow-up crises + crisis::generate_crisis()
  14. RNG write-back + scientist roster sync
  15. Regional collapse checks (may trigger crisis)
  16. Defeat conditions + mercy rule
  17. History recording (sparkline data)
```

## State

One JSON file = one complete save. Includes simulation state (outbreaks, research, resources) AND UI state (open menus, selections, cursor position). If it affects what you see on screen, it's in the file.

`state.rs` is pure data + query methods. It defines all structs, enums, and constants. Game logic lives in engine, not in state — but convenience queries like `available_field_projects()` and `personnel_available()` live on `GameState` so both engine and UI can use them without coupling to each other.

## Snapshot Mode

The game can be run non-interactively for testing:

```
pandemic-cli --snapshot                        # dump the screen and auto-create a resumable save under ./saves/
pandemic-cli saves/playtest.json --snapshot    # load/save an explicit snapshot playthrough file
pandemic-cli saves/playtest.json --snapshot --key "r"
pandemic-cli saves/playtest.json --snapshot --days 1
```

Each invocation: load state → apply inputs → output new state as text → write updated state back to the snapshot save file. If no save path is passed, the CLI auto-creates one under `./saves/` and prints the resume command before the rendered screen. This is how Claude and manual playtesting interact with the game.

## Determinism

RNG is seeded (ChaCha8Rng). Seed is stored in state. Same state + same inputs = same outputs, always.

## Real-Time With Pause

Simulation advances in discrete ticks (internal unit). The UI displays "days" (1 day = 120 ticks, so 5 ticks = 1 hour). "Real-time" = ticks fire automatically. "Pause" = they don't. `SimState` tracks running/paused/event modes. Saves are always at a tick boundary. The engine has no concept of wall-clock time.

## Module Map

```
src/
  lib.rs           — apply_action() routing, re-exports format_number() from state.rs
  main.rs          — CLI args (clap), interactive loop, file I/O
  state.rs         — GameState and all data structs, constants, query methods
  action.rs        — Action enum, key-to-action mapping
  snapshot.rs      — Non-interactive snapshot mode for testing
  engine/
    mod.rs         — tick() orchestrator, execute_command() dispatcher
    research.rs    — Research project commands + tick completion
    medicine.rs    — Medicine deployment + shipment delivery + auto-deploy
    policy.rs      — Policy toggle, decrees, governor actions, per-tick costs
    crisis.rs      — Crisis event generation + resolution
    spread.rs      — Within-region spread, cross-region spread, mutation
    disease.rs     — Disease emergence (spawning new threats mid-game)
    personnel.rs   — Scientist assignment, burnout, recovery
  ui/
    mod.rs         — Layout orchestration, panel routing, process_events()
    home.rs        — Defeat screen
    region_list.rs — World map grid with connections
    threats.rs     — Disease info panel
    medicines.rs   — Medicine deployment wizard
    research.rs    — Research project panel
    policy.rs      — Policy management panel
    resources.rs   — Header status bar
    hotkey_bar.rs  — Footer hotkey legend + status messages
    scientists.rs  — Scientists roster panel
```
