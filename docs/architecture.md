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
pandemic-cli --snapshot save.json --ticks 10   # advance game time, dump result
```

Each invocation: load state → apply inputs → output new state as text. Stateless. This is how Claude and automated tests interact with the game.

## Determinism

RNG is seeded. Seed is stored in state. Same state + same inputs = same outputs, always.

## Real-Time With Pause

Simulation advances in discrete ticks. "Real-time" = ticks fire automatically. "Pause" = they don't. Saves are always at a tick boundary. The engine has no concept of wall-clock time.
