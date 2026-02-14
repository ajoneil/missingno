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
4. **Never guess at fixes.** Invoke `/instrument` to measure what the emulator is actually doing. The log files tell you what's happening — your mental model of the code is not a substitute.
5. **Never trace execution in your head.** If you want to know what value a register has at a specific dot, or what mode the PPU is in when a particular instruction executes — invoke `/instrument`. Do not manually count M-cycles, dots, or pipeline stages. Your mental model will be wrong. The emulator is already a cycle-accurate simulator; let it simulate. **This applies to ALL code** — this emulator, reference emulators, anything. If you catch yourself stepping through a reference emulator's state machine iteration by iteration to figure out what it does, you are doing the same thing as tracing timing in your head. Stop and hand the question to `/research`.
6. **Never build on unverified changes.** After any code change — even "obviously correct" ones — run the full test suite (`cargo test`) before building further changes on top. If a foundational change (e.g. LY timing, mode transitions) introduces regressions, you must know immediately — not after stacking three more changes on top. This is a blocking prerequisite: do not start the next change until the current one passes regression checks.
7. **Never read reference emulator source directly.** If you need to know how another emulator implements a behavior, formulate the question and invoke `/research`. Do not open the file yourself, do not `grep` through it, do not `sed` or `cat` it. The research skill can clone repos, read source, and report back with the specific facts you need. Reading reference source yourself leads to rabbit holes: you read one function, then need to understand its callers, then its data structures, then you're tracing execution (violating rule 5). The research skill has scope discipline to prevent this — you don't. One question in, one answer out.

## Periodic self-check

**Every 3-4 tool calls, pause and ask yourself these questions:**

1. **Am I running tests or reading code?** If the last 3+ actions were all file reads, grep searches, or bash commands reading emulator source — you're in an analysis loop. Break out: form a hypothesis, invoke `/instrument`, run the test.
2. **Am I tracing timing in my head?** If you've written more than ~4 lines of cycle/timing reasoning since the last log file, you're guessing. Invoke `/instrument`.
3. **Do I have an unanswered hardware question?** If you're unsure what the hardware does and you're trying to figure it out by reading emulator source code, stop. Invoke `/research` with a specific question instead. Reference emulators are the LAST resort, not the first.
3b. **Am I reading a reference emulator?** If any of your last 2+ tool calls read, grepped, or fetched files from a reference emulator (SameBoy, Gambatte, etc.), you are in a rabbit hole. Stop immediately. Formulate the specific question you're trying to answer and invoke `/research`. You should never need more than one glance at reference source — if one excerpt didn't answer your question, the answer requires deeper analysis that `/research` is better equipped to do with scope discipline.
4. **Is my current approach making progress?** Compare where you are now to where you were 3 tool calls ago. If the answer is "I understand the problem better but haven't changed anything" for more than one cycle, you're stuck. Either invoke `/instrument`, or invoke `/research`.
5. **Have I updated summary.md recently?** If not, update it now. The act of writing down where you are often clarifies what to do next.

**The default action when uncertain is: invoke `/instrument`.** Not: read more source code. Not: reason about timing. Not: check another emulator. Measure and observe.

## Working style: hypothesize, test, interpret

Follow this loop for every investigation step:

1. **Form a hypothesis** — a short, testable statement. Write it down in summary.md. ("The STAT read in the counting loop sees mode 3 when it should see mode 0 because mode 3 is 4 dots too long.")
2. **Invoke `/instrument`** — hand off the hypothesis with specific measurement points. The instrument skill adds targeted logging, runs the test, and reports what the output shows.
3. **Read the results** — extract the specific values that answer your hypothesis from `/instrument`'s report.
4. **Update summary.md** — state whether the hypothesis was confirmed or refuted, cite the log evidence, and write the next hypothesis.

**If you catch yourself writing more than ~4 lines of timing/cycle analysis without a log file open, stop.** You are guessing. Invoke `/instrument` instead.

**If a fix attempt fails and you don't know why, do not analyze the code harder.** Invoke `/instrument` with more targeted logging on the specific area that surprised you.

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

### 5. Instrument and diagnose

**Do not guess at fixes. Do not reason through timing in your head.** The goal is to collect precise information about what the emulator is actually doing vs what it should do. If you're unsure what the emulator is doing at a particular point, invoke `/instrument` — don't try to trace through the code mentally.

**Use the `instrument` skill** (`/instrument`) for all diagnostic work. This includes:
- Adding `eprintln!` tracing to the emulator code
- Running the test and capturing output to log files
- Reporting what the output shows
- Baseline comparisons (running the same logging on main vs current branch)

**How to hand off instrumentation requests:**
- Formulate a **specific hypothesis** before invoking `/instrument`. Not "add some logging" but "I need to know what STAT mode the CPU sees at the exact dot when the polling loop's first LDH A,(STAT) executes on LY=68."
- Include **the hypothesis, the measurement points, and the log file path**. The instrument skill handles the mechanics (where to add log lines, how to gate output, running the test); you provide the question.
- **One measurement per invocation.** If you need to measure mode 3 length AND interrupt timing AND register values at a specific point, those can be one invocation if they're all part of the same hypothesis. But if you have two unrelated hypotheses, invoke `/instrument` twice.
- When `/instrument` returns, **you** interpret the measurements in context of your investigation. The instrument skill reports what happened; you figure out what it means.

**Instrumentation is not just for step 5.** Any time during the investigation that you need to know what the emulator is actually doing — while diagnosing, while verifying a fix, while investigating a regression — stop and invoke `/instrument`. If you find yourself reasoning about what value a register has at a particular dot, or what mode the PPU is in when a particular instruction executes, that's a signal to instrument instead.

**Instrumentation is a subroutine.** After `/instrument` returns with measurements, your very next action must be interpreting those measurements in context of your investigation — updating summary.md, adjusting your hypothesis, editing code. Never end your turn immediately after instrumentation. The pattern is: instrument → interpret findings → act → continue investigation.

**MANDATORY: Every `cargo test` invocation must be saved to a log file.** This applies whether you run the test yourself or hand it to `/instrument`. No exceptions. No piping through `grep`/`tail`/`head` instead of saving.

### 6. Analyze and fix

#### Recognize when you're stuck

**Stuck means: you've spent more than one hypothesis-test cycle without making progress.** Symptoms:

- You're mentally tracing through PPU/CPU/timer state transitions to predict what should happen at a specific dot or cycle. **Stop. Invoke `/instrument`.**
- You're counting M-cycles or dots by hand to figure out when an instruction executes. **Stop. Invoke `/instrument` to log the dot counter at that point.**
- You're unsure what value a register should have at a particular point. **Stop. Formulate the question and invoke `/research`.**
- You've written more than ~4 lines of timing analysis without citing log output. **Stop. You are guessing. Invoke `/instrument`.**
- Your fix attempt didn't work and you're re-reading the same code trying to figure out why. **Stop. Invoke `/instrument` with more targeted logging on the area that surprised you.**
- You're reading diagnostic output and can't tell whether the behavior is correct or wrong. **Stop. Write down what specific hardware behavior you need to know, and invoke `/research` with that question.**
- Your existing research documents contradict each other, or diagnostic output contradicts what a research document says. **Stop. Formulate the specific contradiction as a question and invoke `/research` to get the authoritative answer, then correct the wrong document.**

The fix for every kind of stuck is the same: either invoke `/instrument` to measure what the emulator is doing, or invoke `/research` to learn what the hardware should do. Never reason your way out of being stuck — and never send vague requests to either skill. Write the specific question or hypothesis down first.

#### Root cause analysis

- Study the diagnostic output to identify the root cause.
- **Update summary.md** with your hypothesis before attempting a fix.
- Fix only the identified issue. Don't refactor surrounding code.
- **Design fixes based on hardware behavior, not other emulators' code.** Research tells you *what* the hardware does (timing values, state transitions, edge cases). Your fix should implement that behavior within your existing architecture. Do not copy data structures, variable names, or architectural patterns from reference emulators — they have different designs and their implementation choices may not fit yours.
- **Validate every fix attempt with diagnostic output.** Invoke `/instrument` to run with logging before and after the fix. If the numbers don't match expectations, invoke `/instrument` again with more targeted logging rather than reasoning about why — let the output tell you what happened.
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
