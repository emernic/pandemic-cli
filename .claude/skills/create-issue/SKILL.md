---
name: create-issue
description: Write up a well-structured GitHub issue based on something you experienced during this session
disable-model-invocation: false
---

# Create Issue

You are writing a GitHub issue based on something you encountered during this conversation — a bug, a missing feature, an idea, or a cleanup task. Follow this process carefully.

## Core Principles

1. **Describe what you actually experienced.** What were you doing? What happened? What was confusing, broken, or missing? Write from your real experience — do not fabricate or embellish.
2. **Do NOT prematurely plan the solution.** The issue describes a *problem* or *need*, not an implementation plan. Resist the urge to design the fix. The person working on this issue will figure out the right approach themselves.
3. **If you have a solution idea**, you may include it, but it MUST go in a clearly marked section: `## Possible Solution (do not use directly)`. This section should be treated as a rough sketch, not a spec. The implementer should feel free to ignore it entirely.
4. **Be self-contained.** The person reading this issue will NOT have access to your conversation. They need to understand the problem fully from the issue alone. Include relevant code paths, reproduction steps, and context.
5. **Keep scope narrow.** One issue = one problem or one feature. If you noticed multiple things, file multiple issues.

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

**Priority** (pick one label):
- `P0-critical` — Game-breaking, blocks core functionality
- `P1-high` — Important, should address soon
- `P2-medium` — Normal priority, address when convenient
- `P3-low` — Nice to have, backlog material

### Step 3: Draft the Issue

Think carefully about the title and body before writing. The title should be specific and scannable — someone skimming a list of 50 issues should immediately understand what this is about.

**For bugs**, use this structure. **IMPORTANT:** Read the version from `Cargo.toml` — do NOT guess or use a memorized value.
```markdown
## Description
[What's wrong, in plain language]

## Version
[Read from Cargo.toml — run `grep '^version' Cargo.toml` to get the current value]

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
- [ ] **Title is specific and scannable** (not vague like "Fix bug" or "Improve things")
- [ ] **Description is grounded in real experience**, not hypothetical
- [ ] **No premature solution design** leaked into the description (keep it in the optional section if anywhere)
- [ ] **Self-contained** — a reader with no other context can fully understand the problem
- [ ] **Scope is narrow** — one problem per issue
- [ ] **Code references are included** so the reader can orient themselves
- [ ] **Version is included** for bug reports
- [ ] **Acceptance criteria are included** for enhancements and chores

### Step 5: Create the Issue

Use `gh issue create` with appropriate labels:

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
