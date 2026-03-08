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
2. **Issue type** — bugs before enhancements, all else being equal.
3. **Scope** — prefer small, well-defined, self-contained issues you can finish in one pass. Skip sprawling or vague issues.
4. **Dependencies** — skip issues that clearly depend on unfinished work.

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
   - **It's actually fine** (~30%) — close the issue with a comment explaining why the behavior is correct and makes sense. This is a perfectly valid and common outcome. Closing investigate issues is good hygiene, not a failure.
   - **It works but is confusing** (~30%) — add documentation, rename things, add a code comment, or restructure to make the behavior obvious. The code was right, but understandably confusing.
   - **It's actually a problem** (~40%) — fix the bug, clean up the design, or file a more specific bug/enhancement issue if the fix is too large for this pass. The fix might be for the specific thing the issue mentioned, or it might be a broader redesign of the area.

5. Investigate issues are often small and self-contained. Many can be resolved quickly — but don't use that as an excuse to skip the "step back and think" part.

## Playtest Feedback Issues

Issues labeled `playtest-feedback` were filed based on automated playtest sessions. Treat these with a grain of salt. Playtesters don't always accurately describe what they saw, and they definitely don't know what they want. An incorrect bug report or a silly feature request can still be pointing at a part of the game that genuinely needs work — but the *diagnosis* and *solution* in the issue are probably wrong. Read the issue to understand what area of the game triggered the feedback, then look at that area with fresh eyes and form your own opinion about what (if anything) needs to change.

## Step 4: Read and Understand the Issue

Read the full issue body:

```bash
gh issue view <number>
```

Make sure you understand:
- What the problem or request is
- What the acceptance criteria are (if provided)
- Which files/code are referenced

If the issue has a "Possible Solution" section, read it for context but do NOT follow it blindly — form your own approach.

## Step 5: Do the Work

Implement the fix or feature. Follow the project's conventions (see CLAUDE.md). Run tests with `cargo test` to make sure nothing is broken.

## Step 6: Completion

When the work is done:

1. **Commit** your changes (with user approval).
2. **Push** the branch and **create a PR** that references the issue:
   - Include `Closes #<number>` in the PR body so the issue auto-closes on merge.
3. **Small issues** (one-line fixes, label changes, typo corrections, etc.) — go ahead and merge the PR yourself without asking. **Larger issues** — ask the user to review and approve the merge first.

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

- For small issues, merge without asking. For anything non-trivial, get user permission first.
- NEVER abandon without user permission.
- If you get stuck or the issue is unclear, ask the user rather than guessing.
- If you realize the issue is a duplicate or invalid while working on it, tell the user before closing anything.
