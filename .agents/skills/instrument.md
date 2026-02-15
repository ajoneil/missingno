# Instrument

Add targeted diagnostic logging, run the diagnostic, and report what the output shows.

## Scope discipline

**You are a measurement tool, not a problem-solver.** Your report must follow the instrument report format defined in the skill invocation protocol in AGENTS.md. If you catch yourself writing interpretation, root cause analysis, or fix suggestions — stop, delete it, and return to reporting measurements.

The caller sent you a Question (hypothesis), Context (where to instrument), and Log path (where to save output). Measure what was asked. Do not reason about the caller's problem, propose fixes, or expand scope. If you notice something unexpected, note it as a one-liner in the "Also observed" section of your report.

## Before you start

1. **Read the hypothesis.** The caller should have told you what they're testing and why. If the request is vague ("add some logging"), ask for a specific hypothesis before proceeding.
2. **Check existing instrumentation.** Read the files you'll be modifying to see if diagnostic logging is already in place. Build on existing log lines rather than duplicating or conflicting with them.
3. **Identify the measurement points.** Before writing any code, list the exact locations in the source where you'll add `eprintln!` calls. Each log line must directly address the hypothesis.

## What to instrument

Based on the caller's request, add targeted `eprintln!` tracing that captures:

- **State transitions**: Log the exact cycle/dot/tick when modes, phases, or states change. Include the old and new state.
- **Timing events**: Log when interrupts fire, when registers are read/written, when DMA transfers occur — with precise cycle counts.
- **Data flow**: Log pixel values, FIFO contents, fetcher steps, audio sample values — whatever the test is measuring.
- **Decision points**: Log the values that drive conditional logic (comparison results, flag checks, counter values).

## How to instrument

- Add logging to the emulator code at the points relevant to the failing test. Gate output to only the lines/frames/cycles the test cares about to keep output manageable.
- Use a consistent tag format: `[SUBSYSTEM] context: key=value key=value`
- **Every log line must include enough context to interpret in isolation.** A line that says `mode=0` is useless without knowing which line, which dot, which frame, and which phase of the test ROM produced it. Include: LY, dot counter, and whatever discriminates "measurement I care about" from "background noise."

## Review before running

**Before running the test, review your log lines and ask: "Will this output directly confirm or refute the hypothesis?"** Bad instrumentation wastes a test run. Common mistakes:

- **Logging the wrong layer.** If the test ROM reads a register, log at the register read site (what the CPU actually sees), not at the internal state machine (what the hardware is doing internally). These can differ — e.g. `stat_mode()` vs `mode()`. If you need both, log both explicitly, but know which one the test ROM observes.
- **Logging without enough context.** Include the discriminating context: frame number, PC, loop counter, LY, dot — whatever distinguishes "this is the measurement I care about" from noise.
- **Logging at the wrong granularity.** If the test checks a CPU register value after a sequence of instructions, logging PPU state at every dot produces thousands of irrelevant lines. Log at the decision point. Conversely, if the test measures a 1-2 dot timing window, logging at M-cycle boundaries (every 4 dots) might miss the critical transition.
- **Not logging what the test actually checks.** If the test compares register B against a constant, log B at the comparison point. Don't just log the subsystem state and hope you can reconstruct what B was.
- **Assuming which code path executes.** If the test ROM has sync code, setup code, and measurement code that all read the same register, your log lines capture ALL of them. Add context to distinguish which reads belong to the measurement phase.

**The review question is: "If I see this log output, can I state with certainty what the emulator did?"** If the answer is "I'll need to reason about timing to interpret it," your instrumentation isn't targeted enough. Refine it before running.

## How to run

**MANDATORY: Every `cargo test` invocation must be saved to a log file.** No exceptions.

```bash
# CORRECT — always use this pattern:
cargo test <test_name> -- --nocapture 2>&1 | tee <log_path>

# WRONG — never do any of these:
cargo test <test_name> -- --nocapture 2>&1 | grep "pattern"
cargo test <test_name> -- --nocapture 2>&1 | tail -50
cargo test <test_name> -- --nocapture              # no tee = lost output
```

The log path should be in the active investigation's `logs/` folder with a descriptive name:
```
receipts/investigations/<session>/logs/<descriptive-name>.log
```

**Set generous timeouts.** `cargo test` in debug mode is slow. Use at least 120s for individual tests.

## What good diagnostic output looks like

```
[SUBSYSTEM] context: state_before -> state_after (key_values)
```

For example:
```
[PPU] LY=66 dot=252: Mode3->Mode0 (mode3_len=172, SCX=0)
[IRQ] LY=66 dot=252: STAT rising edge (flags=0x08, mode=BetweenLines)
```

Dense enough to pinpoint the behavior, filtered enough to read. Gate output to the relevant lines/frames.

## Make tests as specific as possible

**When a test ROM contains multiple sub-tests, your test harness must tell you exactly which sub-test passed and which failed.** Do not settle for a binary pass/fail result that forces the investigator to reason about which sub-test broke.

For Mooneye tests specifically, the convention is Fibonacci values in registers (B=3, C=5, D=8, E=13, H=21, L=34) assigned in order as sub-tests pass. If a sub-test fails, its register (and all subsequent ones) gets 0x42. The test harness should report: "Sub-tests passed: 2/6 (failed at sub-test 3, register D=0x42, expected 8)" — not just "registers don't match."

**This extends to diagnostic logging too.** If the test ROM runs a sequence of sub-tests with different configurations (e.g. varying sprite counts, positions, timing), your logs should clearly label which sub-test iteration is running. Add a sub-test counter or configuration summary so the investigator can immediately see which sub-test produces wrong values.

## Reporting results

After the test run, read the log file and report:

1. **Test result**: pass/fail, which sub-test failed if applicable.
2. **Key measurements**: The specific values the hypothesis was about, extracted from log output with line references.
3. **Raw data**: A compact summary of the relevant log lines (not the entire log — just the lines that answer the hypothesis).

Do NOT interpret what the measurements mean for the investigation. Just report them.

The log file on disk is the primary record. Your report extracts the key measurements, but the full data lives in the log file. Reference the log path — don't reproduce large amounts of raw output in conversation.

## Baseline comparisons

When the caller asks for a baseline comparison:

1. Run the test with logging on the current (failing) code. Save to `logs/<name>-current.log`.
2. Stash changes, checkout main (or specified known-good state), apply the same logging, run again. Save to `logs/<name>-baseline.log`.
3. Diff the two outputs. Report the first divergence point and the surrounding context.
4. Restore the working branch (unstash).

## After instrumentation is complete

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

**You MUST continue working after writing your report.** The instrumentation phase is over; now resume as the caller. Concretely:

1. Write your report (Test result / Measurements / Raw data / Also observed).
2. Write the caller's interpretation of the measurements to `summary.md`, referencing the log file path. The measurements are now on disk in two places (the log file and the summary) — conversation memory of the raw output is no longer needed.
3. Re-read the caller's skill file (e.g. `.agents/skills/investigate.md`) and the active investigation's `summary.md` to restore the caller's context from disk. Work from the file state, not from conversation memory.
4. **Immediately continue the caller's workflow** — proceed to the next step based on what `summary.md` says, not on what you remember.

Do NOT end your turn after the report. Do NOT wait for further input. The report is a return value, not a stopping point.
