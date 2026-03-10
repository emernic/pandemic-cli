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

### ⚠️ You Are the Game Designer — Not Just an Engineer

**Your job is as much game design as it is software development. These tasks are not separable. Take design at least as seriously as you take code.**

"That's a design decision" is not a reason to defer. Designed by whom? There is no game designer waiting in the wings. There is no design team. There is no "later" when someone will think about game feel. If a mechanic is unfun, confusing, creates no interesting decisions, or soft-locks the player — **that is your problem to fix or file an issue about**. If you don't, nobody will. The next session starts from scratch and will walk past the exact same problem.

**Taking design seriously means thinking like an adult, not brainstorming like a manic 13-year-old.** Don't generate lists of wacky feature ideas. Think carefully about the systems you're building — how they interact with existing systems, whether they create interesting decisions, whether they reuse the building blocks already in the game. The best new features are new combinations of existing legos, not new legos. Every system should earn its complexity by creating emergent gameplay when combined with other systems.

**Design documents require MORE care than code, not less.** Code gives you fast feedback — tests fail, the compiler complains, playtests reveal problems. Design documents give you no feedback at all until you've shipped a month of work based on them. A wrong design doc silently poisons everything built on top of it. This means: revise obsessively, question every sentence, think about how each piece connects to the whole. Don't treat docs as a formality to rush through on the way to "real work."

This means:
- When you fix a bug, ask **why** the bug mattered. If the answer reveals a design gap, file an issue for the design gap — don't just patch the symptom.
- When you close an investigate issue, ask whether the *system* makes sense, not just whether the *code* is correct. Correct code implementing a bad design is still a problem.
- When you add or change a mechanic, ask: "Would a real player enjoy this? Does this create interesting choices?" Not just "does it compile."
- **Never** dismiss a design problem as "outside scope" or "a decision for later." You saw it. You own it. File the issue.

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

### ⚠️ THIS GAME IS UNWINNABLE — THIS IS AN AXIOM, NOT A DESIGN CHOICE

**There is no win condition. There will never be a win condition. Do not add one. Do not add anything that implies one.** This is a survival/endurance challenge like a roguelike — diseases will eventually overwhelm the player. The only end state is defeat. The goal is to last as long as possible and save as many lives as you can. Without intervention, the game ends within 15-45 days. Don't make balance changes that trivialize the challenge.

## Game Inspirations

These games represent the design values we're aiming for. When making design decisions, think about what makes these games work.

- **Bitburner** — The closest game in any genre to what we're building. An extremely well-designed, tight game that makes you feel like a hacker god. Great aesthetic, deep systems that interlock cleanly, easy to get lost in. Study how it creates flow.
- **Command & Conquer: Red Alert** — Sense of humor, artistic style, the fine line between parody and serious dystopian world. Tone is everything.
- **Crusader Kings 2** — Gameplay grounded in a world that pulls from real history but creates something far more fun, zany, sometimes funny, yet always engaging, thrilling, tense, and grand. Proof that realism and fun aren't opposites.
- **Kerbal Space Program** — Takes a real domain (orbital mechanics), breaks it down to its bare essentials, finds the fun subset, and brings it to life in a gamified world that still draws heavily on reality. This is exactly what we're doing with epidemiology.
- **Oxygen Not Included** — Interlocking systems that create emergent complexity. Every resource connects to three other resources. Every solution creates a new problem. Deeper than we can hope to achieve here, but the design philosophy is the north star.
- **Steel Panthers / Operational Art of War** — Timeless because the underlying strategy — derived from real-world scenarios — is genuinely interesting. The UI is a product of its era, but the games are deeply polished where it counts. Proof that compelling systems are what make a game last.
- **Other inspirations** (great games worth studying even where they don't apply directly): Kenshi, Mount & Blade, Noita, Starsector, Knights of the Old Republic, RimWorld, Grid World (obscure Steam game simulating cellular life).

## Quick Start

```bash
cargo build                    # build
cargo test                     # run all tests
cargo run                      # interactive mode (starts running, Space to pause)
cargo run -- --snapshot        # snapshot mode (for AI/automated testing)
```

### Testing Philosophy

- **Unit tests** are the primary safety net. Test game logic (engine.rs), state transitions, and edge cases.
- **Smoke tests** in `tests/snapshots.rs` verify the UI renders without panicking using structural assertions (checking for key strings like "PANDEMIC DEFENSE", "World Map"). They should NOT use exact-match snapshots — balance changes would break them constantly.
- **Snapshot mode** (`--snapshot`) is excellent for manual and AI playtesting. Use it freely for verification. But don't confuse "useful for manual testing" with "should be an automated test."

## Architecture

All game state lives in one `GameState` struct (src/state.rs). Two pure functions drive everything:
- `tick()` (src/engine/mod.rs) — advances simulation one step
- `apply_action()` (src/lib.rs) — routes player input to UI state machines or engine commands

Both clone-and-mutate. Deterministic via seeded ChaCha8Rng.

Key files: `src/state.rs` (data), `src/engine/` (game logic — research.rs, medicine.rs, policy.rs, crisis.rs), `src/lib.rs` (action routing), `src/ui/` (rendering), `src/snapshot.rs` (snapshot mode).

Design docs: `docs/architecture.md`, `docs/gameplay.md`, `docs/target-architecture.md`

### Key Game Systems

- **Research pipeline**: Unknown threat → Identify (field research) → Develop medicine (applied research) → Clinical trial (field) → Deploy. Three tracks run simultaneously: field, applied, and basic. Don't touch research without understanding this full lifecycle.
- **Therapy/pathogen matching**: Medicines have a `TherapyType` (Antiviral, Antibiotic, BroadSpectrum), diseases have a `PathogenType` (RnaVirus, DnaVirus, Bacterium, Prion). Efficacy depends on the match. This affects deployment, balance, and player strategy.
- **Mutation system**: Diseases mutate over time based on pathogen type. Medicines track which strain generation they were calibrated against. Drift reduces efficacy, prompting re-trials. This creates ongoing pressure even after developing a medicine.

### Game Balance Thresholds — DO NOT NERF DISEASES

These are hard requirements. If your changes violate any of these, your balance is wrong:

- **45 days max without intervention** (100% of seeds must lose by day 45, median under 35)
- **First collapse no earlier than day 10** — players need minimum time for initial decisions
- **First collapse no earlier than day 3** — absolute minimum even for bad seeds

The game must be threatening. Diseases must kill fast enough that players feel genuine pressure from day 1. If you find yourself reducing disease lethality, infectivity, or cross-region spread, you are almost certainly making the game worse. The `game_is_lost_within_45_days_without_intervention` test enforces the 45-day deadline across 10 seeds.

### Navigation Convention — Left/Right Always Controls Regions

**Left/right arrow keys (h/l) always navigate the region map**, even when a panel is open. Up/down arrow keys (j/k) navigate panel items when a panel is open, or the map when no panel is open. This split lets players browse threats/research/medicines/policies with up/down while simultaneously cycling through regions with left/right.

- **Never use left/right for panel item navigation.** All panel lists (threats, research categories, medicines, policies) must use up/down only.
- Left/right use **reading order with wrap-around**: NA → Europe → Asia → SA → Africa → Oceania → NA (and reverse). This means players can reach any region with just left/right arrows.
- Up/down on the map move within the same column (no wrap).

### Architectural Direction — THIS IS YOUR JOB

The UI/engine separation is done. The engine god file has been broken into subsystem modules. See `docs/target-architecture.md` for the full picture. The short version:

- **engine/ only contains game logic** — `tick()` orchestrates subsystems, `execute_command()` dispatches player commands. Subsystem modules (research.rs, medicine.rs, policy.rs, crisis.rs) handle domain-specific logic with `pub(super)` visibility.
- **UI owns its own state machines** — Panel open/close, wizard forward/back, selection bounds. The UI layer translates user intent into `GameCommand`s.
- **Layering: state.rs ← engine/ ← ui/ ← lib.rs ← main.rs** — Each layer only imports from layers below it. UI and engine are peers that both depend on state.rs but never on each other.

**This structure must be actively maintained.** Every time you touch this codebase:
1. Don't add UI state machine logic to engine/. Ever.
2. Don't add engine imports to UI modules. Ever.
3. New game systems get their own `engine/newsystem.rs` module following the subsystem pattern.
4. If you see a violation, file an issue or fix it. Not "maybe someday" — now.
5. Read `docs/target-architecture.md` if you haven't. It describes the subsystem conventions.

## Play the Game Yourself

**Before starting any feature or bug fix, play a few frames of the game yourself.** Not a sub-agent. Not the playtest agent. YOU. Run snapshot commands directly with the Bash tool so you see the rendered output with your own eyes. This grounds you in what the game actually looks like and how it behaves right now.

```bash
cargo run -- --snapshot                          # see initial state (auto-creates a resumable save under saves/)
cargo run -- --snapshot --days 1                 # advance 1 day and print the resume command
cargo run -- --snapshot --key right              # navigate panels
cargo run -- --snapshot --key m --days 0.5       # open medicines, advance half a day
```

### Interleaved steps with `--do`

The `--do` flag lets you interleave days and key actions in a single invocation. Use `d<N>` for days, anything else is a key:

```bash
cargo run -- --snapshot --do d0.5 --do r --do enter --do enter --do enter  # advance half a day, start research
cargo run -- --snapshot --do d0.5 --do p --do enter --do enter --do d1     # advance half day, toggle policy, advance 1 more day
```

This eliminates the need for save files in simple multi-step tests. For longer sessions, save files are still useful.

### Snapshot mode event handling

Crisis events **interrupt tick advancement**, exactly as they do in interactive mode. When a crisis fires mid-sequence:

1. Tick advancement stops immediately.
2. Subsequent key steps (e.g. `--do enter`) still fire, so you can dismiss inline: `--do d60 --do enter --do d5`. Subsequent `--days` steps are skipped until the crisis is dismissed.
3. The rendered screen shows the current state — including the crisis popup.

Game over also stops execution immediately.

**Do NOT add code that silently skips events in snapshot mode.** If an event would pause a human player, it must also pause snapshot mode. The whole point of snapshot playtesting is to experience the game as a player would.

**⛔ NEVER add an `--auto-crises` flag or any equivalent.** This has been implemented and deleted multiple times. Crisis events are a core gameplay mechanic. Playtests must handle them, not bypass them — if playtest agents complain about crises, the answer is to improve the crisis events, not skip them. Any flag, option, or code path that auto-resolves or skips crisis events in snapshot mode is permanently off-limits.

### Snapshot persistence and real playtesting

`--snapshot` always plays a real sequence of inputs. The only question is whether you want to continue that same run later.

- If you pass a save path explicitly, snapshot mode loads and saves that file.
- If you don't pass a save path, snapshot mode now auto-creates one under `./saves/`, prints the path before the screen output, and tells you the exact command to resume.
- A single invocation can still contain a full scripted sequence via repeated `--do`, so explicit save files are mainly for longer sessions or branching from a known state.

To continue the same playthrough across multiple invocations, either reuse the auto-created file or pass one yourself:

```bash
# Explicit save path:
cargo run -- saves/manual-playtest.json --snapshot --days 1
cargo run -- saves/manual-playtest.json --snapshot --key r --key enter

# Or use the auto-created file printed by the first run:
cargo run -- --snapshot --days 1
cargo run -- saves/playtest-12345-67890.json --snapshot --key r --key enter
```

The old foot gun was running multiple separate `cargo run -- --snapshot ...` commands and assuming they shared state. They did not. With the new auto-save behavior, every snapshot run gives you a resumable file by default.

Do this **every time** you start working on something. It takes seconds and prevents you from coding blind. You cannot write good UI or game logic if you haven't looked at the game.

For extended playtesting (e.g., as a final check after a feature is complete), use the playtest agent. Tell it specifically what to test — describe the feature you built, the key behaviors to verify, and suggest specific snapshot commands to exercise it. A guided playtest catches far more issues than a generic one. Tell the playtest agent to keep using the printed `saves/...` file if the flow spans multiple commands.

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
- **Snapshot mode (`--snapshot`) is safe for concurrent use** — it writes to a local worktree save file, either the explicit path you pass or an auto-created file under `./saves/`. Interactive mode (`cargo run` without `--snapshot`) defaults to `./save.json` in the working directory. Both are local to the worktree and safe for concurrent agents.

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

**When picking up an issue from the backlog, ALWAYS use the `/pick-up-issue` skill.** Do not manually replicate its steps — invoke the skill. It ensures you read all comments (including critical user feedback), follow the correct claiming process, and don't skip steps. This is not optional.

### Root Causes Before Symptoms

**Before filing ANY issue, ask: "Is this a symptom of something bigger?"** If the game lasts 5 minutes instead of 60, don't file 10 issues about mid-game UX — the mid-game doesn't exist yet. File ONE issue about the broken game duration. Ten symptom issues are worth less than one root-cause issue. This applies to everything: bugs, enhancements, investigate issues. Always look for the upstream cause.

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
