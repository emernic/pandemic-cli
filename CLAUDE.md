# Pandemic CLI

Inverse Plague Inc. — defend humanity against diseases in a sci-fi future. Rust + ratatui TUI.

## Quick Start

```bash
cargo build                    # build
cargo test                     # run all tests (unit + insta snapshots)
cargo run                      # interactive mode (starts paused, Space to unpause)
cargo run -- --snapshot        # snapshot mode (for AI/automated testing)
cargo insta review             # review snapshot test changes
```

## Architecture

All game state lives in one `GameState` struct (src/state.rs). Two pure functions drive everything:
- `tick()` (src/engine.rs) — advances simulation one step
- `apply_action()` (src/engine.rs) — handles player input

Both clone-and-mutate. Deterministic via seeded ChaCha8Rng.

Key files: `src/state.rs` (data), `src/engine.rs` (logic), `src/action.rs` (input mapping), `src/ui/` (rendering), `src/snapshot.rs` (snapshot mode).

Design docs: `docs/architecture.md`, `docs/gameplay.md`

## Conventions

- Rust 2024 edition: `gen` is reserved — use `r#gen()` for `rand::Rng::gen()`
- Diseases use vec index, not an id field (`RegionInfection.disease_idx`)
- `tick()` is NOT an Action — simulation steps and player actions are separate concepts
- Bump version in `Cargo.toml` when making a release
- Always create branches from `origin/master`, not local `master`:
  ```
  git fetch origin && git checkout -b my-branch origin/master
  ```
  Local `master` may be checked out in another worktree, which blocks `git checkout master`.
