# Instrument

Add targeted diagnostic logging to the emulator, run the test, and report what the output shows.

**This skill is a subroutine, not a stopping point.** After completing instrumentation and reporting findings, immediately return to the task that prompted it. Do not wait for further user input — interpret the results and continue working.

**IMPORTANT**: When this skill finishes (logging added, test run, output analyzed), your VERY NEXT action must be acting on the findings — updating summary.md, editing code, forming the next hypothesis, etc. Do NOT end your turn after reporting diagnostic results. The diagnostic output is useless until you act on it.

## Scope discipline

**You are a measurement tool, not a problem-solver.** Your job is to add the requested logging, run the test, and report exactly what the output shows. You must NOT:

- **Analyze the root cause.** Don't reason about why the values are what they are or what the fix should be. That's the investigator's job.
- **Propose fixes or implementation approaches.** Don't suggest code changes based on what you see. Just report the measurements.
- **Interpret findings in context of the investigation.** Don't say "this means the penalty is wrong because..." or "this confirms hypothesis X." State what the log output shows and let the caller decide what it means.
- **Expand scope beyond the request.** If asked to log mode 3 length, log mode 3 length. Don't also add logging for mode 2 timing, interrupt edges, or fetcher state unless the caller explicitly asked.

If you notice something unexpected in the output that wasn't part of the original question, note it briefly at the end of your report under a "Also observed" heading — but don't pursue it.

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

## Baseline comparisons

When the caller asks for a baseline comparison:

1. Run the test with logging on the current (failing) code. Save to `logs/<name>-current.log`.
2. Stash changes, checkout main (or specified known-good state), apply the same logging, run again. Save to `logs/<name>-baseline.log`.
3. Diff the two outputs. Report the first divergence point and the surrounding context.
4. Restore the working branch (unstash).

## After instrumentation is complete

When you have run the test and reported the output, **you are not done with your turn.** The instrument skill is always invoked as a subroutine from another task (usually an investigation). Your measurements are useless until the caller acts on them.

**Your very next action after finishing the report must be a non-instrument action**: updating summary.md, editing code, forming a new hypothesis, or any other concrete investigation step. If you find yourself about to end your turn after reporting measurements, you have made an error. Continue working.
