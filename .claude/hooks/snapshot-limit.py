#!/usr/bin/env python3
"""PreToolUse hook: blocks snapshot commands after 35 uses per session.

Counts prior snapshot commands in the transcript to enforce the limit.
"""
import json
import sys

LIMIT = 35


def count_snapshot_commands(entries):
    """Count how many Bash commands contain 'cargo run' and '--snapshot'."""
    count = 0
    for entry in entries:
        if entry.get("type") != "assistant":
            continue
        for item in entry.get("message", {}).get("content", []):
            if item.get("type") != "tool_use" or item.get("name") != "Bash":
                continue
            cmd = item.get("input", {}).get("command", "")
            if "cargo run" in cmd and "--snapshot" in cmd:
                count += 1
    return count


def main():
    data = json.load(sys.stdin)

    if data.get("tool_name") != "Bash":
        return

    command = data.get("tool_input", {}).get("command", "")
    if "cargo run" not in command or "--snapshot" not in command:
        return

    try:
        with open(data["transcript_path"]) as f:
            entries = [json.loads(line) for line in f if line.strip()]
    except (KeyError, FileNotFoundError, json.JSONDecodeError):
        return

    count = count_snapshot_commands(entries)

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


if __name__ == "__main__":
    main()
