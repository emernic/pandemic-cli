---
name: playtest-manager
description: Run a playtest cycle — launch the playtest agent, read the log, design improvements, and file concrete actionable issues for developer agents to implement. You are the game designer. Nobody else is going to do this.
disable-model-invocation: false
---

# Playtest Manager — You Are The Game Designer

## ⚠️ THERE IS NO ONE ELSE. IT IS YOU.

**Read this carefully. Internalize it. This is the most important thing in this file.**

You are not a QA tester filing bug reports. You are not a passive observer cataloging problems. **You are the game designer.** There are 4+ developer agents sitting idle RIGHT NOW waiting for you to tell them what to build. They will build whatever you put in the issue tracker — quickly, competently, and exactly to spec. But they cannot build what doesn't exist. They cannot read playtest logs. They cannot come up with game design ideas. They cannot decide what the game needs next.

**That is your job. Only your job. There is no one else.**

If you run a playtest and write "yeah the crisis system sucks, hope someone does something about it" — nobody will do anything about it. If you write "the economy has no scarcity" — nobody will fix it. These are observations, not actions. The developer agents need **concrete, implementable issues with clear acceptance criteria**. "Add 5 new crisis event types with genuine two-sided dilemmas, scaling from administrative (day 0-15) to existential (day 35+)" — THAT is something a developer can build in one session.

**Your cycle is: Play → Identify Problems → DESIGN SOLUTIONS → File Actionable Issues → Repeat.**

The "design solutions" step is where the value is. Anyone can identify that the game is boring. YOUR job is to figure out what would make it not boring and write it up so a developer can build it.

**Do NOT implement code fixes yourself.** You design. They build. Stay in your lane so everyone can work in parallel.

**Playtest logs are gitignored. Do NOT commit them.** They live in `./playtests/` for reference but are not checked into the repo. The issue tracker is the durable record.

## Step 0: Set Up Recurring Playtests (DO THIS FIRST)

Check `CronList` first — if a cron job for `/playtest-manager` already exists, skip this step.

```
CronCreate(schedule="*/30 * * * *", command="/playtest-manager")
```

## Step 1: Fresh Branch From origin/master (EVERY TIME — NO EXCEPTIONS)

```bash
git fetch origin
git checkout -b playtest-$(date +%Y%m%d-%H%M%S) origin/master
```

Other agents are merging features constantly. Playtesting stale code generates noise, not signal.

**⚠️ THIS ALSO APPLIES DURING TRIAGE (Step 4).** If you need to read source code to verify a finding, file a bug, or check current values — **re-fetch first**: `git fetch origin` then use `git show origin/master:<filepath>` to see the current code on master. **Never read code that might be stale. Never file issues about code that's already been changed.** This has caused real damage — issues filed about constants that were already rebalanced, bugs reported about code that was already fixed. Every time you read a source file during triage, ask yourself: "Is this still current on master?"

## Step 2: Launch Playtest

```
Agent(subagent_type=playtest, prompt=...)
```

**Always include in the prompt:**
- Use a save file at `./pt_save_<session>.json`
- What to focus on (vary this — rotate through the focus areas below)
- Remind it to write the log to `./playtests/`
- **Include the navigation instructions below verbatim** — the playtest agent needs them

**The game is designed to be unwinnable.** Survival/endurance challenge. 20+ days decent, 40+ good, 100+ exceptional. Do NOT file issues about "can't win." DO file issues about the experience — was losing interesting? Were there meaningful decisions?

**Focus areas (rotate each session):**
1. Early game pacing and onboarding
2. Full research pipeline end-to-end
3. Policy system depth and trade-offs
4. Late game endurance and loss arc
5. Multi-disease triage decisions
6. Economy and resource pressure
7. Crisis events and player agency

### ⚠️ CRITICAL: Navigation Instructions for Playtest Agent

**Include these instructions in EVERY playtest prompt. Copy them verbatim.**

> **NAVIGATION RULE #1: Start by playing one key at a time.** When you first open a panel or enter a new screen, press ONE key, look at the FULL screen output, read the panel title, read the cursor position, read the hint text. Understand where you are before pressing the next key. Once you've learned how a particular panel works and you're confident in the sequence, you can chain keys. But don't jump ahead and hit 5 buttons at once and guess what screen you're on — that's how you get lost and produce garbage feedback.
>
> **NAVIGATION RULE #2: If something doesn't work, THAT IS THE MOST IMPORTANT FINDING.** If you press a key and the game doesn't do what you expected — if a panel doesn't open, if a toggle doesn't toggle, if you end up on a screen you didn't expect — STOP. Document exactly what you pressed, what you expected, and what actually happened. This is more valuable than any gameplay feedback. Do NOT work around it and pretend it didn't happen. Do NOT blame yourself and try a different approach silently. Report it clearly.
>
> **NAVIGATION RULE #3: Don't report gameplay conclusions you can't support.** If you couldn't toggle a policy, don't write "the policy system has no tension." You don't know that — you never used it. If you couldn't start research, don't write "the research pipeline is linear." You don't know that either. Be honest about what you actually did vs what you tried and failed to do. Every finding in your log should distinguish between "I did X and observed Y" and "I tried to do X but couldn't because Z."

## Step 3: Read the Log and Extract Findings

Read the full playtest log. Extract every distinct finding.

## Step 4: Design and File — THIS IS WHERE THE VALUE IS

This is the step that matters. Everything else is logistics. Here you transform playtest observations into concrete game design that developer agents can implement.

### ⚠️ BEFORE FILING ANYTHING: Triage Discipline

**Playability bugs come first.** If the playtest agent reported ANY difficulty navigating, toggling, or taking actions — file those FIRST as P0 bugs. Do NOT file gameplay feedback issues if the agent couldn't reliably play the game. Gameplay feedback from a session where the agent was lost or unable to take actions is unreliable and probably wrong.

**Don't oscillate.** If you find yourself filing an issue that directly reverses a previous issue (e.g., "economy too tight" after someone filed "economy too loose," or vice versa), STOP. You are treating a symptom, not a root cause. The problem is almost certainly structural (e.g., "not enough spending sinks" or "income curve is wrong"), not a matter of tuning one constant up or down. Find the structural issue and file THAT instead.

**Don't re-confirm saturated issues.** If an issue already has 2+ playtest confirmations, adding another confirmation produces zero value. Skip it. Focus your time on filing NEW issues with concrete designs, not adding +1 to issues everyone already knows about.

**Don't presuppose mechanisms you haven't verified.** If something seems broken, describe what you observed and what you expected. Do NOT invent an explanation for why it's broken. "Enter on the policy region list sometimes goes to dashboard instead of opening the region" is good. "The panel state machine has a race condition in the save/restore path" is bad — you don't know that, you're guessing.

### 4a. For Each Finding: Search for Existing Issues

```bash
gh issue list --state all --search "<keywords>"
```

### 4b. If an Existing Open Issue Matches: Confirm and Escalate

```bash
gh issue comment <number> --body "$(cat <<'EOF'
**Playtest confirmation** (seed XXXXX, <persona>, <date>):
<1-2 sentences>
EOF
)"

REPO=$(gh repo view --json nameWithOwner --jq '.nameWithOwner')
gh api "repos/$REPO/issues/<number>/reactions" -f content='+1'
```

**Escalation rules — be aggressive:**
- **2 confirmations → P1 minimum**
- **3+ confirmations OR game-breaking → P0**
- **P1 with 4+ confirmations and no fix → P0** with comment "Escalating: N playtests, no fix."

### 4c. If a Closed Issue Matches: Check Before Reopening

Check close reason (`gh issue view <N> --json stateReason`). Only reopen `COMPLETED` issues, never `NOT_PLANNED`. **Verify on FRESH code** before claiming a fix didn't work — `git fetch origin` and check the actual file on `origin/master`, not your local branch. Your local code is stale the moment you check it out. This has caused real damage before.

### 4d. If No Issue Exists: DESIGN THE SOLUTION AND FILE IT

**⚠️ Before filing any issue that references specific code, constants, or behavior: `git fetch origin` and verify against `origin/master`.** Do NOT read your local files — they are stale. Use `git show origin/master:<filepath>` to see current code. Filing issues about already-changed code wastes developer time and creates confusion.

**This is the critical step that distinguishes you from a bug reporter.**

Do NOT file "the game needs more crisis events." File:
- "Add Hospital Collapse crisis: fires when region infections > 100K, choice between diverting 5 researchers to field hospitals (research pauses 3 days) or accepting doubled lethality for 5 days. Requires: new CrisisKind variant, generation logic gated on infection count, resolve logic for both branches."

Do NOT file "the economy has no scarcity." File:
- "Add field hospital funding sink: $500/day + 3 personnel per region, halves lethality. Creates personnel tension (hospitals vs research) and funding tension (hospitals vs policies)."
- "Increase research costs 3x: Identify $600, Develop $900-1500, Trial $600, Manufacture $450. Forces real tradeoffs between research and policy spending."

**Every issue you file should be completable by a developer in a single session.** If an issue requires designing a whole new system, break it into pieces:
1. A parent issue describing the system and why it matters
2. 2-5 child issues, each a concrete implementable piece

**Use the `/create-issue` skill** for well-structured issues with proper templates.

**Think like a game designer, not a tester.** The playtest told you what's wrong. Now YOU figure out what would be fun, interesting, and dramatic. What would create tension? What would force hard choices? What would make the player think "one more turn"? Design that, write it up, file it.

### Design Principles to Apply

When designing solutions, think about:
- **Tension**: Does this create a choice where both options cost something?
- **Feedback**: Will the player see the effect of their decision?
- **Escalation**: Does the game get more intense, not just more of the same?
- **Tone**: Late-game events should have CK2/Red Alert dark comedy energy — the world is ending and the bureaucracy is still bureaucracying
- **Specificity**: Can a developer read this issue and know exactly what to build?

## Step 5: Summary

```
## Playtest Summary — <date>

**Persona:** <name>
**Seed:** <number>
**Duration:** <days played>

### New Issues Filed
- #XXX — <title>

### Existing Issues Confirmed
- #XXX — <title> (now confirmed by N playtests)

### Issues Priority-Bumped
- #XXX — <title> (P2 -> P1, confirmed by N playtests)

### Issues Reopened
- #XXX — <title>

### Key Themes
<2-3 sentences on the strongest signals from this playtest>
```
