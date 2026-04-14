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

This API operates at half-phase, dot, and instruction granularity. With `step-phase`, `step-dot`, bus watchpoints, `/ppu` (including scan counter), and `/ppu/pipeline`, most mid-scanline observations are possible. It **cannot** observe:

- **Sub-dot timing** (what happens within a single dot tick — e.g., the order of operations inside one PPU clock)
- **Audio channel internals** (sample values, timer counters, sweep state)
- **Memory bus conflicts** (DMA bus contention, OAM/VRAM locking during specific modes)

If the question you've been asked requires observing any of the above, you **must** stop immediately and report this to the user — do not attempt a partial answer, do not substitute a coarser measurement and hope it's "close enough", and do not silently return results that don't actually answer the question. The report must be specific:

- **What you were asked**: restate the question.
- **What you can't observe**: name the specific limitation.
- **What would be needed**: either a new debugger endpoint or `/instrument` with code instrumentation.

The user will decide whether to extend the debugger or fall back to `/instrument`. Do not make that decision yourself.

## Observation plan (mandatory first step)

**Before making any API calls**, write the following plan as a preamble in the measurement receipt file:

```markdown
## Observation plan

### Question
<What specific question is this measurement answering? One sentence.>

### Strategy
<What navigation steps will answer it? Be specific:>
- Start server with: gb_start <rom_path>
- Navigate to: gb_goto <scanline> <mode> (or gb_run_frames <n>, etc.)
- Read state with: gb_ppu, gb_pipeline, gb_sprites_on <scanline>, etc.
- If stepping dots: how many dots, and what data to capture at each?

### Expected data shape
<What fields from which endpoints? What values would confirm vs refute?>
- Endpoint X returns field Y — expecting value Z if hypothesis is correct
- If Y != Z, hypothesis is refuted because...

### Confounds to check
<What could interfere with the measurement? Check BEFORE collecting data.>
- Sprites overlapping the area of interest?
- Window enabled on this scanline?
- SCX/SCY shifting tiles away from expected positions?
```

**Write this plan to the receipt file first, then execute it.** The plan prevents wasted round-trips — you catch problems (like sprite overlays, wrong scanline, missing tile data) before burning API calls on them.

If the plan reveals a confound (e.g. sprites cover the area of interest), adjust the strategy in the plan before proceeding. If no clean observation is possible, report that in the receipt — do not collect data that can't answer the question.

## Server management

**Always use the helper library** for server lifecycle. Source it at the start of every inspect session:

```bash
. scripts/debugger.sh
```

### Starting a server

```bash
gb_start crates/missingno-gb/tests/accuracy/roms/dmg-acid2/dmg-acid2.gb
# Prints: ready (pid 12345)
```

`gb_start` handles everything: kills any existing server on the port, starts a new headless server, waits for it to be ready (polls `/cpu` with retries), and prints "ready" or fails with an error. **Never manage the server process manually** — no `cargo run &`, no `pkill`, no `lsof`.

**Boot ROM.** If the investigation needs to observe boot state, ask the user for a DMG boot ROM path (boot ROMs are proprietary and cannot be in the repo), then pass `--boot-rom <path>` to the headless server. Since `gb_start` doesn't accept extra flags, start the server manually in this case:
```bash
gb_stop 2>/dev/null
cargo run -- "$rom_path" --headless --boot-rom "$boot_rom_path" &>/dev/null &
GB_PID=$!
# Then poll for readiness as gb_start does
```
Only use this when boot state is suspected to play a role — the boot ROM adds significant startup time.

### Checking and stopping

```bash
gb_ensure          # Returns 0 if server is running, 1 if not
gb_stop            # Kills the server cleanly
```

### ROM paths

Test ROMs live under `crates/missingno-gb/tests/accuracy/roms/` (e.g. `crates/missingno-gb/tests/accuracy/roms/dmg-acid2/dmg-acid2.gb`). Always verify the path exists before starting the server.

## Helper functions reference

The helper library (`scripts/debugger.sh`) provides functions for all common operations. **Always use these instead of raw curl commands.** They handle JSON parsing with `jq` — no inline Python.

**CRITICAL: Use helpers for data collection, not raw curl.** When collecting data in a loop (stepping phases, stepping dots, reading state), use `gb_step_phases`, `gb_step_dots`, `gb_ppu`, `gb_pipeline`, etc. Do NOT write your own curl loops that duplicate what the helpers already do. If a helper's output format doesn't include a field you need, you may supplement with a single raw curl call per step — but the stepping and primary data collection should go through the helpers. This prevents fragile hand-rolled jq filters and keeps measurements consistent across inspections.

### Navigation

**`gb_reset`** — Reset the Game Boy and clear all watchpoints. No output.

**`gb_run_frames <n>`** — Step N frames silently. No output. Uses POST `/step-frame` in a loop.

**`gb_goto <scanline> <mode>`** — Jump to a specific scanline and mode. Sets a compound watchpoint (scanline + ppu_mode), runs step-frame, clears the watchpoint, then prints PPU state via `gb_ppu`. Mode values: `oam_scan`, `drawing`, `hblank`, `vblank`. Output: same as `gb_ppu` (one line).

**`gb_step_to_px <value>`** — Jump to a specific pixel_counter value on the current scanline. Reads current LY, sets a compound watchpoint (scanline + drawing + pixel_counter), runs step-frame, clears the watchpoint, then prints pipeline state via `gb_pipeline`. Output: same as `gb_pipeline` (one line).

**`gb_step_dots <n>`** — Step N dots, printing a table. Each row calls POST `/step-dot` then GET `/ppu`. Output columns:
```
step  lx    pc   loaded  lo     hi     sprite
1     20    0    true    255    0      none
2     20    1    true    255    0      none
```
- `step`: 1-based index
- `lx`: M-cycle counter from `/ppu`
- `pc`: pixel_counter from `/step-dot` response
- `loaded`: bg_shifter.loaded (true/false)
- `lo`/`hi`: bg_shifter.low/high
- `sprite`: sprite_fetch phase or "none"

**`gb_step_phases <n>`** — Step N half-phases, printing a table. Each row calls POST `/step-phase` then GET `/ppu`. Output columns:
```
step  phase  lx   scan  mode  pc   loaded
1     high   0    5     0     0    false
2     low    0    5     0     0    false
```
- `step`: 1-based index
- `phase`: "high" or "low" (clock level after this step)
- `lx`: M-cycle counter from `/ppu`
- `scan`: scan_counter from `/ppu` (OAM scan counter 0-39, or "-" when null)
- `mode`: stat.mode_number from `/ppu` (0-3)
- `pc`: pixel_counter from `/step-phase` response
- `loaded`: bg_shifter.loaded (true/false)

### State reading

**`gb_ppu`** — Read PPU registers. Calls GET `/ppu`. Output:
```
LY=0 lx=20 mode=3 scan=39 SCX=0 SCY=0 WX=0 WY=0 BGP=[0,3,3,3]
```
`scan` shows "n/a" when scan_counter is null (not rendering).

**`gb_pipeline`** — Read pixel pipeline state. Calls GET `/ppu/pipeline`. Output:
```
pc=5 loaded=true lo=255 hi=0 phase=null sprite=none
```

**`gb_cpu`** — Read CPU registers. Calls GET `/cpu`. Output:
```
A=145 B=0 C=19 D=0 E=216 H=1 L=77 PC=352 SP=65534 IME=false halted=false
```

**`gb_timers`** — Read timer registers. Calls GET `/timers`. Output:
```
DIV=44 TIMA=0 TMA=0 TAC=0 enabled=false freq=4096 internal=00b0
```

**`gb_screenshot <path>`** — Save the screen as a BMP file. Calls GET `/screen/bitmap` and writes to the given path. Use this to visually compare test output. Example: `gb_screenshot /tmp/lcdon_test.bmp`. The file can then be read with the Read tool (which can display images).

**`gb_screen_row <row>`** — Read one screen row. Calls GET `/screen`. Output: space-separated color indices (0-3), 160 values.

**`gb_sprites_on <scanline>`** — List sprites visible on a scanline. Calls GET `/sprites`, filters by Y range. Output: one line per sprite, or "no sprites on scanline N".
```
id=0 x=8 y=16 tile=0 prio=above_bg
```

**`gb_tile_data <tile_id> [row]`** — Decode tile pixels from VRAM (2bpp → color indices 0-3). Calls GET `/memory`. Without row arg: all 8 rows. With row arg: single row. Output:
```
row 0: 0 0 0 0 0 0 0 0
row 1: 3 3 3 3 3 3 3 3
```

**`gb_tile_map_row <row>`** — Read one row of the BG tile map (32 entries). Calls GET `/memory`. Output: `col:tile_id` pairs.
```
0:0  1:0  2:1  3:1  ...
```

### Raw API access

For operations not covered by helpers (breakpoints, bus watchpoints, memory reads), use curl directly with `$GB_URL`:

```bash
# Set a breakpoint
curl -s -X PUT "$GB_URL/breakpoints/0150"

# Read a memory range
curl -s "$GB_URL/memory/9800/32" | jq '.bytes'

# Set a bus-write watchpoint
curl -s -X PUT "$GB_URL/watchpoints/bus-write/FF4B"

# Clear all watchpoints
curl -s -X DELETE "$GB_URL/watchpoints"

# Step one instruction
curl -s -X POST "$GB_URL/step"

# Step one frame (or to next breakpoint/watchpoint)
curl -s -X POST "$GB_URL/step-frame"
```

**Always use `jq` for JSON parsing**, not Python. The `jq` filters are tested against the actual response shapes and won't break on field name mismatches.

## API reference

### JSON field names

These are the exact field names in API responses. Use these in `jq` filters — do not guess.

**`/cpu`**: `a`, `b`, `c`, `d`, `e`, `h`, `l`, `sp`, `pc`, `zero`, `negative`, `half_carry`, `carry`, `ime`, `halted`

**`/ppu`**: `lcdc` (object with `lcd_enable`, `window_tile_map`, `window_enable`, `bg_tile_data`, `bg_tile_map`, `obj_size`, `obj_enable`, `bg_window_enable`), `stat` (object with `mode` (string), `mode_number` (int 0-3)), `ly`, `lx` (M-cycle counter, increments every 4 dots, 0-113 per scanline), `lyc`, `scan_counter` (int 0-39 or null when not rendering — OAM scan counter, XUPY-clocked, triggers AVAP at 39), `scx`, `scy`, `wx`, `wy`, `bgp` (object with `colors` array), `obp0`, `obp1`

**`/ppu/pipeline`**: `pixel_counter`, `render_phase`, `bg_shifter` (object with `low`, `high`, `loaded` (bool)), `obj_shifter` (object with `low`, `high`, `palette`, `priority`), `sprite_fetch` (string or null), `sprite_tile_data`

**`/sprites`**: Bare JSON array (not `{"sprites": [...]}`). Each entry: `id`, `x`, `y`, `tile`, `priority` (`"above_bg"` or `"behind_bg"`), `flip_x`, `flip_y`, `palette` (`"obp0"` or `"obp1"`), `visible`

**`/screen`**: `pixels` (144-element array of 160-element arrays of ints 0-3)

**`/screen/ascii`**: `lines` (144-element array of 160-char strings)

**`/memory/{addr}/{len}`**: `bytes` (array of ints), `hex` (array of hex strings). Length parameter is **decimal** (e.g. `/memory/8000/16` for 16 bytes, not `/memory/8000/10`).

**`/step-dot`**: Same shape as `/ppu/pipeline` — returns pipeline state after the dot.

**`/step-phase`**: Same shape as `/ppu/pipeline` plus a `phase` field (`"high"` or `"low"`) indicating the master clock level after execution. `"high"` = rise() just ran (next is fall). `"low"` = fall() just ran (next is rise).

**`/timers`**: `div`, `tima`, `tma`, `tac`, `timer_enabled` (bool), `clock_select` (int 0-3), `frequency` (int Hz), `internal_counter` (hex string), `internal_counter_decimal` (int)

### Endpoints

| Endpoint | Method | Returns |
|----------|--------|---------|
| `/cpu` | GET | Registers, flags, IME, halted |
| `/ppu` | GET | LCDC, STAT, LY, lx, LYC, scan_counter, scroll/window regs, palettes |
| `/ppu/pipeline` | GET | Pixel pipeline: shifters, pixel_counter, render_phase, sprite_fetch |
| `/screen` | GET | 144x160 color index array (0-3) — large, prefer `/screen/ascii` or `/screen/bitmap` |
| `/screen/ascii` | GET | 144 strings of 160 chars: ` `=lightest `.`=light `o`=dark `#`=darkest |
| `/screen/bitmap` | GET | 160x144 greyscale BMP image (`Content-Type: image/bmp`). Save to file and view. |
| `/tiles/bitmap` | GET | All 384 tiles (3 blocks × 128) in a 16×24 grid, 128×192 greyscale BMP. |
| `/tilemap/0/bitmap` | GET | Tile map 0 as 256×256 greyscale BMP (32×32 tiles). |
| `/tilemap/1/bitmap` | GET | Tile map 1 as 256×256 greyscale BMP (32×32 tiles). |
| `/sprites` | GET | All 40 OAM entries (bare array) |
| `/timers` | GET | Timer registers (DIV, TIMA, TMA, TAC) and internal counter |
| `/interrupts` | GET | IE and IF values + per-interrupt enabled/requested flags |
| `/instructions` | GET | 20 disassembled instructions from current PC |
| `/memory/{hex_addr}` | GET | Single byte: value + hex |
| `/memory/{hex_addr}/{length}` | GET | Byte range (length is decimal, 1-4096) |
| `/vram` | GET | Full VRAM: 3 tile blocks (decoded) + 2 tile maps |
| `/breakpoints` | GET | List of breakpoint addresses |
| `/step` | POST | Execute one instruction, return CPU state |
| `/step-dot` | POST | Execute one PPU dot, return pipeline state |
| `/step-phase` | POST | Execute one half-phase (rise or fall), return pipeline state + phase |
| `/step-frame` | POST | Run to frame/breakpoint/watchpoint, return CPU state + `watchpoint_hit` |
| `/step-over` | POST | Step over current instruction |
| `/reset` | POST | Reset the Game Boy |
| `/breakpoints/{hex_addr}` | PUT/DELETE | Set/clear breakpoint |
| `/watchpoints` | GET/POST/DELETE | List/add/clear watchpoints |
| `/watchpoints/bus-read/{hex_addr}` | PUT/DELETE | Bus read watchpoint |
| `/watchpoints/bus-write/{hex_addr}` | PUT/DELETE | Bus write watchpoint |
| `/watchpoints/dma-read/{hex_addr}` | PUT/DELETE | DMA source read watchpoint |
| `/watchpoints/dma-write/{hex_addr}` | PUT/DELETE | DMA destination write watchpoint |
| `/watchpoints/scanline/{n}` | PUT/Delete | Scanline watchpoint |
| `/watchpoints/pixel-counter/{n}` | PUT/DELETE | Pixel counter watchpoint (matches during Mode 3 only) |
| `/watchpoints/ppu-mode/{mode}` | PUT/DELETE | PPU mode watchpoint |

### Compound watchpoints

All conditions must match simultaneously:
```bash
curl -s -X POST "$GB_URL/watchpoints" \
  -d '{"type":"all","conditions":[{"type":"scanline","value":58},{"type":"ppu_mode","mode":"drawing"}]}'
```

**Note on LY timing**: LY increments a few dots before OAM scan begins. A scanline-only watchpoint stops at the first dot where LY matches, which is in the previous scanline's hblank. To stop at the start of actual rendering, use a compound watchpoint: `scanline=N AND mode=oam_scan` or `scanline=N AND mode=drawing`.

## Understanding screen color values

The `/screen` endpoint returns **post-palette color indices** (0-3), not raw tile data. The PPU applies the palette register (BGP/OBP0/OBP1) before writing to the screen buffer.

The color index scale is: **0 = lightest (white), 3 = darkest (black).**

The test harness (`screen_to_greyscale`) converts these to 8-bit greyscale: `0 → 0xFF, 1 → 0xAA, 2 → 0x55, 3 → 0x00`. So a screen value of 3 corresponds to test greyscale `0x00` (black), not `0xFF`.

## Scope discipline

**You are an observation tool, not a problem-solver.** Follow the same reporting contract as `/instrument`. Your report must contain measurements, not interpretation. If you catch yourself writing "this means..." or "the fix should be..." — stop, delete it, and return to reporting observations.

**Never read source code.** You have the complete API reference above — endpoint names, JSON field names, helper functions. Do not read `.rs` files, `grep` through the codebase, or explore the code structure. If you catch yourself opening a source file to understand how an endpoint works, stop. The API reference in this skill file is your single source of truth. If an endpoint doesn't exist in the reference, it doesn't exist.

## Debugging strategy: use watchpoints, not step loops

**Prefer targeted watchpoints over stepping.** The debugger has powerful watchpoint support — use it to jump directly to the state you need to observe rather than stepping through hundreds of dots or instructions manually.

### Anti-pattern: step loops and guess-stepping
Do NOT write loops that step dot-by-dot looking for a condition, and do NOT step an estimated number of dots hoping to land near a target:
```bash
# BAD — step loop looking for a condition
for i in $(seq 1 200); do
  result=$(curl -s -X POST "$GB_URL/step-dot")
  pc=$(echo "$result" | jq '.pixel_counter')
  if [ "$pc" -ge 112 ]; then break; fi
done

# BAD — guess-stepping: estimating dot count to reach a pixel_counter value
for i in $(seq 1 60); do curl -s -X POST "$GB_URL/step-dot" > /dev/null; done
# "should be near PX=85..." — no, use a watchpoint
```

### Correct pattern: use helpers and watchpoints
```bash
. scripts/debugger.sh
gb_start crates/missingno-gb/tests/accuracy/roms/dmg-acid2/dmg-acid2.gb
gb_run_frames 10
gb_goto 60 drawing        # Jump directly to scanline 60, Mode 3
gb_step_to_px 85           # Jump directly to pixel_counter=85
gb_step_dots 5             # Step 5 dots from here for fine observation
gb_sprites_on 60           # Check for sprite confounds
```

### Navigating to a specific pixel_counter value
Use `gb_step_to_px` to jump directly to a pixel_counter value. It sets a compound watchpoint (scanline + drawing + pixel_counter) and uses step-frame. **Never estimate dot counts to reach a pixel_counter value** — sprite fetches, window stalls, and fine scroll all affect the mapping between dots and pixel_counter, making estimates unreliable.

```bash
gb_goto 40 drawing        # Navigate to scanline 40, start of Mode 3
gb_step_to_px 88          # Jump to pixel_counter=88
gb_pipeline               # Read pipeline state at that point
gb_step_dots 3            # Step 3 more dots for fine observation
```

You can also use pixel_counter in compound watchpoints directly:
```bash
curl -s -X POST "$GB_URL/watchpoints" \
  -d '{"type":"all","conditions":[{"type":"scanline","value":40},{"type":"ppu_mode","mode":"drawing"},{"type":"pixel_counter","value":88}]}'
curl -s -X POST "$GB_URL/step-frame"
```

### When to use bus watchpoints
Bus watchpoints are the most powerful tool for answering "when does X happen":
- **When is a register written?** `bus-write/{addr}` — catches the exact instruction that writes to a PPU register, VRAM address, or I/O port. Read `gb_ppu` immediately after.
- **When is VRAM read?** `bus-read/{addr}` — catches tile data fetches.
- **When does a DMA transfer touch an address?** `dma-read` / `dma-write`.

### When to use step-dot
`step-dot` (via `gb_step_dots`) is for observing how the pipeline state changes dot-by-dot within a small window. Always navigate to the area of interest first with `gb_goto` or `gb_step_to_px`, then use `gb_step_dots` for the final few dots of fine observation.

**Before stepping dots**, the observation plan must state:
1. Exactly how many dots to step
2. What pipeline fields to watch at each dot
3. What transition or value would confirm/refute the hypothesis

## Reporting results

Write a measurement receipt to the investigation's `measurements/` folder:

```markdown
# Measurement: <short title>

## Observation plan

### Question
<What specific question is this measurement answering?>

### Strategy
<Navigation and data collection steps>

### Expected data shape
<What fields, what values would confirm vs refute>

### Confounds to check
<Potential interference — sprites, window, scroll — and results of checking>

## Test result
<what was observed>

## Measurements
<specific values from the debugger API responses>

## Also observed
<unexpected findings — optional>
```

## After measurement is complete

1. Write the measurement receipt.
2. **Do not update `summary.md`.** The caller owns summary.md.
3. **Stop.** Your job is done. The caller reads the receipt file and decides what to do next.
