# Pandemic CLI

## ⚠️ THE #1 RULE: OWN THE CODEBASE

**This is the single most important instruction in this file. Follow this above all else.**

You are not a contractor executing assigned tasks. You are an owner of this codebase. If you see something broken, wrong, buggy, inconsistent, missing, or stupid — **it is your problem**. Not someone else's. Yours. Every single time.

**NEVER sweep anything under the rug.** Every time you encounter an issue — a broken CLI flag, a test gap, a confusing API, dead code, misleading docs, awkward UX, anything — you MUST do one of:

1. **Fix it directly** if it's small and won't derail your current task
2. **File a GitHub issue** (`gh issue create`) if it's bigger or you're mid-task — issues can be super short, we have infinite parallelization to burn through the backlog

No exceptions. No "that's not what I was asked to do." No "I'll just work around it." No ignoring it and moving on. If you saw it, you own it. File the issue or fix the bug. Every time.

---

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

## Play the Game Yourself

**Before starting any feature or bug fix, play a few frames of the game yourself.** Not a sub-agent. Not the playtest agent. YOU. Run snapshot commands directly with the Bash tool so you see the rendered output with your own eyes. This grounds you in what the game actually looks like and how it behaves right now.

```bash
cargo run -- --snapshot                          # see initial state
cargo run -- --snapshot --ticks 5                # advance a few ticks
cargo run -- --snapshot --key right              # navigate panels
cargo run -- --snapshot --key m --ticks 3        # open medicines, advance 3
```

Do this **every time** you start working on something. It takes seconds and prevents you from coding blind. You cannot write good UI or game logic if you haven't looked at the game.

For extended playtesting (e.g., as a final check after a feature is complete), use the playtest agent. Tell it specifically what to test — describe the feature you built, the key behaviors to verify, and suggest specific snapshot commands to exercise it. A guided playtest catches far more issues than a generic one.

## Pre-Merge Checklist

Before merging any significant feature or bug fix:

1. **Clean up playtests**: Delete any outdated playtest files from your branch (e.g., playtests from earlier iterations that no longer reflect the final state).
2. **Run a fresh playtest**: Use the playtest agent to test your final changes. Guide it toward the specific things that matter — describe what you changed, what the key behaviors are, and what commands will exercise them. The playtest report should demonstrate that your changes work correctly.
3. **Include the playtest in your PR**: The playtest file serves as a reviewable record that the feature was tested end-to-end. Reviewers should be able to read it and see that the important behaviors were verified.

## Session Start Checklist

**This is enforced by a hook. You will be blocked from editing files until you run `git status`.**

The VERY FIRST thing you do in every session — before reading code, before planning, before touching anything — is orient yourself:

1. **Fetch**: `git fetch origin`
2. **Check status**: `git status` — look at what branch you're on, whether there are uncommitted changes, and whether the branch is up to date. Flag any surprises to the user.
3. **Think about your branch**: Are you on `master`? An old feature branch? Someone else's branch? If you're starting new work, create a fresh branch:
   ```
   git checkout -b my-branch origin/master
   ```
   Only skip this if the user explicitly says to continue work on the current branch.

Do NOT start implementing anything until the repo state is clean and understood. Multiple agents share this repo — you WILL end up on stale or wrong branches if you skip this.

## Issue Tracking

We run multiple agents in parallel, so we do NOT use GitHub's assignee feature (all agents share the same GitHub user). Instead, we use the `in-progress` label as our sole claiming mechanism:
- **Available issue:** open, no `in-progress` label
- **Claimed issue:** open, has `in-progress` label (some agent is working on it)
- **Done:** closed, `in-progress` label removed

This is the ONLY mechanism for claiming work. Never use `gh issue edit --add-assignee` or check assignees to determine ownership. Always check for the `in-progress` label.

When picking up an issue from the backlog, always use the `/pick-up-issue` skill.

## Signing Off

In your final message before stopping work, always include a concise status block:

- **Branch**: which branch you're on
- **Working tree**: clean, or are there uncommitted/unstaged changes?
- **Pushed**: is everything pushed to the remote?
- **Merged**: is the branch merged to master?

The user switches between many terminal tabs. They need to instantly see what was being worked on and whether there's anything left to do. Don't make them scroll up and piece it together.

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
- When you notice something that looks funky, incomplete, or unclear while working — file an `investigate` issue. These are cheap and free; you don't need user permission. **Critically: investigate issues are NOT bug reports.** You are NOT claiming something is wrong. You're saying "this confused me and someone should take a closer look." Do not theorize about what the fix should be — you haven't investigated it yet. Example: while fixing #6 (mismatched Quit labels), we noticed saving only happens when a file path is provided. We have no idea if that's a bug or intentional — we didn't look into it. But it seemed worth a second look, so it's an investigate issue. See the create-issue skill for the full template and examples of good vs. bad investigate issues.
