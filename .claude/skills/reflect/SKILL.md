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

## Architecture Check

**This is mandatory. Do not skip it.**

Look at the code you touched and the code surrounding it. Ask yourself:

1. **Did you add UI state machine logic to engine.rs?** If so, that's a regression. The target architecture (see `docs/target-architecture.md`) says engine.rs should only contain game logic. Fix it or file an issue.
2. **Did you add engine imports to UI modules?** Same problem, same expectation.
3. **Did you walk past existing architecture violations without doing anything?** If you touched a file that has layering violations, you should have either fixed one or filed an issue. "I noticed it but it wasn't my task" is not acceptable.
4. **Is the code you wrote making the overall architecture simpler or more complex?** Good changes almost always make things simpler. If you added complexity, ask why.

## Ownership Check

**Also mandatory. Also do not skip it.**

Think back through your entire session:

1. **Did you notice anything that seemed off, confusing, or wrong — even tangentially?** If so, did you file an investigate issue? If not, do it now.
2. **Did you encounter any "known limitations," TODOs, or workarounds in the code?** Did you question whether they're actually acceptable, or did you just accept them?
3. **Did any commands from CLAUDE.md or skills not work as documented?** If so, fix the docs.
4. **Did you read code that confused you and just move on?** File an investigate issue pointing at it.

**If you have zero investigate issues to file after a non-trivial session, you almost certainly weren't paying enough attention.** Go back and look harder.

## Complexity Ratchet Check

**Also mandatory. This is how complexity silently kills projects.**

Think about everything you interacted with this session — not just the code you wrote, but the processes, tests, tools, and infrastructure you used:

1. **Did you spend time maintaining something that didn't catch any real problems?** Tests that always pass by "accept all," review steps that are always rubber-stamped, processes that feel like busywork. If a system's maintenance cost exceeds its value, file an investigate issue.
2. **Did you add complexity?** New files, new abstractions, new config, new test infrastructure. Was each addition truly necessary, or were you building for hypothetical future needs?
3. **Is there anything in the codebase that feels too big or too established to question?** That's exactly the thing you should question. The scarier it feels to remove, the more likely it's the complexity ratchet at work.
4. **Could something you interacted with be simplified or removed entirely?** Not refactored — *removed*. The best code is no code.

**The test:** If you spent more time maintaining a system than benefiting from it, something is wrong. File an investigate issue.

## Process

**NOTE: Focus on NEW aspects that you have not already considered and discussed. Actually READ your code and surrounding code and THINK about what we're overlooking.**

After coming up with your first draft list of concerns, do not present it to the user. Think more about each one and whether it represents a genuine concern that the user may not be aware of yet (or whether it's a silly objection that doesn't actually make sense in the context of what we're working on). Once you're confident in your thinking, you can present the final distilled list.

Be concise in your thinking and your response.

This whole process might take 15 seconds (if the changes are small) or 20 minutes (for extremely large complicated changes). The important thing is that you are precise in your thinking and thorough in your analysis without being redundant or sloppy.

**IMPORTANT: Focus exclusively on potential problems or things to followup on. Do not spend ANY time thinking about or summarizing "what's been implemented correctly" (this creates a runaway halo effect).**

Also, focus exclusively on changes you made in this Claude Code session since your last reflection (or start of conversation if you haven't reflected yet). That is to say, you should be looking at our conversation history here and what **you** actually did and your process. **DO NOT** use git to see all changes on the branch (that's not what's relevant here).

Make sure you ONLY return and display your final distilled/validated concerns, not all your intermediate thinking.

After reflecting, you MUST address the concerns you identified — don't just list them and stop. Fix straightforward issues immediately. For anything non-trivial, confirm with the user first. If the PR is already merged, open and merge a follow-up PR for the fixes.

**Investigate issues:** If you notice anything that looks funky, incomplete, or unclear — even tangential to what you were working on — file an `investigate` issue. You don't need user permission; these are free. But remember: an investigate issue says "this confused me, someone should look" — it does NOT diagnose the problem or propose a fix. You haven't investigated it; you just noticed it in passing. See the create-issue skill for the template and the good/bad examples.
