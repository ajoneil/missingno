# Investigate

Run a structured investigation into any technical problem — a compatibility bug, a performance issue, a crash, an architectural question, or anything else that requires systematic diagnosis.

## Discipline requirements

The investigate skill is a **pure dispatcher**. It decides which subroutine to invoke, reads their receipts, and updates summary.md. It does NOT read source code, trace behavior, interpret data, design fixes, or implement changes.

### What investigate does directly
- Update `summary.md` (before every dispatch, after every return — no exceptions)
- Run `./scripts/test-report.sh --diff` (bookkeeping, not instrumentation)
- Read receipts, skill files, and summary.md

### What gets delegated (always)
| Need | Skill | Mode |
|------|-------|------|
| External info (docs, specs, URLs, other projects) | `/research` | subagent |
| Runtime observation (values, state, timing) | `/inspect` or `/compare-traces` | subagent |
| Code instrumentation (logging, eprintln) | `/instrument` | subagent |
| Interpreting data or measurements | `/analyze` | subagent |
| Reading source code (this project or others) | `/research` or `/inspect` | subagent |
| Generating testable hypotheses | `/hypothesize` | in-context |
| Planning code changes | `/design` | in-context |
| Making code changes | `/implement` | in-context |

**Subagent skills** are fact-finding tasks that produce large diagnostic outputs (logs, source reads, measurement data) — running them as Task subagents keeps that noise out of main context.

**In-context skills** are synthesis tasks where conversation continuity is load-bearing — the design rationale, the user's clarifications, what was just tried. They run on the main agent under their own scope discipline.

Both flavors produce a receipt as their durable deliverable; both require a Question/Context brief written into summary.md before invocation.

### Critical rules
1. **summary.md before and after every dispatch.** If context compacted right now, could you continue from summary.md alone?
2. **Skills are subroutine calls, not stopping points.** After a subagent returns or an in-context skill exits, immediately read the receipt, update summary.md, and continue in the same turn.
3. **Never trace behavior in your head.** If you need a value, state, or timing — observe it via `/inspect`. Questions requiring cycle-counting are `/inspect` questions, not `/research` questions.
4. **Never build on unverified changes.** Run verification after every code change before stacking more changes.
5. **Hardware is the source of truth.** Research targets hardware behavior, not emulator implementations. Frame all questions as "what does the hardware do?" not "what does emulator X do?"
6. **Never cargo-cult values from reference emulators.** Before using any externally-sourced value, understand what it controls in *our* code via `/research`. A value correct for the hardware may need a different mechanism in our architecture.
7. **Never design or interpret inline.** If you're writing >3 sentences about what code to change → invoke `/design`. If you're writing >3 sentences about what results mean → invoke `/analyze`.

## Dispatch mechanism

Skills come in two flavors. Both produce a receipt file as the durable deliverable; both require a Question/Context brief in summary.md before invocation.

### Subagent skills — `/research`, `/analyze`, `/instrument`, `/inspect`, `/compare-traces`

These run as Task subagents (`subagent_type: "general-purpose"`). They produce large diagnostic outputs (file reads, source exploration, measurement data, test output) that would pollute the main context. Running them as subagents keeps that noise on disk in the receipt and out of conversation memory.

Subagents operate in the same working directory. Logging added and removed by instrument persists. Debugger sessions launched by inspect run and complete within the subagent.

To dispatch a subagent skill:

1. Update summary.md with what you're invoking and why.
2. Read the skill file from `.agents/skills/<skill>.md`.
3. Launch a Task. The prompt must include:
   - The full content of the skill file (paste it into the prompt so the subagent has the instructions).
   - The skill arguments (Question, Context, output path) formatted per the skill invocation protocol in AGENTS.md.
   - The path to the investigation's summary.md (for skills that need it).
4. When the Task returns, read the receipt file it produced.
5. Update summary.md with the findings.
6. Continue the investigation.

### In-context skills — `/hypothesize`, `/design`, `/implement`

These run on the main agent. They are synthesis tasks where conversation continuity (your reasoning, the user's clarifications, mid-flight course corrections) is load-bearing. A subagent has to reconstitute that from a brief; the main agent already has it.

To invoke an in-context skill:

1. Update summary.md with what you're invoking and why, including the brief (Question/Context, or Design/Context for `/implement`).
2. Read the skill file from `.agents/skills/<skill>.md`. Switch into that skill's mode and follow its scope discipline strictly — exit `/investigate` mode for the duration.
3. Produce the receipt at the path the skill specifies. The skill's "After ... is complete" section explains what to do at the end.
4. On exit, read your own receipt back if needed (it survives compaction; conversation memory does not), update summary.md, and resume the investigation.

The in-context skill files contain scope-discipline rules (e.g. "do not investigate while designing", "do not design while implementing"). These rules are critical — the main agent must follow them as strictly as a subagent would, and the only thing keeping you honest is re-reading the skill file at invocation.

## Periodic self-check

**Every 3-4 tool calls, check:**

1. **Is summary.md current?** If context compacted now, could you continue from it alone?
2. **Am I reading source code or doing research/analysis/design inline?** If your last 2+ tool calls weren't reading receipts/summary.md/skill files, you're doing subroutine work. Stop and delegate.
3. **Am I tracing behavior in my head?** More than ~4 lines of state/timing reasoning = guessing. Invoke `/inspect`.
4. **Am I making progress?** Same understanding as 3 tool calls ago with no changes = stuck. Invoke `/inspect` or `/research`.
5. **Am I in trial-and-error mode?** More than 1 code change without new diagnostic data = guessing. A failed fix means the model is wrong — get new information, not new code.
6. **Two-strike rule:** More than 2 implementations without updating `## Hardware model` = stop implementing, go back to `/research`.
7. **Am I reasoning about rise/fall ordering?** Reframe in terms of DFF capture edges, not procedural call order.
8. **Zero-effect fix?** Check for multi-stage pipelines where another stage compensates.

**Default action when uncertain: invoke `/inspect`.**

## Working style: hypothesize, measure, analyze

The core loop:

1. `/hypothesize` → receipt with ranked hypotheses
2. Update summary.md — add top hypothesis to RCA tree as `[ ] **bold** ← ACTIVE`
3. `/compare-traces` or `/inspect` → observation receipt
4. `/analyze` → interpretation receipt (confirmed/refuted/inconclusive)
5. Update summary.md — mark hypothesis (`[x]` confirmed, `[x] ~~struck~~` refuted), update Current understanding
6. Loop back to step 1 if not solved

**summary.md is owned exclusively by investigate.** No subroutine writes to it. When `/research` returns new data, pass it through `/analyze` before updating summary.md.

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
- **Capture a baseline.** Run `./scripts/test-report.sh --diff` directly (not via a subroutine) and record the pass/fail/ignored counts in summary.md's `## Baseline` section. This is investigate's bookkeeping — like branch hygiene, it's not delegated. This is a blocking prerequisite: no other work happens until the baseline is recorded.
- Invoke `/analyze` to interpret the initial results.
- Classify the problem type and write it in summary.md.

**For compatibility investigations:**
- After capturing the baseline, **check available data sources before generating new data.** In order:
  1. **PPU timing model spec** (`receipts/ppu-overhaul/reference/ppu-timing-model-spec.md`): For PPU-related failures, this is the canonical hardware reference. Dispatch `/research` to check whether the behaviour under test is covered. If the spec has a gap a dmg-sim run could fill, **raise the gap with the user** before proceeding so the spec can be updated — don't route around it via emulator source.
  2. **gekkio's gb-ctr** (https://gekkio.fi/files/gb-docs/gbctr.pdf): Primary written reference for non-PPU subsystems (timers, interrupts, CPU, DMA, etc.). Dispatch `/research` to consult.
  3. **Hardware timing data** (`receipts/resources/gb-timing-data/campaigns/`): Check if a measurement campaign covers the behavior. Campaign CSV data provides definitive cycle-level measurements from real hardware. If a relevant campaign exists but has no results yet, note that it would be valuable when complete.
  4. **`/compare-traces`**: Invoke to find the exact divergence point between missingno and a reference emulator. **Prefer the GateBoy adapter** — die-photo-derived, closest to hardware. Fall back to Gambatte / SameBoy / DocBoy only when GateBoy doesn't pass the test. For small focused tests, direct diff is productive. For larger tests, use individual trace inspection (`gbtrace query`, `gbtrace render`) to understand behavior.
  5. **`/inspect`** (debugger): Fall back when you need sub-dot observation, internal pipeline state, or information that traces can't provide.
  6. **dmg-sim** (`receipts/resources/dmg-sim/`, via `scripts/dmg-sim-observe.sh`): Gate-level simulation. Runs are slow and waveform analysis is specialised — **surface dmg-sim needs to the user** (with the specific signals / timing event to measure) rather than invoking it from a skill dispatch. When the user runs dmg-sim, the findings should go into the PPU timing model spec.
  7. **Slowpeek** (`receipts/resources/slowpeek/`): If the investigation requires measuring a specific hardware behavior that no existing data source covers, note that a Slowpeek sweep test could provide the definitive answer. **Hardware serial path not yet complete** — do not attempt to use hardware mode, but do flag when it would be useful.
- **When `/compare-traces` is not enough:** If the trace comparison can't answer the question (e.g. you need internal pipeline state, sub-dot phase timing, or the reference emulator has no trace available), invoke `/inspect` for targeted observation at the divergence point that `/compare-traces` identified.
- **Boot ROM consideration.** If boot state is suspected to play a role (e.g., tests that depend on initial register values, VRAM contents, or hardware state that the boot ROM sets up differently from post-boot initialization), ask the user for a DMG boot ROM path and re-run the specific failing test with `DMG_BOOT_ROM=<path>`. Boot ROMs are proprietary and cannot be in the repo. Do NOT run the entire test suite with the boot ROM — it adds significant startup time per test. Use it only on targeted tests.
- Classify the failure type:
  - **Register mismatch**: Expected vs actual CPU/hardware register values after test execution.
  - **Screenshot mismatch**: Pixel differences between rendered output and reference image.
  - **Timeout/hang**: The ROM never reached a halt condition — likely wrong control flow or missing hardware behavior.

### 2. Understand the domain and research correct behavior

- Identify the hardware subsystem. Use `/research` to fill knowledge gaps — frame questions as "what does the hardware do?" not "what does emulator X do?"
- Use `/research` any time you're uncertain about expected behavior — not just at the start.
- Pass research results through `/analyze` before updating summary.md.

### 3. Track test state

Run `./scripts/test-report.sh --diff` directly (investigate's bookkeeping) after every code change. Update summary.md `## Baseline` with current counts.

### 4. Observe and diagnose

Observation priority: `/compare-traces` → `/inspect` → `/instrument` (with user approval). Don't guess — observe.

- `/inspect` first for runtime observation. If it can't answer, ask the user before falling back to `/instrument`.
- `/instrument` only after `/inspect` is insufficient. More invasive — modifies code temporarily.
- Pass all observation results through `/analyze` before updating summary.md.

### 5. Analyze and fix

**Stuck = more than one hypothesis-test cycle without progress.** The fix is always: invoke `/inspect` to observe, or `/research` to learn. Never reason your way out of being stuck.

#### Hardware model gate (blocking prerequisite for `/design`)

Before invoking `/design`, summary.md must have all three sections filled (not "Unknown" or blank):
1. `## Hardware model` — what the hardware does step by step (from a research receipt)
2. `## Model divergence` — what our emulator does differently, naming specific structs/enums (from an analysis receipt)
3. **Emulator mechanism understanding** (in `## Model divergence`) — how our code uses the values we're about to change. **Most commonly skipped gate.** Without this, you'll cargo-cult values from references without understanding their effect in our architecture.

If any section is missing → `/research` or `/inspect` + `/analyze`, not `/design`.

#### Design → implement → verify

1. `/design` — produces a design receipt. Do not design inline.
2. `/implement` — applies the design, runs verification, reports results. Do not implement inline.
3. Update summary.md with results. `/implement` runs the full test suite — check for regressions.

#### When a fix fails

A failed fix means the hardware model or model divergence is wrong — not that the code needs tweaking.

1. Stop implementing. Record expected vs actual in summary.md.
2. Identify which understanding is wrong: hardware model or model divergence?
3. `/research` or `/inspect` + `/analyze` to fill the gap.
4. Redesign only after updating the relevant summary.md section.

**Reference value trap**: If a value from a reference emulator doesn't work, the next step is NEVER "try a different value" — it's `/research` to understand what the value controls in *our* code.

#### Spinning off a new investigation

When the root cause is in a different subsystem than scoped, or the investigation name no longer describes the problem:
1. Set current summary.md Status to `resolved → spawned new investigation`
2. Create new investigation folder with correct problem name
3. New branch from `main`: `git checkout -b <new-name>`
4. Carry forward validated findings only, not dead-end history

Don't spin off for: refuted hypotheses within scope, more complex fixes, or changes to helper functions in other files.

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

Use the date and time (to the minute) of the investigation and a short kebab-case name describing the issue (e.g. `2026-02-13-1430-stat-mode0-timing`, `2026-02-13-0915-slow-test-builds`). **Get the current time from `date +%Y-%m-%d-%H%M`** — do not guess or estimate the timestamp.

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

## Hardware model
<What the hardware does in this scenario, step by step. Cite research receipts.
Must be filled in before any /design invocation. "Unknown" is valid early on —
the investigation's job is to fill this in through /research and /inspect.>

## Model divergence
<How the emulator's data model differs from the hardware model above. Name the
specific structs, enums, state machines that are wrong and HOW they're wrong.
Cite analysis receipts. Must be filled in before any /design invocation.>

## Root cause analysis

<A tree of hypotheses about HARDWARE BEHAVIOR, not about code changes.
Each node states what the hardware does and where the model diverges.
Indent children under parents. Cross off dead ends.>

- [ ] **Hardware overlaps first tile fetch with FIFO priming** — first pixel at dot 8, not dot 12 ← ACTIVE
  - [x] Pixel offset confirmed as exactly 4 (`logs/09-pixel-offset-measurement.log`)
  - [ ] Research: does the hardware fetch the first tile while simultaneously priming the FIFO?
- [x] ~~Separate scanline-position counter for STAT~~ — refuted, same counter (`analysis/10-stat-timing.md`)

## Current understanding
<2-4 sentences: the best working model right now. What you believe
the hardware does and where the emulator diverges. No history,
no dead ends — just the current state of knowledge.>

```

##### Rules

- **Root cause analysis tree is mandatory.** Start it after the first measurement. Update it after every `/analyze` return. Every hypothesis goes in the tree — confirmed, refuted, or active. This is the primary navigation structure for the investigation.
- **Hypotheses describe hardware behavior, not code changes.** Each entry in the tree must be a claim about what the hardware does and where the model diverges. "Hardware overlaps first tile fetch with FIFO priming — first pixel at dot 8 not dot 12" is a hypothesis. "Add idle dots at Mode 3 start" is NOT a hypothesis — it's a proposed fix that skips the understanding step. If a hypothesis reads like an implementation plan, reframe it: what hardware behavior would make that implementation correct?
- **Active hypothesis goes first.** The active line of inquiry must be the first entry in the tree so it's immediately visible. Refuted hypotheses sink to the bottom. When a hypothesis is refuted, move it (and its children) below the active line. When a new hypothesis becomes active, move it to the top.
- **Use `[x] ~~struck~~` for refuted hypotheses.** Include a one-line reason and a receipt link. Do not delete refuted hypotheses — they document dead ends. **Keep refuted entries to one line.** The format is: `[x] ~~Hypothesis title~~ — reason (receipt-link)`. Do not preserve the children, sub-findings, or detailed notes from when the hypothesis was active — those details live in the linked receipts. When a hypothesis is refuted, collapse it to a single line and remove all its children.
- **Use `[ ] **bold**` for the active hypothesis.** Mark it with `← ACTIVE`. There should be exactly one at any time.
- **Use `[x]` (no strike) for confirmed findings** that support the active line but aren't hypotheses themselves.
- **Indent child hypotheses** under their parent. A refuted parent means all children are implicitly dead — remove the children when the parent is refuted (they're preserved in the receipts).
- **Current understanding is a snapshot, not a history.** It should read as "here's what we know right now" — not "first we discovered X, then Y". Rewrite it from scratch when the model changes rather than appending. **There is exactly one Current understanding section.** If you find yourself writing the current understanding in multiple places (e.g., at the end of the Hardware model section, in the RCA tree, and in Current understanding), you are duplicating. The Hardware model section describes the hardware. The Model divergence section describes where our code differs. The Current understanding section is a 2-4 sentence synthesis of both. Do not repeat the same information across sections — each section has a distinct purpose.
- **No "What's been tried" log.** The RCA tree captures this. Each crossed-off hypothesis IS a record of what was tried. Receipt links provide detail.
- **No duplicating receipt content.** If a finding is documented in an analysis receipt, link to it — don't reproduce the finding in summary.md.
- **Prune aggressively.** summary.md is a dashboard, not an archive. When updating, actively look for content that has become stale, redundant, or moved to receipts, and remove it. If a section is growing beyond its intended size (Hardware model > ~5 lines, Model divergence > ~5 lines, Current understanding > ~4 sentences, RCA tree > ~10 active/refuted entries), it needs pruning. Move detail to receipts and replace with links.

### 9. Commit format

```
Short summary of what changed

Detailed explanation of:
- What the problem was (observable symptom)
- Why it happened (root cause)
- How the fix works (what changed and why it's correct)

Fixes <test_name> / Resolves <issue>.
```
