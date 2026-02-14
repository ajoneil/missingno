# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here.
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored.

### Staying aligned with skill directives

Skills contain specific, detailed instructions (e.g. "use curl not WebFetch", "always tee to a log file", "update summary.md before every test run") that are critical to follow. These details are easy to lose — either through context compaction or simply drifting during a long session.

**After context compaction**: When context is compacted mid-skill, the full skill text is lost. Before continuing work, **re-read the active skill file** from `.agents/skills/` and the active investigation's `summary.md` (if any). Do not rely on the compaction summary to preserve skill directives — it won't.

**Periodically during long sessions**: Every ~20 tool calls, re-read the active skill file and `summary.md` to check that you're still following the skill's rules. Drift happens gradually — you start cutting corners on logging, skip summary updates, or forget discipline rules. A periodic re-read catches this before it compounds.

### Skill invocation protocol

Skills invoke other skills as subroutines (e.g. investigate invokes research and instrument). Every cross-skill invocation must follow this protocol. The protocol exists to enforce strict context boundaries — the caller owns interpretation and decision-making; the callee owns fact-finding and measurement.

#### Request format (caller → callee)

Every skill invocation must include exactly these fields. Extraneous context is a protocol violation.

```
**Question**: <one specific, concrete, testable question — one sentence>
**Context**: <only what the callee needs to find the answer — file paths, subsystem names, output location>
**Log path**: <where to save command output> (instrument only)
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

Skills invoked as subroutines are not stopping points. After a callee returns its report, the caller must immediately act on the findings **in the same response** — reading the callee's output, interpreting it, updating summary.md, and proceeding to the next investigation step. A skill invocation is a function call that returns a value, not a handoff to another agent. The caller's turn does not end when the callee returns — the caller must continue working.

**Violation test:** If your response ends with a Skill tool call and nothing after it, you have violated subroutine discipline. After every Skill tool result, you must produce further tool calls or text that acts on the result.

**Context restoration:** After a subroutine returns, its skill text will have displaced the caller's instructions from working memory. Before continuing work, re-read the caller's skill file from `.agents/skills/` and the active investigation's `summary.md` (if any). This is not optional — the subroutine's instructions are irrelevant now and the caller's instructions need to be fresh.

**Callee handoff:** After a callee finishes its report, it must not end the turn. The callee must: (1) write the report, (2) re-read the caller's skill file and active summary.md, (3) immediately continue working as the caller. The report is a return value — the same turn continues with the caller's workflow. If the turn ends after a report with no further action, subroutine discipline has been violated.

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

## Emulation Philosophy

- **Hardware fidelity**: Model the hardware as closely as possible so correct behavior emerges naturally. Avoid hacks and special-case workarounds — if something needs a hack to work, the underlying model is wrong.
- **Code as documentation**: The code should teach the reader how the hardware works. Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself, not from comments. Strike a balance between clarity and jargon; assume the reader is a competent programmer but not necessarily a domain expert in the specific hardware.
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
