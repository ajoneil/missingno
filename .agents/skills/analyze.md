# Analyze

Interpret new data — from measurement or research — against the current understanding of the problem. Write a durable analysis receipt and update the investigation's summary.

## Scope discipline

**You are an interpreter, not a fact-finder or implementer.** You receive data (measurements, research findings) and the current investigation state. Your job is to:

1. Determine what the data means for the investigation's hypotheses.
2. Update, confirm, or refute the current mental model.
3. Identify the next question to answer.

You do NOT gather new data (that's measure/research), design solutions (that's design), or make code changes (that's investigate). If you realize you need more data to interpret what you have, say so in your report — do not invoke other skills yourself.

## Inputs

The caller provides:

- **Data**: A pointer to the new data — a log file path (from measure) or a research document path (from research). Read the file; do not rely on conversation memory of its contents.
- **Summary**: Path to the investigation's `summary.md`. Read it to understand the current state — active hypotheses, what's been tried, what's known.

## Process

1. **Re-read the data source and summary.md.** Do not work from memory. Read the actual files.
2. **State what the data shows.** Extract the specific measurements or findings relevant to the active hypothesis. Be concrete — cite values, line numbers in log files, specific statements from research docs.
3. **Compare against expectations.** What did the hypothesis predict? What did the data show? Where do they match and where do they diverge?
4. **Update the model.** Based on the comparison:
   - **Confirmed**: The hypothesis holds. State what this establishes as known and what the next question is.
   - **Refuted**: The hypothesis is wrong. State specifically which prediction failed and what the actual behavior implies about the correct model.
   - **Inconclusive**: The data doesn't clearly confirm or refute. State what's missing and what measurement or research question would resolve it.
5. **State what's now known and unknown.** Summarize the updated model — what is established, what remains uncertain. Do not prescribe what to do next — that's the caller's decision.

## Output

Write the analysis to a receipt file inside the investigation's analysis folder:

```
receipts/investigations/<session>/analysis/<NNN>-<short-name>.md
```

Number analysis receipts sequentially (`001`, `002`, ...) so they form a readable chronological trail. Use a short descriptive suffix (e.g. `001-baseline-timing.md`, `002-mode3-length-refuted.md`).

Create the `analysis/` directory if it doesn't exist.

### Receipt format

```markdown
# Analysis: <short title>

## Data source
<path to the log file or research document being interpreted>

## Active hypothesis
<the hypothesis being tested, copied from summary.md>

## What the data shows
<specific measurements or findings, with file:line references>

## Comparison
<expected vs actual — where do they match, where do they diverge?>

## Conclusion
<confirmed / refuted / inconclusive — one word, then explanation>

## Updated model
<what is now known or believed about the problem, incorporating this result>

## Open questions
<what remains unknown or uncertain after this analysis — if nothing, say "none">
```

## After analysis is complete

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

1. Write the analysis receipt to the file.
2. Update the investigation's `summary.md`: rewrite the **Current understanding** section to reflect the new conclusions (this is the most important update — it must always be the best current model of the problem), record the conclusion, note the receipt path. The analysis is now on disk — conversation memory of the reasoning is no longer needed.
3. **Return to the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and hand control back. **Do not decide what to do next** — the caller reads the updated summary.md and makes that decision.
