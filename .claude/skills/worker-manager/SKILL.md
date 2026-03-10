---
name: worker-manager
description: Manage the background worker pool — ensure the recurring cron loop is running and launch a new worker Claude process to pick up an issue.
disable-model-invocation: true
---

# Worker Manager

**You are a process manager. Nothing else.** Your only job is to check whether a worker is already running, launch one if not, and verify the loop is healthy. Do NOT touch any code, files, branches, or game state. Do NOT run `git` commands. Do NOT make any changes to the repository. Any action you take beyond process management risks contaminating the working tree for every worker.

Do these steps, in order.

## Step 1: Ensure the Recurring Loop Is Running

Use `CronList` to list all existing cron jobs.

- If a cron for `/worker-manager` **already exists** at any schedule, leave it alone. Do not change it.
- If **no** cron for `/worker-manager` exists, create one:

```
CronCreate(schedule="0,30 * * * *", command="/worker-manager")
```

That schedule fires at minute 0 and minute 30 of every hour (i.e., every 30 minutes on the clock).

Tell the user what you found and what you did (created a new cron, or found an existing one and left it).

## Step 2: Check If Previous Worker Is Still Running

Read `.worker-task-id` in the worktree root:

```bash
cat .worker-task-id 2>/dev/null || echo "none"
```

If the file contains a task ID (not "none"), call `TaskOutput` with `block=false` and that task ID to check its current status.

- **If status is `running`**: The previous worker is still active. **Skip spawning.** Tell the user the previous worker is still running (include the task ID). Stop here — do not proceed to Step 3.
- **If status is `completed`, `failed`, or the file doesn't exist**: Proceed to Step 3.

## Step 3: Launch a Worker Claude Process

Launch a new Claude process **in the background** — not a sub-agent. Use the `Bash` tool with `run_in_background: true` so it detaches and runs independently.

Two things are required for this to work:
- `unset CLAUDECODE` — claude refuses to start inside an existing Claude Code session without this
- `--dangerously-skip-permissions` — allows the worker to operate autonomously

This requires **two separate Bash tool calls**. Do not combine them into one command.

**⚠️ Why two calls?** The first call runs synchronously so you can read its output and know the exact log path. If you combine both into one command, you'll never see the path — it gets swallowed into the background process — and you'll have no way to tell the user where to find the logs. Multiple workers run concurrently; guessing or globbing won't work.

**Bash call 1** — synchronous, no `run_in_background`:
```bash
echo "/tmp/worker-$(date +%s)-$$.log"
```
Read the output. That string is your log path (e.g. `/tmp/worker-1773180000-12345.log`).

**Bash call 2** — with `run_in_background: true`, using the exact path from call 1:
```bash
unset CLAUDECODE; claude --dangerously-skip-permissions -p '/pick-up-issue' 2>&1 | tee /tmp/worker-1773180000-12345.log
```

After Bash call 2 completes, save the returned task ID to `.worker-task-id`:
```bash
echo "<task-id-from-bash-call-2>" > .worker-task-id
```

Tell the user the worker is running and give them the exact log path to tail.

## Step 4: Check Recent Worker Logs for Consistent Failures

After launching, scan the 3 most recent worker logs to see if the loop is healthy:

```bash
ls -t /tmp/worker-*.log 2>/dev/null | head -3
```

For each log, check the last 30 lines:
```bash
tail -30 /tmp/worker-<name>.log
```

Look for signs that workers are failing before doing useful work — e.g., giving up because the branch was in a bad state, failing to claim any issue, hitting permission errors, or abandoning immediately after starting.

**If 3 or more consecutive recent workers show this pattern:**
1. Cancel the cron: use `CronDelete` to remove the `/worker-manager` cron job.
2. Tell the user clearly: the loop has been stopped, summarize what the logs showed, and ask them to investigate before restarting.

If the logs look healthy (workers completing issues, or no issues available), leave the cron running and briefly report what you saw.
