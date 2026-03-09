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

## Game Design Ownership Check

**Also mandatory. This is a game. You are the game designer. There is nobody else.**

Every time you touch game logic, you are making design decisions — whether you realize it or not. "That's a design decision" is not a reason to defer. Designed by whom? There is no designer waiting in the wings. There is no game design team. If a mechanic is unfun, confusing, or broken, **you** need to fix it or file an issue. If you don't, nobody will. The next Claude session starts from scratch with zero memory of what you noticed.

Ask yourself:

1. **Did you encounter a game mechanic that doesn't make sense?** A cost with no counterplay, a choice that's always obvious, a system that creates no interesting decisions? Don't wave it away — file an issue or fix it.
2. **Did you fix a symptom without addressing the underlying design problem?** Clamping a value at 0 is a band-aid. If the real problem is "players have no way to manage this resource," say so. File the design issue.
3. **Would a real player enjoy the thing you just built or fixed?** Not "does it compile" or "do the tests pass" — would it be *fun*? Would it create interesting choices? If not, that's a problem you own.
4. **Did you dismiss something as "a design decision for later"?** There is no "later." There is no one coming after you to make that decision. If you noticed the gap, you are the one responsible for it. File the issue at minimum.

**The failure mode:** You fix a bug, notice the surrounding system is poorly designed, and think "well, that's beyond the scope of this issue." No it isn't. You saw it. You own it. The game being badly designed is YOUR problem, because you are the only one who will ever notice it in this session.

## Complexity Ratchet Check

**Also mandatory. This is how complexity silently kills projects.**

Think about everything you interacted with this session — not just the code you wrote, but the processes, tests, tools, and infrastructure you used:

1. **Did you spend time maintaining something that didn't catch any real problems?** Tests that always pass by "accept all," review steps that are always rubber-stamped, processes that feel like busywork. If a system's maintenance cost exceeds its value, file an investigate issue.
2. **Did you add complexity?** New files, new abstractions, new config, new test infrastructure. Was each addition truly necessary, or were you building for hypothetical future needs?
3. **Is there anything in the codebase that feels too big or too established to question?** That's exactly the thing you should question. The scarier it feels to remove, the more likely it's the complexity ratchet at work.
4. **Could something you interacted with be simplified or removed entirely?** Not refactored — *removed*. The best code is no code.

**The test:** If you spent more time maintaining a system than benefiting from it, something is wrong. File an investigate issue.

## Elephant in the Room Check

**This is the most important check. Do not skip it.**

The #1 failure mode in this project is the "silent consensus": you notice something fundamentally wrong, assume someone else must have already thought about it, and stay silent. Every other session does the same thing, for the same reason, and the problem persists for weeks.

Ask yourself these questions and answer them honestly:

1. **What am I hesitating to say?** What seems too big, too obvious, or too established to question? That's exactly the thing you need to say. The playtest system was broken for weeks because every session assumed "surely someone else has noticed this."

2. **What did I work around instead of fixing or flagging?** If you had to adjust your approach because something seemed off, that "something" needs an investigate issue — even if you're not sure it's actually broken.

3. **Am I producing real value, or performing a ritual?** Did your playtests actually test gameplay? Did your tests actually catch bugs? Did your review actually find problems? If you spent time on a process that produced no actionable findings, question whether the process itself is broken.

4. **If I told a smart colleague everything I did this session, what would they call out as suspicious or wasteful?** That's your elephant.

**The test for whether you've done this check properly:** If you have zero elephants to report, you're almost certainly not being honest with yourself. Go back and think harder. The cost of a false positive (filing an issue that gets closed) is near zero. The cost of staying silent is potentially weeks of wasted work.

## Document Meticulousness Check

**If you wrote or edited anything — documents, comments, descriptions, renamed variables — go back and re-read every word.** Is every single word as correct and clear as it can be? If not, fix it. Documents have no compiler. Wrong claims silently poison every future session.

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
