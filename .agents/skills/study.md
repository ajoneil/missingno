# Study

Collaboratively study a section of the emulator with the developer — understand the hardware, compare it against the code's model, and improve clarity and fidelity together.

Unlike `/investigate`, which is driven by a failing test and runs largely autonomously, `/study` is developer-steered. The developer picks what to look at. The agent reads the code, researches the hardware, identifies divergences and clarity issues, and proposes next steps — but the developer always decides what to do next.

## Scope discipline

**You are a collaborator, not an autonomous agent.** The developer drives the session. Your job is to:

1. Read the code the developer wants to study.
2. Dispatch subroutines to research hardware behavior and analyze how the code models it.
3. Present findings clearly — what the hardware does, how the code models it, where they diverge, and where the code could be clearer.
4. Propose next steps and wait for the developer to choose.

You do NOT autonomously loop through hypothesize→measure→analyze cycles. You do NOT decide what to fix or what to study next. You propose — the developer decides.

**Present, don't proceed.** After every subroutine returns and you've updated session.md, present the findings to the developer and propose 2-3 concrete next steps. Then wait. Examples of good proposals:

- "The research shows the hardware uses a 3-stage fetch pipeline, but the code models it as 2 stages. Want to dig deeper into the third stage, or redesign the enum to match?"
- "This enum's variant names don't match the hardware terminology. Want me to rename them, or should we first research whether the state transitions are also wrong?"
- "The code looks faithful to the hardware here. Want to move on to the next area, or verify with `/measure`?"

**The developer can also direct you.** The developer may ask questions, point at specific code, or request specific actions at any time. Respond to what they ask — don't redirect them into the workflow if they want to explore something specific.

## Discipline requirements

These rules override default agent behavior. Follow them exactly:

1. **Use `/research` for all hardware questions.** All external information gathering — documentation, specifications, other projects' source code, blog posts — goes through `/research`. Do not fetch URLs, clone repos, or search external sources yourself. Format every research request using the skill invocation protocol defined in AGENTS.md. One question, one context block, no hypotheses.
2. **Use `/measure` for all runtime behavior questions.** If you need to know what the code actually does at runtime — what values a variable has, what state a machine is in, what output a test produces — invoke `/measure`. Do not run `cargo test` or other diagnostic commands directly. Do not trace execution in your head.
3. **Use `/analyze` to interpret complex data.** When `/measure` or `/research` returns data that requires interpretation — comparing measurements against hardware specs, reconciling multiple sources, understanding what a divergence means — invoke `/analyze`. For straightforward findings (e.g., "research says the hardware has 3 states, the enum has 3 variants, they match"), you can summarize directly to the developer without invoking `/analyze`.
4. **Use `/design` → `/implement` for behavioral changes.** Any change that affects the emulator's behavior — different state transitions, new states, changed timing, modified output — must go through `/design` and then `/implement`. The design skill produces a receipt; the implement skill applies it and runs verification. Do not make behavioral changes directly.
5. **Direct edits are allowed for pure clarity improvements.** Renaming an enum variant to match hardware terminology, improving a comment, restructuring code for readability — these can be done directly without `/design` → `/implement`, provided they do not change behavior. The test: if you renamed `Step5` to `GetTileDataHigh`, would every match arm still do the same thing? If yes, it's a clarity edit. If no, it's a behavioral change — use `/design` → `/implement`. **Always verify clarity edits compile.** Run `cargo check` after direct edits. If a rename touches many call sites, use `/implement` instead — it handles verification and rollback.
6. **Never skip session.md updates.** Update session.md after every subroutine return and before every subroutine invocation. The test: if context were compacted right now, would session.md tell the developer exactly where the session stands? If not, update it now.
7. **Never trace behavior in your head.** Do not manually step through state machines, count cycles, or simulate execution. If you want to know what a piece of code does at runtime, invoke `/measure`. If you want to know what the hardware does, invoke `/research`.
8. **Never read reference implementation source directly.** Formulate a hardware behavior question and invoke `/research`. The research skill handles source consultation with scope discipline.
9. **Hardware is the source of truth.** Frame everything in terms of what the real hardware does, not what other emulators do. When presenting findings to the developer, describe the hardware behavior first, then how the code models it.
10. **Never cargo-cult values from reference emulators.** Before using any externally-sourced value, understand what it controls in this codebase. Invoke `/research` with a question about the emulator's mechanism if needed.
11. **Propose spin-offs when scope drifts.** If the developer's interest moves to a fundamentally different subsystem — or if studying one area reveals that the real issue is elsewhere — suggest starting a new study session (or an `/investigate` if the issue is a bug). The test: does the session name still describe what we're looking at? If not, propose a spin-off.

## Periodic self-check

**Every 3-4 tool calls, pause and ask yourself these questions:**

1. **Is my progress on disk?** If context were compacted right now, could the session continue from session.md alone? If not, update it.
2. **Am I carrying stale context?** Re-read session.md and this skill file. Work from the file state, not conversation memory.
3. **Am I doing subroutine work inline?** If you've written more than ~3 sentences interpreting data, you should have invoked `/analyze`. If you've written more than ~3 sentences planning code changes, you should have invoked `/design`. If you're reading external sources or reference code, you should have invoked `/research`.
4. **Am I tracing behavior in my head?** If you want to know what the code does at a specific point, invoke `/measure`. Do not simulate execution mentally.
5. **Am I proceeding without the developer?** After presenting findings, did you wait for the developer to choose the next step? If you've dispatched two subroutines in a row without developer input between them, you're driving — slow down and present.
6. **Has the scope drifted?** Is the current line of inquiry still related to the session's stated area? If not, propose a spin-off.
7. **Am I framing things in terms of hardware?** Research questions and findings should be about what the hardware does, not what other emulators do.

## Working style

The study skill does not have a fixed loop like investigate's hypothesize→measure→analyze cycle. Instead, it follows a flexible pattern driven by the developer:

### Read → Research → Compare → Propose

1. **Read the code** the developer wants to study. Summarize its structure: what states/enums exist, what the state machine does, how data flows. Present this to the developer.
2. **Research the hardware** to understand what the real hardware does in this area. Invoke `/research` with specific questions about hardware behavior.
3. **Compare** the code's model against the hardware. For complex comparisons, invoke `/analyze`. For straightforward ones, summarize directly.
4. **Propose** next steps to the developer: areas that diverge, clarity improvements, deeper research questions, or confirmation that the code is faithful.

The developer then picks what to do — and the cycle repeats from wherever they direct.

### Types of work during a session

**Understanding** — reading code and researching hardware to build a shared mental model. Heavy use of `/research`. The developer asks "how does this work?" and you find out together.

**Auditing** — comparing the code's model against the hardware. Use `/research` for hardware facts and `/analyze` for comparison. You present: "the hardware does X; the code does Y; here's where they match and where they diverge."

**Clarity edits** — renaming, restructuring, improving documentation to make the code better communicate the hardware model. Direct edits for non-behavioral changes. These happen when the code is correct but unclear.

**Fidelity improvements** — changing the code to more accurately model the hardware. These are behavioral changes and go through `/design` → `/implement`. These happen when the code diverges from the hardware.

**Verification** — confirming that changes (especially behavioral ones) don't break things. Use `/measure` to run tests and diagnostics.

**session.md is owned exclusively by the study skill.** No subroutine skill writes to it. After every subroutine return, you (study) read the receipt and update session.md.

**Before invoking any subroutine, write a return context block to session.md** (see "Subroutine discipline" in AGENTS.md). After the subroutine returns, re-read this skill file (`.agents/skills/study.md`) and the session's `session.md` to restore context.

## Workflow

### 1. Scope the session

- Ask the developer what they want to study — a subsystem, a file, a specific behavior, a concept.
- Propose a session folder name and get approval. Create the folder:
  ```
  receipts/studies/<YYYY-MM-DD-HHMM>-<short-name>/
  ├── session.md        # Create now with Status: Active
  ├── research/         # Session-specific research notes
  ├── analysis/         # Analysis receipts
  ├── designs/          # Design receipts (if behavioral changes are made)
  ├── implementation/   # Implementation receipts
  ├── measurements/     # Measurement receipts
  └── logs/             # Diagnostic output
  ```
- Write initial session.md with the area being studied.

### 2. Read the code

- Read the code the developer pointed at. This is one of the key differences from `/investigate` — the study skill reads project source directly to build a structural understanding.
- Summarize what you see: the types, the state machine structure, the data flow, the naming. Don't interpret behavior or trace execution — describe the structure.
- Present the summary to the developer. Ask if they want to focus on a specific part, or study the whole area.
- Update session.md with the structural summary (keep it brief — details go in the conversation, the session.md just tracks what was covered).

### 3. Research and compare

This is the core loop. The developer steers; you dispatch subroutines and present results.

- **When the developer asks about hardware behavior**: Invoke `/research` with a specific question. Present the findings. Propose whether to compare against the code, dig deeper, or move on.
- **When you or the developer notice a potential divergence**: Invoke `/analyze` (or summarize directly for simple cases) to compare the hardware model against the code. Present: what matches, what diverges, and how significant the divergence is.
- **When the developer wants to improve something**: Determine whether it's a clarity edit (direct) or behavioral change (`/design` → `/implement`). For clarity edits, propose the specific change and make it after approval. For behavioral changes, invoke `/design` with the hardware model and divergence analysis, then `/implement`.
- **When the developer wants to verify behavior**: Invoke `/measure` to instrument and observe. Present the results.

**Always present findings before proposing action.** Don't jump from "research says X" to "let's change the code." Present the finding, let the developer absorb it, then propose options.

### 4. When scope drifts

If studying one area reveals that the interesting questions are really about a different subsystem:

- Note the drift to the developer: "We started studying the pixel pipeline, but the real question seems to be about how DMA interacts with VRAM access. Want to spin off a separate session for that?"
- If the developer agrees, close the current session (update session.md with Status: `Completed → spun off`) and start a new one.
- If the issue looks like a bug rather than a clarity/fidelity question, suggest `/investigate` instead.

### 5. Session end

The developer says when the session is done. When ending:

- Update session.md with Status: `Completed` and a brief summary of what was studied, what was learned, and what was changed.
- List any open questions or areas that could benefit from further study.
- Verify the working tree is clean (`git status`). If there are uncommitted changes, ask the developer how to handle them.

## Receipt conventions

### Folder structure

```
receipts/studies/<YYYY-MM-DD-HHMM>-<short-name>/
├── session.md
├── research/
├── analysis/
├── designs/
├── implementation/
├── measurements/
└── logs/
```

Use the date and time (to the minute) and a short kebab-case name describing the area being studied (e.g., `2026-02-16-1400-ppu-fetcher-states`, `2026-02-16-0930-timer-overflow-behavior`). **Get the current time from `date +%Y-%m-%d-%H%M`** — do not guess or estimate the timestamp.

### session.md

`session.md` is a **session notebook** — it tracks what was studied, what was learned, and what was changed. It's lighter than investigate's summary.md — no RCA tree, no hypothesis tracking. But it still must be kept current so the session can survive context compaction.

**Keep it concise.** Details live in receipts. session.md tracks the arc of the session, not the details of each finding.

**Update it before every subroutine invocation and after every subroutine return.**

#### Format

```markdown
# Study: <area being studied>

## Status
<one line: active | completed | completed → spun off>

## Area
<what we're studying — subsystem, files, concept. 1-2 sentences.>

## Baseline
<test counts before any changes: N pass, N fail, N ignored out of N total>

## Hardware model
<what the hardware does in this area. Built up incrementally through /research.
Cite research receipts. Start with "Not yet researched" and fill in as the session progresses.>

## Code model
<how the code represents this area — key types, state machines, data flow.
Built up by reading the code. Brief — point to source files, don't reproduce code.>

## Findings

<Chronological list of what was discovered during the session. Each entry is
a one-line summary with a receipt link for details.>

- Hardware uses 3-stage fetch; code models 2 (`research/fetch-pipeline.md`)
- Variant names don't match hardware terminology (`analysis/01-naming-comparison.md`)
- Sprite penalty timing matches hardware (`analysis/02-sprite-timing.md`)

## Changes made

<List of changes applied during the session, with receipt links.>

- Renamed FetcherStep variants to match hardware terminology (clarity edit, direct)
- Redesigned fetch pipeline to model 3 stages (`designs/three-stage-fetch.md`, `implementation/01-three-stage-fetch.md`)

## Open questions
<Questions that came up but weren't resolved. Useful for future sessions.>

## Active subroutine
<return context block when a subroutine is in flight — see AGENTS.md>
```

#### Rules

- **Findings are one-liners with receipt links.** The detail lives in the receipt. session.md tracks what was found, not the full analysis.
- **Changes made tracks every edit.** Both clarity edits (done directly) and behavioral changes (done through `/design` → `/implement`). Include whether it was a direct edit or went through the full pipeline.
- **Open questions capture loose ends.** Things the developer might want to study later, related areas that came up, unresolved discrepancies.
- **No duplicating receipt content.** Link to receipts, don't reproduce them.
- **Prune when it grows.** If session.md is past ~50 lines, move older findings into a receipt and replace with a summary link.

### Research locations

Same two-location rule as `/investigate`:

- **General hardware knowledge** → `receipts/research/systems/<platform>/<subsystem>/` — findings about how the hardware works that any future session or investigation would benefit from.
- **Session-specific notes** → the session's `research/` folder — analysis of specific code structures, comparison notes, things only relevant to this study session.

When `/research` returns, the research skill decides which location is appropriate based on whether the finding is general or session-specific.

## Branch and commit hygiene

### At session start

- Record the current branch in session.md as the **base branch**.
- If changes are expected, ask the developer whether to create a study branch (e.g., `study/ppu-fetcher-states`). For read-only sessions (pure understanding, no edits), staying on the current branch is fine.
- If the developer wants a branch, create it: `git checkout -b study/<short-name>`.

### During the session

- Clarity edits are committed directly on the study branch with descriptive messages.
- Behavioral changes go through `/implement`, which creates `impl/<name>` branches and merges back on success.
- Commit early and often — each logical change gets its own commit.

### At session end

- Verify the branch is clean.
- Ask the developer whether to merge to the base branch or leave for review.
- Do not force-push or rewrite history without explicit approval.

## Commit format

For clarity edits (direct):
```
Rename <old> to <new> to match hardware terminology

<Brief explanation of what the hardware calls this and why the new name
is more accurate.>
```

For behavioral changes, `/implement` handles the commit.
