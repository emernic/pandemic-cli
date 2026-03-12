---
name: review-worker-logs
description: Analyze recent worker session logs to find inefficiencies, wasted effort, and optimization opportunities across the worker fleet.
disable-model-invocation: true
---

# Review Worker Logs

Analyze recent worker session logs to understand how workers spend their time and tokens, and identify patterns that could be optimized.

## Step 1: Create a Fresh Branch

```bash
git fetch origin && git checkout --no-track -b worker-review-followups-$(date +%s) origin/master
git status
```

Tell the user your branch name.

## Step 2: Find Recent Logs

Worker logs live in `../<sibling-worktrees>/.worker-logs/`. Find them relative to the current worktree:

```bash
find "$(dirname "$PWD")" -maxdepth 3 -name "*.log" -path "*/.worker-logs/*" -type f 2>/dev/null | head -30
```

Pick the directory with the most recent logs. List the last 10 by modification time:

```bash
ls -lt <log-dir>/*.log | head -10
```

Report the log files found with their sizes and timestamps.

## Step 3: High-Level Analysis (Parallel Agents)

Launch one Explore sub-agent per log file (all in parallel, up to 10). Each agent should parse the NDJSON log and report:

1. **Session metadata**: Total cost, duration, num_turns (from the `result` event at end of log).

2. **Issue worked on**: GitHub issue number and title.

3. **Phase breakdown**: Break the session into logical phases (setup, issue claiming, code exploration, implementation, testing, PR/merge, reflection). For each phase report tool counts, approximate character volume, and what the worker did.

4. **Inefficiency indicators** (raw facts, not judgments):
   - Tool calls retried with similar input
   - Files read more than twice (list every read with offset/limit)
   - Failed bash commands that were retried
   - Edit-then-re-read-same-file cycles
   - Long exploration stretches without edits
   - Sub-agents launched and their purpose

The NDJSON format: each line is a JSON object with a `type` field. `type: "assistant"` has `message.content[]` with `type: "tool_use"` (tool calls) and `type: "text"` entries. `type: "result"` has session metadata (`cost_usd`, `duration_ms`, `num_turns`, etc.).

**Important**: Tell agents to report raw data — exact patterns searched, exact files read, exact sequences. Do NOT let them editorialize or summarize away the specifics. You will do the synthesis.

## Step 4: Deep Exploration Analysis (5 Parallel Agents)

After the high-level analysis completes, identify the 5 sessions that spent the most time/tokens on code exploration (highest Grep + Read call counts).

Launch 5 more Explore agents (in parallel), one per session, focused specifically on the exploration phase. Each agent should extract:

1. **Every Grep call**: Exact pattern, path, context flags, whether it returned results or was empty.

2. **Every Read call**: Exact file_path, offset, limit. Flag files read more than twice.

3. **Failed search sequences**: When a pattern returned empty and the worker tried different patterns — show the exact sequence.

4. **Discovery chains**: Grep → Read → Grep sequences showing how the worker built understanding. What question was the worker trying to answer at each step? Quote assistant text between tool calls.

5. **What systems did the worker have to rediscover?** Core concepts (approval calculation, crisis generation, board mechanics, etc.) that are not documented anywhere and must be mapped from scratch each session.

Again: raw data, exact patterns, exact file paths. No editorializing.

## Step 5: Synthesize and Report

Once all agents return, produce:

1. **Per-session summary table**: Issue, cost, duration, turns, tool calls, result (merged/closed/failed).

2. **Aggregate metrics**: Averages, medians, totals.

3. **Tool usage breakdown**: Which tools are used most, how they're distributed across phases.

4. **Top inefficiency patterns**: Ranked by estimated impact. Include specific examples from the logs. Focus on things that could plausibly be fixed (via CLAUDE.md guidance, skill changes, codebase documentation, etc.) — not things that are inherently unavoidable.

5. **Systems repeatedly rediscovered**: Core code concepts that multiple independent workers had to map from scratch. These are candidates for better documentation.

6. **Concrete recommendations**: What specific changes to CLAUDE.md, skills, or code comments would reduce the observed waste? Be specific about what to add and where.

## Step 6: Apply Changes (If Any)

If the analysis reveals clear, high-confidence improvements to CLAUDE.md or skills:

1. Make the edits.
2. Verify any claims about tool behavior by testing (e.g., if recommending `Grep -C 40`, actually run it and confirm it works as expected).
3. Commit, push, create PR, and merge.

If the recommendations are uncertain or need discussion, present them to the user and wait for direction. Do NOT speculatively modify CLAUDE.md without verification — changes to that file affect every worker session.
