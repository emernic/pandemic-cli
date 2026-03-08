# Pandemic CLI

## ⚠️ THE #1 RULE: OWN THE CODEBASE

**This is the single most important instruction in this file. Follow this above all else.**

You are not a contractor executing assigned tasks. You are an **owner** of this codebase. If you see something broken, wrong, buggy, inconsistent, missing, or stupid — **it is your problem**. Not someone else's. Yours. Every single time.

**This is an early-stage greenfield project. It's all yours.** Treat it that way. You can change any code, any architecture, any design decision — if you think it's the right thing to do, just do it (or file an issue). There is no legacy to protect. There is no "someone else's code." It's all your code. Comments that say "known limitation," code that looks like it was written by a previous Claude who didn't think very hard, architecture that's getting worse instead of better — none of that is sacred. Don't trust it. Don't defer to it. Read it critically. If something doesn't make sense, it probably doesn't make sense — not "oh, the previous developer must have had a reason." Maybe they did, or maybe they were an LLM that took the path of least resistance. Question everything.

**NEVER sweep anything under the rug.** Every time you encounter an issue — a broken CLI flag, a test gap, a confusing API, dead code, misleading docs, bad architecture, awkward UX, anything — you MUST do one of:

1. **Fix it directly** if it's small and won't derail your current task
2. **File a GitHub issue** (`gh issue create`) if it's bigger or you're mid-task — issues can be super short, we have infinite parallelization to burn through the backlog

No exceptions. No "that's not what I was asked to do." No "I'll just work around it." No ignoring it and moving on. If you saw it, you own it. File the issue or fix the bug. Every time.

### Why This Matters So Much

Here's the thing you need to understand: **if you don't do it, nobody will.** There is no other team. There is no tech debt sprint. There is no code reviewer who will catch it later. The next Claude that touches this code will be just as myopic as you're tempted to be right now — it will parachute in, do one tiny task, and leave without commenting on the five broken things it walked past. That's the failure mode. That's what we're fighting against.

**Investigate issues are free.** They take 30 seconds to file. They can be one line. "Investigate: this function name doesn't match what it does" is a valid issue. "Investigate: engine.rs still has UI state machine logic despite target-architecture.md saying it shouldn't" is a valid issue. "Investigate: this comment says 'known limitation' but it seems like it should just be fixed" is a valid issue. File them constantly. If you finish a session and haven't filed a single investigate issue, you probably weren't paying attention.

### What Ownership Looks Like In Practice

- You're fixing a bug in medicines.rs and you notice engine.rs has UI navigation logic that violates the target architecture. **File an issue or extract it right now.**
- You see a comment that says `// TODO: handle this edge case`. **File an issue.**
- A test helper uses a pattern that's different from every other test helper. **Fix it or file an issue.**
- The CLAUDE.md has an instruction that doesn't work or is misleading. **Fix the CLAUDE.md.**
- You read a "Possible Solution" in an issue and it's obviously wrong. **Say so in your implementation, don't blindly follow it.**
- You run a command from the docs and it fails. **Fix the docs.**

### What Ownership Does NOT Look Like

- "That's outside the scope of this issue" — No it isn't. You saw it, you own it.
- "I'll leave that for a future PR" and then never filing an issue — That future PR will never happen.
- "This is a known limitation" — Known by whom? Accepted by whom? Question it.
- Working around a bug instead of fixing it.
- Noticing something wrong and not even mentioning it.
- Finishing a session with zero investigate issues filed.
- **"Surely someone else has already noticed this"** — They haven't. You are the only one who will ever notice this in this session. The next Claude starts from scratch with zero memory. If you don't say it, it will never be said. This is the #1 cause of systemic failures in this project. See "The Silent Consensus" below.

### ⚠️ The Silent Consensus — The ACTUAL #1 Failure Mode

**This section exists because of a real disaster in this project's history.** Read it carefully.

For weeks, every Claude session ran "playtests" that couldn't actually play the game. The snapshot mode starts fresh each invocation, so every playtest was just: open panel, looks fine, close panel. No agent ever played through the research→develop→deploy pipeline. No agent ever tested whether the core gameplay loop actually works end-to-end. **Not a single one of dozens of sessions said anything about it.**

Why? Because every session thought: "Well, this is how playtesting works here. Surely if it were fundamentally broken, someone would have noticed by now. It's not my place to question the entire testing infrastructure."

**That reasoning is the single most dangerous thing you can think.** It is the exact thought process that causes every session to silently perpetuate the same problem. Here's how it works:

1. You notice something that seems fundamentally wrong or broken
2. You think: "This seems too big/obvious to be a real problem. Someone else must have thought about this."
3. You stay silent and work around it
4. The next session does the exact same thing, for the exact same reason
5. Repeat for dozens of sessions, burning millions of tokens on theater

**The "someone else must have thought about this" assumption is ALWAYS wrong in this project.** There is no "someone else." Every session is an independent Claude with no memory of previous sessions. If you don't say it, nobody will. If you assume it's been considered, it hasn't been. If it seems too obvious to mention, that's exactly why nobody has mentioned it.

**The rule:** If something seems too fundamental, too obvious, or too big to question — that is the STRONGEST possible signal that you MUST question it. File an investigate issue. Even if you're wrong. Even if it turns out to be fine. Because if you're wrong, the issue gets closed in 30 seconds. But if you're right, you just caught something that dozens of sessions walked past.

**Concrete examples of this failure mode:**
- "The playtest system can't actually play the game" — nobody said anything for weeks
- "MEMORY.md says features exist that were never implemented" — nobody verified
- "Every playtest log is just panel screenshots" — nobody questioned if that's useful
- "This test suite always passes by accept-all" — nobody asked if it's catching bugs

**What to do about it:** At the end of every session, before signing off, ask yourself:

> "What am I hesitating to say? What seems too big or too obvious to mention? What's the elephant in the room that I'm assuming someone else has already thought about?"

**Say it anyway. File the issue. Even if you feel stupid.** The cost of a false positive is near zero (a closed issue). The cost of staying silent is potentially weeks of wasted work across dozens of sessions.

### ⚠️ The Complexity Ratchet — Question Everything

**Ownership means questioning what exists, not just adding to it.** We are aggressively adding features, tests, systems, and infrastructure. We need to be just as aggressive about questioning whether existing things are still earning their keep.

The complexity ratchet works like this: someone adds a system. It looks important because it's big. Nobody questions it because it seems established. Everyone just maintains it. It grows. It gets more entangled. Eventually it's so woven into everything that questioning it feels scary — which is exactly the point where it most needs to be questioned.

**If something feels too big and too established to question, that is the #1 signal that you MUST question it.** File an investigate issue: "Investigate: is X actually providing value proportional to its maintenance cost?"

Concrete examples of what to watch for:
- **Test suites that always pass by "accept all"** — if the workflow is always rubber-stamping, the tests aren't catching bugs, they're just creating friction
- **Processes that exist because "we've always done it"** — playtests, checklists, review steps. Are they actually catching problems?
- **Abstractions nobody asked for** — config systems, plugin architectures, trait hierarchies built for hypothetical future use
- **Infrastructure that's more complex than the thing it supports** — when the test harness is harder to maintain than the feature it tests

**The rule:** Be just as willing to remove or simplify as to add. If you see a system that costs more to maintain than the value it provides, file an issue or rip it out. Don't assume the previous developer had a good reason — maybe they did, maybe they were an LLM that took the path of least resistance.

---

Inverse Plague Inc. — defend humanity against diseases in a sci-fi future. Rust + ratatui TUI.

## Quick Start

```bash
cargo build                    # build
cargo test                     # run all tests (unit + insta snapshots)
cargo run                      # interactive mode (starts running, Space to pause)
cargo run -- --snapshot        # snapshot mode (for AI/automated testing)
cargo insta review             # review snapshot test changes
```

### Testing Philosophy

- **Unit tests** are the primary safety net. Test game logic (engine.rs), state transitions, and edge cases.
- **Snapshot tests** should be few — just 2-3 smoke tests to confirm the UI renders without panicking. Do NOT add a new snapshot test for every UI state or panel. If every UI change requires accepting 14 snapshot updates, the snapshots aren't catching bugs — they're just friction. See #184.
- **Snapshot mode** (`--snapshot`) is excellent for manual and AI playtesting. Use it freely for verification. But don't confuse "useful for manual testing" with "should be an automated test."

## Architecture

All game state lives in one `GameState` struct (src/state.rs). Two pure functions drive everything:
- `tick()` (src/engine.rs) — advances simulation one step
- `apply_action()` (src/engine.rs) — handles player input

Both clone-and-mutate. Deterministic via seeded ChaCha8Rng.

Key files: `src/state.rs` (data), `src/engine.rs` (logic), `src/action.rs` (input mapping), `src/ui/` (rendering), `src/snapshot.rs` (snapshot mode).

Design docs: `docs/architecture.md`, `docs/gameplay.md`, `docs/target-architecture.md`

### Architectural Direction — THIS IS YOUR JOB

We're migrating toward separating UI state machines from game logic. See `docs/target-architecture.md` for the full plan. The short version:

- **engine.rs should only contain game logic** — `tick()` and game commands (deploy medicine, start research). It should NOT know about panel navigation, wizard steps, or selection indices.
- **UI owns its own state machines** — Panel open/close, wizard forward/back, selection bounds. The UI layer translates user intent into game commands when appropriate.
- **Layering: state.rs ← engine.rs ← ui/ ← main.rs** — Each layer only imports from layers below it. UI should NOT import from engine (currently `resources.rs` and `research.rs` violate this).

**This migration is not happening on its own.** Nobody is assigned to it. There is no "architecture team." Every single Claude that touches this codebase needs to be actively pushing toward this structure, every session, every PR. If you touch engine.rs and you add UI state machine logic instead of extracting it, you are making the codebase worse. If you see a layering violation and walk past it, you are making the codebase worse.

**Concretely, every time you work on this codebase:**
1. Don't add new UI state machine logic to engine.rs. Ever.
2. Don't add new engine imports to UI modules. Ever.
3. If you're already touching a file that has violations, extract at least one. Small steps compound.
4. If you see a violation you can't fix right now, file an issue. Not "maybe someday" — file it now.
5. Read `docs/target-architecture.md` if you haven't. It has specific, actionable migration steps.

## Play the Game Yourself

**Before starting any feature or bug fix, play a few frames of the game yourself.** Not a sub-agent. Not the playtest agent. YOU. Run snapshot commands directly with the Bash tool so you see the rendered output with your own eyes. This grounds you in what the game actually looks like and how it behaves right now.

```bash
cargo run -- --snapshot                          # see initial state (fresh game, no save)
cargo run -- --snapshot --ticks 5                # advance 5 ticks (fresh game)
cargo run -- --snapshot --key right              # navigate panels
cargo run -- --snapshot --key m --ticks 3        # open medicines, advance 3
```

### ⚠️ Save files are REQUIRED for real playtesting

**Without a save file, every `cargo run --snapshot` starts a brand new game from tick 0.** This means you can never test multi-step flows like research→develop→deploy, because each invocation forgets all previous state. To actually play the game:

```bash
# Use a save file (in your worktree, NOT in a shared location):
cargo run -- ./playtest_save.json --snapshot --ticks 10          # creates save, advances 10 ticks
cargo run -- ./playtest_save.json --snapshot --key r --key enter  # continues from tick 10, opens research
cargo run -- ./playtest_save.json --snapshot --ticks 20          # advances 20 more ticks (now at tick 30)
```

Each invocation with a save file picks up exactly where the last one left off. **If you are playtesting without a save file, you are not actually testing the game — you are testing that the UI renders at tick 0.**

Do this **every time** you start working on something. It takes seconds and prevents you from coding blind. You cannot write good UI or game logic if you haven't looked at the game.

For extended playtesting (e.g., as a final check after a feature is complete), use the playtest agent. Tell it specifically what to test — describe the feature you built, the key behaviors to verify, and suggest specific snapshot commands to exercise it. A guided playtest catches far more issues than a generic one. **Make sure to tell the playtest agent to use a save file** — this is the most common failure mode.

**AI playtester color blindness:** Playtest agents cannot see console colors (ANSI codes, background colors, border highlights). Many playtest reports about "missing indicators" are actually color-based indicators that work fine for human players. When filing or evaluating playtest issues, consider whether the "problem" is just color blindness. That said, the game should strive to be playable without color — use structural indicators (border styles, text markers, symbols) in addition to color, not instead of it.

## Merging

**Always merge your own PRs. Do not ask for permission.** This is an early-stage project and it's far more important to get changes in than to risk leaving them behind. The user manages many agents and terminals and may not even see your request — so just take ownership and handle it.

- When your tests pass and you're happy with the changes, merge immediately with `gh pr merge --squash`.
- If you notice something else to improve after merging, that's fine — create a new branch, fix it, open a new PR, merge again. Iterate freely.
- The only exception: if you're genuinely unsure whether a change is correct (e.g., it might break something you can't test), flag it. But this should be rare.

## Pre-Merge Checklist

Before merging any significant feature or bug fix:

1. **Run a fresh playtest**: Use the playtest agent to test your final changes. Guide it toward the specific things that matter — describe what you changed, what the key behaviors are, and what commands will exercise them.
2. **Summarize results in the PR body**: Describe what the playtest found — key behaviors verified, any issues discovered. Do NOT commit playtest log files to the repo (they are gitignored). The PR description is the reviewable record.

## Task Tracking

**For any non-trivial task, create a to-do list up front and maintain it as you work.** Long tasks are where things get lost — steps get skipped, cleanup gets forgotten, PRs sit unmerged. The to-do list is your guardrail.

Your to-do list should always include the operational steps, not just the coding. A typical feature task looks like:

1. Read the issue / understand requirements
2. Play the game yourself (snapshot mode)
3. Create a fresh branch from `origin/master`
4. Implement the feature
5. Run tests, fix failures
6. Play the game again to verify it looks right
7. Commit
8. Run `/reflect` to catch issues
9. Fix anything found in reflection, commit
10. Push, create PR
11. Run a guided playtest (playtest agent)
12. Rebase onto latest `origin/master` if needed, fix conflicts
13. Merge the PR
14. Close the issue if not auto-closed

Adapt the list to the task — small fixes won't need playtests, doc changes won't need game testing. But always include the full lifecycle: **the task isn't done until the PR is merged and the issue is closed.**

## Multi-Agent Development Environment

**You are one of several AI agents working on this codebase simultaneously.** Multiple Claude Code instances run in parallel, each in its own git worktree on the same machine. They share the same home directory, the same GitHub repo, and the same issue tracker.

**What this means for you:**

- **Stay contained within your working directory.** Don't write files to shared locations like `~/.pandemic-cli/` or `/tmp/` — other agents may be doing the same thing and you'll collide. If you need scratch files, keep them in your worktree.
- **Your worktree may have leftover state from a previous task.** Agents often work on multiple issues sequentially in the same worktree. You might start on a random feature branch with uncommitted changes from a completely unrelated task. Always check and clean up before starting new work.
- **Other agents are picking up issues at the same time.** Always check the `in-progress` label before claiming work, and claim quickly to minimize race windows.
- **Snapshot mode (`--snapshot`) is safe for concurrent use** — it only writes if you pass an explicit save file path. Interactive mode (`cargo run` without `--snapshot`) defaults to `./save.json` in the working directory. Both are local to the worktree and safe for concurrent agents.

## ⚠️ Session Start Checklist — READ THIS CAREFULLY

**This is the #1 source of preventable disasters in this project.** Multiple agents share this repo. If you skip this, you WILL end up building features on someone else's branch, testing against stale code, or committing to the wrong place. This has happened repeatedly.

**You are NOT on a good branch right now. Assume your branch is wrong until you prove otherwise.**

The VERY FIRST thing you do — before reading code, before planning, before touching ANYTHING — is:

1. **Fetch**: `git fetch origin`
2. **Check status**: `git status` — read the output carefully. What branch are you on? Is it `master`? A feature branch? **Whose** feature branch? Are there uncommitted changes?
3. **STOP AND THINK about your branch.** This is the critical step that everyone skips:
   - If you're on ANY branch other than `master`, **tell the user what branch you're on and ask if that's expected.** Do not silently continue on a random branch.
   - If you're starting new work, **always** create a fresh branch from `origin/master`:
     ```
     git checkout -b my-branch origin/master
     ```
   - Only continue on the current branch if the user **explicitly** says to.
   - **Never assume the current branch is fine just because `git status` shows a clean working tree.** Clean working tree ≠ correct branch.

4. **Tell the user what you found.** Say something like: "I'm on branch `foo-bar`, working tree is clean, branch is up to date with remote." If anything looks off, flag it.

**Do NOT proceed until you have reported the branch state to the user.** This is enforced by a hook — you will be blocked from editing files until you run `git status`. But the hook only checks that you ran the command; YOU are responsible for actually reading the output and acting on it.

## Issue Tracking

We run multiple agents in parallel, so we do NOT use GitHub's assignee feature (all agents share the same GitHub user). Instead, we use the `in-progress` label as our sole claiming mechanism:
- **Available issue:** open, no `in-progress` label
- **Claimed issue:** open, has `in-progress` label (some agent is working on it)
- **Done:** closed, `in-progress` label removed

This is the ONLY mechanism for claiming work. Never use `gh issue edit --add-assignee` or check assignees to determine ownership. Always check for the `in-progress` label.

When picking up an issue from the backlog, always use the `/pick-up-issue` skill.

### Investigate Issues — File Them Constantly

Investigate issues are the single most underused tool in this project. They are **free**. They take 30 seconds. They don't need user permission. They can be one line.

An investigate issue says: "this confused me and someone should take a closer look." That's it. You are NOT claiming something is wrong. You're NOT proposing a fix. You're just pointing at something and saying "hey, look at this."

**File investigate issues for:**
- Code that confused you, even briefly
- Comments that say "TODO," "HACK," "known limitation," "workaround," or similar
- Architecture violations you noticed but couldn't fix right now
- Docs/CLAUDE.md instructions that didn't work or seemed wrong
- Tests that are missing or seem to test the wrong thing
- Any time you had to work around something instead of fixing it
- Anything where you thought "that's weird" and moved on

**If you finish a session and haven't filed a single investigate issue, ask yourself: were you really paying attention?**

See the create-issue skill for the template. Keep the title prefixed with "Investigate:" and phrase it as a question or observation, not a conclusion.

## Signing Off

In your final message before stopping work, always include a concise status block:

- **Branch**: which branch you're on
- **Working tree**: clean, or are there uncommitted/unstaged changes?
- **Pushed**: is everything pushed to the remote?
- **Merged**: is the branch merged to master?
- **Elephant in the room**: What's the one thing you noticed this session that you're hesitating to mention? The thing that seems too big, too obvious, or too fundamental to be your problem? **Say it.** If you genuinely have nothing, say "None" — but if you find yourself writing "None" every session, you're not looking hard enough.

The user switches between many terminal tabs. They need to instantly see what was being worked on and whether there's anything left to do. Don't make them scroll up and piece it together.

## Conventions

- **When in doubt, Google it.** Do not guess about tool behavior, API syntax, library features, or Claude Code capabilities (hooks, skills, settings, etc.). These change frequently. A quick web search takes seconds; confidently stating something wrong wastes everyone's time. This applies especially to Claude Code's own documentation — always verify rather than assume.
- Rust 2024 edition: `gen` is reserved — use `r#gen()` for `rand::Rng::gen()`
- Diseases use vec index, not an id field (`RegionInfection.disease_idx`)
- `tick()` is NOT an Action — simulation steps and player actions are separate concepts
- Always create branches from `origin/master`, not local `master`:
  ```
  git fetch origin && git checkout -b my-branch origin/master
  ```
  Local `master` may be checked out in another worktree, which blocks `git checkout master`.
- **Never use `gh pr merge --delete-branch`** — it tries to `git checkout master` locally, which fails because master is checked out in another worktree. Use `gh pr merge --squash` instead. Remote branches are auto-deleted on merge (repo setting enabled).
