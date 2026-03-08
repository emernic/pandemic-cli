---
name: pick-up-issue
description: Pick up a GitHub issue to work on — claims it, creates a branch, and guides you through completion
disable-model-invocation: false
---

# Pick Up Issue

You are picking up a GitHub issue to work on. This process ensures no two agents work on the same issue and that the issue is properly tracked through completion.

## Step 1: Select an Issue

**If a specific issue was provided** (via arguments or conversation), use that issue number and skip to Step 2.

**Otherwise**, find a good issue to work on. Start by listing available issues:

```bash
gh issue list --state open --search "-label:in-progress" --json number,title,labels,createdAt
```

When choosing which issue to recommend, consider:
- **Priority labels** — P0-critical and P1-high issues should generally be addressed before P2/P3.
- **Issue type** — bugs are usually more urgent than enhancements or chores, all else being equal.
- **Scope** — prefer issues that are well-defined and self-contained. Vague or sprawling issues are harder to complete successfully.
- **Dependencies** — if an issue clearly depends on another unfinished issue, skip it for now.
- **Age** — older issues that keep getting skipped may be worth a look, but don't pick them just because they're old.

Present a short summary of the top candidates (3-5 issues) with your reasoning, and recommend one. Ask the user which to pick up. If there are no available issues, tell the user.

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

1. **Start with no assumptions.** Read the code. Trace the logic. Understand the current behavior. The current behavior might be exactly correct — that's a fine outcome.
2. **Determine the outcome** (only after you actually understand what's going on):
   - **It's actually fine** (~30%) — close the issue with a comment explaining why the behavior is correct and makes sense. This is a perfectly valid and common outcome. Closing investigate issues is good hygiene, not a failure.
   - **It works but is confusing** (~30%) — add documentation, rename things, add a code comment, or restructure to make the behavior obvious. The code was right, but understandably confusing.
   - **It's actually a problem** (~40%) — fix the bug, clean up the design, or file a more specific bug/enhancement issue if the fix is too large for this pass.
3. Investigate issues are often small and self-contained. Many can be resolved quickly.

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
