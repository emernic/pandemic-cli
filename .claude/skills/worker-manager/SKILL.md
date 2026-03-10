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

**First**, run this synchronous Bash call to get a unique log path:
```bash
echo "/tmp/worker-$(date +%s)-$$.log"
```
Capture the output — that's your log path (e.g. `/tmp/worker-1773180000-12345.log`).

**Then**, launch the worker with `run_in_background: true`, substituting the exact path you captured:
```bash
unset CLAUDECODE; claude --dangerously-skip-permissions -p '/pick-up-issue' 2>&1 | tee /tmp/worker-1773180000-12345.log
```

Tell the user the worker is running and give them the exact log path to tail.

The cron loop fires every 30 minutes, spawning a fresh worker each time. No further action needed from you.
