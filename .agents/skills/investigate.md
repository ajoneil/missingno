# Investigate

Run a structured investigation into any technical problem — a compatibility bug, a performance issue, a crash, an architectural question, or anything else that requires systematic diagnosis.

## Discipline requirements

These rules override default agent behavior. Follow them exactly:

1. **Never run diagnostic commands directly.** All test invocations, benchmarks, profiling runs, and diagnostic commands go through `/measure`. Do not run `cargo test` or any other diagnostic command yourself — hand it to the measure skill, which handles logging, output capture, and reporting.
2. **Never skip summary.md updates.** Update summary.md after every subroutine return and before every subroutine invocation — no exceptions. This is a write-after-every-action rule, not a write-when-convenient rule. If you just read an analysis receipt, update summary.md before doing anything else. If you're about to invoke `/measure`, update summary.md with what you're testing and why first. If the last thing you did was NOT update summary.md, you have already fallen behind. The test: if context were compacted right now, would summary.md alone tell you exactly where you are and what to do next? If not, update it now.
3. **Never do ad-hoc research.** Use the `research` skill for ALL external information gathering. This includes documentation, source code from other projects, specifications, blog posts, and any `curl`, `WebFetch`, or `WebSearch` call for technical content. If you catch yourself about to fetch a URL or clone a repo, stop and invoke `/research` instead. Format every research request using the skill invocation protocol defined in AGENTS.md. **Before invoking any subroutine (`/research`, `/measure`, `/analyze`, `/hypothesize`, `/design`, `/implement`), write a return context block to summary.md** (see "Subroutine discipline" in AGENTS.md). **CRITICAL: After any subroutine returns, you MUST immediately continue the investigation in the same turn. Skill invocations are subroutine calls, not stopping points. Never end your turn after receiving a skill report. Before continuing, re-read this skill file (`.agents/skills/investigate.md`) and the investigation's `summary.md` to restore the investigate context — the subroutine's skill text will have displaced these instructions from your working memory.**
4. **Never interpret data inline.** When `/measure` or `/research` returns new data, invoke `/analyze` to interpret it. Do not reason about what measurements mean or what research findings imply in the investigate skill itself. The analyze skill writes a durable receipt of the interpretation and updates summary.md. You then re-read summary.md and continue from the updated state. **This includes after failed fixes.** When a test run shows the fix didn't work, do NOT write paragraphs about why it failed or what the results mean — invoke `/analyze` with the log file. The analyze skill will interpret the failure and update summary.md. Your only inline job is dispatching to subroutines, not thinking.
5. **Never guess at changes.** Invoke `/measure` to measure what the system is actually doing before making changes. Log output tells you what's happening — your mental model of the code is not a substitute.
6. **Never trace behavior in your head.** If you want to know what value a variable has at a specific point, or what state a system is in when a particular event occurs — invoke `/measure`. Do not manually trace execution paths, count cycles, or simulate state machines. Your mental model will be wrong. **This applies to ALL code** — this project, reference implementations, anything. If you catch yourself stepping through a state machine iteration by iteration to figure out what it does, stop and hand the question to `/research` or `/measure`. This also applies after failed fixes. If a fix attempt produces unexpected results, do not reason about why — invoke `/measure` to measure what actually happened, or `/research` to verify your understanding of the expected behavior.
7. **Never build on unverified changes.** After any code change — even "obviously correct" ones — run verification (tests, benchmarks, the relevant diagnostic) before building further changes on top. If a foundational change introduces regressions, you must know immediately — not after stacking three more changes on top. This is a blocking prerequisite: do not start the next change until the current one passes verification.
8. **Hardware is the source of truth.** The goal of every investigation is to understand what the **real hardware** does and model that behavior. Research should target hardware documentation, specifications, test ROM analysis, and hardware-level observations — not how other emulators implement things. Reference emulators can be useful as a secondary data point to confirm *what* the hardware does (e.g., confirming a timing value or state transition), but they are never the primary source and never a model to copy. The question is always "what does the hardware do?" not "what does emulator X do?"
10. **Never design fixes inline.** When you know what needs to change, invoke `/design` — do not write multi-paragraph plans in conversation about what code to modify and how. The design skill reads the architecture, reviews the code, and produces a receipt. Then invoke `/implement` to apply it. If you catch yourself writing sentences like "the simplest approach would be..." or "what if we..." or "the trick is..." — you are designing inline. Stop and invoke `/design`. The only exception is a true one-line experiment explicitly framed as a hypothesis test (not a fix attempt).
11. **Never read reference implementation source directly.** If you need to know how another project handles a behavior, formulate the question as a hardware behavior question and invoke `/research`. Do not open the file yourself, do not `grep` through it, do not `sed` or `cat` it. The research skill can consult sources and report back with the specific facts you need. Reading reference source yourself leads to rabbit holes: you read one function, then need to understand its callers, then its data structures, then you're tracing execution (violating rule 6). The research skill has scope discipline to prevent this — you don't. One question in, one answer out. **When reference source is consulted, the research report should translate implementation details into hardware behavior facts** — "the hardware does X at cycle Y" not "emulator Z implements X by doing Y".
12. **Never implement fixes directly.** When `/design` returns a design receipt, invoke `/implement` to apply it. Do not make code changes yourself — the implement skill reads the design, modifies the code, runs verification, and reports results. If you catch yourself editing source files, writing code, or running `cargo test` to verify a change — stop, you are implementing inline. Hand it to `/implement`.
13. **Never read project source code directly.** The investigate skill is a pure dispatcher — it decides which subroutine to invoke next, reads their reports, and updates summary.md. It does not read `.rs` files, `grep` through the codebase, or explore the code structure. If you need to understand how the code works, that's a `/research` question ("How does subsystem X work in this codebase?"). If you need to know what the code is doing at runtime, that's a `/measure` question. If you catch yourself opening a source file, searching for a function, or reading module structure — stop. Formulate the question and hand it to the right subroutine. The only files investigate reads are: summary.md, skill files, receipts, and analysis/design documents.

## Periodic self-check

**Every 3-4 tool calls, pause and ask yourself these questions:**

1. **Is my progress on disk?** If context were compacted right now, could you continue from `summary.md` alone? If not, stop and write. Every finding, hypothesis, measurement, and decision must be in `summary.md` or a research doc — not just in conversation history.
2. **Am I carrying stale context?** If you're relying on memory of earlier conversation turns rather than re-reading files, you're drifting. Re-read `summary.md` and your skill file. Work from the file state, not from what you remember.
3. **Am I reading anything other than receipts and summary.md?** If the last 2+ actions were file reads, grep searches, or bash commands — and the targets weren't summary.md, skill files, or receipt documents — you're doing work that belongs to a subroutine. Stop. Formulate the question and invoke the right skill.
4. **Am I tracing behavior in my head?** If you've written more than ~4 lines of state/timing/logic reasoning since the last log file, you're guessing. Invoke `/measure`.
4b. **Am I analyzing or designing inline?** If you've written more than ~3 sentences interpreting data (what results mean, why something failed, what the implications are), you're doing `/analyze`'s job inline. If you've written more than ~3 sentences about what code to change and how (approaches, tradeoffs, mechanisms), you're doing `/design`'s job inline. In either case: stop, delete what you wrote, invoke the appropriate skill. The investigate skill is a dispatcher — it decides WHICH subroutine to call next, not what the subroutine should conclude.
5. **Do I have an unanswered domain question?** If you're unsure how something is supposed to work and you're trying to figure it out by reading source code, stop. Invoke `/research` with a specific question instead.
5b. **Am I reading source code?** If any of your last 2+ tool calls read, grepped, or globbed `.rs` files (or any source files), you are violating rule 13. The investigate skill does not read source code — not this project's, not reference implementations', not anyone's. Stop immediately. Formulate the specific question you're trying to answer and invoke `/research` (for understanding code or hardware) or `/measure` (for runtime behavior). The only files you should be reading are summary.md, skill files, and receipts.
5c. **Am I framing questions in terms of hardware or in terms of other emulators?** If your research questions or hypotheses mention what another emulator does rather than what the hardware does, reframe them. "What does the hardware do when X?" not "How does emulator Y handle X?"
6. **Is my current approach making progress?** Compare where you are now to where you were 3 tool calls ago. If the answer is "I understand the problem better but haven't changed anything" for more than one cycle, you're stuck. Either invoke `/measure`, or invoke `/research`.
7. **Am I in trial-and-error mode?** If I've made a code change and re-run the test more than once without invoking `/measure` or `/research` in between, I'm guessing. A failed fix means my model is wrong — I need new information, not new code.

**The default action when uncertain is: invoke `/measure`.** Not: read more source code. Not: reason about behavior. Not: check another implementation. Measure and observe.

## Working style: hypothesize, measure, analyze

Follow this loop for every investigation step:

1. **Invoke `/hypothesize`** — generates ranked testable hypotheses, writes a receipt.
2. **Update summary.md** — read the receipt, add the top hypothesis to the RCA tree as `[ ] **bold** ← ACTIVE`.
3. **Invoke `/measure`** — hand off the active hypothesis with specific measurement points. Writes a report.
4. **Invoke `/analyze`** — hand the measure report (log file path) and summary.md. Writes an analysis receipt with confirmed/refuted/inconclusive.
5. **Update summary.md** — read the analysis receipt, mark the hypothesis in the RCA tree (`[x]` confirmed, `[x] ~~struck~~` refuted), rewrite Current understanding. This is the investigate skill's job — subroutines never touch summary.md.
6. **Re-read and continue** — re-read this skill file and summary.md. If the problem isn't solved, loop back to step 1.

The same loop applies when `/research` returns new data: invoke `/analyze` with the research document path, then update summary.md yourself, then `/hypothesize` if a new direction is needed.

**summary.md is owned exclusively by investigate.** No subroutine skill writes to it. After every subroutine return, you (investigate) read the receipt and update summary.md — typically one or two lines in the RCA tree plus a rewrite of Current understanding if the model changed.

**If you catch yourself forming hypotheses inline instead of invoking `/hypothesize`, stop.** Hand it to the skill so the reasoning is recorded in a receipt.

**If you catch yourself writing more than ~4 lines of analysis without invoking `/analyze`, stop.** You are doing interpretation inline. Hand it to `/analyze` so it's recorded in a receipt.

**If a change attempt fails and you don't know why, do not analyze the code harder.** Invoke `/measure` with more targeted logging on the specific area that surprised you, then `/analyze` to interpret the results, then `/hypothesize` to generate new hypotheses from the updated understanding.

## Workflow

### 1. Scope the problem

- Ask the developer what's wrong and what the expected vs actual behavior is.
- Once the scope is clear, propose a receipt folder name and get approval. Create the receipt folder immediately with this structure:
  ```
  receipts/investigations/<YYYY-MM-DD-HHMM>-<short-name>/
  ├── summary.md        # Create now with Status: Diagnosing
  ├── research/         # Investigation-specific notes
  ├── analysis/         # Analysis receipts (numbered chronologically)
  ├── designs/          # Design receipts
  ├── implementation/   # Implementation receipts
  ├── measurements/     # Measurement receipts
  └── logs/             # Raw diagnostic output captures
  ```
- Invoke `/measure` to run an initial diagnostic, establish the current state, and confirm the problem.
- Invoke `/analyze` to interpret the initial results.
- Classify the problem type and write it in summary.md.

**For compatibility investigations:**
- Invoke `/measure` to run the failing test and capture output.
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
- Use `/research` to understand what **hardware behavior** the test ROM is validating — what the real hardware does, not how other emulators pass the test. Research the test ROM's source code and any relevant hardware documentation or specifications.

### 3. Research correct behavior

- **Format every `/research` request using the protocol in AGENTS.md.** One question, one context block, no hypotheses or diagnostic output.
- **Frame research questions in terms of hardware behavior.** "What does the Game Boy PPU do when SCX is non-zero during mode 3?" not "How does emulator X handle SCX scrolling?" The answer should describe what the silicon does, grounded in specifications, hardware tests, and authoritative documentation. Reference emulators may be consulted as secondary evidence, but the research report must translate any implementation details into hardware behavior facts.
- **Research is not just for steps 2-3.** Any time during the investigation that you're uncertain about expected behavior — while diagnosing, while interpreting diagnostic output, while designing a change — stop and formulate a research question.
- **Use research to resolve contradictions.** If existing research documents contradict each other — or if diagnostic output contradicts what a research document claims — invoke `/research` with the specific contradiction as the question. Prefer hardware documentation and test ROM evidence over what any particular emulator does.
- **Invoke `/analyze` to interpret.** When research returns, invoke `/analyze` with the research document path and summary.md. The analyze skill writes an analysis receipt. Then update summary.md yourself with the conclusions. Do not interpret research findings inline.

### 4. Establish baseline

- Compare against a known-good state to determine if this is a regression or a pre-existing issue.
- **Update summary.md** with the baseline comparison.

**For compatibility investigations:**
- Compare against the `main` branch: `git diff main..<branch> -- <relevant files>`
- If it's a regression, diff the relevant files to narrow the scope.
- If pre-existing, still worth fixing but important to know the baseline failure count.

### 5. Instrument and diagnose

**Do not guess at changes. Do not reason through behavior in your head.** The goal is to collect precise information about what the system is actually doing vs what it should do. If you're unsure what the system is doing at a particular point, invoke `/measure` — don't try to trace through the code mentally.

**Use the `measure` skill** (`/measure`) for all diagnostic work. This includes:
- Adding targeted logging (`eprintln!`, `dbg!`, print statements, etc.)
- Running tests/benchmarks/diagnostics and capturing output to log files
- Reporting what the output shows
- Baseline comparisons (running the same measurement on a known-good vs current state)

**Format every `/measure` request using the protocol in AGENTS.md.** The Question is your hypothesis (what you expect to observe and where). The Context is which files/subsystems to instrument. The Log path is where to save output. Do not include your reasoning about what the answer might mean.

**Instrumentation is not just for step 5.** Any time during the investigation that you need to know what the system is actually doing — while diagnosing, while verifying a change, while investigating a regression — stop and invoke `/measure`.

**Invoke `/analyze` to interpret.** When `/measure` returns, invoke `/analyze` with the log file path and summary.md. The analyze skill writes an analysis receipt. Then update summary.md yourself with the conclusions. Do not interpret measure results inline.

**Every diagnostic command goes through `/measure`**, which handles log file capture. Do not run diagnostics directly.

### 6. Analyze and fix

#### Recognize when you're stuck

**Stuck means: you've spent more than one hypothesis-test cycle without making progress.** Symptoms:

- You're mentally tracing through state transitions to predict what should happen at a specific point. **Stop. Invoke `/measure`.**
- You're counting steps or cycles by hand to figure out when something executes. **Stop. Invoke `/measure` to log the actual value at that point.**
- You're unsure what value something should have at a particular point. **Stop. Formulate the question and invoke `/research`.**
- You've written more than ~4 lines of behavioral analysis without citing log output. **Stop. You are guessing. Invoke `/measure`.**
- Your change attempt didn't work and you're re-reading the same code trying to figure out why. **Stop. Invoke `/measure` with more targeted logging on the area that surprised you.**
- You're reading diagnostic output and can't tell whether the behavior is correct or wrong. **Stop. Write down what specific hardware behavior you need to understand, and invoke `/research` with that question.** Frame it as "what does the real hardware do when X?" not "what does emulator Y do when X?"
- Your existing research documents contradict each other, or diagnostic output contradicts what a research document says. **Stop. Formulate the specific contradiction as a question and invoke `/research` to get the authoritative answer, then correct the wrong document.**
- You've made more than one fix attempt without new diagnostic data between them. **Stop. The second attempt is a guess.** Go back to the hypothesis→measure loop. Invoke `/measure` to measure what the first attempt actually changed, then invoke `/research` if the measurements reveal a domain knowledge gap.

The fix for every kind of stuck is the same: either invoke `/measure` to measure what the system is doing, or invoke `/research` to learn what it should do. Never reason your way out of being stuck — and never send vague requests to either skill. Write the specific question or hypothesis down first.

#### Root cause analysis

- Study the diagnostic output to identify the root cause.
- **Update summary.md** with your hypothesis before attempting a fix.
- **Invoke `/design` before writing any fix.** The design skill reads the architectural requirements, reviews the current code and research, and produces a solution that aligns with the project's philosophy. Do not skip this step — do not design fixes inline. Format the request using the skill invocation protocol: Question (what needs to change), Context (files, research docs, summary.md path). The design skill returns a receipt.
- **Invoke `/implement` to apply the design.** The implement skill reads the design receipt, makes the code changes, runs verification, and reports results. Do not make code changes yourself — hand them to `/implement`. Format the request using the skill invocation protocol: Design (path to design receipt), Context (summary.md path). The implement skill handles: reading the code, making changes, running `cargo check`, running the test suite, removing diagnostic logging, and reporting pass/fail with test counts.
- **Update summary.md** after `/implement` returns — whether it succeeded or not, how results changed, what you'll try next if it didn't work. This must happen before you move on to anything else.

#### When a fix attempt fails

A failed fix is diagnostic data, not a prompt to tweak and retry.

**Never attempt fix N+1 without new diagnostic evidence that fix N didn't have.** If you're changing code based on the same information that produced the failed fix, you're guessing.

When a fix produces unexpected results:

1. **Stop implementing.** Do not tweak the fix. Do not stack another change on top.
2. **Record what the failure tells you.** Update summary.md: expected result, actual result, which hypothesis this invalidates.
3. **Identify the knowledge gap.** The fix failed because your model is wrong. Write the gap as a specific question.
4. **Fill the gap.** Invoke `/measure` to measure what's actually happening, or `/research` to learn what the hardware should do. Then invoke `/analyze` to interpret the new data.
5. **Redesign only after the gap is filled.** Once you've updated summary.md with the new conclusions from `/analyze`, invoke `/design` again with the corrected understanding. Do not patch the old design — the old design was based on wrong assumptions.

The loop is: **`/hypothesize` → `/measure` → `/analyze` → (repeat until confident) → `/design` → `/implement` → verify.** A failed verification sends you back to `/hypothesize`, not back to "implement with tweaks".

**For compatibility investigations:**
- **Design fixes based on hardware behavior, not other emulators' code.** The intermediate step is always understanding what the real hardware does — then modeling that behavior in your architecture. Never shortcut from "emulator X does Y" to "we should do Y". Instead: research establishes what the hardware does → design models that behavior in your architecture → implementation follows the design. Reference emulators are evidence about hardware behavior, not templates to copy. Do not copy data structures, variable names, or architectural patterns from reference emulators — they have different designs and their implementation choices may not fit yours.
- `/implement` runs the full test suite as part of its verification — check its report for regression counts.

### 7. Branch and commit hygiene

The investigate skill owns the investigation branch. The implement skill creates per-implementation branches from it and merges back on success.

**At investigation start:**
- Record the current branch as the **base branch** (usually `main`). Write it in summary.md.
- If on `main`, create an investigation branch: `git checkout -b <investigation-short-name>` (e.g., `write-conflict-flush-fix`). Ask the user for approval before creating.
- If already on a feature branch, stay on it — that's the investigation branch.

**During the investigation:**
- `/implement` creates `impl/<name>` branches from the investigation branch, commits there, and merges back on success or leaves unmerged on failure. The investigation branch always reflects the latest successful state.
- Failed implementation branches (`impl/<name>`) stay around as recoverable state. Do not delete them — they're part of the investigation record.
- If you need to return to a clean state, the investigation branch is always safe to `git checkout` back to.

**After the investigation resolves:**
- Verify the investigation branch is clean (`git status`).
- Ask the user whether to merge to the base branch (fast-forward, squash, or regular merge) or leave it for review.
- Do not force-push or rewrite history without explicit user approval.
- Do not delete `impl/*` branches — leave cleanup to the user.

### 8. Receipt conventions

The receipt folder is created in step 1 and written into continuously throughout the investigation. This section documents the format and conventions.

#### Folder structure

```
receipts/investigations/<YYYY-MM-DD-HHMM>-<short-name>/
├── summary.md        # Living investigation summary (required)
├── research/         # Investigation-specific notes
├── analysis/         # Analysis receipts (numbered chronologically)
├── designs/          # Design receipts
├── implementation/   # Implementation receipts
├── measurements/     # Measurement receipts
├── logs/             # Raw diagnostic output captures
└── ...               # Any other artifacts (diffs, screenshots, profiles)
```

Use the date and time (to the minute) of the investigation and a short kebab-case name describing the issue (e.g. `2026-02-13-1430-stat-mode0-timing`, `2026-02-13-0915-slow-test-builds`).

#### Two research locations

General domain knowledge should already be in `receipts/research/` — you documented it using the `research` skill during steps 2 and 3. The session's `research/` folder is for investigation-specific notes only: test analysis, diagnostic interpretations, hypotheses for this particular problem. If a future investigation into the same area would benefit from a finding, it belongs in `receipts/research/`, not here.

#### summary.md

`summary.md` is a **lightweight dashboard**, not a log. A developer should be able to read it in under a minute and know exactly where things stand. Details live in receipts — summary.md points to them.

**Keep it short.** If summary.md is growing past ~60 lines, you're putting too much in it. Move detail into receipts and replace it with a link.

**Update it before every subroutine invocation and after every subroutine return.** This is a blocking prerequisite — the subroutine invocation or next action does not happen until summary.md is written. The update should be a line or two — not paragraphs. The cadence is: every single action the investigate skill takes should be bracketed by summary.md writes. Read receipt → update summary → invoke next skill. Skill returns → read receipt → update summary → decide next step. If you're ever unsure whether to update, update.

##### Format

```markdown
# Investigation: <title>

## Status
<one line: diagnosing | fix in progress | resolved | blocked>

## Problem
<2-3 sentences: what's wrong, what should happen instead>

## Baseline
<test counts: N pass, N fail, N ignored out of N total>

## Root cause analysis

<A tree of hypotheses. This is the heart of the investigation.
Each node is a hypothesis with a status. Indent children under parents.
Cross off dead ends. Mark the active line of inquiry.>

- [ ] **Write-conflict accumulation window** — expand from 5→9 pending dots ← ACTIVE
  - [x] Pixel offset confirmed as exactly 4 (`logs/09-pixel-offset-measurement.log`)
  - [ ] Design: fixed pre-write flush + expanded window (`designs/05-write-conflict-offset.md`)
- [x] ~~Pipeline latency mismatch at PPU rendering level~~ — FIFO-empty gap blocks all PPU-side approaches (`designs/04-startup-suppression.md`)
  - [x] ~~PipelineFill phase during second startup fetch~~ — FIFO empty, no-op (`logs/07-pipeline-fill-fifo-state.log`)
  - [x] ~~position_in_line +4 at mode 3 start~~ — doesn't affect write-conflict timing, breaks sprites (`analysis/10-position-in-line-failure.md`)
  - [x] ~~Shorten startup to 8 dots + suppress pixels_drawn~~ — structurally unviable (`designs/04-startup-suppression.md`)

## Current understanding
<2-4 sentences: the best working model right now. What you believe
is the root cause and what approach is being pursued. No history,
no dead ends — just the current state of knowledge.>

## Active subroutine
<return context block when a subroutine is in flight — see AGENTS.md>
```

##### Rules

- **Root cause analysis tree is mandatory.** Start it after the first measurement. Update it after every `/analyze` return. Every hypothesis goes in the tree — confirmed, refuted, or active. This is the primary navigation structure for the investigation.
- **Active hypothesis goes first.** The active line of inquiry must be the first entry in the tree so it's immediately visible. Refuted hypotheses sink to the bottom. When a hypothesis is refuted, move it (and its children) below the active line. When a new hypothesis becomes active, move it to the top.
- **Use `[x] ~~struck~~` for refuted hypotheses.** Include a short reason and a receipt link. Do not delete refuted hypotheses — they document dead ends.
- **Use `[ ] **bold**` for the active hypothesis.** Mark it with `← ACTIVE`. There should be exactly one at any time.
- **Use `[x]` (no strike) for confirmed findings** that support the active line but aren't hypotheses themselves.
- **Indent child hypotheses** under their parent. A refuted parent means all children are implicitly dead.
- **Current understanding is a snapshot, not a history.** It should read as "here's what we know right now" — not "first we discovered X, then Y". Rewrite it from scratch when the model changes rather than appending.
- **No "What's been tried" log.** The RCA tree captures this. Each crossed-off hypothesis IS a record of what was tried. Receipt links provide detail.
- **No duplicating receipt content.** If a finding is documented in an analysis receipt, link to it — don't reproduce the finding in summary.md.

### 9. Commit format

```
Short summary of what changed

Detailed explanation of:
- What the problem was (observable symptom)
- Why it happened (root cause)
- How the fix works (what changed and why it's correct)

Fixes <test_name> / Resolves <issue>.
```
