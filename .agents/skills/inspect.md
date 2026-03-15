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
gb_start core/tests/game_boy/roms/dmg-acid2.gb
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

Test ROMs live under `core/tests/game_boy/roms/` (e.g. `core/tests/game_boy/roms/dmg-acid2.gb`). Always verify the path exists before starting the server.

## Helper functions reference

The helper library (`scripts/debugger.sh`) provides functions for all common operations. **Always use these instead of raw curl commands.** They handle JSON parsing with `jq` — no inline Python.

### Navigation

| Function | Description |
|----------|-------------|
| `gb_reset` | Reset the Game Boy and clear all watchpoints |
| `gb_run_frames <n>` | Step N frames silently |
| `gb_goto <scanline> <mode>` | Set compound watchpoint, step to it, clear watchpoint, print PPU state. Mode: `oam_scan`, `drawing`, `hblank`, `vblank` |
| `gb_step_to_px <value>` | Jump to a specific pixel_counter value on the current scanline (sets compound watchpoint: scanline + drawing + pixel_counter, step-frame, clears). Prints pipeline state |
| `gb_step_dots <n>` | Step N dots, print table: step, lx, pixel_counter, loaded, lo, hi, sprite |

### State reading

| Function | Output |
|----------|--------|
| `gb_ppu` | `LY=N lx=N mode=N SCX=N SCY=N WX=N WY=N BGP=[...]` |
| `gb_pipeline` | `pc=N loaded=T/F lo=N hi=N phase=X sprite=X` |
| `gb_cpu` | `A=N B=N C=N D=N E=N H=N L=N PC=N SP=N IME=N halted=T/F` |
| `gb_screen_row <row>` | Space-separated color indices (0-3) for one screen row |
| `gb_sprites_on <scanline>` | Sprites visible on that scanline: `id=N x=N y=N tile=N prio=X` |
| `gb_tile_data <tile_id> [row]` | Decoded tile pixels (2bpp → color indices). All 8 rows, or one row if specified |
| `gb_tile_map_row <row>` | Tile indices for one BG tile map row: `col:tile_id` pairs |

### Raw API access

For operations not covered by helpers (breakpoints, bus watchpoints, memory reads), use curl directly with `$GB_URL`:

```bash
# Set a breakpoint
curl -s -X PUT "$GB_URL/breakpoints/0150"

# Read a memory range
curl -s "$GB_URL/memory/9800/32" | jq '.bytes'

# Set a bus-write watchpoint
curl -s -X PUT "$GB_URL/watchpoints/bus-write/FF4B"
```

**Always use `jq` for JSON parsing**, not Python. The `jq` filters are tested against the actual response shapes and won't break on field name mismatches.

## API reference

### JSON field names

These are the exact field names in API responses. Use these in `jq` filters — do not guess.

**`/cpu`**: `a`, `b`, `c`, `d`, `e`, `h`, `l`, `sp`, `pc`, `zero`, `negative`, `half_carry`, `carry`, `ime`, `halted`

**`/ppu`**: `lcdc` (object with `lcd_enable`, `window_tile_map`, `window_enable`, `bg_tile_data`, `bg_tile_map`, `obj_size`, `obj_enable`, `bg_window_enable`), `stat` (object with `mode` (string), `mode_number` (int 0-3)), `ly`, `lx` (M-cycle counter, increments every 4 dots, 0-113 per scanline), `lyc`, `scx`, `scy`, `wx`, `wy`, `bgp` (object with `colors` array), `obp0`, `obp1`

**`/ppu/pipeline`**: `pixel_counter`, `render_phase`, `bg_shifter` (object with `low`, `high`, `loaded` (bool)), `obj_shifter` (object with `low`, `high`, `palette`, `priority`), `sprite_fetch` (string or null), `sprite_tile_data`

**`/sprites`**: Bare JSON array (not `{"sprites": [...]}`). Each entry: `id`, `x`, `y`, `tile`, `priority` (`"above_bg"` or `"behind_bg"`), `flip_x`, `flip_y`, `palette` (`"obp0"` or `"obp1"`), `visible`

**`/screen`**: `pixels` (144-element array of 160-element arrays of ints 0-3)

**`/screen/ascii`**: `lines` (144-element array of 160-char strings)

**`/memory/{addr}/{len}`**: `bytes` (array of ints), `hex` (array of hex strings). Length parameter is **decimal** (e.g. `/memory/8000/16` for 16 bytes, not `/memory/8000/10`).

**`/step-dot`**: Same shape as `/ppu/pipeline` — returns pipeline state after the dot.

### Endpoints

| Endpoint | Method | Returns |
|----------|--------|---------|
| `/cpu` | GET | Registers, flags, IME, halted |
| `/ppu` | GET | LCDC, STAT, LY, dot, LYC, scroll/window regs, palettes |
| `/ppu/pipeline` | GET | Pixel pipeline: shifters, pixel_counter, render_phase, sprite_fetch |
| `/screen` | GET | 144x160 color index array (0-3) — large, prefer `/screen/ascii` |
| `/screen/ascii` | GET | 144 strings of 160 chars: ` `=lightest `.`=light `o`=dark `#`=darkest |
| `/sprites` | GET | All 40 OAM entries (bare array) |
| `/interrupts` | GET | IE and IF values + per-interrupt enabled/requested flags |
| `/instructions` | GET | 20 disassembled instructions from current PC |
| `/memory/{hex_addr}` | GET | Single byte: value + hex |
| `/memory/{hex_addr}/{length}` | GET | Byte range (length is decimal, 1-4096) |
| `/vram` | GET | Full VRAM: 3 tile blocks (decoded) + 2 tile maps |
| `/breakpoints` | GET | List of breakpoint addresses |
| `/step` | POST | Execute one instruction, return CPU state |
| `/step-dot` | POST | Execute one PPU dot, return pipeline state |
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
gb_start core/tests/game_boy/roms/dmg-acid2.gb
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
