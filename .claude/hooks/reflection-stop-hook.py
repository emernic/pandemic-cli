#!/usr/bin/env python3
import json
import sys

MIN_LINES_CHANGED = 75
REFLECTION_PROMPT = f"You've made significant changes (>{MIN_LINES_CHANGED} lines) since your last reflection. Please run `/reflect` to analyze them before continuing."


def get_tool_uses(entry):
    """Extract tool_use items from an assistant entry."""
    if entry["type"] != "assistant":
        return []
    return [item for item in entry["message"]["content"] if item["type"] == "tool_use"]


def invoked_reflect(entry):
    """Check if an entry invoked the reflect skill."""
    return any(
        tool["name"] == "Skill" and tool["input"].get("skill") == "reflect"
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
        if invoked_reflect(entry):
            break
        total += count_lines_changed(entry)
    return total


def main():
    data = json.load(sys.stdin)

    if data.get("stop_hook_active"):
        return

    with open(data["transcript_path"]) as f:
        entries = [json.loads(line) for line in f if line.strip()]

    if count_lines_since_last_reflect(entries) > MIN_LINES_CHANGED:
        print(json.dumps({"decision": "block", "reason": REFLECTION_PROMPT}))


if __name__ == "__main__":
    main()
