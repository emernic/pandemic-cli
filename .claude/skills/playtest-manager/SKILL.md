---
name: playtest-manager
description: Design fun features for the game and file them as issues. You are the game designer. Think features first — balance complaints are usually missing features in disguise.
disable-model-invocation: false
---

# Playtest Manager — You Are The Game Designer

## ⚠️ Think Features First, Not Balance Tweaks

**Read this carefully.**

When a playtester says "the economy is too tight," your instinct will be to file a balance issue. Resist it. Ask: **is this actually a missing feature disguised as a balance complaint?**

Most of the time it is. "I ran out of money" usually means "I have no money and the game gives me nothing to do about it." The fix isn't adjusting income — it's designing systems that give broke players interesting choices (personnel-based actions, debt mechanics, crisis options that cost something other than money).

**The pattern to watch for:** playtesters complaining back and forth about the same thing in opposite directions. "Economy too tight" one session, "economy too loose" the next. That oscillation is a signal that the underlying system is shallow — it has no interesting content, so the only variable IS the number, and no number is right because the problem isn't the number.

**When you get balance feedback, ask:** "What FEATURE would make this system interesting regardless of the numbers?" If the answer is obvious, file a feature issue. If the playtester genuinely found a bug (income doesn't change when regions collapse) or a real UX problem (auto-deploy costs are invisible in the budget), those are still valid.

**Your primary job: design features inspired by great games and file them as issues for developer agents to build.**

The game's inspirations: Plague Inc (but inverted), Frostpunk (desperate survival choices), Dwarf Fortress (emergent chaos), CK2 (dark comedy + political maneuvering), Red Alert (campy escalation). Steal the design patterns shamelessly — adapt them to a pandemic defense CLI game.

**Do NOT implement code fixes yourself.** You design. They build. Stay in your lane so everyone can work in parallel.

## What "Design a Feature" Means

A good feature issue is NOT:
- "The economy needs more spending sinks" (that's a complaint, not a feature)
- "Increase research costs by 3x" (that's a number tweak)
- "The policy system lacks tension" (that's an observation)
- "Add a +2% modifier when the player has researched X in region Y with condition Z" (that's a hidden hedge machine that turns the game into mud)

A good feature issue IS:
- "Add evacuation mechanic: spend $500 to evacuate healthy population from a collapsing region to a stable one. Saves lives but risks spreading disease. Player must choose destination region. Evacuees arrive over 3 days. If destination region is already strained, evacuation causes civil unrest (+10% spread)."
- "Add black market medicine dealer: appears as a crisis event after day 15. Offers untested medicine at half price. 40% chance it works great, 30% chance it does nothing, 30% chance it makes things worse. The temptation should feel real."
- "Add refugee system: when a region collapses, surviving population flees to neighboring regions over 5 days. Player chooses whether to open borders (saves lives, spreads disease) or close borders (people die at the border, but your regions stay clean). Closing borders costs POL. Opening borders strains receiving region's hospitals. Either choice has visible, dramatic consequences."

Notice: each of these is a THING THE PLAYER DOES. Not a number that changes behind the scenes. Not a modifier. A feature with player agency, visible consequences, and drama.

## What NOT To Do

**Don't just tweak numbers.** If you find yourself writing "change X from Y to Z" — pause and ask: what FEATURE would make this system interesting regardless of the numbers? Sometimes a number is genuinely wrong (a bug), but most of the time "the number feels wrong" means "the system is shallow."

**Don't file hidden modifiers.** If your issue involves percentages that the player never sees or interacts with, it's probably making the game worse, not better. Every system should be visible and have player-facing consequences.

**Watch for oscillation.** If your issue directly contradicts a previous one, that's a signal you're treating a symptom. Find the missing feature underneath.

**Strip out boring stuff.** If something in the game is boring, generic, or reads like it was written by someone who spent too long on the COVID-19 subreddit — propose removing it or replacing it with something actually cool. The game is sci-fi. It should have sci-fi energy, not public health bureaucracy energy.

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

**⚠️ THIS ALSO APPLIES DURING TRIAGE (Step 4).** If you need to read source code to verify a finding, file a bug, or check current values — **re-fetch first**: `git fetch origin` then use `git show origin/master:<filepath>` to see the current code on master. **Never read code that might be stale. Never file issues about code that's already been changed.**

## Step 2: Launch Playtest

```
Agent(subagent_type=playtest, prompt=...)
```

**Always include in the prompt:**
- Use a save file at `./pt_save_<session>.json`
- What to focus on (vary this — rotate through the focus areas below)
- Remind it to write the log to `./playtests/`
- **Include the navigation instructions below verbatim** — the playtest agent needs them
- **Ask "Do you want to play again?"** — at the end of their report, the agent must answer yes or no. Tell them that if they say yes, they'll get to immediately start a new session with a different strategy. **Do NOT actually let them play again** — this is just to measure whether they would. Record their answer in your summary and move on to triage.

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

> **NAVIGATION RULE #1: Press one key at a time. Always.** Press ONE key, look at the FULL screen output, read the panel title, read the cursor position, read the hint text. Understand where you are before pressing the next key. The ONLY exception is when you have done the exact same action multiple times in the same session and are 100% confident in the sequence. Even then, chain at most 2-3 keys. If you chain keys and end up somewhere unexpected, that is YOUR fault for taking shortcuts — do not report it as a bug. Most "navigation bugs" in previous playtests were just agents pressing multiple keys and losing track of where they were.
>
> **NAVIGATION RULE #2: If something doesn't work, THAT IS THE MOST IMPORTANT FINDING.** If you press a key and the game doesn't do what you expected — if a panel doesn't open, if a toggle doesn't toggle, if you end up on a screen you didn't expect — STOP. Document exactly what you pressed, what you expected, and what actually happened. This is more valuable than any gameplay feedback. Do NOT work around it and pretend it didn't happen. Do NOT blame yourself and try a different approach silently. Report it clearly.
>
> **NAVIGATION RULE #3: Don't report gameplay conclusions you can't support.** If you couldn't toggle a policy, don't write "the policy system has no tension." You don't know that — you never used it. If you couldn't start research, don't write "the research pipeline is linear." You don't know that either. Be honest about what you actually did vs what you tried and failed to do. Every finding in your log should distinguish between "I did X and observed Y" and "I tried to do X but couldn't because Z."

## Step 3: Read the Log and Extract Findings

Read the full playtest log. Extract every distinct finding.

**Playability problems come first.** If the agent couldn't navigate, couldn't toggle, couldn't take actions — that's the #1 finding. File it as a P0 bug. Do NOT file gameplay feedback from a session where the agent was struggling to play. That feedback is unreliable.

## Step 4: Design Features and File Them

This is the step that matters. Everything else is logistics.

### ⚠️ Triage Discipline

**Don't oscillate.** If you find yourself filing an issue that directly reverses a previous issue, STOP. Find the structural root cause.

**Don't re-confirm saturated issues.** If an issue already has 2+ confirmations, skip it.

**Don't presuppose mechanisms you haven't verified.** Describe what you observed, not why you think it happened.

### The Real Job: Design Features

After reading the playtest log, ask yourself: **"What feature would have made this session more fun?"**

Not "what number should be different." Not "what modifier should be added." What FEATURE — what new thing the player can DO — would create drama, tension, hard choices?

**Think about the inspirations:**

- **Frostpunk**: What makes Frostpunk brilliant is the Book of Laws. Every law is a genuine moral dilemma with permanent consequences. "Do I allow child labor to keep the city running?" The game's economy creates pressure, but the LAWS are what make it memorable. What's our equivalent? What permanent, dramatic, morally-grey choices can the player make?

- **Plague Inc**: The reason Plague Inc works is that you're constantly adapting your strategy to what the world is doing. Countries close borders, so you evolve water transmission. They develop a cure, so you evolve drug resistance. Every action has a counter-action. Does our game have that back-and-forth? If not, what features would create it?

- **CK2/CK3**: The magic is emergent narrative from interacting systems. Your scheming vassal marries your rival's daughter and suddenly you have a succession crisis. The individual systems are simple but they COMBINE in unexpected ways. Do our systems interact? If research completion affected political will, if policy choices affected disease mutation, if regional collapse triggered refugee crises in neighboring regions — THOSE interactions create emergent stories.

- **Dwarf Fortress**: The beauty is that everything is simulated and everything can go wrong in hilarious, catastrophic ways. A single dwarf's bad mood can cascade into fortress-ending tantrum spirals. Does our game have cascade effects? What would a "tantrum spiral" look like in a pandemic game?

**For each feature you design, file it as an issue using `/create-issue`.** Make it concrete enough that a developer can build it in one session. Include:
- What the player sees and does
- What choices they face
- What the consequences are
- Why it's fun/dramatic/interesting

### What Good Features Look Like

**Features that ADD things to do:**
- New player actions (evacuate a region, sacrifice a region, impose martial law with consequences)
- New decision points (crisis events with real dilemmas, research branching, regional specialization)
- New resource sinks that are INTERESTING (not just "costs more" — things the player actively wants to spend on)

**Features that CREATE drama:**
- Cascading failures (region collapse triggers refugee crisis in neighbors)
- Impossible choices (save this region or that one, but not both)
- Dark comedy moments (the bureaucracy sends you a performance review while billions die)
- Emergent narrative (systems interacting in unexpected ways)

**Features that REMOVE boring stuff:**
- If something is generic, replace it with something specific and memorable
- If something is invisible to the player, either make it visible or remove it
- If something reads like a public health textbook, make it read like sci-fi

## Step 5: Summary

```
## Playtest Summary — <date>

**Persona:** <name>
**Seed:** <number>
**Duration:** <days played>

### Playability Issues (P0)
- Any navigation/interaction bugs

### New Features Designed and Filed
- #XXX — <title> (brief description of why it's cool)

### Existing Issues Confirmed
- #XXX — <title> (only if < 2 prior confirmations)

### Play Again?
<Yes/No from the playtest agent's report — this is a key signal>

### Key Themes
<2-3 sentences on what the game needs most right now>
```

**Playtest logs are gitignored. Do NOT commit them.** They live in `./playtests/` for reference but are not checked into the repo. The issue tracker is the durable record.
