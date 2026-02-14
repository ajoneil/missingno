# Investigate

Run a structured investigation into any technical problem — a compatibility bug, a performance issue, a crash, an architectural question, or anything else that requires systematic diagnosis.

## Discipline requirements

These rules override default agent behavior. Follow them exactly:

1. **Never run diagnostic commands without `tee` to a log file.** Every test invocation, benchmark, profiling run, or diagnostic command must be saved. No exceptions. No piping through `grep`/`tail`/`head` instead of saving. For test suites: `cargo test ... 2>&1 | tee <log_path>`. For other commands: `<command> 2>&1 | tee <log_path>`.
2. **Never skip summary.md updates.** Update it before and after every diagnostic run and every change attempt. If you're about to run a diagnostic, write in summary.md what you're testing and why first.
3. **Never do ad-hoc research.** Use the `research` skill for ALL external information gathering. This includes documentation, source code from other projects, specifications, blog posts, and any `curl`, `WebFetch`, or `WebSearch` call for technical content. If you catch yourself about to fetch a URL or clone a repo, stop and invoke `/research` instead. Format every research request using the skill invocation protocol defined in AGENTS.md. When research returns, **you** interpret the findings — the research skill reports facts; you figure out what they mean. **CRITICAL: After `/research` or `/instrument` returns, you MUST immediately continue the investigation in the same turn — read the callee's output, interpret it, update summary.md, and proceed to the next step. Skill invocations are subroutine calls, not stopping points. Never end your turn after receiving a skill report. Before continuing, re-read this skill file (`.agents/skills/investigate.md`) and the investigation's `summary.md` to restore the investigate context — the subroutine's skill text will have displaced these instructions from your working memory.**
4. **Never guess at changes.** Invoke `/instrument` to measure what the system is actually doing before making changes. Log output tells you what's happening — your mental model of the code is not a substitute.
5. **Never trace behavior in your head.** If you want to know what value a variable has at a specific point, or what state a system is in when a particular event occurs — invoke `/instrument`. Do not manually trace execution paths, count cycles, or simulate state machines. Your mental model will be wrong. **This applies to ALL code** — this project, reference implementations, anything. If you catch yourself stepping through a state machine iteration by iteration to figure out what it does, stop and hand the question to `/research` or `/instrument`.
6. **Never build on unverified changes.** After any code change — even "obviously correct" ones — run verification (tests, benchmarks, the relevant diagnostic) before building further changes on top. If a foundational change introduces regressions, you must know immediately — not after stacking three more changes on top. This is a blocking prerequisite: do not start the next change until the current one passes verification.
7. **Never read reference implementation source directly.** If you need to know how another project implements a behavior, formulate the question and invoke `/research`. Do not open the file yourself, do not `grep` through it, do not `sed` or `cat` it. The research skill can clone repos, read source, and report back with the specific facts you need. Reading reference source yourself leads to rabbit holes: you read one function, then need to understand its callers, then its data structures, then you're tracing execution (violating rule 5). The research skill has scope discipline to prevent this — you don't. One question in, one answer out.

## Periodic self-check

**Every 3-4 tool calls, pause and ask yourself these questions:**

1. **Am I running diagnostics or just reading code?** If the last 3+ actions were all file reads, grep searches, or bash commands reading source — you're in an analysis loop. Break out: form a hypothesis, invoke `/instrument`, run the diagnostic.
2. **Am I tracing behavior in my head?** If you've written more than ~4 lines of state/timing/logic reasoning since the last log file, you're guessing. Invoke `/instrument`.
3. **Do I have an unanswered domain question?** If you're unsure how something is supposed to work and you're trying to figure it out by reading source code, stop. Invoke `/research` with a specific question instead.
3b. **Am I reading a reference implementation?** If any of your last 2+ tool calls read, grepped, or fetched files from a reference project, you are in a rabbit hole. Stop immediately. Formulate the specific question you're trying to answer and invoke `/research`. You should never need more than one glance at reference source — if one excerpt didn't answer your question, the answer requires deeper analysis that `/research` is better equipped to do with scope discipline.
4. **Is my current approach making progress?** Compare where you are now to where you were 3 tool calls ago. If the answer is "I understand the problem better but haven't changed anything" for more than one cycle, you're stuck. Either invoke `/instrument`, or invoke `/research`.
5. **Have I updated summary.md recently?** If not, update it now. The act of writing down where you are often clarifies what to do next.

**The default action when uncertain is: invoke `/instrument`.** Not: read more source code. Not: reason about behavior. Not: check another implementation. Measure and observe.

## Working style: hypothesize, test, interpret

Follow this loop for every investigation step:

1. **Form a hypothesis** — a short, testable statement. Write it down in summary.md.
2. **Invoke `/instrument`** — hand off the hypothesis with specific measurement points. The instrument skill adds targeted logging/measurement, runs the diagnostic, and reports what the output shows.
3. **Read the results** — extract the specific values that answer your hypothesis from `/instrument`'s report.
4. **Update summary.md** — state whether the hypothesis was confirmed or refuted, cite the log evidence, and write the next hypothesis.

**If you catch yourself writing more than ~4 lines of analysis without a log file open, stop.** You are guessing. Invoke `/instrument` instead.

**If a change attempt fails and you don't know why, do not analyze the code harder.** Invoke `/instrument` with more targeted logging on the specific area that surprised you.

## Workflow

### 1. Scope the problem

- Ask the developer what's wrong and what the expected vs actual behavior is.
- Once the scope is clear, propose a receipt folder name and get approval. Create the receipt folder immediately with this structure:
  ```
  receipts/investigations/<YYYY-MM-DD>-<short-name>/
  ├── summary.md        # Create now with Status: Diagnosing
  ├── research/         # Investigation-specific notes
  └── logs/             # Diagnostic output captures
  ```
- Run an initial diagnostic to establish the current state and confirm the problem.
- Classify the problem type and write it in summary.md.

**For compatibility investigations:**
- If a test name is given, run it with output visible: `cargo test <test_name> -- --nocapture`
- Classify the failure type:
  - **Register mismatch**: Expected vs actual CPU/hardware register values after test execution.
  - **Screenshot mismatch**: Pixel differences between rendered output and reference image.
  - **Timeout/hang**: The ROM never reached a halt condition — likely wrong control flow or missing hardware behavior.

### 2. Understand the domain

- Identify the subsystem or area involved.
- **Use the `research` skill** (`/research`) to fill any domain knowledge gaps — specifications, documentation, expected behavior from authoritative sources.
- **Update summary.md** with the problem description and domain context.
- Capture investigation-specific notes in the session's `research/` folder.

**For compatibility investigations:**
- Identify the hardware subsystem (video/PPU, audio/APU, timers, interrupts, memory mapping, DMA, input, serial, etc.).
- Use `/research` to find and read the test ROM's source code, understand what hardware behavior is being validated, and document findings.

### 3. Research correct behavior

- **Format every `/research` request using the protocol in AGENTS.md.** One question, one context block, no hypotheses or diagnostic output.
- **Research is not just for steps 2-3.** Any time during the investigation that you're uncertain about expected behavior — while diagnosing, while interpreting diagnostic output, while designing a change — stop and formulate a research question.
- **Use research to resolve contradictions.** If existing research documents contradict each other — or if diagnostic output contradicts what a research document claims — invoke `/research` with the specific contradiction as the question.
- **Interpret the report yourself.** When research returns, read the Findings section and figure out what it means for your investigation. The research skill reports facts — you own the interpretation. Update summary.md with both the findings and your interpretation.

### 4. Establish baseline

- Compare against a known-good state to determine if this is a regression or a pre-existing issue.
- **Update summary.md** with the baseline comparison.

**For compatibility investigations:**
- Compare against the `main` branch: `git diff main..<branch> -- <relevant files>`
- If it's a regression, diff the relevant files to narrow the scope.
- If pre-existing, still worth fixing but important to know the baseline failure count.

### 5. Instrument and diagnose

**Do not guess at changes. Do not reason through behavior in your head.** The goal is to collect precise information about what the system is actually doing vs what it should do. If you're unsure what the system is doing at a particular point, invoke `/instrument` — don't try to trace through the code mentally.

**Use the `instrument` skill** (`/instrument`) for all diagnostic work. This includes:
- Adding targeted logging (`eprintln!`, `dbg!`, print statements, etc.)
- Running tests/benchmarks/diagnostics and capturing output to log files
- Reporting what the output shows
- Baseline comparisons (running the same measurement on a known-good vs current state)

**Format every `/instrument` request using the protocol in AGENTS.md.** The Question is your hypothesis (what you expect to observe and where). The Context is which files/subsystems to instrument. The Log path is where to save output. Do not include your reasoning about what the answer might mean.

**Instrumentation is not just for step 5.** Any time during the investigation that you need to know what the system is actually doing — while diagnosing, while verifying a change, while investigating a regression — stop and invoke `/instrument`.

**Interpret the report yourself.** When `/instrument` returns, read the Measurements section and figure out what it means for your investigation. The instrument skill reports what happened — you own the interpretation. Update summary.md with both the measurements and your interpretation.

**MANDATORY: Every diagnostic command invocation must be saved to a log file.** This applies whether you run the command yourself or hand it to `/instrument`. No exceptions.

### 6. Analyze and fix

#### Recognize when you're stuck

**Stuck means: you've spent more than one hypothesis-test cycle without making progress.** Symptoms:

- You're mentally tracing through state transitions to predict what should happen at a specific point. **Stop. Invoke `/instrument`.**
- You're counting steps or cycles by hand to figure out when something executes. **Stop. Invoke `/instrument` to log the actual value at that point.**
- You're unsure what value something should have at a particular point. **Stop. Formulate the question and invoke `/research`.**
- You've written more than ~4 lines of behavioral analysis without citing log output. **Stop. You are guessing. Invoke `/instrument`.**
- Your change attempt didn't work and you're re-reading the same code trying to figure out why. **Stop. Invoke `/instrument` with more targeted logging on the area that surprised you.**
- You're reading diagnostic output and can't tell whether the behavior is correct or wrong. **Stop. Write down what specific behavior you need to understand, and invoke `/research` with that question.**
- Your existing research documents contradict each other, or diagnostic output contradicts what a research document says. **Stop. Formulate the specific contradiction as a question and invoke `/research` to get the authoritative answer, then correct the wrong document.**

The fix for every kind of stuck is the same: either invoke `/instrument` to measure what the system is doing, or invoke `/research` to learn what it should do. Never reason your way out of being stuck — and never send vague requests to either skill. Write the specific question or hypothesis down first.

#### Root cause analysis

- Study the diagnostic output to identify the root cause.
- **Update summary.md** with your hypothesis before attempting a fix.
- Fix only the identified issue. Don't refactor surrounding code.
- **Validate every change with diagnostic output.** Invoke `/instrument` to run with logging before and after the change. If the numbers don't match expectations, invoke `/instrument` again with more targeted logging rather than reasoning about why — let the output tell you what happened.
- **Remove all diagnostic logging before committing.**
- Run the full verification suite after each change (also saved with `tee` to `logs/`).
- Verify no new regressions.
- **Update summary.md** after each change attempt — whether it worked or not, how results changed, what you'll try next if it didn't work. This must happen before you move on to anything else.

**For compatibility investigations:**
- **Design fixes based on hardware behavior, not other emulators' code.** Research tells you *what* the hardware does (timing values, state transitions, edge cases). Your fix should implement that behavior within your existing architecture. Do not copy data structures, variable names, or architectural patterns from reference emulators — they have different designs and their implementation choices may not fit yours.
- Run the full test suite after each fix: `cargo test` (saved with `tee` to `logs/`).
- Verify no new regressions (failure count must not increase).

### 7. Commit

- If on a **feature branch**: commit each fix separately before moving to the next issue.
- If on **main**: ask the user whether to commit directly to main or create a feature branch first.

### 8. Receipt conventions

The receipt folder is created in step 1 and written into continuously throughout the investigation. This section documents the format and conventions.

#### Folder structure

```
receipts/investigations/<YYYY-MM-DD>-<short-name>/
├── summary.md        # Living investigation summary (required)
├── research/         # Investigation-specific notes
├── logs/             # Diagnostic output captures
└── ...               # Any other artifacts (diffs, screenshots, profiles)
```

Use the date of the investigation and a short kebab-case name describing the issue (e.g. `2026-02-13-stat-mode0-timing`, `2026-02-13-slow-test-builds`).

#### Two research locations

General domain knowledge should already be in `receipts/research/` — you documented it using the `research` skill during steps 2 and 3. The session's `research/` folder is for investigation-specific notes only: test analysis, diagnostic interpretations, hypotheses for this particular problem. If a future investigation into the same area would benefit from a finding, it belongs in `receipts/research/`, not here.

#### summary.md

`summary.md` is a living document — the developer should be able to read it at any point and understand exactly where the investigation stands. Update it as you work, not at the end.

**MANDATORY: update summary.md before every diagnostic run and after every diagnostic run.** This is a blocking prerequisite — do not invoke a diagnostic command until you have written in summary.md what you are about to test and why. After the run completes, update summary.md with what the output showed before doing anything else. This applies to every single diagnostic invocation, including quick verification runs and regression checks — not just major milestones. If you find yourself running diagnostics without updating the summary first, you have violated this rule.

Include:
- **Status**: Current investigation state (e.g. "diagnosing", "fix in progress", "resolved", "blocked")
- **Problem**: What's wrong and what the expected behavior is
- **What's been tried**: Diagnostic approaches, hypotheses tested, change attempts — even failed ones
- **Findings**: What diagnostic output revealed, root cause if known
- **Resolution**: What was changed and what now works (once fixed)
- **Remaining**: Any related issues or open questions

### 9. Commit format

```
Short summary of what changed

Detailed explanation of:
- What the problem was (observable symptom)
- Why it happened (root cause)
- How the fix works (what changed and why it's correct)

Fixes <test_name> / Resolves <issue>.
```
