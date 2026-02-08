# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

MissingNo. is a Game Boy emulator and debugger written in Rust. Only Tetris is known to be fully playable.

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

## Architecture

Three layers with strict separation:

- **`src/game_boy/`** — Core emulation (UI-independent). Contains CPU, memory, video (PPU), audio (APU), cartridge/MBC, joypad, interrupts, and timers. `GameBoy` struct owns a `Cpu` and `MemoryMapped` (aggregates all hardware subsystems). The `step()` method executes one instruction and ticks all hardware, returning `bool` for whether a new video frame was produced.

- **`src/debugger/`** — Debugging backend (UI-independent). Breakpoint management, stepping logic, and instruction disassembly. Wraps `GameBoy`.

- **`src/app/`** — Iced 0.14 GUI using Elm architecture (Message enum → `update()` → `view()`). Contains emulator mode, debugger mode (with panes for CPU state, disassembly, breakpoints, audio/video inspection), wgpu shader-based rendering, and audio output via cpal. `App` owns the `AudioOutput` and drains audio samples from the `GameBoy` after each emulator/debugger update.

### Key Patterns

- **Memory-mapped I/O**: `MemoryMapped` struct routes `read()`/`write()` to hardware subsystems by address, allowing independent borrowing.
- **Iterator-based instruction decoding**: `GameBoy` implements `Iterator<Item=u8>` so `Instruction::decode()` consumes bytes naturally.
- **State machine for UI modes**: `Game` enum (`Unloaded | Loading | Loaded`) and `LoadedGame` enum (`Debugger | Emulator`) manage application state transitions.
- **Trait-based MBC dispatch**: `MemoryBankController` trait with implementations for NoMbc, MBC1, MBC2, MBC3, selected at runtime from cartridge header byte 0x147.
- **Cycle-accurate simulation**: Timers, video, and audio tick based on instruction cycle counts; interrupts checked after each instruction.
- **Audio pipeline**: APU ticks once per M-cycle in the step loop, generating samples at 44100 Hz into an internal buffer. The app layer drains this buffer and pushes samples through a lock-free ring buffer (rtrb) to a cpal output stream running on a separate thread.
