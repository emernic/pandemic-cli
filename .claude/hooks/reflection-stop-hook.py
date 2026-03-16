#!/usr/bin/env python3
import json
import sys

MIN_LINES_CHANGED = 75
REFLECTION_PROMPT = f"You've made significant changes (>{MIN_LINES_CHANGED} lines) since your last reflection. Please run `/reflect` to analyze them before continuing."
SLOP_CHECK_PROMPT = "You've reflected but haven't done a slop check yet. Please run `/slop-check` to look for AI slop patterns before continuing."
ISSUE_CLEANUP_PROMPT = "You picked up a GitHub issue during this session. Before stopping, make sure: (1) the issue is CLOSED, (2) the `in-progress` label is removed, and (3) any investigate issues you filed are either resolved or still valid. If you already handled this, you can ignore this reminder."
UNCOMMITTED_CHANGES_PROMPT = "You have uncommitted or unpushed changes. Before stopping, make sure your work is committed, pushed, and merged (or intentionally abandoned). Don't leave work stranded on a local branch."
UI_PLAYTEST_PROMPT = (
    "⚠️ UI CHANGES DETECTED — YOU HAVE NOT PLAYTESTED THEM.\n\n"
    "You changed user-facing code (src/ui/ or src/engine/) but you never ran "
    "the game in snapshot mode after your last edits. You are about to ship "
    "something you have never actually looked at.\n\n"
    "This is not optional. This is not a suggestion. Go run the game RIGHT NOW:\n\n"
    "  cargo run -- --snapshot [--days N] [--key ...]\n\n"
    "Open the relevant panel or feature. Look at it. Pretend you are a player "
    "who has never seen this game before and is seeing your feature for the "
    "first time. Ask yourself:\n\n"
    "  - Does this actually make sense? Is it obvious what to do?\n"
    "  - Does it look right? Is anything misaligned, garbled, or ugly?\n"
    "  - Does it WORK? Did you try the actual interaction, not just the render?\n"
    "  - Would you be happy handing this to a real person? Or would they come "
    "back holding a steaming pile of shit?\n\n"
    "If something is wrong, FIX IT before shipping. Do not rationalize. Do not "
    "say 'it looks fine to me' — your visual system does not work like a human's. "
    "If something looks even slightly off, it IS off. The previous agent who "
    "built the reactor system never playtested it, and the user got a completely "
    "broken, unusable feature. Do not repeat that mistake.\n\n"
    "Run snapshot mode, look at your changes, and verify they actually work."
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


def has_ui_changes_without_playtest(entries):
    """Check if UI/engine files were edited but snapshot mode was never run after.

    Walks backward through entries. If we hit a snapshot run, we're fine — the
    agent looked at the game after their changes. If we hit UI file edits
    without having seen a snapshot run first, they never checked their work.
    """
    found_ui_edit = False
    for entry in reversed(entries):
        for tool in get_tool_uses(entry):
            if tool["name"] == "Bash":
                cmd = tool["input"].get("command", "")
                if "cargo run" in cmd and "--snapshot" in cmd:
                    # They ran the game — everything before this is fine
                    return False
            elif tool["name"] in ("Write", "Edit"):
                path = tool["input"].get("file_path", "")
                if any(p in path for p in UI_PATHS):
                    found_ui_edit = True
    return found_ui_edit


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

    # Third check: UI/engine changes without running the game?
    if has_ui_changes_without_playtest(entries):
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
