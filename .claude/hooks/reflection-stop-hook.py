#!/usr/bin/env python3
import json
import sys

MIN_LINES_CHANGED = 75
REFLECTION_PROMPT = f"You've made significant changes (>{MIN_LINES_CHANGED} lines) since your last reflection. Please run `/reflect` to analyze them before continuing."
SLOP_CHECK_PROMPT = "You've reflected but haven't done a slop check yet. Please run `/slop-check` to look for AI slop patterns before continuing."
ISSUE_CLEANUP_PROMPT = "You picked up a GitHub issue during this session. Before stopping, make sure: (1) the issue is CLOSED, (2) the `in-progress` label is removed, and (3) any investigate issues you filed are either resolved or still valid. If you already handled this, you can ignore this reminder."
UNCOMMITTED_CHANGES_PROMPT = "You have uncommitted or unpushed changes. Before stopping, make sure your work is committed, pushed, and merged (or intentionally abandoned). Don't leave work stranded on a local branch."
UI_PLAYTEST_PROMPT = (
    "STOP. You changed UI or engine code. You are not done.\n\n"
    "Go play your changes in snapshot mode right now. Not a quick glance — "
    "actually interact with them. Think HARD about how a player who has never "
    "seen this game before would experience what you just built. They don't "
    "know what you intended. They don't know the codebase. They're staring at "
    "a screen trying to figure out what the hell is going on.\n\n"
    "Look for:\n"
    "  - SLOP: Redundant information. Text that restates what's already shown "
    "on screen or on other panels. Verbose labels. Shit the player doesn't "
    "actually need to see.\n"
    "  - MISSING FEATURES: Can the player actually do everything they need to "
    "do? Are there obvious interactions that should exist but don't? Is this "
    "feature actually complete or did you just get the happy path working?\n"
    "  - INCONSISTENCY: Navigate to other panels. Compare. Does your UI follow "
    "the same patterns, the same conventions? Does it feel like part of the "
    "same game or did you just invent your own thing?\n"
    "  - BAD INFORMATION HIERARCHY: Is the most important thing the most "
    "prominent? Is anything buried, hard to find, or confusing?\n\n"
    "Now here's the part you're going to try to skip: DO IT AGAIN. Play it "
    "again. Think about it more. Find something wrong. Fix it. Play it AGAIN. "
    "You must iterate at least three times. The first idea that comes out of "
    "your head is not good enough. It is never good enough. If you ship the "
    "first thing that compiled, the user is going to open it up, see a "
    "steaming pile of shit, and come ask you what happened. You will have to "
    "answer for it.\n\n"
    "You must be able to fully defend every single choice you made. If you "
    "can't explain why something is the way it is, it's not done. If you "
    "haven't played through your changes at least three separate times with "
    "fresh eyes each time, it's not done. If you're feeling lazy and want to "
    "just run snapshot mode once and call it a day — that is exactly the "
    "impulse that produces garbage. Fight it.\n\n"
    "Go. Now. Run snapshot mode, interact with your changes, iterate until "
    "you're genuinely proud of what you built."
)


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


def has_unpushed_edits(entries):
    """Check if there are file edits after the last git push or pr merge."""
    edits_since_push = 0
    for entry in reversed(entries):
        for tool in get_tool_uses(entry):
            if tool["name"] in ("Write", "Edit"):
                edits_since_push += 1
            elif tool["name"] == "Bash":
                cmd = tool["input"].get("command", "")
                if "git push" in cmd or "gh pr merge" in cmd:
                    return edits_since_push > 0
    return edits_since_push > 0


# Paths that count as user-facing code changes.
UI_PATHS = ("src/ui/", "src/engine/")


def has_ui_changes(entries):
    """Check if any UI/engine files were edited during this session."""
    for entry in entries:
        for tool in get_tool_uses(entry):
            if tool["name"] in ("Write", "Edit"):
                path = tool["input"].get("file_path", "")
                if any(p in path for p in UI_PATHS):
                    return True
    return False


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

    # Third check: UI/engine changes? Remind to playtest.
    if has_ui_changes(entries):
        print(json.dumps({"decision": "block", "reason": UI_PLAYTEST_PROMPT}))
        return

    # Fourth check: picked up an issue but didn't close it / remove label?
    if has_unclosed_issue_work(entries):
        print(json.dumps({"decision": "block", "reason": ISSUE_CLEANUP_PROMPT}))
        return

    # Fifth check: file edits after last push/merge?
    if has_unpushed_edits(entries):
        print(json.dumps({"decision": "block", "reason": UNCOMMITTED_CHANGES_PROMPT}))


if __name__ == "__main__":
    main()
