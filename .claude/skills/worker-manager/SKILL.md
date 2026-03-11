---
name: worker-manager
description: Manage the background worker — ensure the recurring cron loop is running and launch a new worker Claude process to pick up an issue.
disable-model-invocation: true
---

# Worker Manager

**You are a process manager. Nothing else.** Your only job is to check whether a worker is already running, launch one if not, and verify the loop is healthy. Do NOT touch any game code or game files. The only files you may write are process-management state files (`.worker-task-id`, log files). Any action beyond process management risks contaminating the working tree for every worker.

Do these steps, in order.

## Step 1: Unclaim Abandoned In-Progress Issues

Before anything else, clean up issues that were claimed but never completed by a previous worker.

1. List all open issues with the `in-progress` label:

```bash
gh issue list --state open --label "in-progress" --json number,title --limit 50
```

2. For each issue returned, find when the `in-progress` label was most recently added by checking the issue timeline:

```bash
gh api repos/$(gh repo view --json nameWithOwner -q .nameWithOwner)/issues/NUMBER/timeline \
  --paginate \
  --jq '[.[] | select(.event == "labeled" and .label.name == "in-progress")] | last | .created_at'
```

3. Compare that timestamp to the current time. If the label was added **more than 2 hours ago**, the worker that claimed it is presumed dead — unclaim it:

```bash
# Example time comparison (Linux date):
NOW=$(date -u +%s)
LABELED_AT=$(date -u -d "TIMESTAMP_FROM_API" +%s)
AGE=$((NOW - LABELED_AT))
# If AGE > 7200 (2 hours), unclaim:
gh issue edit NUMBER --remove-label "in-progress"
gh issue comment NUMBER --body "This issue appears to have been abandoned by a worker (in-progress label was set more than 2 hours ago with no completion). Removing the label so another worker can pick it up."
```

Skip any issue where the label was applied within the last 2 hours — the worker may still be actively working on it.

If there are no open `in-progress` issues, skip this step silently and proceed.

## Step 2: Check If Previous Worker Is Still Running

Read `.worker-task-id` in the worktree root:

```bash
cat .worker-task-id 2>/dev/null || echo "none"
```

If the file contains a task ID (not "none"), call `TaskOutput` with `block=false` and that task ID to check its current status.

- **If status is `running`**: The previous worker is still active. **Stop here immediately.** Tell the user the previous worker is still running (include the task ID). Do NOT create a branch, do NOT touch the cron, do NOT spawn anything.
- **If status is `completed`, `failed`, or `TaskOutput` returns an error (task ID no longer tracked)**: Proceed to Step 3.
- **If the file doesn't exist**: Proceed to Step 3.

## Step 3: Create a Fresh Branch for the Worker

Master changes frequently. Checking out the latest `origin/master` ensures the worker launches with up-to-date skill files (including `/pick-up-issue`). The worker will create its own issue branch from here.

```bash
git fetch origin && git checkout --no-track -b worker-$(date +%s) origin/master
```

Tell the user the branch name that was created.

## Step 4: Ensure the Recurring Loop Is Running

Use `CronList` to list all existing cron jobs.

- If a cron for `/worker-manager` **already exists** at any schedule, leave it alone. Do not change it.
- If **no** cron for `/worker-manager` exists, create one:

```
CronCreate(schedule="0,30 * * * *", command="/worker-manager")
```

That schedule fires at minute 0 and minute 30 of every hour (i.e., every 30 minutes on the clock).

Tell the user what you found and what you did (created a new cron, or found an existing one and left it).

## Step 5: Launch a Worker Claude Process

Launch a new Claude process **in the background** — not a sub-agent. Use the `Bash` tool with `run_in_background: true` so it detaches and runs independently.

Two things are required for this to work:
- `unset CLAUDECODE` — claude refuses to start inside an existing Claude Code session without this
- `--dangerously-skip-permissions` — allows the worker to operate autonomously

This requires **two separate Bash tool calls**. Do not combine them into one command.

**⚠️ Why two calls?** The first call runs synchronously so you can read its output and know the exact log path. If you combine both into one command, you'll never see the path — it gets swallowed into the background process — and you'll have no way to tell the user where to find the logs. Multiple past worker logs may exist; guessing or globbing the path won't work.

**Bash call 1** — synchronous, no `run_in_background`:
```bash
mkdir -p .worker-logs && echo ".worker-logs/worker-$(date +%s)-$$.log"
```
Read the output. That string is your log path (e.g. `.worker-logs/worker-1773180000-12345.log`).

**Bash call 2** — with `run_in_background: true`, using the exact path from call 1:
```bash
unset CLAUDECODE; claude --dangerously-skip-permissions -p '/pick-up-issue' --output-format stream-json --verbose 2>&1 | tee .worker-logs/worker-1773180000-12345.log
```

`--output-format stream-json --verbose` makes Claude emit each event (tool calls, assistant messages, tool results) as a newline-delimited JSON object in real time. `--verbose` is required — `stream-json` without it errors out silently. Without these flags, `-p` only outputs the final response — a one-paragraph summary at the very end of a 20-minute session.

After Bash call 2 completes, save the returned task ID to `.worker-task-id`:
```bash
echo "<task-id-from-bash-call-2>" > .worker-task-id
```

Tell the user the worker is running and give them the exact log path to tail.

## Step 6: Check Recent Worker Logs for Consistent Failures

After launching, scan the 3 most recent **previous** worker logs (skip the one just launched — it's empty) to see if the loop is healthy:

```bash
ls -t .worker-logs/worker-*.log 2>/dev/null | tail -n +2 | head -3
```

For each log, parse the NDJSON to extract a structured event summary:
```bash
python3 - <<'EOF' .worker-logs/worker-<name>.log
import sys, json
events = []
for line in open(sys.argv[1]):
    line = line.strip()
    if not line: continue
    try: obj = json.loads(line)
    except: continue
    t = obj.get('type')
    if t == 'assistant':
        for c in obj.get('message', {}).get('content', []):
            if c.get('type') == 'text' and c['text'].strip():
                events.append(('[assistant]', c['text'].strip()[:120]))
            elif c.get('type') == 'tool_use':
                inp = c.get('input', {})
                desc = (inp.get('command') or inp.get('description') or
                        inp.get('file_path') or inp.get('pattern') or
                        inp.get('prompt', ''))
                events.append(('[tool_use]', f"{c['name']}: {str(desc)[:100]}"))
    elif t == 'result':
        events.append(('[result]', obj.get('result', '')[:300]))
N = 6
shown = (events[:N] + [('...', f'({len(events)-2*N} events skipped)')] + events[-N:]
         if len(events) > 2*N else events)
for tag, msg in shown: print(f'{tag} {msg}')
EOF
```

Look for signs that workers are failing before doing useful work: crashing immediately, abandoning early, or ending without completing anything.

**If 3 or more consecutive recent workers show this pattern:**
1. Cancel the cron: use `CronDelete` to remove the `/worker-manager` cron job.
2. Tell the user clearly: the loop has been stopped, summarize what the logs showed, and ask them to investigate before restarting.

If the logs look healthy (workers completing issues, or no issues available), leave the cron running and briefly report what you saw.
