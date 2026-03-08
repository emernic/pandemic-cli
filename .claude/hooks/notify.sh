#!/usr/bin/env bash
# Sends notifications when Claude Code needs attention.
#
# 1. OSC 777 escape sequence — picked up by the "Terminal Notification" VS Code extension
#    (wenbopan.vscode-terminal-osc-notifier) for in-editor notifications.
# 2. Desktop notification via notify-send (WSL/Linux) or macOS fallbacks.
#
# Usage: echo '{"message":"..."}' | notify.sh [title]
#   or:  notify.sh [title] [message]

TITLE="${1:-Claude Code}"
MESSAGE="${2:-}"

if [ -z "$MESSAGE" ]; then
  INPUT=$(cat)
  MESSAGE=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('message','Finished'))" 2>/dev/null || echo "Finished")
  CWD=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('cwd',''))" 2>/dev/null || echo "")
fi

# VS Code in-editor notification (via Terminal Notification extension)
printf '\033]777;notify;%s;%s\007' "$TITLE" "$MESSAGE" > /dev/tty 2>/dev/null || true

# Desktop notification
if command -v notify-send &>/dev/null; then
  # WSL / Linux
  notify-send "$TITLE" "$MESSAGE" 2>/dev/null || true
elif command -v terminal-notifier &>/dev/null; then
  # macOS with terminal-notifier
  TN_ARGS=(-title "$TITLE" -message "$MESSAGE" -sound Glass -sender com.microsoft.VSCode)
  if [ -n "$CWD" ]; then
    TN_ARGS+=(-execute "/usr/local/bin/code '$CWD'")
  else
    TN_ARGS+=(-activate com.microsoft.VSCode)
  fi
  terminal-notifier "${TN_ARGS[@]}" 2>/dev/null || true
else
  # macOS fallback
  osascript -e "display notification \"$MESSAGE\" with title \"$TITLE\" sound name \"Glass\"" 2>/dev/null || true
fi
