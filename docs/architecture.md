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
│  state.rs                           │  Domain model
│  Data structures, derived           │
│  computations, UI state machines.   │
└─────────────────────────────────────┘
```

Dependencies flow downward only. UI never imports from engine. Engine never touches UiState.

## Engine Structure

The engine is a module directory. `mod.rs` is the orchestrator — it owns `tick()`, `execute_command()`, and `initialize_game()`, which sequence and dispatch to subsystem modules:

```
engine/
  mod.rs            — tick(), execute_command(), initialize_game(): orchestration + cross-cutting logic
  research.rs       — Research projects, scientist assignment, burnout/recovery
  medicine.rs       — Medicine deployment, shipment delivery, auto-deploy
  policy.rs         — Policy toggle, decrees, governor actions, infrastructure builds
  crisis.rs         — Crisis event generation + resolution, board budget calculation
  spread.rs         — Within-region spread, cross-region spread, mutation, adaptation
  disease.rs        — Disease emergence (spawning new scaled diseases mid-game)
  board.rs          — Board member generation and satisfaction
  corporations.rs   — Corporation generation, manufacturer assignment, stock price ticks
  contracts.rs      — Funding contract offers, condition checking, acceptance/rejection
  loans.rs          — Emergency loans, interest accrual
  infrastructure.rs — Hospital/intel infrastructure degradation
```

Each subsystem module exposes `pub(super)` functions in two categories:

- **Tick helpers** — called from `tick()` each simulation step (e.g., `tick_research()`, `tick_enforce_costs()`). These advance ongoing processes.
- **Command handlers** — called from `execute_command()` when the player acts (e.g., `start_research()`, `deploy_medicine()`). These handle one-shot player decisions.

Subsystem modules depend only on `state.rs`, never on each other or on UI.

### How to add a new game system

1. Create `engine/newsystem.rs`
2. Add tick helper(s) if it has per-tick behavior → call from `tick()` in mod.rs
3. Add command handler(s) if the player interacts with it → add a `GameCommand` variant in state.rs, dispatch in `execute_command()`
4. Add a `GameEvent` variant if tick events need UI feedback → handle in `events::process_events()`
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
  1.  spread::tick_spread_within/cross_region/mutation — disease transmission + evolution
  2.  research::tick_research()         — advance/complete research, scientist burnout/recovery
  3.  medicine::try_auto_deploy()       — auto-deploy to worst-affected regions
  4.  medicine::tick_shipments()        — deliver in-transit shipments
  5.  infrastructure::tick_infrastructure() — hospital/intel degradation
  6.  crisis::tick_crisis_operations()  — temporary personnel commitments
  7.  policy::tick_enforce_costs()      — suspend unaffordable policies, deduct costs
  8.  loans (maybe_queue_loan_offer + tick_loans) — emergency loans after suspensions
  9.  policy::tick_governor_cooperation/actions/standing_orders/screening
  10. contracts::tick_check/offer_contracts — funding contract conditions + new offers
  11. corporations::tick_corporations() + board::update_board_satisfaction()
  12. Resource income, personnel upkeep, authority drift, attrition
  13. Disease emergence, detection, threat escalation, intel briefings
  14. Pending crises, board meetings, crisis::generate_crisis()
  15. RNG write-back
  16. Regional collapse (may trigger refugee crisis)
  17. Defeat conditions + history recording
```

## State

One JSON file = one complete save. Includes simulation state (outbreaks, research, resources) AND UI state (open menus, selections, cursor position). If it affects what you see on screen, it's in the file.

`state.rs` is the domain model layer — not just passive data. It contains data structures, derived computations (read-only methods needed by both engine and UI, like `approval_target()` and `funding_income_rate()`), and UI state machines (`UiState` methods for panel navigation). See `docs/target-architecture.md` for the full breakdown. Game logic mutations live in engine/, not in state.

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
    mod.rs            — tick(), execute_command(), initialize_game()
    research.rs       — Research projects, scientist assignment, burnout/recovery
    medicine.rs       — Medicine deployment, shipment delivery, auto-deploy
    policy.rs         — Policy toggle, decrees, governor actions
    crisis.rs         — Crisis generation + resolution, board budget
    spread.rs         — Within-region spread, cross-region spread, mutation
    disease.rs        — Disease emergence (new threats mid-game)
    board.rs          — Board member generation and satisfaction
    corporations.rs   — Corporation generation, stock prices
    contracts.rs      — Funding contract offers and conditions
    loans.rs          — Emergency loans, interest accrual
    infrastructure.rs — Hospital/intel degradation
  events.rs        — Event consequence application (log, notifications, UI resets)
  ui/
    mod.rs         — Layout orchestration, panel routing
    home.rs        — Defeat screen
    region_list.rs — World map grid with connections
    threats.rs     — Disease info panel
    medicines.rs   — Medicine deployment wizard
    research.rs    — Research project panel
    policy.rs      — Policy management panel
    operations.rs  — Decrees and field operations panel
    board.rs       — Board members and satisfaction panel
    ledger.rs      — Stock trading and financial ledger
    resources.rs   — Header status bar
    hotkey_bar.rs  — Footer hotkey legend + status messages
```
