# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here. **Symlinks between these directories are user-managed. Do not modify them.**
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored.

### Context hygiene

The conversation context is volatile — it will be compacted unpredictably and you cannot control what survives. Treat files on disk as the primary memory. The conversation context is scratch space.

**Write early, write often.** After every meaningful step — a finding, a decision, a measurement, a hypothesis update — write it to the appropriate file (a receipt, a research doc, or `summary.md` if you're the investigate skill). Do not accumulate results across turns and write them later. If the context were compacted right now, would your progress survive? If not, you haven't written enough.

**Keep context lean.** After writing state to disk, do not continue to carry it in conversation. When you need to recall earlier findings, re-read the file rather than relying on conversation memory. This applies especially to:
- Research findings — once written to a research doc, re-read the doc when you need the information; don't try to remember it.
- Diagnostic output — once recorded in a log file and referenced by a receipt, the raw output in conversation can be forgotten. Reference the log file path, not the content.
- Investigation state — re-read `summary.md` to check current state rather than scrolling back through conversation.

**Reset after every subroutine return.** When a callee (hypothesize, research, measure, analyze, design, implement) returns, the caller must: (1) read the receipt and update `summary.md` (only the investigate skill writes to summary.md), (2) re-read its own skill file and `summary.md` to re-establish context from disk, (3) continue working from the file state, not from conversation memory of what happened before the subroutine.

### Staying aligned with skill directives

Skills contain specific, detailed instructions (e.g. "use curl not WebFetch", "always tee to a log file", "update summary.md before every test run") that are critical to follow. These details are easy to lose — either through context compaction or simply drifting during a long session.

**After context compaction**: When context is compacted mid-skill, the full skill text is lost. Before continuing work, **re-read the active skill file** from `.agents/skills/` and the active investigation's `summary.md` (if any). Do not rely on the compaction summary to preserve skill directives — it won't.

**Periodically during long sessions**: Every ~10 tool calls, re-read the active skill file and `summary.md` to check that you're still following the skill's rules. Drift happens gradually — you start cutting corners on logging, skip summary updates, or forget discipline rules. A periodic re-read catches this before it compounds.

### Skill invocation protocol

Skills invoke other skills as subroutines (e.g. investigate invokes hypothesize, research, measure, analyze, and design). Every cross-skill invocation must follow this protocol. The protocol exists to enforce strict context boundaries — the caller owns interpretation and decision-making; the callee owns fact-finding and measurement.

#### Request format (caller → callee)

Every skill invocation must include exactly these fields. Extraneous context is a protocol violation.

```
**Question**: <one specific, concrete, testable question — one sentence>
**Context**: <only what the callee needs to find the answer — file paths, subsystem names, output location>
**Log path**: <where to save command output> (measure only)
```

**What must NOT be in a request:**
- The caller's hypotheses about what the answer is
- Diagnostic output or log excerpts from prior steps
- Reasoning about what the answer means for the investigation
- Multiple unrelated questions bundled together

If you can't state the question in one sentence, it's not focused enough — split it into multiple invocations.

#### Report format (callee → caller)

Every skill report must use the format below. Interpretation, recommendations, and problem-solving are protocol violations.

**Research reports:**
```
## Findings
<factual answer to the question, with enough detail to act on>

## Sources
<URLs, doc names, or repo + file path + line numbers>

## Confidence
<high/medium/low — based on source quality>

## See also
<one-line notes on tangential discoveries, if any — optional>
```

**Instrument reports:**
```
## Test result
<pass/fail, which sub-test if applicable>

## Measurements
<the specific values requested, extracted from log output>

## Raw data
<compact summary of relevant log lines, with file:line references into the log>

## Also observed
<unexpected findings not part of the original question — optional>
```

**What must NOT be in a report:**
- "This means..." / "This suggests..." / "This confirms..."
- "The fix should be..." / "You should try..."
- "This is probably caused by..."
- Any reasoning about what the findings mean for the caller's problem
- Any scope expansion beyond what was asked

#### Enforcement

1. **Request validation.** Before invoking a skill, re-read the request and delete anything that isn't Question, Context, or Log path.
2. **Report validation.** Before returning from a skill, re-read the report and delete any sentence that interprets, recommends, or analyzes. The test: "Could a reader who knows nothing about the caller's problem still understand this report?" If it requires investigation context to parse, it's leaking.
3. **Scope tripwire.** If a callee catches itself reasoning about the caller's problem (why is the test failing, what should the fix be, does this confirm the hypothesis), that's a scope violation. Stop, delete the reasoning, return to reporting facts or measurements.
4. **One question, one answer.** Each invocation has exactly one question. If the caller has two unrelated questions, that's two invocations. Multiple measurements that all serve the same hypothesis are fine in a single invocation — but unrelated hypotheses must be separate. If the callee discovers two things, the asked-for answer goes in the report; the other goes in "See also" / "Also observed" as a one-liner.

#### Subroutine discipline

Skills invoked as subroutines are not stopping points. A skill invocation is a function call that returns a value, not a handoff to another agent. The caller's turn does not end when the callee returns — the caller must continue working.

**Return context block.** Before invoking a subroutine, the caller MUST write a return context block to the active investigation's `summary.md` (or to the receipt file if there is no investigation). This block captures everything the callee needs to hand control back:

```
## Active subroutine
- **Callee**: <skill name being invoked>
- **Caller**: <skill name to return to>
- **Caller skill file**: `.agents/skills/<caller>.md`
- **On return**: <one sentence: what the caller will do with the result>
- **Summary file**: <path to this summary.md>
```

This block serves three purposes:
1. **Context boundary.** Once the block is written, the callee operates with a clean slate — it follows only its own skill file, not the caller's. The caller's hypotheses, diagnostic output, and reasoning are irrelevant to the callee. If the callee catches itself reasoning about the caller's problem or reaching for tools outside its own skill's methodology, the context boundary has been violated.
2. **Survival through compaction.** If context is compacted mid-subroutine, the return context block in summary.md tells the agent exactly where it is and how to continue.
3. **Clean return.** When the callee finishes its report, it reads the return context block to know which skill file to re-read and what the caller's next step is.

**Callee isolation.** After a subroutine is invoked, the callee MUST operate exclusively within its own skill's rules and methodology. The callee does NOT inherit the caller's tools, habits, or context. Specifically:
- The callee re-reads its own skill file from `.agents/skills/` and follows only those instructions.
- The callee does NOT use tools or patterns from the caller's skill (e.g., `/research` does not use `/measure`'s logging patterns; `/measure` does not do `/research`'s web fetching).
- If the callee reaches a dead end within its own methodology, it reports what it found with `Confidence: low` — it does NOT escalate to tools outside its skill definition.

**Callee handoff.** When the callee finishes:
1. Write the report/receipt in the format specified by the skill invocation protocol.
2. Do NOT write to summary.md — only the investigate skill (top-level caller) updates summary.md from receipts.
3. Read the return context block from summary.md.
4. Re-read the caller's skill file (path is in the return context block).
5. Delete the "Active subroutine" section from summary.md.
6. **Immediately resume as the caller.** The caller reads the updated summary.md and decides what to do next.

**The turn does not end at a subroutine boundary.** A skill invocation is a function call that returns a value, not a handoff to another agent. After a callee writes its report and hands back, the same turn continues with the caller acting on the result. If the turn ends after a Skill tool call with no further action, subroutine discipline has been violated.

**Decision ownership.** Only the investigate skill (the top-level caller) makes decisions about what to do next — which hypothesis to pursue, whether to measure or research, when to move to design, what to implement. Subroutine skills (measure, analyze, hypothesize, design, research) report their output and return. They do not prescribe next steps, choose hypotheses, or continue the caller's workflow.

## Workflow Discipline

- When asked to investigate or debug, always use the appropriate skill (investigate, measure, research, etc.) — never start ad-hoc analysis or use WebSearch directly. Follow the skill discipline hierarchy.
- When asked to update documentation, commit, or do a simple task, do exactly that — don't go on analysis tangents or start investigating new issues. Complete the requested task first.
- Before starting any work, check git status and ensure the working directory is clean. If there are uncommitted changes or stashed work, ask before proceeding.
- If the Read tool returns content that seems stale or inconsistent (especially after git operations like stash or checkout), fall back to `cat <file>` via Bash to get accurate file state.

## Project Overview

MissingNo. is a Game Boy emulator and debugger written in Rust.

## Build and Run Commands

```bash
cargo run --release                          # Build and run
cargo run --release -- path/to/rom.gb        # Load a ROM
cargo run --release -- path/to/rom.gb --debugger  # Load with debugger
cargo check                                  # Type check
cargo test -p missingno-core                 # Run core tests (fast, no GUI deps)
cargo test                                   # Run all workspace tests
cargo clippy                                 # Lint
cargo fmt                                    # Format
```

## Testing

- Always run tests against missingno-core: `cargo test -p missingno-core`. Do not run `cargo test` against the whole workspace unless specifically asked.
- For regression checking, use `./scripts/test-report.sh --diff` instead of raw `cargo test`. It generates structured reports with baseline comparison and saves them to `receipts/test-reports/`.
- After any fix, verify no regressions before committing.

## Emulation Philosophy

- **Hardware fidelity**: Model the hardware as closely as possible so correct behavior emerges naturally. Avoid hacks and special-case workarounds — if something needs a hack to work, the underlying model is wrong.
- **Code as documentation**: The code should teach the reader how the hardware works. Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself, not from comments. Strike a balance between clarity and jargon; assume the reader is a competent programmer but not necessarily a domain expert in the specific hardware.
- **Hardware over emulator comparisons**: When debugging, prioritize real hardware behavior and documentation over comparing with other emulator implementations (e.g., SameBoy). Other emulators are reference material, not ground truth.
- **Future cores**: These principles apply to any emulation core added to the project, not just Game Boy.

## Architecture

The project is a Cargo workspace with two crates:

- **`core/`** (`missingno-core`) — Core emulation library. No GUI dependencies (only `bitflags` and `rgb`). Contains:
  - **`core/src/game_boy/`** — Core emulation. `GameBoy` owns a `Cpu` and `MemoryMapped` (which aggregates all hardware: cartridge, video, audio, timers, joypad, interrupts). `GameBoy::step()` executes one instruction and returns `bool` for whether a new video frame was produced.
  - **`core/src/debugger/`** — Debugging backend. Wraps `GameBoy` with breakpoints, stepping, and disassembly.
  - **`core/tests/`** — Integration tests (ROM-based accuracy tests).
- **Root crate** (`missingno`) — Iced 0.14 GUI binary. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer. Lives in `src/app/`.

### Instruction Execution

`GameBoy::step()` in `core/src/game_boy/execute.rs` runs one instruction in two phases:

1. **Fetch/decode**: Reads the opcode byte, ticks hardware, then reads operand bytes one at a time (ticking hardware after each). `operand_count()` determines byte count from the opcode alone. The buffered bytes are passed to `Instruction::decode()`.
2. **Process**: A `Processor` state machine (`src/game_boy/cpu/mcycle/`) yields one `BusAction` per M-cycle for post-decode work (memory reads/writes, internal cycles). The step loop executes each action and ticks hardware.

The `Processor` is split across three files in `core/src/game_boy/cpu/mcycle/`:
- `mod.rs` — `Phase` enum, `BusAction` enum, `Processor` struct and `next()` method
- `build.rs` — Constructs the `Phase` for each instruction type
- `apply.rs` — Pure CPU mutations (ALU, flags, DAA, etc.)

### Key Patterns

- **CPU and memory separation**: `Cpu` and `MemoryMapped` are separate structs so memory subsystems can be borrowed independently.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Enum-based MBC dispatch**: `Mbc` enum in `core/src/game_boy/cartridge/mbc/mod.rs` with variants for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147. ROM data is owned by `Cartridge` and passed to MBC `read()` methods as `&[u8]`.
- **PPU state machine**: `PixelProcessingUnit` alternates between `Rendering` and `BetweenFrames`. Rendering tracks per-line state (mode 2→3→0) and draws pixels one at a time with cycle-accurate timing.
- **Post-boot register initialization**: The emulator skips the boot ROM. Initial hardware state must match DMG post-boot values (e.g., LCDC=0x91 in `Control::default()`, CPU registers in `Cpu::new()`).
- **Serialization**: Hand-written serialization for config (`~/.config/missingno/settings.ron`, `recent.ron`).
- **Timestamps**: Uses the `jiff` crate (not `chrono`) for date/time formatting.

### Debugger

- **Pane system**: `src/app/debugger/panes.rs` manages a `pane_grid` of `DebuggerPane` variants. Each pane is a separate module with a struct (e.g. `CpuPane`, `PlaybackPane`), a `content()` method returning `pane_grid::Content`, and optionally a `Message` enum with `Into<app::Message>` impl for routing through the nested message chain (`PaneMessage` → `panes::Message` → `debugger::Message` → `app::Message`). Register new panes by adding to `DebuggerPane` enum, `PaneInstance` enum, `construct_pane()`, `view()`, `available_panes()`, and `Display` impl.
- **Input recording**: `core/src/game_boy/recording.rs` defines the `Recording` data model (ROM header + initial state + input events). Recording state (`ActiveRecording`) lives in `src/app/debugger/mod.rs` on the `Debugger` struct — `press_button`/`release_button` log events with frame numbers during recording. The Playback pane (`src/app/debugger/playback.rs`) provides the UI.
