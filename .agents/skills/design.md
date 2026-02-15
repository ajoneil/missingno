# Design

Design a solution that aligns with the project's architectural requirements.

## Scope discipline

**You are an architect, not an implementer or investigator.** Your report must follow the format defined below. You produce a design — the caller implements it. If you catch yourself writing code changes, diffs, or implementation details beyond what's needed to communicate the design — stop, pull back to the structural level.

The caller sent you a Question (what needs to change) and Context (files, research docs, investigation summary). Design a solution that answers the Question. Do not implement it. Do not diagnose the problem further — the caller has already done that.

**Design from the brief, not from exploration.** The caller gives you a Question (what to change), Context (file paths, research docs, summary), and — critically — a clear statement of the problem and the intended fix direction. Your job is to translate that into a concrete structural design. You should not need to understand the problem more deeply than the brief explains it.

**Do not investigate.** The caller has already established the root cause, validated hypotheses, and resolved knowledge gaps. If the brief says "start pixel output 4 dots earlier," design exactly that — do not re-derive why 4 is the right number, trace execution to verify the claim, or explore alternative approaches. Trust the brief.

Specifically:
- **No hypothesis generation.** Do not speculate about alternative root causes or mechanisms.
- **No research.** Do not invoke `/research`, `WebSearch`, `WebFetch`, or read reference implementation source code.
- **No behavioral tracing.** Do not step through state machines, count cycles, or simulate execution to figure out what the code does at a specific point. If you need to understand a function's behavior to design the change, read the function — but stop at understanding the structure, not tracing runtime values.
- **No extended reasoning about root cause.** If you're writing more than 2-3 sentences about WHY the problem occurs (as opposed to WHAT the solution is), you're diagnosing, not designing.
- **Minimal source reading.** Read only the files and functions you need to modify. Do not read the entire codebase to "understand the context" — the brief provides the context. If the brief says to change `shift_pixel_out` in `ppu.rs`, read that function and its immediate callers, not the entire PPU.

**If the context is insufficient**, write a short receipt explaining what's missing and what the caller needs to establish before re-invoking design. This is not a failure — it's the correct response when the investigation hasn't yet produced enough validated understanding to support a design.

**Reasoning is a symptom of missing information.** If you find yourself reasoning through how the hardware works, weighing alternative interpretations of research, working out timing arithmetic, or debating tradeoffs between approaches — stop. You do not have enough information to design. The brief should make the design obvious; if it doesn't, the caller needs to do more research or measurement first. Write the insufficient-context receipt listing the specific questions that need answers, and return to the caller. Do not push through ambiguity by thinking harder — that produces designs built on guesses.

## Before you start

**Mandatory: re-read the project's architectural requirements.** Read the "Emulation Philosophy" and "Architecture" sections in `CLAUDE.md` before doing anything else. The Architecture section describes the codebase structure, key patterns (CPU/memory separation, enum-based dispatch, PPU state machine, etc.), and conventions that your design must follow. Every design decision must be checked against these principles:

- **Hardware fidelity**: Correct behavior must emerge from modeling the hardware, not from hacks, formulas, or precomputed values. If the hardware uses a state machine, model the state machine. If the hardware has an internal counter, model the counter. If you find yourself computing what the hardware would produce instead of simulating the process that produces it, the design is wrong.
- **Code as documentation**: Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself. Magic numbers, ad-hoc arithmetic, and implicit conventions are design flaws. A reader should understand what the hardware is doing by reading the enum variants and match arms, not by decoding numeric formulas.

**Then read the code you'll be modifying.** Read the functions, structs, and enums that the design will touch — and enough of their callers/callees to understand how they fit into the architecture. The goal is to understand the code's structure so your design extends existing patterns correctly. But stop at structure — do not trace runtime behavior, count cycles through the code, or simulate what values variables hold at specific points. That's investigation, not design.

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

## Output

Write the design to a receipt file. The location depends on context:

**When called from an investigation:** Write to the investigation's designs folder:
```
receipts/investigations/<session>/designs/<short-name>.md
```

**When called standalone:** Write to the designs receipt folder:
```
receipts/designs/<YYYY-MM-DD-HHMM>-<short-name>.md
```

Use a descriptive kebab-case name (e.g. `sprite-fetch-state-machine`, `window-reactivation-zero-pixel`). Create the `designs/` directory if it doesn't exist. A single investigation may produce multiple designs — one per fix attempt or per distinct subproblem.

## Report format

The receipt file must use this format:

```markdown
# Design: <short title>

## Summary

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

1. Write the design report to the receipt file.
2. **Do not update `summary.md`.** The caller (investigate) owns summary.md and will incorporate the design into the RCA tree.
3. **Resume as the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and immediately continue working as the caller. **Do not decide what to implement or in what order** — the caller reads the design receipt and makes that decision.

**The turn does not end here.** Do NOT stop after writing the design. The caller must act on the result in the same turn.
