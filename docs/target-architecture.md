# Target Architecture

Where we want the codebase to go, incrementally.

## The Core Problem

`engine.rs` handles both game logic and UI state transitions in one massive `apply_action()` function. This means the engine knows about `MedicineUiState::SelectRegion`, `ResearchUiState::BrowseProjects`, `panel_selection` indices, and other UI concerns. It also means UI rendering modules sometimes reach back into the engine for data (`resources.rs` imports `project_costs()` from engine).

The result: engine.rs is ~1,300 lines and growing, with UI navigation logic interleaved with actual simulation commands. Adding a new panel or changing a wizard flow requires editing engine.rs.

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
│  tick(), apply_command()            │
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

**Key change: split `Action` into UI actions and game commands.**

Currently one `Action` enum handles everything from "move cursor down" to "deploy medicine." These are fundamentally different:

- **UI actions** (navigate, open panel, advance wizard step, close panel) — only touch `UiState`, don't affect simulation
- **Game commands** (deploy medicine, start research, toggle pause) — affect simulation state, don't touch UI navigation

```
KeyPress
  → key_to_action()
  → if UI action: update UiState directly (thin, no engine call)
  → if game command: engine::apply_command() (pure game logic)
```

This means `engine.rs` would export something like:
```rust
enum Command {
    TogglePause,
    DeployMedicine { medicine_idx, region_idx, target },
    StartResearch { kind: ResearchKind },
    // ...
}

fn apply_command(state: &GameState, cmd: &Command) -> GameState
```

And `apply_command` never touches `UiState` at all.

## Specific Migrations

### 1. ~~Extract `disease_display_name()` from `research.rs`~~ — DONE

Moved to `Disease::display_name()` in `state.rs`.

### 2. ~~Move `project_costs()` and research query functions to `state.rs`~~ — DONE

`project_costs()` → `ResearchKind::costs()` method. `available_field_projects()` and `available_bench_projects()` → `GameState` methods. UI no longer imports from `engine.rs`.

### 3. Pull UI state machine transitions out of `engine.rs`

The medicine wizard (BrowseMedicines → SelectRegion → SelectTarget → ConfirmDeploy) and research wizard (BrowseCategories → BrowseProjects → ConfirmProject) are purely UI flows. The engine shouldn't know about them.

**Approach:** Add methods to `UiState` that handle:
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

## Migration Strategy

These changes are too large for one PR. Instead, migrate incrementally as we touch each area:

1. When modifying a UI panel, check if its state machine transitions can be pulled out of `engine.rs`
2. When adding new game commands, add them as `Command` variants rather than `Action` variants
3. When you notice a layering violation (UI importing engine, engine touching UiState), fix it locally

Over time, `engine.rs` shrinks to just `tick()` + `apply_command()` with pure game logic, and UI state machines live in the UI layer where they belong.

## What NOT to Change

- **Single `GameState` struct** — This is good. One serializable blob = trivial save/load.
- **`UiState` inside `GameState`** — This is fine. The UI state is part of the save. The issue isn't where it lives in the struct, it's which code touches it.
- **Clone-and-mutate** — Fine at current scale. Don't optimize prematurely.
- **Snapshot mode** — Keep this exactly as-is. It's one of the best things about the codebase.
