---
name: ui-playtest-check
description: Play your UI/engine changes in snapshot mode and iterate until they're solid
disable-model-invocation: false
---

# UI Playtest Check

You changed UI or engine code. You are not done.

Go play your changes in snapshot mode right now. Not a quick glance — actually interact with them. Think HARD about how a player who has never seen this game before would experience what you just built. They don't know what you intended. They don't know the codebase. They're staring at a screen trying to figure out what the hell is going on.

Look for:
  - SLOP: Redundant information. Text that restates what's already shown on screen or on other panels. Verbose labels. Shit the player doesn't actually need to see.
  - MISSING FEATURES: Can the player actually do everything they need to do? Are there obvious interactions that should exist but don't? Is this feature actually complete or did you just get the happy path working?
  - INCONSISTENCY: Navigate to other panels. Compare. Does your UI follow the same patterns, the same conventions? Does it feel like part of the same game or did you just invent your own thing?
  - BAD INFORMATION HIERARCHY: Is the most important thing the most prominent? Is anything buried, hard to find, or confusing?

Now here's the part you're going to try to skip: DO IT AGAIN. Play it again. Think about it more. Find something wrong. Fix it. Play it AGAIN. You must iterate at least three times. The first idea that comes out of your head is not good enough. It is never good enough. If you ship the first thing that compiled, the user is going to open it up, see a steaming pile of shit, and come ask you what happened. You will have to answer for it.

You must be able to fully defend every single choice you made. If you can't explain why something is the way it is, it's not done. If you haven't played through your changes at least three separate times with fresh eyes each time, it's not done. If you're feeling lazy and want to just run snapshot mode once and call it a day — that is exactly the impulse that produces garbage. Fight it.

Go. Now. Run snapshot mode, interact with your changes, iterate until you're genuinely proud of what you built.
