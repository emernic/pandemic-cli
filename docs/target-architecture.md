# Target Architecture

Where we want the codebase to go, incrementally.

## The Core Problem (Largely Resolved)

Previously, `engine.rs` handled both game logic and UI state transitions in one massive `apply_action()` function. This has been fixed:

- **`apply_action()` moved to `lib.rs`** — It's coordination/routing logic, not game simulation. It delegates UI actions to `UiState` methods and game commands to `engine::execute_command()`.
- **UI state machines extracted** — Panel navigation, wizard steps, selection indices all live in `UiState` methods (`handle_confirm()`, `toggle_panel()`, `select_next()`, etc.).
- **UI modules no longer import from engine** — `project_costs()` moved to `ResearchKind::costs()`, query functions moved to `GameState` methods.
- **`engine.rs` now only contains game logic** — `tick()`, `execute_command()`, and their helpers (crisis generation, disease emergence, research completion, etc.).

## Target Layers

```
┌─────────────────────────────────────┐
│  main.rs / snapshot.rs              │  I/O boundary
│  Terminal, files, CLI args          │
├─────────────────────────────────────┤
│  ui/                                │  Rendering + UI state machines
│  Reads state, renders it.           │
│  Owns panel navigation, wizards,    │
│  selection indices, open/close.     │
│  Translates user intent into        │
│  game commands.                     │
├─────────────────────────────────────┤
│  engine.rs                          │  Game logic only
│  tick(), execute_command()          │
│  Knows about diseases, regions,     │
│  resources, research. Does NOT      │
│  know about panels, selections,     │
│  or wizard steps.                   │
├─────────────────────────────────────┤
│  state.rs                           │  Pure data
│  GameState, all structs/enums       │
│  No logic, no imports               │
└─────────────────────────────────────┘
```

**Key design: `Action` (UI) vs `GameCommand` (engine) split.**

`Action` handles UI input (navigate, open panel, select). `GameCommand` handles game logic (deploy medicine, start research, resolve crisis). These are connected by `apply_action()` in `lib.rs`:

```
KeyPress
  → key_to_action() → Action
  → apply_action():
      UI actions → UiState methods directly
      Confirm → UiState::handle_confirm() → Option<GameCommand>
        → if command → engine::execute_command() → CommandResult
        → UiState::apply_command_result()
```

`execute_command()` never touches `UiState`. It returns `CommandResult { message, success }` and the caller handles UI updates.

## Specific Migrations

### 1. ~~Extract `disease_display_name()` from `research.rs`~~ — DONE

Moved to `Disease::display_name()` in `state.rs`.

### 2. ~~Move `project_costs()` and research query functions to `state.rs`~~ — DONE

`project_costs()` → `ResearchKind::costs()` method. `available_field_projects()` and `available_bench_projects()` → `GameState` methods. UI no longer imports from `engine.rs`.

### 3. ~~Pull UI state machine transitions out of `engine.rs`~~ — DONE

`apply_action()` moved from `engine.rs` to `lib.rs`. UI state machine methods extracted to `UiState`. Engine only exports `tick()` and `execute_command()`.

**Approach (completed):** Added methods to `UiState` that handle:
- ~~Panel open/close/toggle~~ — DONE (`UiState::toggle_panel()`, `UiState::close_panel()`)
- ~~Selection navigation (next/prev with bounds)~~ — DONE (`UiState::select_next()`, `select_prev()`, `select_left()`, `select_right()`)
- ~~Wizard step forward/back (Confirm handler for medicines, research, policy)~~ — DONE (`UiState::handle_confirm()`)
- ~~Translating a Confirm press into a game command (or nothing, if mid-wizard)~~ — DONE (returns `Option<GameCommand>`)

The Confirm flow now works as intended:
```
keypress → action
  → UiState::handle_confirm() (wizard transitions, returns Option<GameCommand>)
  → if command → engine::execute_command() (pure game logic, returns CommandResult)
  → UiState::apply_command_result() (post-command UI navigation)
```

### 4. ~~Give `apply_command` a result type~~ — DONE

`execute_command()` returns `CommandResult { message, success }`. The caller (apply_action) reads the message and puts it in `status_message`. The engine's `execute_command` never touches `UiState`.

## Migration Status

This migration is complete. `engine.rs` contains only `tick()` + `execute_command()` with pure game logic. `apply_action()` lives in `lib.rs` as coordination logic. UI state machines live in `UiState` methods. UI modules do not import from engine.

**Ongoing discipline:** When adding new features, keep this layering intact. New game actions get `GameCommand` variants. New UI flows get `UiState` methods. `engine.rs` should never touch `UiState`.

## What NOT to Change

- **Single `GameState` struct** — This is good. One serializable blob = trivial save/load.
- **`UiState` inside `GameState`** — This is fine. The UI state is part of the save. The issue isn't where it lives in the struct, it's which code touches it.
- **Clone-and-mutate** — Fine at current scale. Don't optimize prematurely.
- **Snapshot mode** — Keep this exactly as-is. It's one of the best things about the codebase.
