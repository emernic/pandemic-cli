#!/usr/bin/env python3
import json
import sys

MIN_LINES_CHANGED = 75
REFLECTION_PROMPT = f"You've made significant changes (>{MIN_LINES_CHANGED} lines) since your last reflection. Please run `/reflect` to analyze them before continuing."
SLOP_CHECK_PROMPT = "You've reflected but haven't done a slop check yet. Please run `/slop-check` to look for AI slop patterns before continuing."
ISSUE_CLEANUP_PROMPT = "You picked up a GitHub issue during this session. Before stopping, make sure: (1) the issue is CLOSED, (2) the `in-progress` label is removed, and (3) any investigate issues you filed are either resolved or still valid. If you already handled this, you can ignore this reminder."


def get_tool_uses(entry):
    """Extract tool_use items from an assistant entry."""
    if entry["type"] != "assistant":
        return []
    return [item for item in entry["message"]["content"] if item["type"] == "tool_use"]


def invoked_skill(entry, skill_name):
    """Check if an entry invoked a specific skill."""
    return any(
        tool["name"] == "Skill" and tool["input"].get("skill") == skill_name
        for tool in get_tool_uses(entry)
    )


def count_lines_changed(entry):
    """Count lines changed by Write/Edit tools in an entry."""
    lines = 0
    for tool in get_tool_uses(entry):
        tool_input = tool["input"]
        if tool["name"] == "Write":
            lines += tool_input["content"].count("\n") + 1
        elif tool["name"] == "Edit":
            old_lines = tool_input["old_string"].count("\n")
            new_lines = tool_input["new_string"].count("\n")
            lines += max(old_lines, new_lines) + 1
    return lines


def count_lines_since_last_reflect(entries):
    total = 0
    for entry in reversed(entries):
        if invoked_skill(entry, "reflect"):
            break
        total += count_lines_changed(entry)
    return total


def needs_slop_check(entries):
    """Check if reflect was invoked but no slop-check followed it."""
    for entry in reversed(entries):
        if invoked_skill(entry, "slop-check"):
            return False
        if invoked_skill(entry, "reflect"):
            return True
    return False


def has_unclosed_issue_work(entries):
    """Check if pick-up-issue was invoked but the in-progress label wasn't removed.

    The issue itself may auto-close via 'Closes #N' in the PR body, but
    removing the in-progress label is always a manual step that agents must do.
    """
    found_pickup = False
    saw_label_remove = False
    for entry in entries:
        if invoked_skill(entry, "pick-up-issue"):
            found_pickup = True
            saw_label_remove = False
            continue
        if not found_pickup:
            continue
        for tool in get_tool_uses(entry):
            if tool["name"] == "Bash":
                cmd = tool["input"].get("command", "")
                if "--remove-label" in cmd and "in-progress" in cmd:
                    saw_label_remove = True
    return found_pickup and not saw_label_remove


def main():
    data = json.load(sys.stdin)

    if data.get("stop_hook_active"):
        return

    with open(data["transcript_path"]) as f:
        entries = [json.loads(line) for line in f if line.strip()]

    lines_changed = count_lines_since_last_reflect(entries)

    # First check: need reflection?
    if lines_changed > MIN_LINES_CHANGED:
        print(json.dumps({"decision": "block", "reason": REFLECTION_PROMPT}))
        return

    # Second check: reflected but no slop-check yet?
    if needs_slop_check(entries):
        print(json.dumps({"decision": "block", "reason": SLOP_CHECK_PROMPT}))
        return

    # Third check: picked up an issue but didn't close it / remove label?
    if has_unclosed_issue_work(entries):
        print(json.dumps({"decision": "block", "reason": ISSUE_CLEANUP_PROMPT}))


if __name__ == "__main__":
    main()
