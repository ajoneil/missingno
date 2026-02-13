# Improve Compatibility

Fix a compatibility bug against a test ROM or real game.

## Workflow

### 1. Identify the failure

- Ask the developer what test or game is failing and what the symptom is.
- Once the scope is clear, propose a receipt folder name and get approval (see step 8 for structure). Create the folder and `summary.md` immediately with Status: Diagnosing.
- If a test name is given, run it with output visible: `cargo test <test_name> -- --nocapture`
- Classify the failure type:
  - **Register mismatch**: Expected vs actual CPU/hardware register values after test execution.
  - **Screenshot mismatch**: Pixel differences between rendered output and reference image.
  - **Timeout/hang**: The ROM never reached a halt condition — likely wrong control flow or missing hardware behavior.

### 2. Understand what's being tested

- Search for the test ROM's source code online (many test suites publish assembly sources on GitHub).
- Read the source to understand what specific hardware behavior is being validated.
- Identify the subsystem involved (video/PPU, audio/APU, timers, interrupts, memory mapping, DMA, input, serial, etc.).
- **Update summary.md** with the problem description and subsystem identified.
- **Write a research document immediately** to `research/` capturing what you learned from the test source — what it measures, expected values, and any timing/behavioral details encoded in the test data. Do this now, not later. This is the first research artifact of the investigation.

### 3. Research the correct hardware behavior

- **Consult technical documentation** for the target platform (hardware manuals, community wikis, reverse-engineering docs).
- **Study reference emulator implementations** with permissive licenses (MIT, Apache, zlib, public domain) to understand how others handle the same edge case. Search GitHub for well-known accurate emulators of the target system.
- **Read test ROM documentation** — many test suites include comments explaining expected timing, register states, or behavioral details.
- Cross-reference multiple sources to build confidence in what the correct behavior should be.
- **Update summary.md** with research findings.
- **Write research documents to `research/` as you go** — one file per significant finding, written immediately when you learn something, not deferred to the end. Each document should summarize the behavior, cite the source (URL, doc name, emulator repo + file path), and note any ambiguities or conflicts between sources. If you consult Pan Docs and a reference emulator in the same step, that's two research documents. Examples:
  - `research/mode3-sprite-penalty.md` — how sprites extend mode 3, citing Pan Docs and test ROM data
  - `research/sameboy-sprite-handling.md` — how SameBoy implements the same behavior, citing specific file and line numbers

### 4. Verify regression vs pre-existing

- Compare against the `main` branch to determine if this is a new regression or a pre-existing gap.
- If it's a regression, diff the relevant files to narrow the scope: `git diff main..<branch> -- <relevant files>`
- If pre-existing, still worth fixing but important to know the baseline failure count.
- **Update summary.md** with regression/pre-existing classification.

### 5. Build a diagnostic test harness

**Do not guess at fixes.** The goal is to collect precise information about what the emulator is actually doing vs what it should do. Build temporary diagnostic instrumentation:

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

- **Save all diagnostic output** to the receipt folder. **Update summary.md** with what was instrumented and what the output revealed.

### 6. Analyze and fix

- Study the diagnostic output to identify the root cause.
- Fix only the identified issue. Don't refactor surrounding code.
- **Remove all diagnostic logging before committing.**
- Run the full test suite after each fix: `cargo test`
- Verify no new regressions (failure count must not increase).
- **Update summary.md** with root cause, what was changed, and test results. Set status to "resolved" or "blocked".

### 7. Commit

- If on a **feature branch**: commit each fix separately before moving to the next issue.
- If on **main**: ask the user whether to commit directly to main or create a feature branch first.

### 8. Write receipt

Before starting the investigation, propose a folder name to the developer and get their approval. Then create the investigation folder:

```
receipts/improve-compatibility/<YYYY-MM-DD>-<short-name>/
├── summary.md        # Investigation summary (required)
├── research/         # Hardware behavior findings with sources
├── logs/             # Diagnostic output captures
└── ...               # Any other artifacts (diffs, screenshots)
```

Use the date of the investigation and a short kebab-case name describing the issue (e.g. `2026-02-13-stat-mode0-timing`, `2026-02-13-mbc1-bank-wrap`).

**The `research/` folder should already be populated** — you wrote research documents during steps 2 and 3 as you learned about the hardware behavior. At this point, review what's there and fill any gaps. The goal is that a future investigation into the same subsystem can start from these documents instead of re-researching from scratch.

**Diagnostic logs are the most important artifact.** Save every diagnostic run to the `logs/` folder so the work isn't repeated. Name each log file descriptively so you can tell them apart later — include what was being traced and which branch/state produced it. For example:
- `logs/ppu-mode-timing-main.log` — baseline output from main branch
- `logs/ppu-mode-timing-fix-attempt-1.log` — output after first fix attempt
- `logs/stat-irq-edges-failing.log` — capture of the bug in action

`summary.md` is a living document — create it at the start of the investigation and keep it updated as you go. This is how you communicate progress to the developer. It should always reflect the current state, not just the final outcome.

**Update `summary.md` every time you learn something new** — not just at step boundaries. Specifically:
- After each diagnostic run: what you instrumented, what the output showed
- After forming a hypothesis: what you think the bug is and why
- After each fix attempt: what you changed, whether it worked, how mismatch counts changed
- After hitting a dead end: what you tried, why it didn't pan out
- After any significant observation: patterns in the data, connections between symptoms

The developer should be able to read `summary.md` at any point and understand exactly where the investigation stands, what's been learned, and what's being tried next.

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

## Research strategy

When investigating hardware behavior:

1. **Project docs first**: Check AGENTS.md, README, and existing commit messages — they often document hardware edge cases already discovered.
2. **Platform technical docs**: Search for the definitive hardware reference for the target system.
3. **Test suite sources**: Find the assembly/source for the failing test to understand exactly what it measures.
4. **Reference emulators**: Search GitHub for accurate emulators of the target platform with open licenses. Study how they implement the specific behavior in question. Prefer emulators known for accuracy over speed.
5. **Community resources**: Forums, wikis, and blog posts from the emulation development community often document obscure hardware quirks.
