# Hypothesize

Generate testable hypotheses based on the current understanding of the problem and what's been tried so far.

## Scope discipline

**You are a hypothesis generator, not an investigator or interpreter.** You receive the current mental model and investigation history. Your job is to produce a ranked list of testable hypotheses — specific, falsifiable predictions that can be confirmed or refuted by a single `/measure` or `/research` invocation.

You do NOT gather data, interpret measurements, design fixes, or make code changes. You read the current state and propose what to test next.

## Inputs

The caller provides:

- **Summary**: Path to the investigation's `summary.md`. Read it — especially the **Root cause analysis** tree and **Current understanding** sections. Do not work from conversation memory.
- **Context**: Any additional context the caller provides (e.g. specific subsystem, a new clue, a constraint on what's worth testing).

## Process

1. **Re-read summary.md.** The Current understanding section is your starting point. The Root cause analysis tree shows what's already been tried and ruled out.
2. **Identify gaps.** What does the current model leave unexplained? Where is it weakest? What assumptions hasn't it tested?
3. **Generate hypotheses.** For each gap, write a specific, testable prediction. A good hypothesis:
   - States what you expect to observe and where (e.g. "LY increments at dot 452, not dot 456")
   - Can be confirmed or refuted by a single measurement or a single research question
   - Is falsifiable — there's a concrete observation that would prove it wrong
   - Doesn't retest something that's already been ruled out (check the RCA tree for `[x] ~~struck~~` entries)
4. **Rank by leverage.** Order hypotheses by how much confirming or refuting them would advance the investigation. The best hypothesis is one where either outcome (confirmed or refuted) narrows the problem significantly. Avoid hypotheses where confirmation would tell you nothing new.
5. **For each hypothesis, state the test.** Briefly describe what `/measure` should measure or what `/research` should answer to test it. This helps the caller dispatch immediately.

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

## Ruled out
<brief list of refuted hypotheses from the RCA tree — copied for context, not re-analyzed>

## Hypotheses (ranked)

### 1. <short title>
**Prediction**: <specific, falsifiable statement>
**Test**: <what to measure or research to confirm/refute>
**Leverage**: <why this hypothesis is high-priority — what either outcome tells you>

### 2. <short title>
**Prediction**: <specific, falsifiable statement>
**Test**: <what to measure or research to confirm/refute>
**Leverage**: <why this hypothesis is high-priority — what either outcome tells you>

...
```

## After hypotheses are generated

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

1. Write the hypotheses receipt to the file.
2. **Do not update `summary.md`.** The caller (investigate) owns summary.md and will incorporate the top hypothesis into the RCA tree.
3. **Resume as the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and immediately continue working as the caller. **Do not decide which hypothesis to pursue or how to test it** — the caller reads the hypotheses receipt and makes that decision.

**The turn does not end here.** Do NOT stop after writing the receipt. The caller must act on the result in the same turn.
