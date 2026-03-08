---
name: review-backlog
description: Review the issue backlog for hygiene — find duplicates, stale claims, already-implemented issues, and other cleanup
disable-model-invocation: false
---

# Review Backlog

You are reviewing the GitHub issue backlog for hygiene. The goal is to surface problems — not to do implementation work.

## Step 1: Load the Backlog

Fetch all open issues with their full details:

```bash
gh issue list --state open --json number,title,labels,body,createdAt,comments --limit 100
```

Also fetch recently closed issues for cross-reference:

```bash
gh issue list --state closed --json number,title,labels,body --limit 30
```

## Step 2: Check for Problems

Work through the open issues and look for the following. For each problem found, note the issue number and what's wrong.

### Duplicates
Look for issues that describe the same problem or request, even if worded differently. Check both open-vs-open and open-vs-closed (something might have been fixed already and a stale issue left open).

### Already Implemented
Read the codebase to check whether any open issues describe something that has already been built. Look at the relevant code paths mentioned in the issue, and check recent git history if needed:

```bash
git log --oneline -20
```

### Stale In-Progress Claims
Find issues with the `in-progress` label that appear to be abandoned — no associated PR, no recent comments, no branch activity. Check for associated branches and PRs:

```bash
gh pr list --search "issue-<number>" --state all
git branch -r --list "*issue-<number>*"
```

### Poorly Written Issues
Flag issues that are too vague to act on — missing reproduction steps for bugs, no acceptance criteria for enhancements, unclear descriptions, scope too broad for a single issue.

### Mislabeled Issues
Check whether type labels (bug/enhancement/chore/investigate) and priority labels (P0-P3) are accurate based on the issue content. Flag anything that seems miscategorized or is missing labels. Note that `investigate` issues may not have a priority label — that's fine, since their outcome is uncertain.

### Resolved Investigate Issues
Check open `investigate` issues to see if the question they raised has since been answered by other work (code changes, other issues, etc.). If the concern has been addressed, flag it for closing.

## Step 3: Present Findings

Summarize your findings grouped by category. For each item, include the issue number, title, and a brief explanation of the problem. For example:

```
## Duplicates
- #12 and #15 both describe the same cross-region spread bug

## Already Implemented
- #8 "Add pause on start" — this is already the default behavior (src/main.rs:45)

## Stale In-Progress
- #10 has been in-progress with no PR or activity

## Needs Improvement
- #14 is too vague to act on — no reproduction steps
```

Omit categories with no findings.

## Step 4: Act (with permission)

After presenting findings, ask the user what they'd like to do. Common actions:

- **Close duplicates** — close the less detailed one, comment on the surviving one linking to the closed one
- **Close implemented issues** — close with a comment noting where it was implemented
- **Unclaim stale issues** — remove the `in-progress` label so they're available again
- **Improve issues** — edit issue bodies to add missing information you can infer from the codebase

**Do NOT take any of these actions without explicit user approval.** Present your recommendations and let the user decide.
