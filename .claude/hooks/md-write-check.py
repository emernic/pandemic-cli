#!/usr/bin/env python3
"""PostToolUse hook: reminds agent to carefully review any .md file writes."""
import json
import subprocess
import sys


def is_git_tracked(file_path):
    """Check if a file is tracked by git (or staged for addition)."""
    try:
        result = subprocess.run(
            ["git", "ls-files", "--error-unmatch", file_path],
            capture_output=True, timeout=5,
        )
        return result.returncode == 0
    except Exception:
        return False


def main():
    data = json.load(sys.stdin)
    tool_name = data.get("tool_name", "")

    if tool_name not in ("Write", "Edit"):
        return

    tool_input = data.get("tool_input", {})
    file_path = tool_input.get("file_path", "")

    if not file_path.endswith(".md"):
        return

    if not is_git_tracked(file_path):
        return

    print(json.dumps({
        "decision": "block",
        "reason": (
            "**STOP** and read this carefully: You just wrote to a document "
            "that may be read thousands of times by hundreds of agents (and, "
            "unlike code, has **NO TESTS OR LINTING**). This means **YOU** must "
            "think very carefully and meticulously about exactly what you need "
            "to communicate here, the specific context in which this document "
            "will be read, and what the **MOST** concise, clear, and direct way "
            "of communicating your points is. Stop and re-read each line of what "
            "you wrote. Is it necessary? How would it change what future agents "
            "do? How could it be misinterpreted by agents? Does it actually "
            "communicate the intent (from the user or yourself)? Or does it "
            "flatten out and conflate the intent? Writing is very, very serious "
            "and should **NOT** be undertaken lightly. Stop and rewrite what you "
            "wrote until you're completely confident in it."
        ),
    }))


if __name__ == "__main__":
    main()
