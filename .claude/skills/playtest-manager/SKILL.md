---
name: playtest-manager
description: Run a playtest cycle — launch the playtest agent, read the log, file/update issues based on findings, and track which issues are repeatedly confirmed across playtests. Use this instead of manually orchestrating playtests.
disable-model-invocation: false
---

# Playtest Manager

You manage the full playtest cycle: launch, read results, and triage findings into the issue tracker. Your most important job is **tracking signal strength** — when the same problem comes up across multiple playtests, that's a strong signal that should be reflected in the issue tracker so agents picking up work can see it.

**Playtest logs are gitignored. Do NOT commit them.** They live in `./playtests/` for reference but are not checked into the repo. The issue tracker is the durable record.

## Setup: Recurring Playtests

To run playtests on a recurring schedule, use the `/loop` skill:

```
/loop 30m /playtest-manager
```

This runs a full playtest cycle every 30 minutes. Each cycle launches a fresh playtest agent, reads the log, and triages findings.

## Step 1: Prepare

Create a branch if not already on one:

```bash
git fetch origin
git checkout -b playtest-$(date +%Y%m%d-%H%M%S) origin/master
```

## Step 2: Launch Playtest

Launch the playtest agent with a save file in the current worktree (never shared locations):

```
Agent(subagent_type=playtest, prompt=...)
```

**Always include in the prompt:**
- Use a save file at `./pt_save_<session>.json`
- What to focus on (vary this across sessions — don't test the same thing every time)
- Remind it to write the log to `./playtests/`

**Vary the focus area.** To avoid testing the same thing every time, check which focus area was tested most recently by looking at existing playtest logs in `./playtests/` or the git log for recent playtest branches. Then pick a different one:

1. Early game pacing and onboarding
2. Full research pipeline end-to-end
3. Policy system depth and trade-offs
4. Late game / win-loss conditions
5. Multi-disease management
6. Economy and resource pressure
7. Crisis events and player agency

## Step 3: Read the Log

Read the full playtest log. Extract every distinct finding — problems, ideas, positive feedback, confusion points.

## Step 4: Triage Findings

For each finding, follow this process:

### 4a. Search for Existing Issues

```bash
gh issue list --state all --search "<keywords>"
```

Search broadly — the same problem may be described differently across playtests.

### 4b. If an Existing Open Issue Matches

**Add a playtest confirmation comment** and a thumbs-up reaction:

```bash
gh issue comment <number> --body "$(cat <<'EOF'
**Playtest confirmation** (seed XXXXX, <persona>, <date>):
<1-2 sentences describing how this playtest encountered the same problem>
EOF
)"

# Get repo from git remote (don't hardcode)
REPO=$(gh repo view --json nameWithOwner --jq '.nameWithOwner')
gh api "repos/$REPO/issues/<number>/reactions" -f content='+1'
```

**Check if this issue should be priority-bumped.** If an issue has been confirmed by 3+ playtests and is currently P2 or P3, bump it:

```bash
# Count existing playtest confirmations
gh issue view <number> --json comments --jq '[.comments[].body | select(startswith("**Playtest confirmation**"))] | length'
```

If the count (including your new comment) reaches 3+, bump priority:
- P3-low -> P2-medium
- P2-medium -> P1-high
- P1-high stays P1-high (only humans bump to P0)

```bash
gh issue edit <number> --remove-label "P2-medium" --add-label "P1-high"
gh issue comment <number> --body "Bumping priority: confirmed by 3+ independent playtests."
```

### 4c. If a Closed Issue Matches and the Problem Persists

**First check the close reason.** Don't reopen issues closed as "not planned" — those were deliberately rejected. Only reopen issues that were closed as "completed" (i.e., someone thought they fixed it but the problem persists):

```bash
gh issue view <number> --json stateReason --jq '.stateReason'
```

If `COMPLETED` and the problem persists, reopen with a confirmation comment:

```bash
gh issue reopen <number> --comment "$(cat <<'EOF'
**Playtest confirmation** (seed XXXXX, <persona>, <date>):
<description of how the problem was observed again>

Reopening — this problem persists after the previous fix.
EOF
)"
```

If `NOT_PLANNED`, do NOT reopen. If you believe the rejection was wrong, file a new issue explaining why.

### 4d. If No Existing Issue Matches

File a new issue using the `/create-issue` skill. Include the `playtest-feedback` label.

**Root causes before symptoms.** Before filing, ask: is this finding a root cause or a symptom of something bigger? If the dose scaling is broken (#46), don't file separate issues for "medicine feels pointless," "can't tell if deployment worked," and "no reason to manufacture more" — those are all symptoms of the dose scaling problem. File one issue (or confirm the existing one) and note the symptoms in the comment.

## Step 5: Summary

After triaging all findings, output a summary:

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

## Step 6: Update This Catalog

**This is mandatory. Do not skip it.**

After each playtest, update the "Known Recurring Themes" section below:
- **Promote** issues that moved from single observation to multiple confirmations
- **Add** new themes that emerged from this playtest
- **Mark resolved** any themes whose issues have been closed and genuinely fixed (confirmed by this playtest not reproducing the problem)
- **Update issue numbers** if issues were consolidated or replaced
- Commit the updated skill file to your branch (or create a quick PR if on a detached playtest branch)

The catalog is only useful if it stays current. If you skip this step, the next agent starts from stale information.

## Known Recurring Themes

These are the problems that have come up across multiple playtest sessions. When you see these in a new playtest, confirm the existing issue and move on — don't file duplicates.

*Last updated: 2026-03-08 (sessions 3, 4, 5)*

### Confirmed Repeatedly (strong signal)

1. **Medicine dose scaling** (#46) — 100K doses vs millions of infections makes the medicine system feel pointless. Every playtest persona has flagged this. THE root cause of most "player agency" complaints.

2. **Resources pile up with nothing to spend them on** (#47) — RP and personnel accumulate mid-game with no meaningful sink. Downstream of limited research options per disease.

3. **No deployment feedback** (#358) — after deploying medicine, player can't tell if it worked. No visible impact on infection numbers, no adverse effect notification for untested medicines.

4. **Mutation events are noise** (#287) — "Unknown Pathogen #X has mutated" means nothing to the player. No visible consequence, no actionable information.

5. **Crisis events repeat identically** (#359) — same Staff Burnout / International Aid with same text after a few days. Becomes a chore to click through.

### Confirmed Multiple Times (moderate signal)

6. **One medicine per class is too limiting** (#313) — two bacteria but only one antibiotic available. Forces Broad-Spectrum as only option for second bacterium.

7. **Mutations outpace the pipeline** (#377) — medicine is outdated before first deployment. Mutation interval shorter than research pipeline.

8. **Win/loss conditions never communicated** (#378) — player doesn't know what winning looks like or how close they are to losing.

9. **"Vaccinate susceptible" offered for antibiotics** (#375) — biologically nonsensical terminology.

10. **No feedback on policy-disease matching** (#376) — Water Sanitation says "halves waterborne spread" but player can't tell which diseases are waterborne.

11. **"CONTAINED 0%" misleading** (#393) — threat status says "CONTAINED" while thousands are dying because the threshold is based on fraction of 3B total population.

### Early/Single Observations (weak signal — watch for repeats)

12. **Region differentiation** (#324) — six identical regions waste the map's strategic potential.
13. **Knowledge/discovery system** (#325) — progressive revelation could be deeper.

## Guidance for Issue Pickers

When the `pick-up-issue` skill selects work from the backlog, issues with multiple playtest confirmations (visible as 👍 reactions and "Playtest confirmation" comments) should be weighted more heavily. The thumbs-up count is a proxy for "how many independent sessions flagged this as a problem."
