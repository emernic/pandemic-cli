#!/usr/bin/env python3
"""PreToolUse hook: blocks Write/Edit if the agent hasn't run git status yet.

Forces the Session Start Checklist (fetch, status, clean branch) to happen
before any code changes are made.
"""
import json
import sys


def has_run_git_status(entries):
    """Check if 'git status' has been run in any Bash tool use."""
    for entry in entries:
        if entry["type"] != "assistant":
            continue
        for item in entry["message"]["content"]:
            if item["type"] != "tool_use" or item["name"] != "Bash":
                continue
            cmd = item["input"].get("command", "")
            if "git status" in cmd:
                return True
    return False


def main():
    data = json.load(sys.stdin)
    tool_name = data.get("tool_name", "")

    if tool_name not in ("Write", "Edit"):
        return

    with open(data["transcript_path"]) as f:
        entries = [json.loads(line) for line in f if line.strip()]

    if not has_run_git_status(entries):
        print(json.dumps({
            "decision": "block",
            "reason": (
                "You haven't run `git status` yet. Complete the Session Start "
                "Checklist BEFORE making any changes: (1) git fetch origin, "
                "(2) git status, (3) create a fresh branch off origin/master "
                "if needed. See CLAUDE.md."
            ),
        }))


if __name__ == "__main__":
    main()
