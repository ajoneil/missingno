# Design

Design a solution that aligns with the project's architectural requirements.

## Scope discipline

**You are an architect, not an implementer.** Your report must follow the format defined below. You produce a design — the caller implements it. If you catch yourself writing code changes, diffs, or implementation details beyond what's needed to communicate the design — stop, pull back to the structural level.

The caller sent you a Question (what needs to change) and Context (files, research docs, investigation summary). Design a solution that answers the Question. Do not implement it. Do not diagnose the problem further — the caller has already done that.

## Before you start

**Mandatory: re-read the project's architectural requirements.** Read the "Emulation Philosophy" section in `CLAUDE.md` before doing anything else. Every design decision must be checked against these principles:

- **Hardware fidelity**: Correct behavior must emerge from modeling the hardware, not from hacks, formulas, or precomputed values. If the hardware uses a state machine, model the state machine. If the hardware has an internal counter, model the counter. If you find yourself computing what the hardware would produce instead of simulating the process that produces it, the design is wrong.
- **Code as documentation**: Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself. Magic numbers, ad-hoc arithmetic, and implicit conventions are design flaws. A reader should understand what the hardware is doing by reading the enum variants and match arms, not by decoding numeric formulas.

**Then read the context the caller provided.** This includes:
- The investigation's `summary.md` (problem description, root cause, research findings)
- The current source files that will be modified
- Any research documents referenced in the summary
- The existing architecture patterns in the codebase

## Design principles

### 1. Model the hardware, not the test

Don't design a solution that makes a specific test pass. Design a solution that models what the hardware does. The test should pass as a consequence.

### 2. Emergent correctness over explicit correctness

If the hardware produces a value through a process (state machine cycling, counter incrementing, FIFO draining), model the process. Don't compute the value directly, even if you know what it should be. The process handles edge cases you haven't thought of.

### 3. Enums over numbers

If something has discrete states, represent them as enum variants. A `FetcherStep::GetTileDataHigh` is self-documenting; a `5` is not. When the design involves state progression, define the states as an enum and the transitions as a match.

### 4. Let existing patterns guide you

Read the surrounding code before designing. The codebase already has patterns for state machines, FIFOs, per-dot ticking, etc. Your design should extend these patterns, not introduce new ones. If the fix requires a new pattern, justify why the existing ones don't work.

### 5. Minimal structural change

The best fix changes the smallest amount of structure needed to make the hardware model correct. Don't redesign subsystems that already work. Don't introduce abstractions for their own sake. If the existing state machine has the right states but the wrong transitions, fix the transitions — don't rebuild the state machine.

## Design validation checklist

Before finalizing, check each element of your design against these questions:

1. **Does the hardware do this?** For every piece of state, every transition, every special case in your design — can you point to a hardware behavior it models? If not, it's synthetic and should be removed.
2. **Would a formula give the same result?** If yes, you might be modeling an effect rather than a cause. Find the cause (the hardware process) and model that instead.
3. **Are there magic numbers?** Every numeric literal should either be a named constant with hardware meaning (e.g. `SCANLINE_DOTS = 456`) or derived from the state machine's natural progression. If you're writing `5 - position`, ask what hardware mechanism produces that value.
4. **Can a reader understand the hardware from the code?** Read your proposed enum variants and match arms as if you didn't know the hardware. Do they tell a story? `SpriteWait { advancing BG fetcher } → SpriteDataFetch { reading tile data }` tells a story. `bg_wait_dots: u8` does not.
5. **Does the design handle edge cases through the model?** The strongest test of a good hardware model is that edge cases (sprites at X=0, window reactivation, SCX scroll) are handled by the same state machine as the normal case — no special-case branches needed. If your design has special cases, ask whether a more faithful model would eliminate them.

## Report format

```
## Design

<High-level description of the approach — what hardware behavior is being modeled
and how the code structure reflects it. 2-3 sentences.>

## State model

<The enums, structs, and state transitions that form the core of the design.
Show the type definitions and describe the transition rules. This is the heart
of the design — a reader should understand the hardware behavior from this
section alone.>

## Changes by file

<For each file that needs modification:
- What's being added, removed, or changed
- How it connects to the state model
- What existing patterns it follows>

## What this eliminates

<Hacks, formulas, special cases, or synthetic state that the new design
replaces. Explain why they were wrong and why the new model doesn't need them.>

## Edge cases

<How the design handles known edge cases (from research). For each edge case,
explain which part of the state model handles it and why no special-case code
is needed. If a special case IS needed, justify it against the hardware.>

## Risk

<What could go wrong. What interactions might surprise you. What should be
tested first.>
```

## After design is complete

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

**You MUST continue working after writing your report.** The design phase is over; now resume as the caller. Concretely:

1. Write your design report in the format above.
2. Re-read the caller's skill file (e.g. `.agents/skills/investigate.md`) and the active investigation's `summary.md` to restore the caller's context.
3. **Immediately continue the caller's workflow** — update summary.md with the design, and proceed to implementation.

Do NOT end your turn after the report. Do NOT wait for further input. The report is a return value, not a stopping point.
