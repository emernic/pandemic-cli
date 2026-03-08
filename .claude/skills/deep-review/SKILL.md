---
name: deep-review
description: Deep, chunked PR review — breaks the PR into pieces, reviews each in parallel, and consolidates findings
disable-model-invocation: false
---

# Deep PR Review

**NOTE: THIS WHOLE FRAMEWORK HAS BEEN VERY CAREFULLY CRAFTED. PLEASE FOLLOW IT THROUGH TO THE END, AND PAY SPECIAL ATTENTION TO WHICH PARTS SHOULD BE PERFORMED IN A SUB-AGENT!**

## Step 1: Analysis & Chunking Sub-Agent

**Goal:** Determine the scope of the PR and break it down into logical review tasks.

1.  **Analyze Scope:** Use `git` tools to determine the total diff size (files modified, lines added/removed) relative to `main` and decide on a rough range of how many chunks to use for review.
      * *Guideline:* Small PRs (<50 lines) need ~2-3 chunks (1 slicing strategy). Massive PRs (12+ files, 2000+ lines) may want to use 2-4 slicing strategies resulting in 8-15 chunks.
2.  **Dispatch Sub-Agent:** Launch a sub-agent (important) to analyze the file structure and generate the chunks. It should save its output to a file named at `/tmp/pr-review-<branch-name>.md` (use whatever the current branch name is). Use the following prompt:

```text
The branch I'm on represents a significant PR from one of our engineers. Rather than attempt to review all of it at once, I want to break it down into manageable chunks so I can assign each chunk to one other engineer to review.

Can you poke around using git's various diff tools (I'd start with a list of files modified against main) and help me break down the PR into manageable chunks I can assign? I want each chunk to have very specific files, functions, etc. assigned, but I do **NOT** want you to be opinionated w.r.t. what the engineers should look for (e.g. types of mistakes). That's their job. Just help me chunk it up reasonably so I can assign pieces out.

You can use these commands to get the cleanest representation of the changes on this branch.
git merge-base HEAD origin/main
git --no-pager diff <sha from merge-base>

For this PR, I want you to use [N] slicing strategies, each dividing the PR into [A-B] chunks, for a total of [M] chunks.

NOTE: Your chunks can overlap! Especially for different slicing strategies.

Please save your results to `/tmp/pr-review-<branch-name>-chunking.md`

Again, **DO NOT** include any summaries of the changes or opinions on what's wrong in each chunk. Just help me slice/chunk things up and be specific with what files/functions/sections to review.

Some possible ways to slice things up (you can use more than one) are:
- By architectural layer (e.g. API vs domain vs repo vs model vs migrations, including the tests for that layer if relevant)
- By application code vs tests (e.g. for very simple PRs)
- By vertical feature area (e.g. everything related to molecules vs everything related to constructs)

You must save your output to a file at [/tmp/pr-review-<branch-name>.md].

One example output format is provided below:

# PR Breakdown: [branch-name] ([X] files, +[added]/-[removed])

[Brief description of what the PR does]

## Slicing 1: By Architecture Layer

### Chunk 1: Model Layer
| File | Lines Changed |
|------|--------------|
| `path/to/file.py` | +50 (new) |
| `path/to/other.py` | +37/-2 |

### Chunk 2: API Layer
| File | Lines Changed |
|------|--------------|
| `path/to/another.py` | +60 (new) |
| `path/to/yet_another.py` | +12/-2 |

## Slicing 2: By Feature

### Chunk 3: Construct Deduplication
| File | Lines Changed |
|------|--------------|
| `path/to/file.py` | +50 (new) |
| `path/to/somethingelse.py` | +20 (new) |
```

-----

## Step 2: Parallel Chunk Review Sub-Agents

**Goal:** Execute deep, critical reviews on each chunk simultaneously using parallel sub-agents.

1.  **Launch Parallel Agents:** Based on the chunks defined in Step 1, launch a parallel sub-agent for **each** chunk.
2.  **Assignment:** Ensure each sub-agent is assigned a unique output path (e.g., `/tmp/pr-review-<branch-name>-chunk-01.md`).
3.  **Prompting:** Use the refined template below for every sub-agent. This prompt has been **extremely** meticulously tuned, so please use it.

```text
You are a critical code reviewer. Your job is to find PROBLEMS ONLY - do NOT mention anything that looks correct. No praise, no "looks good". Only issues.

## Context
You are reviewing Chunk [N] of a large PR that introduces [brief feature description].
Total PR: [X] files, +[added]/-[removed] lines.

## Your Chunk: [Chunk Name]
Files to review:
- `path/to/file1.py` (+XX, new)
- `path/to/file2.py` (+XX/-YY changes)

## Your Task
1. Read through the changes in your chunk. You can use commands like `git merge-base HEAD origin/main` + `git --no-pager diff <sha from merge-base> -- some/file.py` to get an accurate representation of changes. Also, make sure you read enough of the existing related code (including files up or down the callstack from the ones that were changed) to understand the full context of the changes.
2. For each modified file, also find a couple pre-existing example files that do similar things (to understand established patterns).
3. **Think critically for yourself** about what's wrong with these changes.
   Do not treat any of this prompt as a checklist. You need to actually understand the code and identify places where it doesn't make sense, diverges from patterns without justification, or could cause real problems.

Examples of things that MIGHT be issues (but think beyond these):
- [Example relevant to this chunk type]
- [Another example]
- [Another example]

**Important: The bullets above are just examples. The most important issue might be something completely different that only becomes apparent when you actually read and understand the code.**

## Output
Save your findings to `/tmp/pr-review-<branch-name>-chunk-[NN].md` using the Write tool. Format:

# Chunk [N] Review: [Chunk Name]

## Critical Issues
- `file.py:line` - Description

## Warnings
- `file.py:line` - Description

## Minor Issues
- `file.py:line` - Description

Write "None found" for empty categories. After saving, confirm the file was written.
```

-----

## Step 3: Consolidation Sub-Agent

**Goal:** Merge all sub-agent reports into a single, de-duplicated, prioritized master report.

1.  **Wait for Completion:** Ensure all parallel sub-agents from Step 2 have finished writing their files.
2.  **Dispatch Consolidator:** Launch a final sub-agent to consolidate all the `/tmp/pr-review-<branch-name>-chunk-*.md` files into a final list.
3.  **Prompt:**

```text
I have run several parallel review agents on specific chunks of a large PR. They have saved their outputs to the file paths listed below.

Your task is to read ALL of these files and consolidate the issues into a single final list.

## Inputs
[List of /tmp/pr-review-<branch-name>-chunk-*.md files]

## Your Task
1. Read every review file.
2. Merge any duplicates (be careful not to remove similar-sounding issues that are actually distinct).
3. Group by severity (Critical, Warning, Minor).
4. If a category has no issues, omit it.

## Output
Save the final report to `/tmp/pr-review-<branch-name>-FINAL.md` using the simple format below:

# Final PR Review Report

## 🔴 Critical Issues
[file/path.py:line]
[Description]

[another/file/path.py:line]
[Description]

## 🟡 Warnings
[file/path.py:line]
[Description]

## 🔵 Minor Issues
[file/path.py:line]
[Description]

```

-----

## Step 4: Final Presentation

After consolidation is complete, provide the path to the consolidated report to the user.

Then **ask the user** (using the tool) which of the following they want you to do next:
1. Summarize the issues
2. Dig into the issues and identify which ones seem most valid and important to address
3. Go through issues one-by-one
      _Note: Start with the most critical, and for each issue provide the user with context on the issue, why it may or may not be worth addressing, and a brief plan for what **specifically** you would do to fix it._
      _Ask them whether to address or skip. At the end, make a todo list and address whichever ones they asked you to address. Make sure you ask about every issue before you start addressing things unless the user specifically requests you start with one immediately._
