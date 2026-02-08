# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

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

## Architecture

Three layers with strict separation — core emulation has no UI dependencies:

- **`src/game_boy/`** — Core emulation. `GameBoy` owns a `Cpu` and `MemoryMapped` (which aggregates all hardware: cartridge, video, audio, timers, joypad, interrupts). `GameBoy::step()` executes one instruction, ticks all hardware for the instruction's cycle count, and returns `bool` for whether a new video frame was produced.
- **`src/debugger/`** — Debugging backend. Wraps `GameBoy` with breakpoints, stepping, and disassembly.
- **`src/app/`** — Iced 0.14 GUI. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer.

### Key Patterns

- **CPU and memory separation**: `Cpu` and `MemoryMapped` are separate structs so memory subsystems can be borrowed independently. CPU instructions return `OpResult(cycles, Option<MemoryWrite>)` rather than writing directly.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Iterator-based instruction decoding**: `GameBoy` implements `Iterator<Item=u8>` (reading bytes at PC), so `Instruction::decode()` consumes opcode bytes naturally.
- **Trait-based MBC dispatch**: `MemoryBankController` trait with implementations for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147.
- **PPU state machine**: `PixelProcessingUnit` alternates between `Rendering` and `BetweenFrames`. Rendering tracks per-line state (mode 2→3→0) and draws pixels one at a time with cycle-accurate timing.
- **Post-boot register initialization**: The emulator skips the boot ROM. Initial hardware state must match DMG post-boot values (e.g., LCDC=0x91 in `Control::default()`, CPU registers in `Cpu::new()`).

