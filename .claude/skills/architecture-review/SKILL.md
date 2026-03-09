---
name: architecture-review
description: Deep architecture review — find places where the architecture is performative rather than real
disable-model-invocation: false
---

# Architecture Review

This project may look well-architected at first glance. It has the right words, the right separations, and the right local structure. But AI-generated codebases often fail in a very specific way: each layer contains a half-truth that looks reasonable in isolation, and those half-truths stack on top of each other until the bottom layer is no longer doing what it appears to be doing.

At any single zoom level, the code may seem coherent. The modules fit together locally. The abstractions sound correct. The responsibilities appear separated. But when you keep the larger system in mind and trace the implications across layers, you often find that the architecture is subtly warped. It is not exactly wrong in one obvious place. It is wrong because many slightly misleading decisions reinforce each other.

Look specifically for cases where:

- the code appears to have good separation of concerns, but the separation is only cosmetic
- a clean abstraction is actually hiding a dirty hack
- a type, function, or subsystem claims to mean one thing, but in practice carries a different responsibility
- a local implementation makes sense only because some upstream assumption quietly distorts the meaning of the layer below it
- the code is technically organized, but the organization conceals conceptual dishonesty
- the system gives the impression of rigor while actually depending on improvised glue, semantic drift, or disguised special cases

The key question is not just "does this code make sense locally?" The key question is: "what does this part of the system seem to be doing, what is it actually doing, and which upstream architectural half-truths caused that gap to appear?"

Treat "looks clean but is secretly built on a hack" as a first-class failure mode. The goal is to identify places where the architecture is performative rather than real.
