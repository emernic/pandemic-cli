---
name: create-issue
description: Write up a well-structured GitHub issue based on something you experienced during this session
disable-model-invocation: false
---

# Create Issue

You are writing a GitHub issue based on something you encountered during this conversation — a bug, a missing feature, an idea, or a cleanup task. Follow this process carefully.

## Core Principles

0. **Root causes before symptoms.** Before filing ANY issue, ask: "Is this a root cause, or a symptom of something bigger?" If the game lasts 5 minutes instead of 60, don't file ten issues about mid-game balance — file ONE issue about the broken game duration and note that your balance feedback is suspect because of it. But **still file genuinely independent issues** — a real bug (victory fires with infected > 0), a real UX problem (no feedback when toggling policies), a real clarity issue (vaccination vs treatment is confusing) — these are valid regardless of game duration. The key distinction: is your feedback *about the balance/pacing* (downstream of the root cause) or *about something that's broken independent of balance* (valid on its own)?
1. **Never claim more than you actually know.** This is the #1 rule. If the user said "the numbers look wrong," write "the numbers look wrong" — do NOT inflate that into detailed claims about which specific computation is wrong, what the correct values should be, or why the bug exists. Reading source code does NOT mean you know what happens at runtime. Skimming a function does NOT mean you understand the full data flow. **State observations at the level of confidence you actually have.** When in doubt, be vaguer, not more specific. A vague-but-true issue is infinitely better than a specific-but-wrong one.
2. **Describe what you actually experienced.** What were you doing? What happened? What was confusing, broken, or missing? Write from your real experience — do not fabricate or embellish.
3. **Do NOT prematurely plan the solution.** The issue describes a *problem* or *need*, not an implementation plan. Resist the urge to design the fix. The person working on this issue will figure out the right approach themselves.
4. **If you have a solution idea**, you may include it, but it MUST go in a clearly marked section: `## Possible Solution (do not use directly)`. This section should be treated as a rough sketch, not a spec. The implementer should feel free to ignore it entirely.
5. **Be self-contained.** The person reading this issue will NOT have access to your conversation. They need to understand the problem fully from the issue alone. Include relevant code paths, reproduction steps, and context.
6. **Keep scope narrow.** One issue = one problem or one feature. If you noticed multiple things, file multiple issues.
7. **Be concise.** Say what's wrong, point to where to look, and stop. Long issues full of analysis are a smell — they usually mean you're hallucinating specificity you don't have. If the issue body is longer than ~15 lines, ask yourself what you can cut.

## Process

### Step 1: Duplicate Check

Before writing anything, search for existing issues that might cover the same ground:

```
gh issue list --state all --search "<keywords>"
```

If a duplicate exists, do NOT create a new issue. Instead, tell the user and suggest commenting on the existing one if there's new information to add.

### Step 2: Classify

Determine the issue type and priority:

**Type** (pick one label):
- `bug` — Something is broken or behaves incorrectly
- `enhancement` — A new feature or improvement to existing functionality
- `chore` — Refactoring, cleanup, tech debt, tooling improvements
- `investigate` — Something looks off or confusing and needs follow-up. **You are NOT claiming there is a problem.** You're asking someone to take a closer look. **File these aggressively and constantly — they are free, they take 30 seconds, and they are the #1 most underused tool in this project.** If you finish a session without filing any, you weren't paying attention. ~30% turn out to be fine, ~30% lead to docs/cleanup, ~40% uncover real issues. All three outcomes are valuable.
- `design` — A thinking task, not a coding task. Someone should play the game, think about this area, and file concrete implementation issues. The deliverable is child issues, not code. Prefix the title with "Design:".

**Priority** (pick one label):
- `P0-critical` — Game-breaking, blocks core functionality
- `P1-high` — Important, should address soon
- `P2-medium` — Normal priority, address when convenient
- `P3-low` — Nice to have, backlog material

### Step 3: Draft the Issue

Think carefully about the title and body before writing. The title should be specific and scannable — someone skimming a list of 50 issues should immediately understand what this is about.

**For bugs**, use this structure:
```markdown
## Description
[What's wrong, in plain language]

## Steps to Reproduce
1. [Concrete step]
2. [Concrete step]
3. [...]

## Expected Behavior
[What should happen]

## Actual Behavior
[What actually happens. Include error messages, panic output, or screenshot descriptions if relevant]

## Relevant Code
[File paths and line numbers that are involved, e.g. `src/engine.rs:142`]

## Possible Solution (do not use directly)
[OPTIONAL — only if you have a concrete idea. The implementer should feel free to ignore this.]
```

**For enhancements**, use this structure:
```markdown
## Description
[What's missing or could be better, and WHY it matters. What problem does this solve? What inspired this idea?]

## Current Behavior
[How things work today, if relevant]

## Acceptance Criteria
- [ ] [What "done" looks like — concrete, verifiable items]
- [ ] [...]

## Relevant Code
[File paths and line numbers the implementer should look at to orient themselves]

## Possible Solution (do not use directly)
[OPTIONAL — only if you have a concrete idea. The implementer should feel free to ignore this.]
```

**For investigate issues**, use this structure:
```markdown
## What I Noticed
[What looked off, incomplete, or unclear. Be specific about what you observed.]

## Context
[What you were doing when you noticed this. What's the current behavior?]

## Why This Caught My Attention
[Why it seemed worth a second look. What confused you or seemed incomplete.]

## Relevant Code
[File paths and line numbers to look at]
```

**For design issues**, point at an area of the game. Don't narrow it — the whole point is for someone to think broadly. Don't describe what's wrong or what's missing; you don't know yet, that's the job of whoever picks it up. The issue is done when child issues have been filed, not when code ships.

**CRITICAL: Investigate issues are about asking questions, not making claims.** You have NOT investigated the thing yet. You noticed something in passing while doing other work. You do not know whether it's a problem, and you do not know what the fix would be if it is. Your job is to say "hey, can someone look at this?" — nothing more.

**Do NOT include:**
- A "Suggested Fix" or "Possible Solution" section. You haven't investigated — you have no basis for suggesting a fix.
- Assertions about what the behavior "should" be. You don't know that yet.
- Confident-sounding diagnoses like "the problem is X" or "this is broken because Y." You don't know if there IS a problem.

**Good vs. bad examples:**

Good title: `Investigate: save-on-quit only triggers when a file path is provided — is that the right UX?`
Good body: "While fixing #6, I noticed the Help panel said 'Quit & save' but saving only happens if a file path was provided via CLI args. I didn't dig into this — I just fixed the label mismatch. But the save behavior itself seemed like it might be incomplete or confusing for players. Someone should take a look and confirm this is working the way we actually want."

Bad title: `Fix save-on-quit to auto-generate save file path`
Bad body: "The save system is broken — it silently discards the player's progress when no file path is provided. The fix is to auto-generate a save file with a random name in ~/.pandemic/ on startup. This would ensure saves always happen."
— This is terrible. The filer didn't investigate anything. They have no idea if auto-generating a save file is the right design. They don't know if the current behavior is intentional. They jumped straight to a confident-sounding diagnosis and solution based on a surface-level observation. This is exactly the kind of premature conclusion that investigate issues are designed to AVOID.

Investigate issues should be filed freely. You do NOT need user permission. Keep the title prefixed with "Investigate:" and phrase it as a question or observation, not a conclusion.

### Playtest Feedback Label

If the issue originates from a playtest session (either your own playtest or an automated playtest agent's report), add the `playtest-feedback` label. This signals to whoever picks up the issue that the report may be imprecise — the playtester's description of the problem and their suggested fix should not be taken at face value. The value is in identifying *which area* of the game triggered feedback, not in the specific diagnosis.

**Color blindness check:** AI playtest agents cannot see console colors. Before filing a playtest issue about a "missing" visual indicator, consider whether it might be a color-based indicator that works fine for human players. If you suspect color blindness is the cause, either skip filing the issue or note it explicitly in the issue body (e.g., "Note: this may be a color-based indicator invisible to AI playtesters").

**For chores**, use this structure:
```markdown
## Description
[What needs cleaning up and why. What's the concrete problem with the current state?]

## Relevant Code
[File paths and line numbers]

## Acceptance Criteria
- [ ] [What "done" looks like]
- [ ] [...]

## Possible Solution (do not use directly)
[OPTIONAL]
```

### Step 4: Review Before Submitting

Before creating the issue, re-read your draft and check:
- [ ] **No hallucinated specificity.** For every factual claim, ask: "Did I actually verify this, or did I infer it from skimming code?" If you inferred it, either soften the language ("appears to", "may be") or remove the claim entirely. This is the most important check.
- [ ] **Concise.** Can you cut the body in half without losing essential information? If so, do it.
- [ ] **Title is specific and scannable** (not vague like "Fix bug" or "Improve things")
- [ ] **Description is grounded in real experience**, not hypothetical
- [ ] **No premature solution design** leaked into the description (keep it in the optional section if anywhere)
- [ ] **Self-contained** — a reader with no other context can fully understand the problem
- [ ] **Scope is narrow** — one problem per issue
- [ ] **Code references are included** so the reader can orient themselves
- [ ] **Acceptance criteria are included** for enhancements and chores
- [ ] **For investigate issues:** Does this read as a question/observation, or as a premature diagnosis? If it sounds like you already know the answer, rewrite it. Strip out any confident claims about what "should" happen or what the fix is.

### Step 5: Create the Issue

Use `gh issue create` with appropriate labels. All issue types should have a priority label, with one exception: investigate issues that were filed by an AI agent (not requested by the user) should NOT have a priority label, since the outcome is uncertain. **Issues that come directly from the user ALWAYS get P0 or P1 priority, regardless of type — including investigate issues.** Anything requested directly by the user is highest priority and requires special careful consideration of their exact wording.

```bash
gh issue create \
  --title "Title here" \
  --label "type-label" \
  --label "priority-label" \
  --body "$(cat <<'EOF'
[issue body here]
EOF
)"
```

After creation, display the issue URL to the user.

## Reminders

- You are writing for an audience that has ZERO context about your conversation. Over-explain rather than under-explain.
- Describe problems, not solutions. The "Possible Solution" section is optional and explicitly labeled as ignorable.
- If the user asks you to file multiple issues, file them one at a time, each with its own duplicate check.
