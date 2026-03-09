# Architecture

## Core Principle

All game state (simulation + UI) lives in a single JSON-serializable struct. The entire game is a pure function: `state + input → new state`.

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
  mod.rs       — tick() orchestrator, execute_command() dispatcher, win/lose/collapse checks
  research.rs  — start_research(), boost_research(), tick_research()
  medicine.rs  — deploy_medicine() and dose/efficacy calculations
  policy.rs    — toggle_policy(), tick_enforce_costs()
  crisis.rs    — generate_crisis(), resolve_crisis()
```

Each subsystem module exposes `pub(super)` functions in two categories:

- **Tick helpers** — called from `tick()` each simulation step (e.g., `tick_research()`, `tick_enforce_costs()`). These advance ongoing processes.
- **Command handlers** — called from `execute_command()` when the player acts (e.g., `start_research()`, `deploy_medicine()`). These handle one-shot player decisions.

Subsystem modules depend only on `state.rs`, never on each other or on UI. Disease spread and mutation logic still lives inline in `tick()` in mod.rs.

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
        → UiState::apply_command_result()
```

### Tick flow

```
tick() in engine/mod.rs:
  1. Disease spread (within-region, cross-region)
  2. Disease mutation
  3. research::tick_research()       — advance/complete research projects
  4. policy::tick_enforce_costs()    — suspend unaffordable policies, deduct costs
  5. Resource income (funding, RP)
  6. Disease emergence (mid-game new threats)
  7. crisis::generate_crisis()       — random crisis events
  8. Regional collapse checks
  9. Win/lose condition checks
  10. History recording (sparkline data)
```

## State

One JSON file = one complete save. Includes simulation state (outbreaks, research, resources) AND UI state (open menus, selections, cursor position). If it affects what you see on screen, it's in the file.

`state.rs` is pure data + query methods. It defines all structs, enums, and constants. Game logic lives in engine, not in state — but convenience queries like `available_field_projects()` and `personnel_available()` live on `GameState` so both engine and UI can use them without coupling to each other.

## Snapshot Mode

The game can be run non-interactively for testing:

```
pandemic-cli --snapshot save.json              # dump what the screen looks like
pandemic-cli --snapshot save.json --key "r"    # apply a keypress, dump result
pandemic-cli --snapshot save.json --days 1     # advance 1 day, dump result
```

Each invocation: load state → apply inputs → output new state as text. Stateless. This is how Claude and automated tests interact with the game.

## Determinism

RNG is seeded (ChaCha8Rng). Seed is stored in state. Same state + same inputs = same outputs, always.

## Real-Time With Pause

Simulation advances in discrete ticks (internal unit). The UI displays "days" (1 day = 120 ticks, so 5 ticks = 1 hour). "Real-time" = ticks fire automatically. "Pause" = they don't. `SimState` tracks running/paused/event modes. Saves are always at a tick boundary. The engine has no concept of wall-clock time.

## Module Map

```
src/
  lib.rs           — apply_action() routing, format_number() utility
  main.rs          — CLI args (clap), interactive loop, file I/O
  state.rs         — GameState and all data structs, constants, query methods
  action.rs        — Action enum, key-to-action mapping
  snapshot.rs      — Non-interactive snapshot mode for testing
  engine/
    mod.rs         — tick() orchestrator, execute_command() dispatcher
    research.rs    — Research project commands + tick completion
    medicine.rs    — Medicine deployment logic
    policy.rs      — Policy toggle + per-tick cost enforcement
    crisis.rs      — Crisis event generation + resolution
  ui/
    mod.rs         — Layout orchestration, panel routing, process_events()
    home.rs        — Game over / victory screen
    region_list.rs — World map grid with connections
    threats.rs     — Disease info panel
    medicines.rs   — Medicine deployment wizard
    research.rs    — Research project panel
    policy.rs      — Policy management panel
    resources.rs   — Header status bar
    hotkey_bar.rs  — Footer hotkey legend + status messages
```
