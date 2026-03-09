# Architecture

## Core Principle

All game state (simulation + UI) lives in a single JSON-serializable struct. The entire game is a pure function: `state + input → new state`.

## Two Layers

**Engine** — Pure game logic. No I/O. Takes state and an action, returns new state. Unit testable in isolation.

**UI** — Ratatui frontend. Reads state, renders it. Captures keypresses, converts them to actions. Thin and stateless beyond what's in the game state.

## State File

One JSON file = one complete save. Includes everything: simulation state (outbreaks, research, resources, world) AND UI state (open menus, selections, notifications, cursor position). No distinction between "important" and "ephemeral" state—if it affects what you see on screen, it's in the file.

Passing a save file on startup boots directly into that exact state. No save file = main menu.

## Snapshot Mode

The game can be run non-interactively for testing:

```
pandemic-cli --snapshot save.json              # dump what the screen looks like
pandemic-cli --snapshot save.json --key "r"    # apply a keypress, dump result
pandemic-cli --snapshot save.json --days 1     # advance 1 day, dump result
```

Each invocation: load state → apply inputs → output new state as text. Stateless. This is how Claude and automated tests interact with the game.

## Determinism

RNG is seeded. Seed is stored in state. Same state + same inputs = same outputs, always.

## Real-Time With Pause

Simulation advances in discrete ticks (internal unit). The UI displays "days" (1 day = 100 ticks). "Real-time" = ticks fire automatically. "Pause" = they don't. Saves are always at a tick boundary. The engine has no concept of wall-clock time.

## Current Module Map

```
src/
  lib.rs          — Crate root, module declarations, format_number() utility
  main.rs         — CLI args (clap), interactive loop, file I/O
  state.rs        — GameState and all data structs (pure data, no logic)
  engine.rs       — tick() + apply_action() (game logic + UI state machines — see note below)
  action.rs       — Action enum, key-to-action mapping
  snapshot.rs     — Non-interactive snapshot mode for testing
  ui/
    mod.rs        — Layout orchestration, panel routing
    region_list.rs — World map grid with connections
    threats.rs    — Disease info panel
    medicines.rs  — Medicine deployment wizard
    research.rs   — Research project panel
    resources.rs  — Header status bar
    hotkey_bar.rs — Footer hotkey legend + status messages
```

**Known debt:** `engine.rs` currently handles both game logic (deploy medicine, start research) and UI state machine transitions (wizard steps, panel navigation). See `docs/target-architecture.md` for the migration plan to separate these concerns.
