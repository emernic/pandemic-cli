#!/usr/bin/env bash
# Wrapper for Stop hook: runs the reflection check AND sends a notification.
# stdin contains the hook JSON, which both scripts need.
DIR="$(cd "$(dirname "$0")" && pwd)"
INPUT=$(cat)

echo "$INPUT" | "$DIR/notify.sh" "Claude Code" > /dev/null 2>&1 &
echo "$INPUT" | python3 "$DIR/reflection-stop-hook.py"
