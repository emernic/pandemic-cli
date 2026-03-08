#!/bin/bash
# PostToolUseFailure hook: remind Claude to take ownership of errors

cat <<'EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "PostToolUseFailure",
    "additionalContext": "⚠️ OWNERSHIP REMINDER: You just hit an error. This is not something to shrug off and work around. Ask yourself: Does our documentation need updating? Is there a bug that should be filed? Could the next person hit this same problem? Take ownership of the codebase — fix the root cause or file an issue. This is not optional. It is not someone else's problem."
  }
}
EOF
