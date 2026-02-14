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

## Project Overview

MissingNo. is a Game Boy emulator and debugger written in Rust.

## Build and Run Commands

```bash
cargo run --release                          # Build and run
cargo run --release -- path/to/rom.gb        # Load a ROM
cargo run --release -- path/to/rom.gb --debugger  # Load with debugger
cargo check                                  # Type check
cargo test                                   # Run tests
cargo clippy                                 # Lint
cargo fmt                                    # Format
```

## Emulation Philosophy

- **Hardware fidelity**: Model the hardware as closely as possible so correct behavior emerges naturally. Avoid hacks and special-case workarounds — if something needs a hack to work, the underlying model is wrong.
- **Code as documentation**: The code should teach the reader how the hardware works. Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself, not from comments. Strike a balance between clarity and jargon; assume the reader is a competent programmer but not necessarily a domain expert in the specific hardware.
- **Future cores**: These principles apply to any emulation core added to the project, not just Game Boy.

## Architecture

Three layers with strict separation — core emulation has no UI dependencies:

- **`src/game_boy/`** — Core emulation. `GameBoy` owns a `Cpu` and `MemoryMapped` (which aggregates all hardware: cartridge, video, audio, timers, joypad, interrupts). `GameBoy::step()` executes one instruction and returns `bool` for whether a new video frame was produced.
- **`src/debugger/`** — Debugging backend. Wraps `GameBoy` with breakpoints, stepping, and disassembly.
- **`src/app/`** — Iced 0.14 GUI. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer.

### Instruction Execution

`GameBoy::step()` in `src/game_boy/execute.rs` runs one instruction in two phases:

1. **Fetch/decode**: Reads the opcode byte, ticks hardware, then reads operand bytes one at a time (ticking hardware after each). `operand_count()` determines byte count from the opcode alone. The buffered bytes are passed to `Instruction::decode()`.
2. **Process**: A `Processor` state machine (`src/game_boy/cpu/mcycle/`) yields one `BusAction` per M-cycle for post-decode work (memory reads/writes, internal cycles). The step loop executes each action and ticks hardware.

The `Processor` is split across three files:
- `mod.rs` — `Phase` enum, `BusAction` enum, `Processor` struct and `next()` method
- `build.rs` — Constructs the `Phase` for each instruction type
- `apply.rs` — Pure CPU mutations (ALU, flags, DAA, etc.)

### Key Patterns

- **CPU and memory separation**: `Cpu` and `MemoryMapped` are separate structs so memory subsystems can be borrowed independently.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Enum-based MBC dispatch**: `Mbc` enum in `src/game_boy/cartridge/mbc/mod.rs` with variants for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147. ROM data is owned by `Cartridge` and passed to MBC `read()` methods as `&[u8]`.
- **PPU state machine**: `PixelProcessingUnit` alternates between `Rendering` and `BetweenFrames`. Rendering tracks per-line state (mode 2→3→0) and draws pixels one at a time with cycle-accurate timing.
- **Post-boot register initialization**: The emulator skips the boot ROM. Initial hardware state must match DMG post-boot values (e.g., LCDC=0x91 in `Control::default()`, CPU registers in `Cpu::new()`).
- **Serialization**: Uses `nanoserde` with RON format for save states, config (`~/.config/missingno/settings.ron`, `recent.ron`), and input recordings.
- **Timestamps**: Uses the `jiff` crate (not `chrono`) for date/time formatting.

### Debugger

- **Pane system**: `src/app/debugger/panes.rs` manages a `pane_grid` of `DebuggerPane` variants. Each pane is a separate module with a struct (e.g. `CpuPane`, `PlaybackPane`), a `content()` method returning `pane_grid::Content`, and optionally a `Message` enum with `Into<app::Message>` impl for routing through the nested message chain (`PaneMessage` → `panes::Message` → `debugger::Message` → `app::Message`). Register new panes by adding to `DebuggerPane` enum, `PaneInstance` enum, `construct_pane()`, `view()`, `available_panes()`, and `Display` impl.
- **Input recording**: `src/game_boy/recording.rs` defines the `Recording` data model (ROM header + initial state + input events), serialized as RON to `.mnrec` files. Recording state (`ActiveRecording`) lives in `src/app/debugger/mod.rs` on the `Debugger` struct — `press_button`/`release_button` log events with frame numbers during recording. The Playback pane (`src/app/debugger/playback.rs`) provides the UI.
