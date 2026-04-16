# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Skill System Rules

These are the top-level rules governing how skills interact. They survive context compaction and override any default agent behavior.

1. **Always use skills — never ad-hoc.** When asked to investigate, debug, research, or analyze, invoke the appropriate skill (`/investigate`, `/research`, `/compare-traces`, etc.). Never start ad-hoc analysis, use WebSearch directly, read reference emulator source yourself, or trace behavior in your head. The skill system exists to enforce scope discipline and produce durable receipts. Bypassing it produces unreliable, unreproducible results that don't survive context compaction.

2. **Hardware is the source of truth.** The goal is always to understand what the real hardware does and model that behavior. Prioritize hardware documentation, decaps, test measurements, and direct hardware observations over any emulator implementation. Other emulators (SameBoy, Gambatte, GateBoy, etc.) are reference material — useful for confirming hardware behavior, but never the primary source and never a model to copy. The question is always "what does the hardware do?" not "what does emulator X do?"

3. **Skills are subroutine calls — never stopping points.** When a skill subagent returns, the caller MUST immediately read the receipt, update `summary.md`, and continue the investigation in the same turn. Never end your turn after receiving a skill report. Skill dispatches are function calls, not async tasks you wait on.

4. **summary.md is the single source of truth for investigation state.** Update it before every skill dispatch and after every skill return — no exceptions. If context were compacted right now, `summary.md` alone must tell you exactly where you are and what to do next. Only the `/investigate` skill writes to `summary.md`.

5. **Use available data before generating new data.** Before instrumenting code or running the debugger, check whether the question can be answered with existing resources: gbtrace execution traces (both comparison and individual inspection), propagation delay analysis (`receipts/resources/gb-propagation-delay-analysis/`), hardware timing data (`receipts/resources/gb-timing-data/`), or existing research documents in `receipts/research/`. Generate new diagnostic data only when existing sources don't answer the question.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here. **Symlinks between these directories are user-managed. Do not modify them.**
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored. **Never reference receipt paths in committed code** (comments, commit messages, etc.) — they are ephemeral working documents, not permanent artifacts.
- **`receipts/resources/`** — External resources: sibling projects, reference emulator source, hardware schematics, etc. Clone or download whatever you need into this directory. It's gitignored, so treat it as a workspace for external material.

### Context hygiene

The conversation context is volatile — it will be compacted unpredictably. Treat files on disk as the primary memory; conversation is scratch space.

- **Write early, write often.** After every meaningful step, write it to the appropriate file. Test: if context were compacted right now, would your progress survive?
- **Keep context lean.** After writing state to disk, re-read the file when you need the information — don't carry it in conversation memory.
- **After context compaction**: Re-read the active skill file from `.agents/skills/` and `summary.md` before continuing. The compaction summary won't preserve skill directives.
- **Periodically during long sessions**: Every ~10 tool calls, re-read the active skill file and `summary.md` to catch drift.

### Skill invocation protocol

Skills invoke other skills as subroutines via Task subagents. The caller owns interpretation and decision-making; the callee owns fact-finding and measurement.

#### Request format (caller → callee)

```
**Question**: <one specific, concrete, testable question — one sentence>
**Context**: <only what the callee needs — file paths, subsystem names, output location>
**Log path**: <where to save command output> (instrument only)
```

Do NOT include: caller's hypotheses, diagnostic output from prior steps, reasoning about what the answer means, or multiple unrelated questions.

#### Report format (callee → caller)

Reports must contain only facts and measurements — no interpretation, recommendations, or analysis. Research reports use: Findings / Sources / Confidence / See also. Instrument reports use: Test result / Measurements / Raw data / Also observed. If a sentence starts with "This means..." or "The fix should be..." — delete it.

#### Subroutine discipline

Each skill subagent receives its skill file in the Task prompt, writes a receipt file, and stops. It does NOT inherit the caller's context or hypotheses. Only `/investigate` makes decisions about what to do next — all other skills report output and stop.

## Workflow Discipline

- When asked to update documentation, commit, or do a simple task, do exactly that — don't go on analysis tangents or start investigating new issues.
- Before starting any work, check git status and ensure the working directory is clean. If there are uncommitted changes or stashed work, ask before proceeding.
- If the Read tool returns stale content (especially after git operations), fall back to `cat <file>` via Bash.

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
- **Hardware over emulator comparisons**: Other emulators are reference material, not ground truth. Always attribute emulator-sourced findings explicitly ("SameBoy does X") rather than as hardware fact.
- **Data-driven debugging**: Use available data resources (gbtrace, gb-propagation-delay-analysis, gb-timing-data, slowpeek) rather than reasoning about behavior from code alone. Observe first, hypothesize second.
- **Future cores**: These principles apply to any emulation core added to the project, not just Game Boy.

## Architecture

The project is a Cargo workspace with crates under `crates/`:

- **`crates/missingno-gb/`** (`missingno-gb`) — Core emulation library. No GUI dependencies (only `bitflags` and `rgb`). Contains:
  - **`crates/missingno-gb/src/`** — Core emulation. `GameBoy` owns all hardware components directly (`Cpu`, `Ppu`, `Audio`, `Joypad`, `Timers`, `Dma`, `ExternalBus`, `HighRam`, etc.). `GameBoy::step()` executes one instruction and returns a `StepResult` with `new_screen` and `dots` (T-cycle count).
  - **`crates/missingno-gb/src/debugger/`** — Debugging backend. Wraps `GameBoy` with breakpoints, stepping, disassembly, and a T-cycle counter.
  - **`crates/missingno-gb/tests/accuracy/`** — Integration tests (ROM-based accuracy tests).
- **`crates/missingno/`** (`missingno`) — Iced 0.14 GUI binary. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer. Lives in `crates/missingno/src/app/`.

### Instruction Execution

`GameBoy::step()` in `crates/missingno-gb/src/execute.rs` runs one instruction in two phases:

1. **Fetch/decode**: Reads the opcode byte, ticks hardware, then reads operand bytes one at a time (ticking hardware after each). `operand_count()` determines byte count from the opcode alone. The buffered bytes are passed to `Instruction::decode()`.
2. **Process**: The `Cpu` state machine (`crates/missingno-gb/src/cpu/mcycle/`) yields one `DotAction` per dot for post-decode work (memory reads/writes, internal cycles). The step loop executes each action and ticks hardware via `next_dot()`.

The M-cycle logic is split across three files in `crates/missingno-gb/src/cpu/mcycle/`:
- `mod.rs` — `DotAction` enum, `BusDot` ring counter model, `BusAction` enum, and `next_dot()` method
- `build.rs` — Constructs the action sequence for each instruction type
- `apply.rs` — Pure CPU mutations (ALU, flags, DAA, etc.)

### Clock Model and Phase Architecture

The Game Boy's master clock produces alternating edges. On hardware, each edge triggers specific circuits — there is no inherent "first" or "second" edge within a dot. The CPU and PPU are clocked by the same master clock and tick in lockstep.

**Emulator model**: `execute.rs` alternates `rise()` and `fall()` calls. One dot = one `rise()` + one `fall()`. The CPU and PPU both do work on each edge:
- `rise()`: PPU pixel output (`ppu.rise()`), CPU state advance (`next_dot()`), CPU reads
- `fall()`: PPU fetcher/control (`ppu.fall()`), CPU bus writes

**There is no ordering between rise and fall.** They are alternating edges in a continuous clock. Do not reason about "rise happens before fall" — think about which edge a DFF captures on and which edge reads it.

**DFF visibility**: When a DFF captures on edge E, the output holds that value until the next capture. No "same edge" vs "next edge" distinction. `DffLatch`: `write()` sets pending, `tick()` resolves to output (capture edge), `output()` reads last captured value.

**CPU bus writes**: Action determined in `rise()` via `next_dot()`, executed in `fall()` via `drive_ppu_bus()`. DFF9 registers (LCDC, SCY, SCX) use the "early write path" before `ppu.fall()`. DFF8 palette registers (BGP, OBP0, OBP1) use early write + `tick_palette_latches()` inside `ppu.fall()`. To add registers to the early write path, update the match at `execute.rs` line ~398.

**GateBoy conventions**: 8 sub-phases (A-H) per M-cycle, 2 per dot. `mcycle_phase` packs ring counter DFFs: 0x0C=B, 0x0F=D, 0x03=F, 0x00=H. `_evn` DFFs latch on EVEN edges; `_odd` on ODD. CPU register writes latch at DELTA_GH (first visible at phase H). See `receipts/research/` for phase mapping docs.

**Common pitfalls**: (1) Never frame timing hypotheses as "move X before/after Y in rise/fall" — think about DFF capture edges and combinational read points. (2) Multi-stage pipeline fixes: if a fix has zero effect, check whether another pipeline stage compensates — both stages may need fixing together.

### Key Patterns

- **Flat component ownership**: `GameBoy` owns all hardware components as separate fields (`cpu`, `ppu`, `audio`, `timers`, `interrupts`, `dma`, etc.) so subsystems can be borrowed independently.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Enum-based MBC dispatch**: `Mbc` enum in `crates/missingno-gb/src/cartridge/mbc/mod.rs` with variants for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147. ROM data is owned by `Cartridge` and passed to MBC `read()` methods as `&[u8]`.
- **PPU state machine**: `Ppu` holds an `Option<Rendering>` — `None` when the LCD is off (hardware reset state), `Some(Rendering)` when on. `Rendering` persists through both active display and VBlank (matching hardware where pixel circuits are always present when LCD is on). Modes are derived from `video.vblank` and scanning state within `Rendering`. Draws pixels one at a time with cycle-accurate timing.
- **Propagation delay analysis**: The sibling project [`gb-propagation-delay-analysis`](https://github.com/ajoneil/gb-propagation-delay-analysis) (local clone: `receipts/resources/gb-propagation-delay-analysis/`) provides static analysis of GateBoy's netlist — signal races, deep combinatorial paths, and propagation delays. Key outputs in `receipts/resources/gb-propagation-delay-analysis/output/`: `race_pairs_report.md` (observable effects by symptom), `critical_paths_report.md` (deepest paths), `signal_concordance.md` (GateBoy cell names ↔ Pan Docs names). For one-dot timing discrepancies, check race pairs first.

- **Execution tracing (gbtrace)**: The sibling project [`gbtrace`](https://github.com/ajoneil/gbtrace) (local clone: `receipts/resources/gbtrace/`) defines a standardised format for recording and comparing Game Boy emulator execution state across multiple emulators. Tracked emulators: gambatte, gateboy, docboy, missingno, sameboy. DocBoy traces at T-cycle granularity. Missingno integrates this behind the `gbtrace` feature flag on `missingno-gb`:
  - **Capturing traces** — `tests/accuracy/common/` wraps `GameBoy` in `TestRun`, which optionally traces each `step()`:
    ```bash
    GBTRACE_PROFILE=gbmicrotest cargo test -p missingno-gb --features gbtrace -- <test_name>
    ```
    Writes to `receipts/traces/<rom_name>.gbtrace`. Profiles are per-suite TOML files in `receipts/resources/gbtrace/test-suites/*/profile.toml`.
  - **gbtrace CLI** — Build with `cargo build -p gbtrace --features cli` from `receipts/resources/gbtrace/`. Key commands:
    - `gbtrace info <file>` — trace metadata summary.
    - `gbtrace query <file> --where pc=0x0150` — find entries matching conditions (`--context N`, `--max N`, `--last N`, `--range START..END`, `--fields`). Multiple `--where` args for AND conditions (not comma-separated).
    - `gbtrace diff <a> <b>` — compare traces (`--sync`, `--fields`, `--exclude`, `--summary`).
    - `gbtrace frames <file>` — frame boundaries from LY.
    - `gbtrace render <file> -o <dir>` — render LCD frames to PNG (`--frames 1,3,5`).
    - `gbtrace convert <file>` — convert JSONL to native `.gbtrace` format.
  - **Reference traces**: Hosted at [ajoneil.github.io/gbtrace](https://ajoneil.github.io/gbtrace/) with manifests for 15 test suites. URL pattern: `tests/{suite}/{test}_{emulator}_{status}.gbtrace`. Use the `/compare-traces` skill for structured comparison and individual trace inspection.
- **Boot ROM support**: Optional DMG boot ROM via `--boot-rom <path>` (CLI) or `DMG_BOOT_ROM=<path>` (tests). Boot ROMs are proprietary — never commit them. Without one, post-boot initialization is used. Only use on targeted tests (adds significant startup time).
- **Config**: `settings.ron` and `recent.ron` in platform config dir via `dirs` crate. Uses `jiff` (not `chrono`) for timestamps.

### Data Sources for Debugging and Research

When investigating emulator issues, these data sources are available in priority order. Always check existing data before generating new diagnostics.

1. **Hardware documentation**: Pan Docs, TCAGBD, hardware manuals.
2. **Propagation delay analysis** (`receipts/resources/gb-propagation-delay-analysis/`): Signal races and deep combinatorial paths. See Key Patterns above.
3. **Cross-emulator execution traces** (gbtrace): 5 emulators, 15 test suites. Use for both `diff` and individual inspection. See Key Patterns above.
4. **Hardware timing measurements** (`receipts/resources/gb-timing-data/`): Empirical cycle-level data from real hardware via Slowpeek. Campaigns cover PPU timing (mode 3 duration, sprite penalties, OAM/VRAM lock boundaries) and timer subsystem timing (DIV phase, TIMA increment). Results are CSV files with multi-dimensional sweep data. **Status: data collection in progress** — check `receipts/resources/gb-timing-data/campaigns/` for available campaigns.
5. **Test ROM sources**: Assembly source reveals exactly what tests measure and what expected values mean.
6. **Hardware test harness** (`receipts/resources/slowpeek/`): Programmable harness for cycle-precise measurements on real Game Boy hardware via interrupt-driven sweeps. **Status: emulator-only for now; hardware serial bridge in development.** Note when a Slowpeek test would provide the definitive answer, but do not attempt hardware mode yet.

### Debugger

- **Pane system**: `crates/missingno/src/app/debugger/panes.rs` manages a `pane_grid` of `DebuggerPane` variants (Screen, Instructions, Tiles, TileMap, Sprites, Audio). Each pane is a separate module with a struct (e.g. `ScreenPane`, `InstructionsPane`), a `content()` method returning `pane_grid::Content`, and optionally a `Message` enum. Register new panes by adding to `DebuggerPane` enum, `PaneInstance` enum, `construct_pane()`, `view()`, `available_panes()`, and `Display` impl.
- **Input recording**: `crates/missingno-gb/src/recording.rs` defines the `Recording` data model (ROM header + initial state + input events).

### Resources

External resources live in `receipts/resources/`. Clone or download whatever you need there — it's gitignored. These sibling projects are referenced throughout the skills and documentation:

| Directory | Repository | Description |
|-----------|------------|-------------|
| `gb-propagation-delay-analysis` | https://github.com/ajoneil/gb-propagation-delay-analysis | GateBoy netlist analysis — signal races, critical paths, propagation delays |
| `gbtrace` | https://github.com/ajoneil/gbtrace | Execution trace capture, diff, and render across emulators |
| `gb-timing-data` | https://github.com/ajoneil/gb-timing-data | Cycle-level hardware timing measurements |
| `slowpeek` | https://github.com/ajoneil/slowpeek | Cycle-precise hardware test harness |
