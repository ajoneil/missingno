# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here. **Symlinks between these directories are user-managed. Do not modify them.**
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored. **Never reference receipt paths in committed code** (comments, commit messages, etc.) — they are ephemeral working documents, not permanent artifacts.

### Context hygiene

The conversation context is volatile — it will be compacted unpredictably and you cannot control what survives. Treat files on disk as the primary memory. The conversation context is scratch space.

**Write early, write often.** After every meaningful step — a finding, a decision, a measurement, a hypothesis update — write it to the appropriate file (a receipt, a research doc, or `summary.md` if you're the investigate skill). Do not accumulate results across turns and write them later. If the context were compacted right now, would your progress survive? If not, you haven't written enough.

**Keep context lean.** After writing state to disk, do not continue to carry it in conversation. When you need to recall earlier findings, re-read the file rather than relying on conversation memory. This applies especially to:
- Research findings — once written to a research doc, re-read the doc when you need the information; don't try to remember it.
- Diagnostic output — once recorded in a log file and referenced by a receipt, the raw output in conversation can be forgotten. Reference the log file path, not the content.
- Investigation state — re-read `summary.md` to check current state rather than scrolling back through conversation.

**Act on every subagent return.** When a skill subagent (hypothesize, research, analyze, design, implement, instrument, inspect) returns, the caller must: (1) read the receipt file the subagent produced, (2) update `summary.md` with the findings (only the investigate skill writes to summary.md), (3) continue working from the file state. Since skills run as Task subagents, the caller's context is not displaced — no re-reading of skill files is needed.

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

Skills run as Task subagents — each skill is launched via the Task tool, operates in isolated context, writes a receipt file, and stops. The caller (investigate) reads the receipt and continues. This provides natural context isolation without any shared-context protocol.

**Callee isolation.** Each skill subagent receives the full content of its skill file in the Task prompt. It follows only those instructions. It does NOT inherit the caller's context, hypotheses, or reasoning. If a skill reaches a dead end within its own methodology, it reports what it found with `Confidence: low` — it does NOT escalate to tools outside its skill definition.

**Decision ownership.** Only the investigate skill (the top-level caller) makes decisions about what to do next — which hypothesis to pursue, whether to measure or research, when to move to design, what to implement. Subroutine skills (research, analyze, hypothesize, design, implement, instrument, inspect) report their output and stop. They do not prescribe next steps, choose hypotheses, or continue the caller's workflow.

## Workflow Discipline

- When asked to investigate or debug, always use the appropriate skill (investigate, measure, research, etc.) — never start ad-hoc analysis or use WebSearch directly. Follow the skill discipline hierarchy.
- When asked to update documentation, commit, or do a simple task, do exactly that — don't go on analysis tangents or start investigating new issues. Complete the requested task first.
- Before starting any work, check git status and ensure the working directory is clean. If there are uncommitted changes or stashed work, ask before proceeding.
- If the Read tool returns content that seems stale or inconsistent (especially after git operations like stash or checkout), fall back to `cat <file>` via Bash to get accurate file state.

## Project Overview

Missingno is a Game Boy emulator and debugger written in Rust.

## Build and Run Commands

**Do not use `--release` unless explicitly asked.** Release builds are slow to compile. Debug builds are the default for development, testing, and debugging.

```bash
cargo run                                    # Build and run (debug)
cargo run -- path/to/rom.gb                  # Load a ROM
cargo run -- path/to/rom.gb --debugger       # Load with debugger
cargo run -- path/to/rom.gb --headless       # Headless debugger (HTTP API)
cargo run -- path/to/rom.gb --boot-rom path/to/dmg_boot.bin  # Run with boot ROM
cargo check                                  # Type check
cargo test -p missingno-gb                 # Run core tests (fast, no GUI deps)
cargo test                                   # Run all workspace tests
cargo clippy                                 # Lint
cargo fmt                                    # Format
```

## Testing

- Always run tests against missingno-gb: `cargo test -p missingno-gb`. Do not run `cargo test` against the whole workspace unless specifically asked.
- For regression checking, use `./scripts/test-report.sh --diff` instead of raw `cargo test`. It generates structured reports with baseline comparison and saves them to `receipts/test-reports/`.
- To save a baseline before experimenting: `./scripts/test-report.sh --save-baseline`. Always save a baseline from `main` (or the known-good state) before making changes, so `--diff` has an accurate reference point.
- To run a specific test with the boot ROM: `DMG_BOOT_ROM=<path> cargo test -p missingno-gb <test_name>`. Boot ROMs are proprietary — ask the user for the path, never commit them. Only use on targeted tests; the boot ROM adds significant startup time per test, making full-suite runs impractical.
- After any fix, verify no regressions before committing.

## Emulation Philosophy

- **Hardware fidelity**: Model the hardware as closely as possible so correct behavior emerges naturally. Avoid hacks and special-case workarounds — if something needs a hack to work, the underlying model is wrong.
- **Code as documentation**: The code should teach the reader how the hardware works. Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself, not from comments. Strike a balance between clarity and jargon; assume the reader is a competent programmer but not necessarily a domain expert in the specific hardware.
- **Hardware over emulator comparisons**: When debugging, prioritize real hardware behavior and documentation over comparing with other emulator implementations (e.g., SameBoy). Other emulators are reference material, not ground truth.
- **Future cores**: These principles apply to any emulation core added to the project, not just Game Boy.

## Architecture

The project is a Cargo workspace with two crates:

- **`crates/missingno-gb/`** (`missingno-gb`) — Core emulation library. No GUI dependencies (only `bitflags` and `rgb`). Contains:
  - **`crates/missingno-gb/src/`** — Core emulation. `GameBoy` owns a `Cpu` and `MemoryMapped` (which aggregates all hardware: cartridge, video, audio, timers, joypad, interrupts). `GameBoy::step()` executes one instruction and returns a `StepResult` with `new_screen` and `dots` (T-cycle count).
  - **`crates/missingno-gb/src/debugger/`** — Debugging backend. Wraps `GameBoy` with breakpoints, stepping, disassembly, and a T-cycle counter.
  - **`crates/missingno-gb/tests/accuracy/`** — Integration tests (ROM-based accuracy tests).
- **Root crate** (`missingno`) — Iced 0.14 GUI binary. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer. Lives in `src/app/`.

### Instruction Execution

`GameBoy::step()` in `crates/missingno-gb/src/execute.rs` runs one instruction in two phases:

1. **Fetch/decode**: Reads the opcode byte, ticks hardware, then reads operand bytes one at a time (ticking hardware after each). `operand_count()` determines byte count from the opcode alone. The buffered bytes are passed to `Instruction::decode()`.
2. **Process**: A `Processor` state machine (`crates/missingno-gb/src/cpu/mcycle/`) yields one `BusAction` per M-cycle for post-decode work (memory reads/writes, internal cycles). The step loop executes each action and ticks hardware.

The `Processor` is split across three files in `crates/missingno-gb/src/cpu/mcycle/`:
- `mod.rs` — `Phase` enum, `BusAction` enum, `Processor` struct and `next()` method
- `build.rs` — Constructs the `Phase` for each instruction type
- `apply.rs` — Pure CPU mutations (ALU, flags, DAA, etc.)

### Key Patterns

- **CPU and memory separation**: `Cpu` and `MemoryMapped` are separate structs so memory subsystems can be borrowed independently.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Enum-based MBC dispatch**: `Mbc` enum in `crates/missingno-gb/src/cartridge/mbc/mod.rs` with variants for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147. ROM data is owned by `Cartridge` and passed to MBC `read()` methods as `&[u8]`.
- **PPU state machine**: `PixelProcessingUnit` alternates between `Rendering` and `BetweenFrames`. Rendering tracks per-line state (mode 2→3→0) and draws pixels one at a time with cycle-accurate timing.
- **PPU propagation delay analysis**: The sibling project [`gmb-ppu-analysis`](https://github.com/ajoneil/gmb-ppu-analysis) (local clone: `../gmb-ppu-analysis/`) provides static analysis of GateBoy's PPU netlist, identifying deep combinatorial paths and signal races that cause propagation delay on real hardware. Key outputs in `../gmb-ppu-analysis/output/`:
  - `critical_paths_report.md` — Overview and key findings (start here)
  - `operational_paths.md` — Per-dot/per-scanline paths by functional area
  - `race_pairs_report.md` — Signal race pairs with observable effects and depth differentials
  - `signal_concordance.md` — GateBoy cell name ↔ Pan Docs register name mapping
  - `race_pairs.json`, `critical_paths.json`, `ppu_graph.json` — Machine-readable data
  - Interactive explorer: [ajoneil.github.io/gmb-ppu-analysis](https://ajoneil.github.io/gmb-ppu-analysis/)

  This data is a primary source for understanding PPU timing. When investigating one-dot discrepancies, consult the race pairs and critical paths to identify which propagation delays could explain the behavior. The signal concordance maps between GateBoy's 4-letter cell names and standard register/signal names.

- **Execution tracing (gbtrace)**: The sibling project [`gbtrace`](https://github.com/ajoneil/gbtrace) (local clone: `../gbtrace/`) defines a standardised format for recording and comparing Game Boy emulator execution state across multiple emulators. Missingno integrates this behind the `gbtrace` feature flag on `missingno-gb`:
  - **`crates/missingno-gb/src/trace.rs`** — `Tracer` struct captures per-instruction state to parquet files using profile-driven field selection.
  - **Test runner integration** — `tests/accuracy/common/` wraps `GameBoy` in `TestRun`, which optionally traces each `step()`. Activated by env var:
    ```bash
    GBTRACE_PROFILE=cpu_basic cargo test -p missingno-gb --features gbtrace -- <test_name>
    ```
    Writes to `receipts/traces/<rom_name>.parquet`.
  - **Profiles** (in `../gbtrace/profiles/`): `cpu_basic` (CPU registers per instruction), `ppu_timing` (CPU + PPU + interrupts), `timer_edge` (CPU + timers + interrupts).
  - **gbtrace-cli** (`../gbtrace/crates/gbtrace-cli/`) — CLI for working with trace files. Build with `cargo build -p gbtrace-cli` from the gbtrace repo. Commands:
    - `gbtrace-cli info <file>` — summary: emulator, model, entry count, cycle range, file size.
    - `gbtrace-cli query <file> --where pc=0x0150` — find entries matching conditions, with optional `--context N` for surrounding entries.
    - `gbtrace-cli diff <trace_a> <trace_b>` — compare two traces, report first divergence and per-field divergence counts. Supports `--skip-boot`, `--fields pc,a,f`, `--exclude ime`, `--align sequence|cycle`.
    - `gbtrace-cli trim <file> --until pc=0x0100` / `--after pc=0x0150` — cut a trace at a condition.
    - `gbtrace-cli strip-boot <file>` — remove boot ROM entries, rebase cycle counts.
    - `gbtrace-cli convert <file>` — convert between JSONL (`.gbtrace`) and Parquet (`.parquet`).
  - **Comparison workflow**: Use the `/compare-traces` skill for structured trace comparison. It handles generating traces, choosing sync points, filtering noisy fields, and interpreting results. For manual use: capture traces from missingno and a reference emulator running the same ROM, then use `gbtrace-cli diff` with `--sync` (align at a meaningful event like `lcdc&0x80` for PPU-on) and `--exclude` (drop noisy initial-state fields like `div,tac,if_`). Pre-built reference traces are in `../gbtrace/docs/tests/gbmicrotest/`. 
- **Boot ROM support**: The emulator optionally runs the DMG boot ROM. Without a boot ROM, it uses post-boot initialization (e.g., LCDC=0x91 in `Control::default()`, CPU registers in `Cpu::new()`). With a boot ROM, it uses power-on state (`Cpu::power_on()`, `Ppu::power_on()`, etc.) and starts execution at 0x0000. Boot ROMs are proprietary and must never be committed to the repo. CLI: `--boot-rom <path>`. Tests: set `DMG_BOOT_ROM` env var. Running the boot ROM adds significant startup time per test — only use it on targeted tests when boot state is suspected to play a role, not across the full test suite.
- **Serialization**: Hand-written serialization for config (`~/.config/missingno/settings.ron`, `recent.ron`).
- **Timestamps**: Uses the `jiff` crate (not `chrono`) for date/time formatting.

### Debugger

- **Pane system**: `src/app/debugger/panes.rs` manages a `pane_grid` of `DebuggerPane` variants. Each pane is a separate module with a struct (e.g. `CpuPane`, `PlaybackPane`), a `content()` method returning `pane_grid::Content`, and optionally a `Message` enum with `Into<app::Message>` impl for routing through the nested message chain (`PaneMessage` → `panes::Message` → `debugger::Message` → `app::Message`). Register new panes by adding to `DebuggerPane` enum, `PaneInstance` enum, `construct_pane()`, `view()`, `available_panes()`, and `Display` impl.
- **Input recording**: `crates/missingno-gb/src/recording.rs` defines the `Recording` data model (ROM header + initial state + input events). Recording state (`ActiveRecording`) lives in `src/app/debugger/mod.rs` on the `Debugger` struct — `press_button`/`release_button` log events with frame numbers during recording. The Playback pane (`src/app/debugger/playback.rs`) provides the UI.
