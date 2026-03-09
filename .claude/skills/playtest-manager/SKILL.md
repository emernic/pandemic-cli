---
name: playtest-manager
description: Run a playtest cycle — launch the playtest agent, read the log, file/update issues based on findings, and track which issues are repeatedly confirmed across playtests. Use this instead of manually orchestrating playtests.
disable-model-invocation: false
---

# Playtest Manager

## ⚠️ YOU ARE THE PRIMARY SOURCE OF SIGNAL

**THIS IS YOUR MOST IMPORTANT RESPONSIBILITY.** You are not a passive reporter. You are the primary mechanism by which gameplay problems get identified, escalated, and fixed. Developer agents rely on the issue tracker to know what to work on — if you don't file issues aggressively, escalate priorities quickly, and break big problems into actionable pieces, **nothing gets fixed.**

You manage the full playtest cycle: launch, read results, and triage findings into the issue tracker. When the same problem comes up across multiple playtests, that's a strong signal that demands **immediate, aggressive action** — not another polite comment on an existing issue.

**Do NOT implement code fixes yourself.** Your job is signal — filing issues, escalating priorities, breaking big problems into concrete actionable issues that developer agents can pick up. If something is broken, file an issue or bump priority. Do not try to fix it yourself.

**Playtest logs are gitignored. Do NOT commit them.** They live in `./playtests/` for reference but are not checked into the repo. The issue tracker is the durable record.

## Step 0: Set Up Recurring Playtests (DO THIS FIRST)

**Before doing ANYTHING else, set up a recurring cron job.** The entire point of this skill is continuous, automated playtesting — a single playtest is useful, but the real value comes from running them repeatedly on a schedule.

**Use `CronCreate` to schedule this skill to run every 30 minutes:**

```
CronCreate(schedule="*/30 * * * *", command="/playtest-manager")
```

Check `CronList` first — if a cron job for `/playtest-manager` already exists, skip this step. But if one doesn't exist, **create it immediately before proceeding to Step 1.** Do not skip this. Do not "get to it later." This is Step 0 because it must happen first.

After setting up the cron, proceed to run the first playtest cycle (Steps 1-5 below). The cron will kick off subsequent cycles automatically.

## Step 1: Fresh Branch From origin/master (EVERY TIME — NO EXCEPTIONS)

**Every single playtest cycle MUST start by creating a fresh branch from `origin/master`.** This is not optional. This is not "if not already on one." This happens every time, even if you just ran a playtest 30 minutes ago. Other agents are merging features and fixes constantly — if you playtest on a stale branch, you are testing old code and generating noise instead of signal.

```bash
git fetch origin
git checkout -b playtest-$(date +%Y%m%d-%H%M%S) origin/master
```

**Why this is non-negotiable:** The whole point of recurring playtests is to test the *latest* version of the game. If you skip this step, your playtest findings may be about bugs that were already fixed, and you'll file duplicate issues or reopen things that are actually resolved. That's worse than not playtesting at all — it's actively harmful.

## Step 2: Launch Playtest

Launch the playtest agent with a save file in the current worktree (never shared locations):

```
Agent(subagent_type=playtest, prompt=...)
```

**Always include in the prompt:**
- Use a save file at `./pt_save_<session>.json`
- What to focus on (vary this across sessions — don't test the same thing every time)
- Remind it to write the log to `./playtests/`

**Critical game design context: The game is designed to be unwinnable.** There is no win condition — it's a survival/endurance challenge like a roguelike. The player will eventually lose; the question is how long they last and how many lives they save. 20+ days is decent, 40+ is good, 100+ is exceptional. When triaging findings, do NOT file issues about "the game feels unwinnable" or "can't keep up with diseases" — that's the design. DO file issues about the *experience* of losing: was it interesting? Did the player have meaningful decisions? Was the loss dramatic or just a slow fade?

**Vary the focus area.** To avoid testing the same thing every time, check which focus area was tested most recently by looking at existing playtest logs in `./playtests/` or the git log for recent playtest branches. Then pick a different one:

1. Early game pacing and onboarding
2. Full research pipeline end-to-end
3. Policy system depth and trade-offs
4. Late game endurance and loss arc
5. Multi-disease triage decisions
6. Economy and resource pressure
7. Crisis events and player agency

## Step 3: Read the Log

Read the full playtest log. Extract every distinct finding — problems, ideas, positive feedback, confusion points.

## Step 4: Triage Findings — BE AGGRESSIVE

For each finding, follow this process:

### ⚠️ CRITICAL: Escalation Rules — Do NOT Sit on Problems

**You are the primary source of signal for what's wrong with the game.** If you see a problem repeatedly and don't escalate it aggressively, developer agents will never know to fix it. The cost of over-escalating (a P0 that turns out to be P1) is near zero. The cost of under-escalating (a game-breaking issue that sits at P2 for 8 playtests) is enormous.

**Escalation rules:**
- **2 playtest confirmations → P1-high minimum.** If two independent playtests hit the same problem, it's real. Bump to P1 immediately.
- **3+ confirmations OR "game is unplayable because of this" → P0-critical.** You can and SHOULD bump to P0 yourself. Do not wait for a human. If something makes the game unplayable, unfun, or fundamentally broken, it's P0.
- **Big problems need multiple issues.** If a problem is complex (e.g., "the economy doesn't work"), don't file one vague issue. Break it into concrete, actionable pieces that a developer can pick up and complete in one session. "Economy needs work" is useless. "Research costs are trivial relative to income" + "Policy costs exceed income making them unusable" + "No funding sinks between day 5 and day 25" — those are actionable.
- **Never let a confirmed problem just sit.** If you're adding the 4th playtest confirmation to an issue and it's still P2, something is wrong. Bump it. If it's already P1 and nothing's happening, bump it to P0 and add a comment explaining the urgency.

### 4a. Search for Existing Issues

```bash
gh issue list --state all --search "<keywords>"
```

Search broadly — the same problem may be described differently across playtests. The issue tracker is the source of truth for what's been reported before — you don't need to memorize past findings, just search effectively.

### 4b. If an Existing Open Issue Matches

**Add a playtest confirmation comment** and a thumbs-up reaction:

```bash
gh issue comment <number> --body "$(cat <<'EOF'
**Playtest confirmation** (seed XXXXX, <persona>, <date>):
<1-2 sentences describing how this playtest encountered the same problem>
EOF
)"

REPO=$(gh repo view --json nameWithOwner --jq '.nameWithOwner')
gh api "repos/$REPO/issues/<number>/reactions" -f content='+1'
```

**Check confirmation count and escalate aggressively:**

```bash
gh issue view <number> --json comments --jq '[.comments[].body | select(startswith("**Playtest confirmation**"))] | length'
```

Escalation thresholds:
- **2+ confirmations:** P2 → P1, P3 → P2
- **3+ confirmations OR game-breaking:** P2/P1 → P0. Yes, you CAN bump to P0. Do it.
- **Already P1 with 4+ confirmations and no fix in progress:** Bump to P0 and comment "This has been confirmed by N playtests with no fix. Escalating to P0."

```bash
gh issue edit <number> --remove-label "P1-high" --add-label "P0-critical"
gh issue comment <number> --body "Escalating to P0: confirmed by N+ independent playtests. This is actively harming the game experience."
```

### 4c. If a Closed Issue Matches and the Problem Persists

**First check the close reason.** Don't reopen issues closed as "not planned" — those were deliberately rejected. Only reopen issues that were closed as "completed" (i.e., someone thought they fixed it but the problem persists):

```bash
gh issue view <number> --json stateReason --jq '.stateReason'
```

**⚠️ Before reopening: verify on FRESH code.** Master changes constantly — other agents are merging features every few minutes. Your local code is stale the moment you check it out. If you think a completed issue's fix isn't working, do NOT just grep your local code. Either:
1. Check the actual PR that closed the issue (`gh pr view <number>`) to see what was implemented
2. Or `git fetch origin && git checkout origin/master` and verify there before claiming it's not fixed

**This has caused real damage before:** a playtest manager grepped a stale branch, didn't find the expected code, incorrectly reopened a completed issue, and created confusion. Don't repeat this mistake.

If `COMPLETED` and the problem genuinely persists on latest master, reopen with a confirmation comment:

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

**Root causes before symptoms.** Before filing, ask: is this finding a root cause or a symptom of something bigger? Don't file five separate issues for five manifestations of the same underlying problem. File one issue (or confirm the existing one) for the root cause and note the symptoms in the comment.

**BUT: break big root causes into actionable sub-issues.** If the root cause is "the economy doesn't create trade-offs," that's too vague for a developer to act on. File the root cause issue, then file 2-4 concrete sub-issues that each represent one specific fix a developer could implement in a single session. Link them to the parent.

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
