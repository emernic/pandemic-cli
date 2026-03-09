---
name: pick-up-issue
description: Pick up a GitHub issue to work on — claims it, creates a branch, and guides you through completion. TRIGGER when the user asks to work on an issue, pick up an issue, grab something from the backlog, or find something to work on.
disable-model-invocation: false
---

# Pick Up Issue

You are picking up a GitHub issue to work on. This process ensures no two agents work on the same issue and that the issue is properly tracked through completion.

## How We Track Issue Ownership

We run multiple agents in parallel, so we do NOT use GitHub's assignee feature. Instead, we use the `in-progress` label as our sole claiming mechanism:
- **Available:** open, no `in-progress` label
- **Claimed:** open, has `in-progress` label
- **Done:** closed, label removed

NEVER check or set GitHub assignees. Always filter by the `in-progress` label.

## Step 1: Select an Issue

**If a specific issue was provided** (via arguments or conversation), use that issue number and skip to Step 2.

**Otherwise**, find an available issue (no `in-progress` label), pick the best one, and start working on it immediately. Do NOT ask the user to choose — just pick one and go.

```bash
gh issue list --state open --search "-label:in-progress" --json number,title,labels,createdAt
```

Selection criteria (in priority order):
1. **Priority labels** — P0-critical > P1-high > P2-medium > P3-low > unlabeled.
2. **Playtest confirmation count** — among equal-priority issues, prefer issues that have been confirmed by multiple independent playtests. Look for 👍 reactions and comments starting with "**Playtest confirmation**". An issue confirmed by 3+ playtests is a stronger signal than one filed by a single session. These are real, repeatedly-observed problems.
3. **Infrastructure first** — among equal-priority issues, prefer issues that affect the development/testing pipeline over game features. Broken playtests, save file corruption, snapshot mode bugs, build issues, and skill/docs inaccuracies ripple through everything — every other agent's work quality depends on these systems being solid. A playtest bug that causes bad issue reports is worse than a missing game feature.
4. **Age** — older issues first, all else being equal. Issues that have been sitting in the backlog longest are the ones nobody picks up. That's exactly why YOU should pick them up. Sort by `createdAt` ascending and default to the oldest available issue unless a higher-priority one exists.
5. **Dependencies** — skip issues that clearly depend on unfinished work.

**DO NOT skip issues because they "don't look like code changes."** Infrastructure issues, documentation issues, design issues, tooling issues — they're all in the backlog because they matter. If you find yourself thinking "this isn't a real feature" or "this isn't self-contained enough," that's the bias talking. Pick it up anyway. Every agent that skips the same issue is proof the issue needs to be picked up.

**DO NOT cherry-pick "easy wins."** The hard, ambiguous, or unfamiliar issues are the ones that rot in the backlog forever because every agent reaches for the comfortable code change instead. If an issue has been open for a while, that's a signal it needs attention, not a signal to skip it.

Briefly tell the user which issue you picked and why, then move to Step 2. If there are no available issues, tell the user.

## Step 2: Claim the Issue

Before starting any work, mark the issue as in-progress so other agents don't pick it up:

```bash
gh issue edit <number> --add-label "in-progress"
```

Also add a comment noting that work has started:

```bash
gh issue comment <number> --body "Picking up this issue."
```

**Minimize the race window:** Run the label and comment commands as early as possible — before reading the issue body, before planning, before anything else. The sooner you claim, the less likely another agent picks the same issue.

## Step 3: Create a Branch

Create a branch from master named after the issue:

```bash
git fetch origin master
git checkout -b issue-<number>-<short-description> origin/master
```

Use a short kebab-case description derived from the issue title (e.g., `issue-7-fix-spread-calc`).

> **Note:** Do not use `git checkout master` — master may be checked out in another worktree. Always branch from `origin/master` after fetching.

## Investigate Issues

Issues labeled `investigate` are fundamentally different from bugs and enhancements. **The person who filed the issue does not know if there is a problem.** They noticed something that looked off while doing other work and asked someone to take a closer look. Your job is to actually do that investigation with an open mind — not to assume the filer was right and jump straight to a fix.

When picking up an investigate issue:

1. **Don't narrow onto the specific thing the issue mentions.** The issue points you to a *neighborhood* of the code that confused someone. Your job is to look at that whole area — not just the one detail they called out. Read the surrounding code, understand the broader system, and think about how it all fits together. The real issue is often something the filer didn't even mention, because they only saw a surface symptom.

2. **Think about what's actually right for the game.** This is the most important step and the one most likely to be skipped. Don't think in terms of "issues and fixes." Think like a game designer: How should this system work? What would make sense for the player? What's the intended experience? Step back from the code and ask yourself what the *right* behavior is before you start evaluating whether the *current* behavior matches it. If you can't articulate what the system should do and why, you haven't thought about it enough yet.

3. **Start with no assumptions.** The current behavior might be exactly correct. The issue filer might have been confused about something that's actually fine. Or the behavior might be wrong, but not in the way the issue suggests — the whole area might be poorly designed in a way that's more fundamental than what was called out.

4. **Determine the outcome** (only after you genuinely understand the area):
   - **It's actually fine** (~30%) — close the issue as not planned (`gh issue close <number> --reason "not planned"`) with a comment explaining why the behavior is correct and makes sense. This is a perfectly valid and common outcome. Closing investigate issues is good hygiene, not a failure.
   - **It works but is confusing** (~30%) — add documentation, rename things, add a code comment, or restructure to make the behavior obvious. The code was right, but understandably confusing.
   - **It's actually a problem** (~40%) — fix the bug, clean up the design, or file a more specific bug/enhancement issue if the fix is too large for this pass. The fix might be for the specific thing the issue mentioned, or it might be a broader redesign of the area.

5. Investigate issues are often small and self-contained. Many can be resolved quickly — but don't use that as an excuse to skip the "step back and think" part.

## Design Issues

Issues labeled `design` are thinking tasks, not coding tasks. **The deliverable is a set of concrete implementation issues, not a PR with code changes.** You are being asked to be a game designer.

When picking up a design issue:

1. **Play the game.** Seriously. Run snapshot commands, experience the area the issue is about. You cannot design what you haven't experienced.
2. **Read the design direction.** Check the Game Inspirations and design philosophy sections in CLAUDE.md. Read `docs/gameplay.md`. Understand what the game is trying to be and how the existing systems work.
3. **Think.** What does this area need? What would create interesting decisions for the player? What existing systems can you build on — new combinations of existing legos, not new legos? What would the games in our inspirations list do here?
4. **File concrete implementation issues.** Each one should be a focused, self-contained enhancement or bug that an agent can pick up and ship. Use the create-issue skill. Reference the parent design issue in each child issue.
5. **Close the design issue** when you've filed the child issues. Comment with a summary of what you filed and why.

**Do not write code.** Do not create a branch. The design issue is complete when the child issues exist. No PR, no merge.

**The trap to avoid:** Don't treat this as a brainstorming exercise where you generate a dozen half-baked ideas. Think carefully about each issue you file. Would you want to implement it? Does it make the game genuinely better? Every issue you file will cost an agent a full session to implement — make sure it's worth that cost.

## Playtest Feedback Issues

Issues labeled `playtest-feedback` were filed by automated playtest agents. **These are the most dangerous issues in the backlog** because they look like real bug reports but are often completely wrong. Playtest agents can't see colors, they play for a few seconds, and they confidently file issues about "problems" that don't exist. They are LLMs describing a game they cannot actually see.

**Do NOT trust the diagnosis.** The playtest agent's description of what's wrong is probably wrong. Their suggested fix is almost certainly wrong. What they *can* tell you is which area of the game triggered confusion — treat the issue as a pointer to a neighborhood, not as a specification.

**Before writing a single line of code**, you MUST:

1. **Play the game yourself** in the area the issue describes. Run snapshot commands, look at the output, understand what the player actually sees and experiences.
2. **Ask yourself: is this actually a problem?** Not "does the issue describe a real thing" — but "if I were a human player, would this bother me? Does the current behavior make sense?" Many playtest issues describe behavior that is correct, intentional, and good.
3. **If you conclude it's not a problem, close the issue as not planned** (`gh issue close <number> --reason "not planned"`). Closing invalid playtest-feedback issues is good hygiene. Explain briefly why the current behavior is correct. This is a common and valid outcome — probably ~40% of playtest-feedback issues should just be closed.
4. **If it IS a problem, form your own solution.** Do not implement the suggestion from the issue. Think about what the right behavior is from a game design perspective, then implement that.

**Example of what NOT to do:** A playtest agent reports "health bars are always solid green" because it can't see colors in snapshot mode. The bars are actually multi-colored and working perfectly. An agent blindly implements sqrt scaling to "fix" the bars, making them actively misleading. The right move was to play the game, realize the bars are fine, and close the issue.

### Color Blindness in AI Playtests

AI playtest agents (Claude Code) **cannot see console colors**. They receive raw text output from snapshot mode and have no awareness of ANSI color codes, background colors, border highlights, or any other color-based visual indicators. This is a permanent limitation — not a bug.

**Many playtest-feedback issues are caused by this color blindness.** Common false reports include:
- "No visual indicator of selected region" — there IS one, it's a color highlight
- "Health bars are always solid green" — they're multi-colored
- "Can't tell which panel is active" — it has a colored border
- "No feedback after action" — there's a colored status message

**When evaluating a playtest-feedback issue, always ask:** "Could this be caused by the playtester not seeing colors?" If the answer is yes, play the game yourself to verify. If the color-based indicator exists and works, close the issue as not planned.

**However:** the game should still strive to be playable without color. If something relies *solely* on color to convey information (no structural/text indicator at all), that's a real accessibility concern worth addressing — but the fix should add a non-color indicator *in addition to* the color one, not replace it.

## Step 4: Read and Understand the Issue

Read the full issue body **AND all comments**:

```bash
gh issue view <number> --comments
```

**⚠️ You MUST read all comments.** Comments often contain critical context, corrections, or direct instructions from the user that override or refine the issue description. The `--comments` flag shows everything — do not skip it.

**Pay special attention to comments from the user (the repo owner).** These are direct instructions. If the user says "this approach is wrong" or "do it this way instead," that takes absolute priority over anything in the issue description. The issue was filed by an AI agent; the user comment is from a human who knows what they want.

Make sure you understand:
- What the problem or request is
- What the acceptance criteria are (if provided)
- Which files/code are referenced
- **What the comments say** — especially any from the user

If the issue has a "Possible Solution" section, read it for context but do NOT follow it blindly — form your own approach.

## Step 5: Research the System — Not Just the Code You'll Touch

**STOP. Do not start coding yet.** Before you can think clearly about what to do, you need to actually understand the system you're about to change. Not just the one function or file the issue points to — the whole neighborhood.

1. **Read the code the issue references**, plus the code that calls it, the code it calls, and anything that shares data structures with it. If the issue is about medicine doses, don't just read the dose values — read how doses are consumed in `deploy_medicine`, how they're replenished via manufacturing, how efficacy modifies them, what the cost/income economy looks like. Understand the full lifecycle.

2. **Play the game yourself.** Run snapshot commands to see what the player actually experiences in this area. Not one command — play through the relevant flow. If it's about medicines, actually do the research→develop→deploy pipeline. If it's about saves, test loading and saving. See what happens.

3. **Map the interactions.** Ask yourself: what other systems does this touch? If you change X, what happens to Y and Z? A dose scaling change affects game balance, economy, win/lose conditions, and the usefulness of the entire research pipeline. A save path change affects every agent, every playtest, and every human player. Write down the systems involved so you don't miss one.

4. **Check the design docs.** Read `docs/target-architecture.md`, `docs/gameplay.md`, and `docs/architecture.md` if they're relevant to what you're changing. They may contain context that changes your approach.

The goal is to understand the system well enough that your change is *precise* — it fixes the actual problem without accidentally breaking something you didn't think about. If you find yourself saying "I'll just change this one number and see if it works," you haven't researched enough.

## Step 6: Think — Then Decide Whether to Do the Work

Now that you understand the system, answer these questions honestly:

1. **Is this actually a problem?** Not every issue describes a real problem. The filer may have been confused, wrong, or working from incomplete information. If the current behavior is correct and makes sense, close the issue as not planned (`gh issue close <number> --reason "not planned"`) with an explanation. This is a valid outcome.
2. **Does the proposed solution make sense?** Even if the problem is real, the suggested fix might be wrong. Think about what would actually be right for the game and the player. Would you make this change if no issue existed and you were just looking at the code?
3. **Is this change making the game simpler or more complex?** Good changes almost always make things simpler. If your planned change adds complexity (new config, special-case logic, non-obvious scaling, etc.), that's a red flag. Step back and ask if there's a simpler approach — or if the "problem" is actually fine as-is.
4. **What are the ripple effects?** Based on your research, what other systems will your change affect? Are those effects intentional and good, or unintended side effects you need to account for?

**If you can't clearly articulate why the change improves the game, don't make it.** Close the issue as not planned (`--reason "not planned"`) or ask the user for guidance.

### You Are the Game Designer

**This is not optional.** You are not just an engineer executing tickets. You are the game designer. There is no one else. If a mechanic is unfun, if a system creates no interesting decisions, if a fix addresses a symptom but leaves the real design problem untouched — that is YOUR problem. Not "a design decision for later." Not "outside the scope of this issue." Yours, right now.

When you fix a bug, ask: **why did this bug matter?** If the answer reveals a deeper design gap (e.g., "funding goes negative because players can't dismiss personnel"), fix the bug AND file an issue for the design gap. Don't just clamp a number and walk away. The game being unfun or nonsensical is a failure that belongs to you, because you are the only one who will ever notice it in this session. The next session starts fresh with zero memory. If you don't act on what you see, nobody will.

## Step 7: Make a Task List, Then Implement

**Before writing code, write down your plan as a task list.** This keeps you honest about what's actually involved and prevents the "I'll just change this one thing" trap where you miss follow-up work.

A good task list for a typical issue:

1. Research the system (Step 5 — you just did this)
2. Decide on approach (Step 6 — you just did this)
3. Make the code change
4. Update any related code that's affected by your change
5. Update docs/comments if behavior changed
6. Run tests, fix failures
7. Play the game to verify it looks/feels right
8. Commit
9. Run `/reflect`
10. Fix anything found, commit
11. Push, create PR, merge
12. Close issue, remove label

Adapt to the task — small fixes need fewer steps, big changes need more. But always include the full lifecycle. **The task isn't done until the PR is merged and the issue is closed.**

Once you're confident in your plan, implement it. Follow the project's conventions (see CLAUDE.md). Run tests with `cargo test` to make sure nothing is broken.

**While you work, look around.** You are not a machine that processes one issue and exits. You are reading real code in a real codebase. If you see something broken, confusing, or architecturally wrong in the code you're touching or the code next to it — file an investigate issue. It takes 30 seconds. If you don't do it, no one will. See CLAUDE.md for the full ownership philosophy.

## Step 8: Completion

When the work is done:

1. **Commit** your changes (with user approval).
2. **Push** the branch and **create a PR** that references the issue:
   - Include `Closes #<number>` in the PR body so the issue auto-closes on merge.
3. **Always merge your own PRs.** Do not ask for permission — this is an early-stage project and it's more important to get changes in than to risk leaving them behind. If you notice something to improve after merging, just create a follow-up PR and merge that too. Iterate freely.

**CRITICAL — you are not done until the issue is closed and cleaned up:**

4. **After merge**, verify the issue was closed by running `gh issue view <number>`. If it's still open, close it manually:
   ```bash
   gh issue close <number>
   ```
5. **Remove the in-progress label**:
   ```bash
   gh issue edit <number> --remove-label "in-progress"
   ```
6. **Confirm** the issue shows as CLOSED before moving on. This is the definition of done.

## Abandoning Work

If you realize you cannot complete the issue (too complex, blocked, unclear, etc.):

1. Tell the user what's wrong and why you're stuck. **Ask for permission before abandoning.**
2. Once approved, remove the `in-progress` label so another agent can pick it up:
   ```bash
   gh issue edit <number> --remove-label "in-progress"
   ```
3. Leave a comment explaining what you tried and where you got stuck, so the next agent doesn't repeat your work:
   ```bash
   gh issue comment <number> --body "Abandoning: <reason>. <what was tried, what's left>"
   ```

## Important

- **Always merge your own PRs** — do not wait for approval. If you find more to fix after merging, open a follow-up PR and merge that too.
- NEVER abandon without user permission.
- If you get stuck or the issue is unclear, ask the user rather than guessing.
- If you realize the issue is a duplicate or invalid while working on it, tell the user before closing anything.
- **Own the codebase.** Your job is not just "close this one issue." Every file you read, every function you call, every test you run — you are responsible for what you see. File investigate issues for anything that looks off. Push the architecture toward the target state. Don't leave the codebase worse than you found it.
