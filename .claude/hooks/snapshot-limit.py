#!/usr/bin/env python3
"""PreToolUse hook: blocks snapshot commands after 35 uses per session.

Uses a counter file (.snapshot_count) that tracks invocations. The file
auto-resets if it's more than 2 hours old (i.e., from a previous session).
"""
import json
import os
import sys
import time

LIMIT = 35
COUNTER_FILE = os.path.join(os.environ.get("CLAUDE_PROJECT_DIR", "."), ".snapshot_count")
MAX_AGE_SECONDS = 7200  # 2 hours


def read_count():
    """Read current count, resetting if the file is stale."""
    try:
        mtime = os.path.getmtime(COUNTER_FILE)
        if time.time() - mtime > MAX_AGE_SECONDS:
            return 0
        with open(COUNTER_FILE) as f:
            return int(f.read().strip())
    except (FileNotFoundError, ValueError):
        return 0


def write_count(count):
    with open(COUNTER_FILE, "w") as f:
        f.write(str(count))


def main():
    data = json.load(sys.stdin)

    if data.get("tool_name") != "Bash":
        return

    command = data.get("tool_input", {}).get("command", "")
    if "cargo run" not in command or "--snapshot" not in command:
        return

    count = read_count()

    if count >= LIMIT:
        print(json.dumps({
            "decision": "block",
            "reason": (
                f"SNAPSHOT LIMIT REACHED ({count}/{LIMIT}). You have used all "
                "your snapshot commands. You MUST write your playtest report "
                "NOW. Do NOT attempt any more snapshot commands — they will "
                "all be blocked. Write your findings to the playtests/ "
                "directory immediately."
            ),
        }))
    else:
        write_count(count + 1)


if __name__ == "__main__":
    main()
