# Investigate

Investigate and fix a compatibility bug against a test ROM or real game.

## Discipline requirements

These rules override default agent behavior. Follow them exactly:

1. **Never run `cargo test` without `tee` to a log file.** Every test invocation — diagnostic, verification, regression check — must be saved. No exceptions. No piping through `grep`/`tail`/`head` instead of saving.
2. **Never skip summary.md updates.** Update it before and after every diagnostic run and every fix attempt. If you're about to run a test, write in summary.md what you're testing and why first.
3. **Never do ad-hoc research.** Use the `research` skill for ALL external information gathering. This includes:
   - Hardware documentation (Pan Docs, GBEDG, wikis, etc.)
   - Test ROM source code (fetching `.s`/`.asm` files, understanding test ROM helper macros, analyzing what a test expects)
   - Reference emulator source code
   - Any `curl`, `WebFetch`, or `WebSearch` call for technical content
   
   If you catch yourself about to fetch a URL or clone a repo, stop and invoke `/research` instead. After research completes, immediately resume the investigation — do not stop and wait for user input.
   
   **How to hand off research questions:**
   - Formulate a **specific, concrete question** before invoking `/research`. Not "how does the wave channel work" but "what initial value does the wave channel frequency timer get on trigger — is there an extra delay beyond `(2048 - period) * 2`?"
   - Include **only the question and any necessary context** (e.g. which file to read, which subsystem, where to write findings). Do NOT include your hypotheses, diagnostic output, or reasoning about what the answer might mean — that's your job after research returns.
   - **One question per invocation.** If you have multiple questions, invoke `/research` multiple times with separate, focused questions. Don't bundle unrelated questions into a single research call.
   - When research returns, **you** interpret the findings in context of your investigation. The research skill reports facts; you figure out what they mean for the bug you're investigating.
4. **Never guess at fixes.** Add instrumentation, run diagnostics, read the output. The log files tell you what's happening — your mental model of the code is not a substitute.
5. **Never trace timing in your head.** If you want to know what value a register has at a specific dot, or what mode the PPU is in when a particular instruction executes — add a log line and run the test. Do not manually count M-cycles, dots, or pipeline stages. Your mental model will be wrong. The emulator is already a cycle-accurate simulator; let it simulate.
6. **Never build on unverified changes.** After any code change — even "obviously correct" ones — run the full test suite (`cargo test`) before building further changes on top. If a foundational change (e.g. LY timing, mode transitions) introduces regressions, you must know immediately — not after stacking three more changes on top. This is a blocking prerequisite: do not start the next change until the current one passes regression checks.

## Working style: hypothesize, test, interpret

Follow this loop for every investigation step:

1. **Form a hypothesis** — a short, testable statement. Write it down in summary.md. ("The STAT read in the counting loop sees mode 3 when it should see mode 0 because mode 3 is 4 dots too long.")
2. **Design a test** — add targeted logging that will confirm or refute the hypothesis. The log output should directly answer the question. If you can't tell what log line to add, your hypothesis isn't specific enough — refine it first.
3. **Run the test** — `cargo test ... 2>&1 | tee logs/<name>.log`. No exceptions.
4. **Read the output** — grep/read the log file. Extract the specific values that answer your hypothesis.
5. **Update summary.md** — state whether the hypothesis was confirmed or refuted, cite the log evidence, and write the next hypothesis.

**If you catch yourself writing more than ~4 lines of timing/cycle analysis without a log file open, stop.** You are guessing. Add a log line and run the test instead.

**If a fix attempt fails and you don't know why, do not analyze the code harder.** Add more logging to the specific area that surprised you, run again, and read the output.

## Workflow

### 1. Identify the failure

- Ask the developer what test or game is failing and what the symptom is.
- Once the scope is clear, propose a receipt folder name and get approval. Create the receipt folder immediately with this structure:
  ```
  receipts/investigations/<YYYY-MM-DD>-<short-name>/
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

- **Formulate specific questions and hand them to `/research`.** Each research invocation should have one clear, answerable question. Include the output location (general `receipts/research/` for hardware behavior, or the investigation's `research/` folder for test-specific analysis) and any context needed to find the answer (e.g. "the SameBoy source is cloned at /tmp/SameBoy"). Do NOT include your hypotheses, diagnostic interpretations, or what you think the answer might be.
- **Research is not just for steps 2-3.** Any time during the investigation that you're uncertain about hardware behavior — while diagnosing, while interpreting diagnostic output, while designing a fix — stop and formulate a research question. If you find yourself reasoning through timing, register values, or state machine behavior without a source to back it up, that's a signal to research first.
- **Use research to resolve contradictions.** If existing research documents contradict each other — or if diagnostic output contradicts what a research document claims — formulate a specific question and invoke `/research` to get the authoritative answer. When research returns, update or correct the contradicting documents.
- **Research is a subroutine.** After `/research` returns with findings, your very next action must be interpreting those findings in context of your investigation — updating summary.md, editing code, running a diagnostic. Never end your turn immediately after research. The pattern is: research → interpret findings → act → continue investigation.
- **Update summary.md** with research findings and your interpretation of what they mean for the investigation.

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

**MANDATORY: Every `cargo test` invocation must be saved to a log file.** This is not optional. Do not run `cargo test` without `tee`. Do not pipe output through `grep`, `tail`, `head`, or any other filter — capture the complete, unfiltered output first, then read the log file afterward to find what you need.

```bash
# CORRECT — always use this pattern:
cargo test <test_name> -- --nocapture 2>&1 | tee receipts/investigations/<session>/logs/<descriptive-name>.log

# WRONG — never do any of these:
cargo test <test_name> -- --nocapture 2>&1 | grep "pattern"
cargo test <test_name> -- --nocapture 2>&1 | tail -50
cargo test <test_name> -- --nocapture              # no tee = lost output
```

**Why this matters:** Filtered output is thrown away. When you filter at the pipe, you lose context that turns out to be important later. Save everything, read selectively afterward using `grep` on the saved log file.

**Timeouts and test scope:** Test runs can take a long time — the full suite may take 2+ minutes, and individual ROMs with high frame counts can take 60+ seconds each. Be strategic:
- **Use focused test runs first.** To verify a specific fix, run only the relevant sub-test(s) — not the entire suite. For example, if fixing CH1 sweep behavior, run only the sweep sub-test first, not all 12 sound tests.
- **Expand to full suite only after focused verification passes.** Regression checks are important but expensive. Do them after confirming the fix works, not as the first verification step.
- **Set generous timeouts.** `cargo test` in debug mode is slow. Use `timeout` values of at least 120s for individual tests and 300s+ for full suites. A test timing out due to an undersized bash timeout is wasted work — you learn nothing except that your timeout was too short.
- **If a test hangs, reduce scope — don't reduce timeout.** If a test ROM never reaches its completion loop, a shorter timeout won't help you debug it. Instead, run the specific hanging ROM individually and add instrumentation to understand why it's stuck.

Name log files descriptively so you can tell them apart later:
- `logs/mode-timing-baseline.log` — initial failing state
- `logs/mode-timing-fix-attempt-1.log` — after first fix
- `logs/mode-timing-main-branch.log` — baseline from main

After saving, **update summary.md** with what you instrumented, what the output showed, and what hypothesis it supports or refutes. Read the log file (using `grep` or `Read`) to extract the relevant details for the summary.

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

#### Recognize when you're stuck

**Stuck means: you've spent more than one hypothesis-test cycle without making progress.** Symptoms:

- You're mentally tracing through PPU/CPU/timer state transitions to predict what should happen at a specific dot or cycle. **Stop. Add a log line.**
- You're counting M-cycles or dots by hand to figure out when an instruction executes. **Stop. Log the dot counter at that point in the code.**
- You're unsure what value a register should have at a particular point. **Stop. Formulate the question and invoke `/research`.**
- You've written more than ~4 lines of timing analysis without citing log output. **Stop. You are guessing.**
- Your fix attempt didn't work and you're re-reading the same code trying to figure out why. **Stop. Add more logging to the area that surprised you and run again.**
- You're reading diagnostic output and can't tell whether the behavior is correct or wrong. **Stop. Write down what specific hardware behavior you need to know, and invoke `/research` with that question.**
- Your existing research documents contradict each other, or diagnostic output contradicts what a research document says. **Stop. Formulate the specific contradiction as a question and invoke `/research` to get the authoritative answer, then correct the wrong document.**

The fix for every kind of stuck is the same: either add logging and run a test, or formulate a specific question and invoke `/research`. Never reason your way out of being stuck — and never send `/research` a vague topic. Write the question down first.

#### Root cause analysis

- Study the diagnostic output to identify the root cause.
- **Update summary.md** with your hypothesis before attempting a fix.
- Fix only the identified issue. Don't refactor surrounding code.
- **Design fixes based on hardware behavior, not other emulators' code.** Research tells you *what* the hardware does (timing values, state transitions, edge cases). Your fix should implement that behavior within your existing architecture. Do not copy data structures, variable names, or architectural patterns from reference emulators — they have different designs and their implementation choices may not fit yours.
- **Validate every fix attempt with diagnostic output.** Run with logging before and after the fix. Every run must use `tee` to save to `logs/` — no exceptions, no inline filtering. If the numbers don't match expectations, add more logging rather than reasoning about why — let the output tell you what happened.
- **Remove all diagnostic logging before committing.**
- Run the full test suite after each fix: `cargo test` (also saved with `tee` to `logs/`).
- Verify no new regressions (failure count must not increase).
- **Update summary.md** after each fix attempt — whether it worked or not, how test results changed, what you'll try next if it didn't work. This must happen before you move on to anything else.

### 7. Commit

- If on a **feature branch**: commit each fix separately before moving to the next issue.
- If on **main**: ask the user whether to commit directly to main or create a feature branch first.

### 8. Receipt conventions

The receipt folder is created in step 1 and written into continuously throughout the investigation. This section documents the format and conventions.

#### Folder structure

```
receipts/investigations/<YYYY-MM-DD>-<short-name>/
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

**MANDATORY: update summary.md before every diagnostic run and after every diagnostic run.** This is a blocking prerequisite — do not invoke `cargo test` until you have written in summary.md what you are about to test and why. After the run completes, update summary.md with what the output showed before doing anything else. This applies to every single `cargo test` invocation, including quick verification runs and regression checks — not just major milestones. If you find yourself running tests without updating the summary first, you have violated this rule.

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
