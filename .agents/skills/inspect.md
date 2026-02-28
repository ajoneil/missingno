# Inspect

Query the headless debugger HTTP API to inspect emulator state without modifying code.

## When to use this instead of `/instrument`

Use this skill when the question can be answered by inspecting state at instruction, dot, or frame boundaries:

- **What are the CPU registers at a given point?** Step to a breakpoint, read `/cpu`.
- **What does the screen look like after N frames?** Step N frames, read `/screen/ascii`.
- **What is the PPU mode/scanline/scroll position at a given PC?** Set a breakpoint, step, read `/ppu`.
- **What is the pixel pipeline state at a specific scanline and mode?** Set a compound watchpoint (e.g. scanline=N AND mode=drawing), step-frame, read `/ppu/pipeline`.
- **What is the pixel pipeline state at a specific dot within a scanline?** Navigate to Mode 3, then use `step-dot` to advance one dot at a time, reading `/ppu/pipeline` after each.
- **When does a mid-scanline register write occur?** Set a bus-write watchpoint, step-frame to catch it, then read `/ppu` for the exact dot/LY/mode at that instruction.
- **Which interrupts are enabled/pending?** Read `/interrupts` at any point.
- **What sprites are active?** Read `/sprites`.
- **What instructions execute from a given address?** Read `/instructions`.
- **What does a ROM do at startup?** Step through instructions and observe state changes.

## When this is NOT enough — stop and tell the user

This API operates at dot and instruction granularity. With `step-dot`, bus watchpoints, and `/ppu/pipeline`, most mid-scanline observations are possible. It **cannot** observe:

- **Sub-dot timing** (what happens within a single dot tick — e.g., the order of operations inside one PPU clock)
- **Audio channel internals** (sample values, timer counters, sweep state)
- **Memory bus conflicts** (DMA bus contention, OAM/VRAM locking during specific modes)

If the question you've been asked requires observing any of the above, you **must** stop immediately and report this to the user — do not attempt a partial answer, do not substitute a coarser measurement and hope it's "close enough", and do not silently return results that don't actually answer the question. The report must be specific:

- **What you were asked**: restate the question.
- **What you can't observe**: name the specific limitation.
- **What would be needed**: either a new debugger endpoint or `/instrument` with code instrumentation.

The user will decide whether to extend the debugger or fall back to `/instrument`. Do not make that decision yourself.

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
| `/ppu/pipeline` | GET | Pixel pipeline internals: bg_shifter (low/high/loaded), obj_shifter (low/high/palette/priority), pixel_counter, render_phase, sprite_fetch phase, sprite_tile_data |
| `/screen` | GET | 144x160 array of **post-palette color indices** (0-3) — large, prefer `/screen/ascii` |
| `/screen/ascii` | GET | 144 strings of 160 chars: ` `=lightest `.`=light `o`=dark `#`=darkest — compact, readable |
| `/sprites` | GET | All 40 OAM entries: id, screen x/y, tile, priority (above_bg/behind_bg), flip_x, flip_y, palette (obp0/obp1), visible |
| `/interrupts` | GET | IE and IF raw values + per-line breakdown (vblank/stat/timer/serial/joypad) with enabled/requested bools |
| `/instructions` | GET | 20 disassembled instructions from current PC (address + mnemonic) |
| `/memory/{hex_addr}` | GET | Single byte at address: value (decimal), hex |
| `/memory/{hex_addr}/{length}` | GET | Range of bytes (1-4096): bytes array, hex array. Bypasses DMA/PPU locking |
| `/vram` | GET | Full VRAM contents: 3 tile blocks (128 tiles each with raw hex, 8x8 pixel grid, non_zero flag) + 2 tile maps (32x32 tile indices) |
| `/breakpoints` | GET | List of breakpoint addresses (hex strings) |

### Execution control

| Endpoint | Method | Returns |
|----------|--------|---------|
| `/step` | POST | Execute one instruction, return CPU state |
| `/step-dot` | POST | Execute one PPU dot, return pipeline state |
| `/step-frame` | POST | Execute until frame boundary, breakpoint, or watchpoint hit. Returns CPU state + `watchpoint_hit` if triggered |
| `/step-over` | POST | Step over current instruction (past CALLs), return CPU state |
| `/reset` | POST | Reset the Game Boy, return CPU state |
| `/breakpoints/{hex_addr}` | PUT | Set breakpoint at address |
| `/breakpoints/{hex_addr}` | DELETE | Clear breakpoint at address |

### Watchpoints

Watchpoints are conditions that stop `step-frame` when matched. Non-bus watchpoints (scanline, mode, registers) check at **dot** granularity. Bus watchpoints check at instruction granularity.

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/watchpoints` | GET | List all active watchpoints |
| `/watchpoints` | POST | Add watchpoint from JSON body (supports compound `all` type) |
| `/watchpoints` | DELETE | Clear all watchpoints |
| `/watchpoints/bus-read/{hex_addr}` | PUT/DELETE | Bus read watchpoint |
| `/watchpoints/bus-write/{hex_addr}` | PUT/DELETE | Bus write watchpoint |
| `/watchpoints/dma-read/{hex_addr}` | PUT/DELETE | DMA source read watchpoint |
| `/watchpoints/dma-write/{hex_addr}` | PUT/DELETE | DMA destination write watchpoint |
| `/watchpoints/scanline/{n}` | PUT/DELETE | Scanline (LY) watchpoint |
| `/watchpoints/ppu-mode/{mode}` | PUT/DELETE | PPU mode watchpoint (hblank/vblank/oam_scan/drawing or 0/1/2/3) |

**Compound watchpoints** (all conditions must match simultaneously):
```bash
curl -s -X POST http://127.0.0.1:3333/watchpoints \
  -d '{"type":"all","conditions":[{"type":"scanline","value":58},{"type":"ppu_mode","mode":"drawing"}]}'
```

**Note on LY timing**: LY increments a few dots before OAM scan begins. A scanline-only watchpoint stops at the first dot where LY matches, which is in the previous scanline's hblank. To stop at the start of actual rendering, use a compound watchpoint: `scanline=N AND mode=oam_scan` or `scanline=N AND mode=drawing`.

## Understanding screen color values

The `/screen` endpoint returns **post-palette color indices** (0-3), not raw tile data. The PPU applies the palette register (BGP/OBP0/OBP1) before writing to the screen buffer. These are the final rendered colors — what the player sees.

The color index scale is: **0 = lightest (white), 3 = darkest (black).**

The test harness (`screen_to_greyscale`) converts these to 8-bit greyscale: `0 → 0xFF, 1 → 0xAA, 2 → 0x55, 3 → 0x00`. So a screen value of 3 corresponds to test greyscale `0x00`, and a screen value of 0 corresponds to `0xFF`. Do not confuse the two scales — screen index 3 is greyscale 0x00 (black), not 0xFF.

## Scope discipline

**You are an observation tool, not a problem-solver.** Follow the same reporting contract as `/instrument`. Your report must contain measurements, not interpretation. If you catch yourself writing "this means..." or "the fix should be..." — stop, delete it, and return to reporting observations.

## Debugging strategy: use watchpoints, not step loops

**Prefer targeted watchpoints over stepping.** The debugger has powerful watchpoint support — use it to jump directly to the state you need to observe rather than stepping through hundreds of dots or instructions manually.

### Anti-pattern: step loops
Do NOT write loops that step dot-by-dot or instruction-by-instruction looking for a condition:
```bash
# BAD — slow, fragile, wastes API calls
for i in $(seq 1 200); do
  result=$(curl -s -X POST http://127.0.0.1:3333/step-dot)
  pc=$(echo "$result" | jq '.pixel_counter')
  if [ "$pc" -ge 112 ]; then break; fi
done
```

### Correct pattern: targeted navigation
Use watchpoints to land exactly where you need, then read state:
```bash
# GOOD — jump directly to the state of interest
curl -s -X POST http://127.0.0.1:3333/watchpoints \
  -d '{"type":"all","conditions":[{"type":"scanline","value":60},{"type":"ppu_mode","mode":"drawing"}]}'
curl -s -X POST http://127.0.0.1:3333/step-frame
curl -s http://127.0.0.1:3333/ppu
curl -s -X DELETE http://127.0.0.1:3333/watchpoints
```

### When to use bus watchpoints
Bus watchpoints are the most powerful tool for answering "when does X happen":
- **When is a register written?** `bus-write/{addr}` — catches the exact instruction that writes to a PPU register, VRAM address, or I/O port. Read `/ppu` immediately after to see the scanline, dot, and mode.
- **When is VRAM read?** `bus-read/{addr}` — catches tile data fetches. Useful for understanding what the PPU is reading during Mode 3.
- **When does a DMA transfer touch an address?** `dma-read` / `dma-write` — catches OAM DMA source/destination accesses.

### Combine watchpoints for precision
Use compound watchpoints to narrow down to exactly the event you care about:
```bash
# Stop when the ROM writes to WX (FF4B) during scanline 55
curl -s -X POST http://127.0.0.1:3333/watchpoints \
  -d '{"type":"all","conditions":[{"type":"scanline","value":55},{"type":"bus_write","address":"FF4B"}]}'
```

### Reading VRAM and tile maps directly
Instead of stepping to observe tile fetches, read the tile map and tile data directly:
```bash
# Read BG tile map row (32 bytes per row, base $9800)
curl -s http://127.0.0.1:3333/memory/9800/20  # first 32 bytes = row 0

# Read a specific tile's data (16 bytes per tile, base $8000)
curl -s http://127.0.0.1:3333/memory/8000/10  # tile 0 data (16 bytes)

# Read the full VRAM dump with decoded tiles and tile maps
curl -s http://127.0.0.1:3333/vram
```

### The only valid uses of step-dot
`step-dot` should be used **sparingly** and only when you need to observe how the pipeline state changes dot-by-dot within a very small window (< 10 dots) — for example, observing the exact dot where a sprite fetch begins, or watching a tile boundary transition. Always navigate to the area of interest with a watchpoint first, then use step-dot for the final few dots.

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

### Catching a register write

```bash
# When does the ROM write to WX (FF4B)?
curl -s -X PUT http://127.0.0.1:3333/watchpoints/bus-write/FF4B
curl -s -X POST http://127.0.0.1:3333/step-frame
# Now stopped at the instruction that wrote to WX
curl -s http://127.0.0.1:3333/ppu    # see LY, dot, mode, and the new WX value
curl -s http://127.0.0.1:3333/cpu    # see PC, registers — what code did this?
curl -s -X DELETE http://127.0.0.1:3333/watchpoints
```

### Observing screen output

```bash
# Step several frames to let the ROM initialize
for i in $(seq 1 60); do curl -s -X POST http://127.0.0.1:3333/step-frame > /dev/null; done

# Read the screen
curl -s http://127.0.0.1:3333/screen/ascii | jq -r '.lines[]'
```

### Navigating to a specific scanline and mode

```bash
# Set compound watchpoint: stop at first dot of drawing mode on scanline 58
curl -s -X POST http://127.0.0.1:3333/watchpoints \
  -d '{"type":"all","conditions":[{"type":"scanline","value":58},{"type":"ppu_mode","mode":"drawing"}]}'

# Step until it hits
curl -s -X POST http://127.0.0.1:3333/step-frame

# Read PPU state — LY, dot, mode, scroll positions, palettes
curl -s http://127.0.0.1:3333/ppu

# If needed, step a few dots to observe a transition
curl -s -X POST http://127.0.0.1:3333/step-dot
curl -s http://127.0.0.1:3333/ppu/pipeline

# Clean up
curl -s -X DELETE http://127.0.0.1:3333/watchpoints
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

Write a measurement receipt to the investigation's `measurements/` folder using the same format as `/instrument`:

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
