# Inspect

Query the headless debugger HTTP API to inspect emulator state without modifying code.

## When to use this instead of `/measure`

Use this skill when the question can be answered by inspecting state at instruction or frame boundaries:

- **What are the CPU registers at a given point?** Step to a breakpoint, read `/cpu`.
- **What does the screen look like after N frames?** Step N frames, read `/screen/ascii`.
- **What is the PPU mode/scanline/scroll position at a given PC?** Set a breakpoint, step, read `/ppu`.
- **Which interrupts are enabled/pending?** Read `/interrupts` at any point.
- **What sprites are active?** Read `/sprites`.
- **What instructions execute from a given address?** Read `/instructions`.
- **What does a ROM do at startup?** Step through instructions and observe state changes.

## When this is NOT enough — stop and tell the user

This API operates at instruction/frame granularity. It **cannot** observe:

- **Sub-instruction timing** (T-cycle or dot-level behavior within a single instruction)
- **Mid-scanline state changes** (what happens at dot 80 vs dot 252 within one scanline)
- **DFF latch propagation** (transitional values that exist for a single dot)
- **FIFO/fetcher internals** (pixel pipeline state that isn't exposed through registers)
- **Audio channel internals** (sample values, timer counters, sweep state)
- **Memory bus conflicts** (DMA bus contention, OAM/VRAM locking during specific modes)

If the question you've been asked requires observing any of the above, you **must** stop immediately and report this to the user — do not attempt a partial answer, do not substitute a coarser measurement and hope it's "close enough", and do not silently return results that don't actually answer the question. The report must be specific:

- **What you were asked**: restate the question.
- **What you can't observe**: name the specific limitation (e.g., "dot-level timing within a scanline", "pixel pipeline FIFO state").
- **What would be needed**: either a new debugger endpoint or `/measure` with code instrumentation.

The user will decide whether to extend the debugger or fall back to `/measure`. Do not make that decision yourself.

## Prerequisites

The headless server must be running. If it isn't, start it:

```bash
cargo run -- <rom_path> --headless &
```

**Do not use `--release` unless the user explicitly asks for it.** Debug builds compile much faster and are sufficient for inspection.

**ROM paths**: Test ROMs live under `core/tests/game_boy/roms/` (e.g. `core/tests/game_boy/roms/mooneye/acceptance/foo.gb`). Note that test source code references paths relative to the `roms/` directory (e.g. `mooneye/acceptance/foo.gb`), but `cargo run` needs the full path from the project root. Always verify the path exists before starting the server.

It listens on `http://127.0.0.1:3333`. All responses are JSON.

## API reference

### State inspection

| Endpoint | Method | Returns |
|----------|--------|---------|
| `/cpu` | GET | Registers (a-l), SP, PC, flags (zero/negative/half_carry/carry), IME, halted |
| `/ppu` | GET | LCDC (decoded flags), STAT (mode name + number), LY, dot (0-455 within scanline), LYC, SCX, SCY, WX, WY, palettes (BGP/OBP0/OBP1 with color index breakdown) |
| `/screen` | GET | 144x160 array of palette indices (0-3) — large, prefer `/screen/ascii` |
| `/screen/ascii` | GET | 144 strings of 160 chars: ` `=lightest `.`=light `o`=dark `#`=darkest — compact, readable |
| `/sprites` | GET | All 40 OAM entries: id, screen x/y, tile, priority (above_bg/behind_bg), flip_x, flip_y, palette (obp0/obp1), visible |
| `/interrupts` | GET | IE and IF raw values + per-line breakdown (vblank/stat/timer/serial/joypad) with enabled/requested bools |
| `/instructions` | GET | 20 disassembled instructions from current PC (address + mnemonic) |
| `/breakpoints` | GET | List of breakpoint addresses (hex strings) |

### Execution control

| Endpoint | Method | Returns |
|----------|--------|---------|
| `/step` | POST | Execute one instruction, return CPU state |
| `/step-frame` | POST | Execute until frame boundary or breakpoint, return CPU state |
| `/step-over` | POST | Step over current instruction (past CALLs), return CPU state |
| `/reset` | POST | Reset the Game Boy, return CPU state |
| `/breakpoints/{hex_addr}` | PUT | Set breakpoint at address |
| `/breakpoints/{hex_addr}` | DELETE | Clear breakpoint at address |

## Scope discipline

**You are an observation tool, not a problem-solver.** Follow the same reporting contract as `/measure`. Your report must contain measurements, not interpretation. If you catch yourself writing "this means..." or "the fix should be..." — stop, delete it, and return to reporting observations.

## How to use

### Basic pattern: observe state at a point of interest

```bash
# Set a breakpoint where you want to observe
curl -s -X PUT http://127.0.0.1:3333/breakpoints/0150

# Run until the breakpoint
curl -s -X POST http://127.0.0.1:3333/step-frame

# Read state
curl -s http://127.0.0.1:3333/cpu
curl -s http://127.0.0.1:3333/ppu
curl -s http://127.0.0.1:3333/interrupts

# Clean up breakpoint
curl -s -X DELETE http://127.0.0.1:3333/breakpoints/0150
```

### Stepping through code

```bash
# Step one instruction at a time and observe
curl -s -X POST http://127.0.0.1:3333/step   # returns CPU state after each step
curl -s -X POST http://127.0.0.1:3333/step
curl -s http://127.0.0.1:3333/instructions    # see what's coming next
```

### Observing screen output

```bash
# Step several frames to let the ROM initialize
for i in $(seq 1 60); do curl -s -X POST http://127.0.0.1:3333/step-frame > /dev/null; done

# Read the screen
curl -s http://127.0.0.1:3333/screen/ascii | jq -r '.lines[]'
```

### Comparing state before and after

```bash
# Capture state before
curl -s http://127.0.0.1:3333/cpu > /tmp/before.json

# Do something
curl -s -X POST http://127.0.0.1:3333/step-frame > /dev/null

# Capture state after and diff
curl -s http://127.0.0.1:3333/cpu > /tmp/after.json
diff /tmp/before.json /tmp/after.json
```

## Reporting results

Write a measurement receipt to the investigation's `measurements/` folder using the same format as `/measure`:

```markdown
# Measurement: <short title>

## Question
<the question being tested>

## Test result
<what was observed>

## Measurements
<specific values from the debugger API responses>

## Also observed
<unexpected findings — optional>
```

## After measurement is complete

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

1. Write the measurement receipt.
2. **Do not update `summary.md`.** The caller owns summary.md.
3. **Resume as the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and immediately continue working as the caller.

**The turn does not end here.** Do NOT stop after writing the receipt.
