#!/bin/bash
input=$(cat)
current_dir=$(echo "$input" | jq -r '.workspace.current_dir')
context=$(echo "$input" | jq -r '.context_window.used_percentage // 0')

branch_part=""
if git rev-parse --git-dir > /dev/null 2>&1; then
    branch=$(git branch --show-current 2>/dev/null)
    if [ -n "$branch" ]; then
        # Same color palette as VS Code window coloring
        colors=(
            "107;45;45"   # #6b2d2d
            "107;74;45"   # #6b4a2d
            "107;107;45"  # #6b6b2d
            "74;107;45"   # #4a6b2d
            "45;107;45"   # #2d6b2d
            "45;107;74"   # #2d6b4a
            "45;107;107"  # #2d6b6b
            "45;74;107"   # #2d4a6b
            "45;45;107"   # #2d2d6b
            "74;45;107"   # #4a2d6b
            "107;45;107"  # #6b2d6b
            "107;45;74"   # #6b2d4a
        )
        hash=$(echo -n "$branch" | md5sum | cut -c1-8)
        idx=$(( 16#$hash % 12 ))
        rgb="${colors[$idx]}"

        # Detect worktree: git-dir differs from git-common-dir in worktrees
        git_dir=$(git rev-parse --git-dir 2>/dev/null)
        git_common=$(git rev-parse --git-common-dir 2>/dev/null)
        suffix=""
        [ "$git_dir" != "$git_common" ] && suffix=" (worktree)"

        branch_part="  \033[38;2;${rgb}m${branch}${suffix}\033[0m"
    fi
fi

echo -e "${current_dir}${branch_part}  Context: ${context}%"
