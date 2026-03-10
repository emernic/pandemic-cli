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

Launch a new Claude process **in the background** — not a sub-agent. Use the `Bash` tool with `run_in_background: true` so it detaches and runs independently. The process needs `CLAUDECODE` unset to avoid the nested-session guard, and `--dangerously-skip-permissions` to operate autonomously.

```
Bash(
  command="unset CLAUDECODE; claude --dangerously-skip-permissions -p '/pick-up-issue'",
  run_in_background=true
)
```

> **Why a real process and not a sub-agent?** Sub-agents run inside this session and block the main conversation. A background process runs independently — this session stays free while the worker does its work. The worker opens its own Claude Code session with full tool access, guided by the `/pick-up-issue` skill.

After launching, tell the user the worker is running.

## That's It

You're done. The cron loop ensures a fresh worker-manager call every 30 minutes, and the worker process is now independently picking up and completing a GitHub issue. No further action needed from you.
