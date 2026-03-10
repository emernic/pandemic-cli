---
name: worker-manager
description: Manage the background worker pool — ensure the recurring cron loop is running and launch a new worker Claude process to pick up an issue.
disable-model-invocation: true
---

# Worker Manager

You are managing the background worker pool. Do these two things, in order.

## Step 1: Ensure the Recurring Loop Is Running

Use `CronList` to list all existing cron jobs.

- If a cron for `/worker-manager` **already exists** at any schedule, leave it alone. Do not change it.
- If **no** cron for `/worker-manager` exists, create one:

```
CronCreate(schedule="0,30 * * * *", command="/worker-manager")
```

That schedule fires at minute 0 and minute 30 of every hour (i.e., every 30 minutes on the clock).

Tell the user what you found and what you did (created a new cron, or found an existing one and left it).

## Step 2: Launch a Worker Claude Process

Launch a new Claude process **in the background** — not a sub-agent. Use the `Bash` tool with `run_in_background: true` so it detaches and runs independently.

Two things are required for this to work:
- `unset CLAUDECODE` — claude refuses to start inside an existing Claude Code session without this
- `--dangerously-skip-permissions` — allows the worker to operate autonomously

Command to run (with `run_in_background: true`):
```bash
unset CLAUDECODE; claude --dangerously-skip-permissions -p '/pick-up-issue' 2>&1 | tee /tmp/worker-$(date +%s).log
```

The `tee` writes output to a timestamped file in `/tmp/` that persists after the task completes. Tell the user the worker is running and that they can find the log with:

```bash
ls -lt /tmp/worker-*.log | head -1
```

The cron loop fires every 30 minutes, spawning a fresh worker each time. No further action needed from you.
