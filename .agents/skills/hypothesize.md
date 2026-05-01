# Hypothesize

Generate testable hypotheses about **hardware behavior** and **data model divergence** based on the current understanding of the problem and what's been tried so far.

## Invocation

This skill runs **in-context on the main agent** (not as a Task subagent). Conversation context — recent measurements, the user's clarifications, what feels off about the current model — is intentionally load-bearing for synthesis work like this.

Even running in-context, the brief is mandatory. Before starting, write to summary.md (or scratch):

**Summary**: path to the investigation's `summary.md`
**Context**: any additional context (e.g. specific subsystem, a new clue, a constraint on what's worth testing)

The deliverable is a receipt file. Receipts and summary.md are durable; conversation context is not.

## Scope discipline

**You are in hypothesis-generation mode.** Your job is to produce a ranked list of testable hypotheses — specific, falsifiable predictions about what the hardware does and how the emulator's data model differs. Re-read summary.md (especially **Root cause analysis** and **Current understanding**) — do not work from conversation memory alone, since the RCA tree may have entries the conversation has lost track of.

**Hypotheses are about hardware and models, not about code changes.** A hypothesis must describe what the real hardware does (a claim about the silicon) and where the emulator's data model diverges. "Add 4 idle dots at Mode 3 start" is not a hypothesis — it's a proposed fix. "The hardware's pixel FIFO begins outputting 4 dots into Mode 3 because the first tile fetch overlaps with pipeline priming, but our model sequences them as two non-overlapping 6-dot phases" is a hypothesis.

You do NOT gather data, interpret measurements, design fixes, or make code changes while in hypothesize mode. Read the current state and propose what to test next.

## Inputs

From your brief:

- **Summary**: Path to the investigation's `summary.md`. Read it — especially the **Root cause analysis** tree and **Current understanding** sections.
- **Context**: Any additional context (e.g. specific subsystem, a new clue, a constraint on what's worth testing).

## Process

1. **Re-read summary.md.** The Current understanding, Hardware model, and Model divergence sections are your starting point. The Root cause analysis tree shows what's already been tried and ruled out.
1b. **For PPU issues, consult the PPU timing model spec first.** Read relevant sections of `receipts/ppu-overhaul/reference/ppu-timing-model-spec.md` (the canonical hardware reference, collated from dmg-sim). The spec describes the gate-level pipeline, race windows, and mode transitions directly — hypotheses grounded in the spec are high-leverage because they map to specific gates (AVAP, WODU, XYMU, etc.) that the emulator either models or doesn't. If the behaviour you're reasoning about isn't covered in the spec but a dmg-sim measurement would answer it, note the spec gap in your receipt so the user can run dmg-sim and extend the spec.
1c. **For non-PPU hardware behaviour, consult gb-ctr.** Gekkio's Game Boy Complete Technical Reference (https://gekkio.fi/files/gb-docs/gbctr.pdf) is the primary written reference for timers, interrupts, CPU, DMA, etc.
1d. **For timing issues, consult the propagation delay analysis.** (PPU races have the largest observable effects, but the analysis covers the full netlist.) Read `receipts/resources/gb-propagation-delay-analysis/output/race_pairs_report.md` and `receipts/resources/gb-propagation-delay-analysis/output/operational_paths.md`. These identify signal races where inputs to a DFF arrive at different gate depths — the hardware captures different values than an emulator that resolves all signals simultaneously. Match the observable effects listed in the race pairs report (e.g. "tile fetch runs one extra cycle", "sprite X-position off by one dot") against the symptoms in the investigation. Known races are high-leverage hypotheses because they have a structural explanation.
2. **Classify the gap.** Before generating hypotheses, identify what type of gap exists between the hardware and the emulator's model. The gap is one or more of:
   - **Missing state**: Hardware has state our model doesn't track (e.g., a pipeline stage, a latch, a counter we don't model)
   - **Wrong transitions**: Our state machine has the right states but transitions at the wrong time or under the wrong conditions
   - **Wrong structure**: Our model decomposes the hardware differently (e.g., two sequential phases where hardware has one overlapping phase, or a single counter where hardware has two independent ones)
   - **Missing interaction**: Two subsystems interact on hardware but are independent in our model (e.g., fetcher and FIFO run in parallel on hardware but are sequenced in our model)
3. **Generate hypotheses.** For each gap, write a hypothesis that has all four parts (see Receipt format below):
   - **Hardware behavior**: A factual claim about what the silicon does — not about code, not about other emulators. This is the core of the hypothesis.
   - **Model divergence**: How the emulator's specific data structures (name the struct, enum, state machine) differ from this hardware behavior, and how.
   - **Prediction**: A specific, falsifiable observation — what you'd see in a measurement or research result if the hardware behavior claim is true.
   - **Test**: What `/instrument` should measure or `/research` should answer.
4. **Validate each hypothesis.** Before including it, check:
   - Does the hardware behavior field describe the silicon, or does it describe a code change? If it says "add", "remove", "change", "set" — it's a fix, not a hypothesis. Reframe it.
   - Does the model divergence field name a specific struct/enum/field? If it says "the code" or "the emulator" generically, it's too vague. Find the specific data model element.
   - **Is it framed in terms of DFF edges, not rise/fall ordering?** A hypothesis like "the write needs to move from fall() to rise()" reasons about the emulator's procedural call order, not about hardware. On hardware, rise and fall are alternating clock edges — there is no "before" or "after" within a dot. Reframe: which DFF edge captures the value, and does the output hold the correct value when the consumer reads it?
   - Could the prediction be wrong? If not, it's not falsifiable.
   - Has this already been ruled out? Check the RCA tree.
5. **Rank by leverage.** Order hypotheses by how much confirming or refuting them would advance the investigation. The best hypothesis is one where either outcome (confirmed or refuted) narrows the problem significantly. Avoid hypotheses where confirmation would tell you nothing new.
6. **For each hypothesis, state the test.** Briefly describe what `/instrument` should measure or what `/research` should answer to test it. This helps the caller dispatch immediately.

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

1. Write the hypotheses receipt to the file.
2. Update summary.md — incorporate the top-ranked hypothesis into the RCA tree, marked `[ ] **bold** ← ACTIVE`. Move the previously-active hypothesis to `[x] ~~struck~~` if it was refuted, or down the tree if it's been deprioritized.
3. Exit hypothesize mode and continue the investigation. Typically the next step is `/instrument`, `/inspect`, or `/research` to test the active hypothesis.
