# Improve Compatibility

Fix a compatibility bug against a test ROM or real game.

## Workflow

### 1. Identify the failure

- Ask the developer what test or game is failing and what the symptom is.
- Once the scope is clear, propose a receipt folder name and get approval. Create the receipt folder immediately with this structure:
  ```
  receipts/improve-compatibility/<YYYY-MM-DD>-<short-name>/
  ├── summary.md        # Create now with Status: Diagnosing
  ├── research/         # Investigation-specific notes
  └── logs/             # Diagnostic output captures
  ```
- If a test name is given, run it with output visible: `cargo test <test_name> -- --nocapture`
- Classify the failure type:
  - **Register mismatch**: Expected vs actual CPU/hardware register values after test execution.
  - **Screenshot mismatch**: Pixel differences between rendered output and reference image.
  - **Timeout/hang**: The ROM never reached a halt condition — likely wrong control flow or missing hardware behavior.

### 2. Understand what's being tested

- Identify the subsystem involved (video/PPU, audio/APU, timers, interrupts, memory mapping, DMA, input, serial, etc.).
- **Use the `research` skill** (`/research`) to find and read the test ROM's source code, understand what hardware behavior is being validated, and document findings. The research skill handles source lookup, technical doc consultation, and writing to `receipts/research/`.
- **Update summary.md** with the problem description and subsystem identified.
- Capture investigation-specific notes (test ROM analysis, expected values for this particular test) in the session's `research/` folder.

### 3. Research the correct hardware behavior

- **Use the `research` skill** (`/research`) for all hardware research. This includes consulting technical documentation, studying reference emulator implementations, and reading test ROM documentation. The research skill will write general hardware knowledge to `receipts/research/`.
- Do not perform research inline with ad-hoc web searches — always invoke the `research` skill so findings are properly documented and reusable.
- **Research is not just for steps 2-3.** Any time during the investigation that you're uncertain about hardware behavior — while diagnosing, while interpreting diagnostic output, while designing a fix — stop and use the `research` skill. If you find yourself reasoning through timing, register values, or state machine behavior without a source to back it up, that's a signal to research first.
- **Update summary.md** with research findings.
- Capture investigation-specific notes in the session's `research/` folder.

### 4. Verify regression vs pre-existing

- Compare against the `main` branch to determine if this is a new regression or a pre-existing gap.
- If it's a regression, diff the relevant files to narrow the scope: `git diff main..<branch> -- <relevant files>`
- If pre-existing, still worth fixing but important to know the baseline failure count.
- **Update summary.md** with regression/pre-existing classification.

### 5. Build a diagnostic test harness

**Do not guess at fixes. Do not reason through timing in your head.** The goal is to collect precise information about what the emulator is actually doing vs what it should do. If you're unsure what the emulator is doing at a particular point, add logging and run it — don't try to trace through the code mentally. Build temporary diagnostic instrumentation:

#### What to instrument

Based on the subsystem involved, add targeted `eprintln!` tracing that captures:

- **State transitions**: Log the exact cycle/dot/tick when modes, phases, or states change. Include the old and new state.
- **Timing events**: Log when interrupts fire, when registers are read/written, when DMA transfers occur — with precise cycle counts.
- **Data flow**: Log pixel values, FIFO contents, fetcher steps, audio sample values — whatever the test is measuring.
- **Decision points**: Log the values that drive conditional logic (comparison results, flag checks, counter values).

#### How to instrument

- Add logging to the emulator code at the points relevant to the failing test. Gate output to only the lines/frames/cycles the test cares about to keep output manageable.
- **Run on both failing and working code.** The most valuable output is a side-by-side comparison:
  1. Run the test with logging on your current (failing) branch.
  2. Stash changes, checkout main (or a known-good state), apply the same logging, run the test again.
  3. Diff the two outputs. The first divergence point is your root cause.
- If no working baseline exists, compare the logged behavior against the expected behavior from documentation or reference emulator source code.

#### How to run diagnostics

**Every diagnostic run must be saved to `logs/`.** Use `tee` to capture output at the point of execution — never run a diagnostic without simultaneously writing it to a log file:

```bash
cargo test <test_name> -- --nocapture 2>&1 | tee receipts/improve-compatibility/<session>/logs/<descriptive-name>.log
```

Name log files so you can tell them apart later:
- `logs/mode-timing-baseline.log` — initial failing state
- `logs/mode-timing-fix-attempt-1.log` — after first fix
- `logs/mode-timing-main-branch.log` — baseline from main

After saving, **update summary.md** with what you instrumented, what the output showed, and what hypothesis it supports or refutes.

#### What good diagnostic output looks like

```
[SUBSYSTEM] context: state_before -> state_after (key_values)
```

For example:
```
[PPU] LY=66 dot=252: Mode3->Mode0 (mode3_len=172, SCX=0)
[IRQ] LY=66 dot=252: STAT rising edge (flags=0x08, mode=BetweenLines)
```

The output should be dense enough to pinpoint the bug but filtered enough to read. Thousands of lines of unfiltered output are not useful — focus on the cycles/events the test actually checks.

### 6. Analyze and fix

- Study the diagnostic output to identify the root cause.
- **If any hardware behavior is unclear**, stop and use the `research` skill before proceeding. Don't guess at what the hardware does.
- **Update summary.md** with your hypothesis before attempting a fix.
- Fix only the identified issue. Don't refactor surrounding code.
- **Design fixes based on hardware behavior, not other emulators' code.** Research tells you *what* the hardware does (timing values, state transitions, edge cases). Your fix should implement that behavior within your existing architecture. Do not copy data structures, variable names, or architectural patterns from reference emulators — they have different designs and their implementation choices may not fit yours.
- **Validate every fix attempt with diagnostic output.** Run with logging before and after the fix, saving each run to `logs/` with `tee`. If the numbers don't match expectations, add more logging rather than reasoning about why — let the output tell you what happened.
- **Remove all diagnostic logging before committing.**
- Run the full test suite after each fix: `cargo test`
- Verify no new regressions (failure count must not increase).
- **Update summary.md** after each fix attempt — whether it worked or not, how test results changed, what you'll try next if it didn't work.

### 7. Commit

- If on a **feature branch**: commit each fix separately before moving to the next issue.
- If on **main**: ask the user whether to commit directly to main or create a feature branch first.

### 8. Receipt conventions

The receipt folder is created in step 1 and written into continuously throughout the investigation. This section documents the format and conventions.

#### Folder structure

```
receipts/improve-compatibility/<YYYY-MM-DD>-<short-name>/
├── summary.md        # Living investigation summary (required)
├── research/         # Investigation-specific notes (test ROM analysis, hypotheses)
├── logs/             # Diagnostic output captures
└── ...               # Any other artifacts (diffs, screenshots)
```

Use the date of the investigation and a short kebab-case name describing the issue (e.g. `2026-02-13-stat-mode0-timing`, `2026-02-13-mbc1-bank-wrap`).

#### Two research locations

General hardware knowledge should already be in `receipts/research/` — you documented it using the `research` skill during steps 2 and 3. The session's `research/` folder is for investigation-specific notes only: test ROM analysis, diagnostic interpretations, hypotheses for this particular failure. If a future investigation into the same subsystem would benefit from a finding, it belongs in `receipts/research/`, not here.

#### summary.md

`summary.md` is a living document — the developer should be able to read it at any point and understand exactly where the investigation stands. Update it as you work, not at the end.

Include:
- **Status**: Current investigation state (e.g. "diagnosing", "fix in progress", "resolved", "blocked")
- **Problem**: The failing test/game and symptom
- **What's been tried**: Diagnostic approaches, hypotheses tested, fix attempts — even failed ones
- **Findings**: What diagnostic output revealed, root cause if known
- **Resolution**: What was changed and which tests now pass (once fixed)
- **Remaining**: Any related failures or open questions

### 9. Commit format

```
Short summary of what changed

Detailed explanation of:
- What the bug was (observable symptom)
- Why it happened (root cause in the emulation logic)
- How the fix works (what changed and why it matches real hardware)

Fixes <test_name>.
```
