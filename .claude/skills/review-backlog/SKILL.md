---
name: review-backlog
description: Review the issue backlog for hygiene — find duplicates, stale claims, already-implemented issues, and other cleanup
disable-model-invocation: false
---

# Review Backlog

Triage open issues. Close what's clearly stale or resolved. Leave everything else alone.

## What to Close

Only close issues that clearly fall into one of these categories:

- **Outdated**: References code, features, or systems that no longer exist
- **Already fixed**: The specific problem described has been resolved (verify in code)
- **Duplicate**: Another open issue covers the same thing (link to it)
- **Answered**: An investigate issue whose question has a clear, verifiable answer

When closing, add a comment explaining why. End with "feel free to reopen if you think this is still relevant."

## What NOT to Close

**When in doubt, don't close it.** Your instinct will be to close more than you should. Specifically:

- **Issues with user comments.** If the user weighed in, they care about it.
- **Balance/design issues.** "This is a design suggestion, not a bug" is NEVER a reason to close. Design is our job.
- **Issues about hacky workarounds that "work."** A workaround that bypasses the game's core model is still worth tracking even if the code runs fine. A code comment saying "intentional" or "deliberate tradeoff" doesn't make the decision good.
- **Borderline issues.** If you're debating, don't close it.

### Real Examples of Premature Closes

These were all suggested as closes during a triage session and overruled by the user:

1. **"This is a platform issue, not a game code issue."** An agent infrastructure issue (whether maxTurns config works) was dismissed as not our problem. The user said it was critical. Dismissing infrastructure issues as "not our code" poisons future agents into ignoring broken tooling.

2. **"Close — the choices seem reasonable"** on a crisis balance issue. The user disagreed and increased the personnel cost. Don't dismiss balance feedback from playtests just because the options look OK on first read.

3. **"Close — deliberate tradeoff, documented in a comment"** on a hacky SEIR bypass. The user saw it as a bad hack worth fixing and bumped to P2. A code comment justifying a hack doesn't make the hack acceptable.

4. **"This is a design suggestion, not a bug."** The user was emphatic: design IS our job. Never use this framing.

5. **"Close — just misleading comments."** The user said delete the comments entirely. Misleading comments actively confuse future readers — they're not harmless.

## Comments

Your comments will be read by future agents. Three rules:

1. **Match confidence to effort.** Say "seems outdated" or "I think this was fixed," not "I confirmed X." You glanced at it — say so.

2. **Don't inject opinions that could mislead future readers.** If you write "this is not a game code issue," future agents will ignore similar issues. If you write "the current design is intentional," future agents will defend a bad design. Stick to facts: "the code referenced no longer exists" or "this was addressed in PR #1234."

3. **Include what you found, hedged to effort.** Say what you looked at and what you think, but make clear how long you spent. Example: "Took a quick look (~30 seconds). It seems like X based on Y, but I haven't dug deeply into this." Future agents need your observations to pick up where you left off — but they also need to know how much weight to give them.

## Workflow

1. Fetch issues (adjust filters as needed):
   ```bash
   gh issue list --state open --json number,title,createdAt,labels,body,comments --limit 100
   ```
2. Sort by creation date (oldest first)
3. For each: read the body and comments, check the code it references, decide close / keep / upgrade
4. Also check for stale `in-progress` labels (no PR, no branch, no activity) and remove them
5. Present a summary to the user with links and one-line reasoning per issue
