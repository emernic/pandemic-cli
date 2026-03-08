---
name: reflect
description: Reflect on recent changes to identify overlooked issues, loose ends, and potential problems
disable-model-invocation: false
---

# Reflection

Please stop and take a step back.

The implementation may be "complete", but it's very possible that we've overlooked several subtle (but important) things.

**Look back** at your process so far and identify any:
- Problems you **stumbled across** but hacked around or glossed over
- Places where you had to **change course** because the original plan was flawed or tricky to implement
- Loose ends we may still need to wrap up
- Refactoring work that could be done ANYWHERE in the codebase that would allow your implementation to be cleaner and simpler
- Fundamental oversights either at the planning phase or the implementation phase. Do your changes actually do something useful?
- Any other nuances or problems that you haven't communicated already
- Specific comments from the **user** (or CLAUDE.md and skills) that you failed to fully incorporate into your work. Are there any details that were specifically mentioned that you didn't act on?

**NOTE: Focus on NEW aspects that you have not already considered and discussed. Actually READ your code and surrounding code and THINK about what we're overlooking.**

After coming up with your first draft list of concerns, do not present it to the user. Think more about each one and whether it represents a genuine concern that the user may not be aware of yet (or whether it's a silly objection that doesn't actually make sense in the context of what we're working on). Once you're confident in your thinking, you can present the final distilled list.

Be concise in your thinking and your response.

This whole process might take 15 seconds (if the changes are small) or 20 minutes (for extremely large complicated changes). The important thing is that you are precise in your thinking and thorough in your analysis without being redundant or sloppy.

**IMPORTANT: Focus exclusively on potential problems or things to followup on. Do not spend ANY time thinking about or summarizing "what's been implemented correctly" (this creates a runaway halo effect).**

Also, focus exclusively on changes you made in this Claude Code session since your last reflection (or start of conversation if you haven't reflected yet). That is to say, you should be looking at our conversation history here and what **you** actually did and your process. **DO NOT** use git to see all changes on the branch (that's not what's relevant here).

Make sure you ONLY return and display your final distilled/validated concerns, not all your intermediate thinking.

After reflecting, you may immediately address the concerns you identified if the fixes are straightforward. For anything non-trivial, confirm with the user first.

**Investigate issues:** If you notice anything that looks funky, incomplete, or unclear — even tangential to what you were working on — file an `investigate` issue. You don't need user permission; these are free. But remember: an investigate issue says "this confused me, someone should look" — it does NOT diagnose the problem or propose a fix. You haven't investigated it; you just noticed it in passing. See the create-issue skill for the template and the good/bad examples.
