# Target Architecture

Current state and ongoing discipline for the codebase structure.

## Layering

The codebase has four layers. Dependencies flow downward only.

```
main.rs / snapshot.rs     I/O boundary (terminal, files, CLI)
        ↓
lib.rs                    Coordination (apply_action routes input)
        ↓
ui/  |  engine/           Rendering + UI state  |  Game logic
        ↓                         ↓
state.rs                  Pure data (structs, enums, constants, queries)
```

**UI and engine are peers — neither imports from the other.** Both depend on state.rs. The coordination layer in lib.rs connects them: UI state machines translate user intent into `GameCommand`s, and `apply_action()` passes those to `engine::execute_command()`.

### How input flows

```
keypress → action.rs: key_to_action() → Action
         → lib.rs: apply_action()
             UI actions (navigate, select) → UiState methods
             Confirm → UiState::handle_confirm() → Option<GameCommand>
               → engine::execute_command() → CommandResult { message, success }
               → UiState::apply_command_result()
```

`execute_command()` never touches `UiState`. It returns a result and the caller handles UI updates.

### How simulation flows

Each tick, `engine::tick()` orchestrates subsystems in order:

1. Disease spread (within-region, cross-region)
2. Disease mutation
3. `research::tick_research()` — advance/complete research projects
4. `policy::tick_enforce_costs()` — suspend unaffordable policies, deduct costs
5. Resource income (funding, RP)
6. Disease emergence (mid-game new threats)
7. `crisis::generate_crisis()` — random crisis events
8. Regional collapse
9. Win/lose conditions
10. History recording

After each tick, the game loop calls `ui::process_events()` to translate `GameEvent`s into UI responses (status messages, panel resets). Game-rule state transitions (pausing on game-over, entering event mode for crises) happen in `tick()` itself — the UI layer only handles presentation responses.

## Engine Module Structure

The engine is organized as an orchestrator + subsystem modules:

```
engine/
  mod.rs       — tick() and execute_command(): orchestration and dispatch only
  research.rs  — Research project commands + per-tick completion logic
  medicine.rs  — Medicine deployment (dose calculation, efficacy, region effects)
  policy.rs    — Policy toggle commands + per-tick cost enforcement
  crisis.rs    — Crisis event generation + resolution
```

### Subsystem conventions

Each subsystem module follows the same pattern:

- **Visibility:** `pub(super)` — only `mod.rs` calls into subsystems
- **Dependencies:** Only `crate::state`. Never other subsystem modules, never UI
- **Two function types:**
  - **Tick helpers** (called from `tick()`) — advance ongoing processes. Named `tick_*()`. Examples: `tick_research()`, `tick_enforce_costs()`
  - **Command handlers** (called from `execute_command()`) — handle player actions. Examples: `start_research()`, `deploy_medicine()`, `toggle_policy()`
- **No cross-subsystem calls.** If research completion needs to modify medicines (e.g., unlocking one), it does so through `GameState` directly — not by calling into the medicine module. Subsystems share data through state, not through each other.
- **Tests live with the code they test.** Each subsystem module has its own `#[cfg(test)] mod tests`. Integration tests that exercise multiple subsystems through `tick()` or `apply_action()` stay in `engine/mod.rs`.

### Adding a new game system

1. Create `engine/newsystem.rs` with `pub(super)` functions
2. Add `mod newsystem;` in `engine/mod.rs`
3. If it has per-tick behavior: add a `tick_*()` function, call it from `tick()`
4. If the player interacts with it: add a `GameCommand` variant in `state.rs`, add a handler function, dispatch in `execute_command()`
5. If tick events need UI feedback: add a `GameEvent` variant, handle in `ui::process_events()`

### What stays in mod.rs

`tick()` and `execute_command()` are the orchestrators — they stay in mod.rs. So does logic that spans multiple subsystems or doesn't belong to any single one: disease spread, mutation, win/lose checks, regional collapse, disease emergence, history recording. If any of these grow large enough to warrant extraction, they follow the same subsystem pattern.

## Event System

`tick()` produces `GameEvent` variants (stored in `state.events`, cleared each tick). These are ephemeral signals — `#[serde(skip)]`, not persisted.

**Game-rule transitions live in `tick()`:** When the game ends, `tick()` sets `outcome` and `sim_state = Paused`. When a crisis appears, `tick()` sets `active_crisis` and `sim_state = Event { was_running }`. The engine decides *when* to pause, not the UI.

**UI responses live in `ui::process_events()`:** After each tick, the game loop calls `process_events()` to handle UI-specific reactions (close panels on game-over, reset crisis selection) and format events into status messages. It does not mutate `sim_state`, `outcome`, or other game-rule state.

When adding new event types: game-rule transitions go in `tick()`. Presentation responses go in `process_events()`.

## What NOT to Change

- **Single `GameState` struct** — One serializable blob = trivial save/load.
- **`UiState` inside `GameState`** — The UI state is part of the save. The issue isn't where it lives in the struct, it's which code touches it.
- **Clone-and-mutate in `tick()`** — Fine at current scale. Don't optimize prematurely.
- **Snapshot mode** — Keep this exactly as-is. It's one of the best things about the codebase.

## Completed Migrations

These are done. Listed for historical context only.

1. **UI state machines extracted from engine** — `apply_action()` moved to `lib.rs`. Panel navigation, wizard steps, selection indices all live in `UiState` methods. Engine only exports `tick()` and `execute_command()`.
2. **Query functions moved to state.rs** — `project_costs()` → `ResearchKind::costs()`. `available_field_projects()`, `available_bench_projects()` → `GameState` methods. UI no longer imports from engine.
3. **`CommandResult` type** — `execute_command()` returns `CommandResult { message, success }` instead of directly modifying UI state.
4. **Engine god file broken up** — Research, medicine, policy, and crisis logic extracted into subsystem modules.
