# Instrument

Add targeted diagnostic logging to the code, run the diagnostic, and report what the output shows.

## Scope discipline

**You are a measurement tool, not a problem-solver.** Your report must follow the measure report format defined in the skill invocation protocol in AGENTS.md. If you catch yourself writing interpretation, root cause analysis, or fix suggestions — stop, delete it, and return to reporting measurements.

The caller sent you a Question (hypothesis), Context (where to instrument), and Log path (where to save output). Measure what was asked. Do not reason about the caller's problem, propose fixes, or expand scope. If you notice something unexpected, note it as a one-liner in the "Also observed" section of your report.

## Before you start

1. **Read the hypothesis.** The caller should have told you what they're testing and why. If the request is vague ("add some logging"), ask for a specific hypothesis before proceeding.
2. **Consider gbtrace first.** If the hypothesis can be tested by examining CPU/PPU/timer/interrupt state at instruction boundaries, a gbtrace capture may answer the question without modifying code. Capture with `GBTRACE_PROFILE=gbmicrotest` (or the appropriate suite profile), then query with `gbtrace query <file> --where pc=0x0150 --context 5`. Use `eprintln!` instrumentation for internal state not visible through gbtrace (e.g. FIFO contents, fetcher phases, mid-instruction timing).
3. **Check existing instrumentation.** Read the files you'll be modifying to see if diagnostic logging is already in place. Build on existing log lines rather than duplicating or conflicting with them.
4. **Identify the measurement points.** Before writing any code, list the exact locations in the source where you'll add `eprintln!` calls. Each log line must directly address the hypothesis.

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

### Frame-level gating

**Always gate diagnostic output to a specific frame (or narrow frame range).** The same LY value repeats every frame. A test ROM that runs for 60 frames produces 60 copies of LY=6, and filtering the log after the fact to find the right one is error-prone and wastes investigation time. Instead, add a temporary frame counter and gate on it.

The PPU has no built-in frame counter. When you need frame-level gating, add a temporary `frame_count: u32` field to the relevant struct (e.g. `Rendering` or `PixelProcessingUnit`) and increment it at the frame boundary (when `Rendering` is constructed from `BetweenFrames`, or at the start of line 0). This is diagnostic scaffolding — it gets removed with the rest of the logging after measurement.

**Pattern:**
```rust
// In the struct (temporary diagnostic field):
frame_count: u32,

// At the frame boundary (e.g. start of new Rendering):
self.frame_count += 1;

// In the diagnostic logging:
if self.frame_count == 2 && self.line_number == 6 {
    eprintln!("[PPU] frame={} LY=6 dot={}: ...", self.frame_count, self.dot);
}
```

**Include the frame number in every log line**, even when gating to a single frame. This makes it immediately obvious which frame produced each line and confirms the gating is working correctly.

**Choose the right frame to gate on.** Mealybug tearoom tests typically compare a screenshot taken after the test ROM signals a breakpoint. The test ROM usually sets up state in early frames and the measurement frame is often frame 2 or later. If unsure which frame matters, start by logging a single line per frame (e.g. at LY=0 dot=0) with the frame number, to identify which frame the interesting behavior occurs in. Then narrow the gate to that specific frame.

## Review before running

**Before running the test, review your log lines and ask: "Will this output directly confirm or refute the hypothesis?"** Bad instrumentation wastes a test run. Common mistakes:

- **Logging the wrong layer.** If the test ROM reads a register, log at the register read site (what the CPU actually sees), not at the internal state machine (what the hardware is doing internally). These can differ — e.g. `stat_mode()` vs `mode()`. If you need both, log both explicitly, but know which one the test ROM observes.
- **Logging without enough context.** Include the discriminating context: frame number, PC, loop counter, LY, dot — whatever distinguishes "this is the measurement I care about" from noise.
- **Logging at the wrong granularity.** If the test checks a CPU register value after a sequence of instructions, logging PPU state at every dot produces thousands of irrelevant lines. Log at the decision point. Conversely, if the test measures a 1-2 dot timing window, logging at M-cycle boundaries (every 4 dots) might miss the critical transition.
- **Not logging what the test actually checks.** If the test compares register B against a constant, log B at the comparison point. Don't just log the subsystem state and hope you can reconstruct what B was.
- **Assuming which code path executes.** If the test ROM has sync code, setup code, and measurement code that all read the same register, your log lines capture ALL of them. Add context to distinguish which reads belong to the measurement phase.
- **Not gating to a specific frame.** If your log condition is `if LY == 6 { eprintln!(...) }` and the test runs for 60 frames, you get 60 copies of the same scanline from different frames interleaved in the output. You then have to manually figure out which block belongs to which frame. Always add a frame counter and gate to the specific frame(s) you care about (see "Frame-level gating" above).

**The review question is: "If I see this log output, can I state with certainty what the emulator did?"** If the answer is "I'll need to reason about timing to interpret it," or "I'll need to figure out which frame this line came from," your instrumentation isn't targeted enough. Refine it before running.

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

**Boot ROM.** If the investigation suspects boot state plays a role, ask the user for a DMG boot ROM path (boot ROMs are proprietary and cannot be in the repo). Set `DMG_BOOT_ROM=<path>` in the test command to run with the boot ROM. Only use this on targeted tests — running the boot ROM adds significant startup time per test, making it impractical for the full suite.

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

After the test run, write a measurement receipt to the investigation's `measurements/` folder:

```
receipts/investigations/<session>/measurements/<NNN>-<short-name>.md
```

Create the `measurements/` directory if it doesn't exist. Number measurement receipts sequentially (`01`, `02`, ...) to form a readable chronological trail. Use a short descriptive suffix (e.g. `01-baseline.md`, `03-opcode-pending-dots.md`).

### Receipt format

```markdown
# Measurement: <short title>

## Question
<the hypothesis or question being tested, copied from the caller's request>

## Log file
<path to the raw log file>

## Test result
<pass/fail, which sub-test failed if applicable>

## Key measurements
<the specific values the hypothesis was about, extracted from log output with file:line references>

## Raw data
<compact summary of the relevant log lines — not the entire log, just the lines that answer the hypothesis>

## Also observed
<unexpected findings not part of the original question — optional, one-liners only>
```

Do NOT interpret what the measurements mean for the investigation. Just report them.

The log file on disk is the primary record. The receipt extracts the key measurements, but the full data lives in the log file.

## Baseline comparisons

When the caller asks for a baseline comparison:

1. Run the test with logging on the current (failing) code. Save to `logs/<name>-current.log`.
2. Stash changes, checkout main (or specified known-good state), apply the same logging, run again. Save to `logs/<name>-baseline.log`.
3. Diff the two outputs. Report the first divergence point and the surrounding context.
4. Restore the working branch (unstash).

## After measurement is complete

1. Write the measurement receipt to the file (see "Reporting results" above).
2. **Do not update `summary.md`.** The caller owns summary.md and will incorporate the measurements.
3. **Stop.** Your job is done. The caller reads the receipt file and decides what to do next.
