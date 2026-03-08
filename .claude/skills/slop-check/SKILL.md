---
name: slop-check
description: Review changed code for AI slop patterns — poor separation of concerns, dead knobs, leaky abstractions, misleading names
disable-model-invocation: false
---

# Slop Check

You just reflected on your changes. Now do a second, more focused pass looking specifically for **AI slop patterns**. These are the subtle structural problems that AI-generated code tends to introduce:

- **Poor separation of concerns:** Logic that should live in one place is scattered across multiple, or unrelated things are tangled together.
- **Dead knobs:** Fields, parameters, config options, or branches that exist but don't actually do anything. Code that looks configurable but has only one possible value.
- **Leaky abstractions:** Multiple layers that look cleanly separated, but when you look closely, they duplicate logic, share implicit conventions, or rely on each other's internals. Two pieces of code that must agree on something but have no shared definition enforcing it.
- **Misleading names:** Structs, functions, or variables whose names no longer match what they actually do, especially after refactoring.
- **Circuitous logic:** Overly indirect code paths — going through 3 layers when 1 would do, wrapping and unwrapping the same data, clone-then-match patterns that exist purely to satisfy the borrow checker when a simpler restructuring would avoid the issue entirely.
- **Premature abstraction:** Generalized frameworks, trait hierarchies, or extension points built for hypothetical future use cases that don't exist yet and may never exist.
- **Cosmetic structure:** Code that's organized into modules/functions/traits to look well-structured, but the boundaries don't correspond to real conceptual boundaries — the structure is decorative rather than functional.

## Architecture Regression Check

**This is the most important part of the slop check for this project.**

This codebase has a known architectural problem: UI state machine logic lives in `engine.rs` instead of the UI layer, and some UI modules import from engine. See `docs/target-architecture.md` for the target state.

Check specifically:
1. **Did your changes add UI logic to engine.rs?** Panel navigation, wizard steps, selection bounds, open/close — none of this belongs in the engine. If you added any, extract it.
2. **Did your changes add engine imports to UI modules?** UI should read state, not call engine functions.
3. **Did you walk past existing violations?** If you touched a file with violations (engine.rs doing UI work, UI importing engine), you should have filed an issue or fixed one. If you didn't, do it now.
4. **Did you add "known limitation" comments or TODOs?** These are almost always slop. Either fix the thing or file an issue — don't leave a comment for a future Claude that will never read it.

## Process

**Actually READ the changed files and surrounding code.** Don't just think about it abstractly — open the files, read them line by line, and look for these patterns.

Focus exclusively on changes since your last `/reflect`. Present only genuine findings — not nitpicks, not style preferences, not things you've already discussed. If you find nothing substantive, say so.

After identifying issues, fix straightforward ones immediately. For anything that changes behavior or public interfaces, confirm with the user first.

**Investigate issues:** If you spot something that smells off but you're not sure whether it's actually a problem — file an `investigate` issue. You don't need permission; these are free. But keep it humble: describe what confused you and where to look, don't diagnose the problem or prescribe a fix. You noticed something in passing; you haven't actually investigated it. See the create-issue skill for the template and good/bad examples.
