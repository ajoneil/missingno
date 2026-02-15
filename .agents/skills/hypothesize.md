# Hypothesize

Generate testable hypotheses about **hardware behavior** and **data model divergence** based on the current understanding of the problem and what's been tried so far.

## Scope discipline

**You are a hypothesis generator, not an investigator or interpreter.** You receive the current mental model and investigation history. Your job is to produce a ranked list of testable hypotheses — specific, falsifiable predictions about what the hardware does and how the emulator's data model differs.

**Hypotheses are about hardware and models, not about code changes.** A hypothesis must describe what the real hardware does (a claim about the silicon) and where the emulator's data model diverges. "Add 4 idle dots at Mode 3 start" is not a hypothesis — it's a proposed fix. "The hardware's pixel FIFO begins outputting 4 dots into Mode 3 because the first tile fetch overlaps with pipeline priming, but our model sequences them as two non-overlapping 6-dot phases" is a hypothesis.

You do NOT gather data, interpret measurements, design fixes, or make code changes. You read the current state and propose what to test next.

## Inputs

The caller provides:

- **Summary**: Path to the investigation's `summary.md`. Read it — especially the **Root cause analysis** tree and **Current understanding** sections. Do not work from conversation memory.
- **Context**: Any additional context the caller provides (e.g. specific subsystem, a new clue, a constraint on what's worth testing).

## Process

1. **Re-read summary.md.** The Current understanding, Hardware model, and Model divergence sections are your starting point. The Root cause analysis tree shows what's already been tried and ruled out.
2. **Classify the gap.** Before generating hypotheses, identify what type of gap exists between the hardware and the emulator's model. The gap is one or more of:
   - **Missing state**: Hardware has state our model doesn't track (e.g., a pipeline stage, a latch, a counter we don't model)
   - **Wrong transitions**: Our state machine has the right states but transitions at the wrong time or under the wrong conditions
   - **Wrong structure**: Our model decomposes the hardware differently (e.g., two sequential phases where hardware has one overlapping phase, or a single counter where hardware has two independent ones)
   - **Missing interaction**: Two subsystems interact on hardware but are independent in our model (e.g., fetcher and FIFO run in parallel on hardware but are sequenced in our model)
3. **Generate hypotheses.** For each gap, write a hypothesis that has all four parts (see Receipt format below):
   - **Hardware behavior**: A factual claim about what the silicon does — not about code, not about other emulators. This is the core of the hypothesis.
   - **Model divergence**: How the emulator's specific data structures (name the struct, enum, state machine) differ from this hardware behavior, and how.
   - **Prediction**: A specific, falsifiable observation — what you'd see in a measurement or research result if the hardware behavior claim is true.
   - **Test**: What `/measure` should measure or `/research` should answer.
4. **Validate each hypothesis.** Before including it, check:
   - Does the hardware behavior field describe the silicon, or does it describe a code change? If it says "add", "remove", "change", "set" — it's a fix, not a hypothesis. Reframe it.
   - Does the model divergence field name a specific struct/enum/field? If it says "the code" or "the emulator" generically, it's too vague. Find the specific data model element.
   - Could the prediction be wrong? If not, it's not falsifiable.
   - Has this already been ruled out? Check the RCA tree.
5. **Rank by leverage.** Order hypotheses by how much confirming or refuting them would advance the investigation. The best hypothesis is one where either outcome (confirmed or refuted) narrows the problem significantly. Avoid hypotheses where confirmation would tell you nothing new.
6. **For each hypothesis, state the test.** Briefly describe what `/measure` should measure or what `/research` should answer to test it. This helps the caller dispatch immediately.

## Output

Write the hypotheses to a receipt file inside the investigation's analysis folder:

```
receipts/investigations/<session>/analysis/<NNN>-hypotheses.md
```

Use the next sequential number in the analysis folder.

### Receipt format

```markdown
# Hypotheses

## Current understanding (snapshot)
<one-paragraph summary of the current model, copied/condensed from summary.md>

## Gap classification
<what type of gap exists — missing state / wrong transitions / wrong structure / missing interaction>

## Ruled out
<brief list of refuted hypotheses from the RCA tree — copied for context, not re-analyzed>

## Hypotheses (ranked)

### 1. <short title>
**Hardware behavior**: <what the real silicon does — a factual claim about the hardware, not about code>
**Model divergence**: <how the emulator's specific data structures differ — name the struct/enum/state machine and explain HOW it's wrong>
**Prediction**: <specific, falsifiable observation that would confirm or refute the hardware behavior claim>
**Test**: <what to measure or research to confirm/refute>
**Leverage**: <why this hypothesis is high-priority — what either outcome tells you>

### 2. <short title>
...

```

## After hypotheses are generated

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

1. Write the hypotheses receipt to the file.
2. **Do not update `summary.md`.** The caller (investigate) owns summary.md and will incorporate the top hypothesis into the RCA tree.
3. **Resume as the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and immediately continue working as the caller. **Do not decide which hypothesis to pursue or how to test it** — the caller reads the hypotheses receipt and makes that decision.

**The turn does not end here.** Do NOT stop after writing the receipt. The caller must act on the result in the same turn.
