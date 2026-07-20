# Inspect

Query the headless debugger to inspect emulator state without modifying code.

The server is the `missingno-debugger` crate, generic over every core (GB, GBC, VCS; NES/SMS behind features). Two surfaces, one API underneath (the `Session`):

- **HTTP + the `dbg_*` helpers (`scripts/debugger.sh`)** — the SCRIPTED surface: measurement loops, data collection into receipt files, watch-driven navigation. This skill's workflow runs on it.
- **MCP (`--mcp`)** — the INTERACTIVE surface for agent sessions: the same capabilities as tools (`describe_machine`, `step_tick`, `get_frame` PNG, `get_tiles`, `get_waveforms` sparklines). Registered in the repo's `.mcp.json` as an idle server — `load_rom {path}` starts a session, `eject` ends it. Prefer MCP tools for one-off exploratory questions when they are available in your session; use the helpers for anything scripted or bulk (a per-call round trip through the model is the wrong shape for step-1000-ticks loops).

There are no family-specific routes: every endpoint works on every core, and per-core detail (PPU/TIA/APU state) arrives through `/sections` — the same semantic schema the GUI sidebar renders.

## When to use this instead of `/instrument`

Use this skill when the question can be answered by inspecting state at instruction, tick (dot / colour clock), or frame boundaries:

- **What are the CPU registers at a given point?** Step to a breakpoint, read `dbg_registers` or the CPU section.
- **What is the PPU/TIA state at a given PC or beam position?** Navigate with a watch, read `dbg_section PPU` (or `TIA`).
- **What does the screen look like after N frames?** `dbg_step_frame N`, then `dbg_screen`.
- **When does a mid-scanline register write occur?** Set a `bus-write` watch, `dbg_run_until_watch`, then read the section state at the stop.
- **What is in VRAM tiles / tile maps / OAM?** `dbg_graphics` — decoded atlases (with region annotations), maps with per-entry attributes, the object table.
- **What is each audio channel's DAC output doing?** `dbg_waveforms` — per-channel capture windows.
- **What instructions execute from a given address?** `dbg_disasm`.
- **How does state change tick by tick?** Navigate first, then `dbg_step_ticks N` for a small window.

## When this is NOT enough — stop and tell the user

The generic API observes at register/section granularity, per tick. It **cannot** observe:

- **Sub-tick timing** (ordering within a single dot/colour clock)
- **Memory bus conflicts** (DMA bus contention, OAM/VRAM locking during specific modes)
- **Pixel-pipeline internals** (GB fetcher stages, shifter contents mid-line, pixel_counter reads, sprite-fetch phases — the FIFO pixel strips in the PPU section show composed pixels while paused mid-Drawing, but the machinery around them has no read surface)
- **Deep APU internals** (prescaler/divider counters, trigger-synchroniser DFF chains, wave-latch pipelines — `/waveforms` shows each channel's DAC output, not the machinery driving it)

If the question requires any of the above, **stop immediately and report** — do not substitute a coarser measurement and hope it's "close enough". The report must be specific:

- **What you were asked**: restate the question.
- **What you can't observe**: name the specific limitation from the list above.
- **What would be needed**: a new seam surface (the deferred Pipeline surface design in `receipts/multisystem-core/DESIGN-generic-ui.md` covers the pixel-pipeline case) or `/instrument` with code instrumentation.

The user will decide whether to extend the debugger or fall back to `/instrument`. Do not make that decision yourself.

## Observation plan (mandatory first step)

**Before making any API calls**, write the following plan as a preamble in the measurement receipt file:

```markdown
## Observation plan

### Question
<What specific question is this measurement answering? One sentence.>

### Strategy
<What navigation steps will answer it? Be specific:>
- Start server with: dbg_start <rom_path>
- Navigate to: dbg_watch <terms> + dbg_run_until_watch (or dbg_step_frame N)
- Read state with: dbg_section PPU, dbg_registers, dbg_graphics, etc.
- If stepping ticks: how many, and what data to capture at each?

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

**Write this plan to the receipt file first, then execute it.** If the plan reveals a confound, adjust the strategy before proceeding. If no clean observation is possible, report that in the receipt — do not collect data that can't answer the question.

## Server management

**Always use the helper library.** Source it at the start of every inspect session:

```bash
. scripts/debugger.sh
```

### Starting and stopping

```bash
dbg_start crates/missingno-gb/tests/accuracy/roms/dmg-acid2/dmg-acid2.gb
# Prints: ready (pid 12345)
dbg_ensure          # Returns 0 if server is running, 1 if not
dbg_stop            # Kills the server cleanly
```

`dbg_start` works for ANY core's ROM (the factory recognises it), launches `missingno-debugger` on port 3333, polls `/status` for readiness, and prints "ready" or fails. **Never manage the server process manually** — no `cargo run &`, no `pkill`, no `lsof`.

**Boot ROM.** If the investigation needs boot state, ask the user for a DMG boot ROM path (proprietary — never in the repo) and pass it as `dbg_start`'s second argument:
```bash
dbg_start "$rom_path" "$boot_rom_path"
```
The machine then starts at PC 0x0000 in the boot ROM rather than at the post-boot state.

### ROM paths

Test ROMs live under `crates/missingno-gb/tests/accuracy/roms/` and `crates/missingno-vcs/tests/accuracy/roms/`. Always verify the path exists before starting the server.

**CGB ROMs** auto-detect from the header and serve on the CGB core; its sections/graphics carry the CGB extras (VRAM bank 1 atlas, CRAM palettes, KEY1/VBK/HDMA rows).

## Helper functions reference

**Always use these instead of raw curl.** They jq-parse tested response shapes — no inline Python. When collecting data in a loop, use the helpers (`dbg_step_ticks`, `dbg_section`, …); if a helper's output misses a field you need, supplement with one raw curl per step, but keep the stepping and primary collection on helpers.

### Navigation and stepping

**`dbg_step [n]`** — step N instructions (default 1). POST `/step`.

**`dbg_step_frame [n]`** — run to frame completion N times (stops early at a breakpoint/watch — check `stop.reason`). POST `/step-frame`.

**`dbg_step_ticks [n]`** — step N sub-instruction ticks (GB: dots; VCS: colour clocks), printing pc + the video status line per tick. POST `/step-tick`. 404s on a core with no tick.

**`dbg_break <hex>`** / **`dbg_breaks`** — set / list breakpoints. PUT `/breakpoints/{hex}`.

**`dbg_watchables`** — list this core's watchable keys with their param kinds. GET `/watchables`.

**`dbg_watch <terms-json>`** — add a watch: a single term `{"key":"bus-write","address":"ff40"}` or a conjunction `{"terms":[{"key":"scanline","value":58},{"key":"ppu-mode","value":3}]}`. PUT `/watches`.

**`dbg_watches`** — list; **`dbg_run_until_watch`** — step-frame until `stop.reason == "watch"`, printing the hit terms.

### State reading

**`dbg_status`** — pc, frame, title, tick name, last stop. GET `/status`.

**`dbg_sections`** — every section (name, summary, active). **`dbg_section <name>`** — one section's full JSON (blocks with typed `kind`s and raw values). GET `/sections`. This is where per-chip state lives: `CPU`, `PPU`/`TIA`/`VDP`, `APU`/`Audio`/`PSG`, `RIOT`, `CRAM`, `Mapper`.

**`dbg_registers`** — register groups with raw + rendered values. GET `/registers`.

**`dbg_memory <hex> [len]`** — byte range (len decimal, ≤4096). GET `/memory/{hex}/{len}`.

**`dbg_disasm`** — disassembly from pc. GET `/disassembly?at=&count=`.

**`dbg_graphics`** — decoded tile atlases (with named regions), tile maps (per-entry attributes + viewports), object table. GET `/graphics`; auto-enables capture (fills from the next frame — an immediate read after enabling can be empty; step a frame first).

**`dbg_waveforms`** — per-channel DAC capture windows (label, rate, depth_bits, levels). GET `/waveforms`; same auto-enable semantics.

**`dbg_screen <path>`** — save the resolved frame as raw RGBA (prints WxH). GET `/frame/bitmap`. For a viewable image use the MCP `get_frame` tool (PNG) or convert the RGBA yourself.

### Deprecated aliases

`gb_start`/`gb_stop`/`gb_run_frames` forward to `dbg_start`/`dbg_stop`/`dbg_step_frame` with a stderr deprecation note. All other `gb_*` helpers are gone — their GB-specific routes no longer exist.

## API reference

### Endpoints

| Endpoint | Method | Returns |
|----------|--------|---------|
| `/status` | GET | `pc` (hex string), `frame`, `title`, `tick` (name or null), `stop` |
| `/sections` | GET | `{sections:[{name, summary, active, detail, blocks:[...]}]}` — every block tagged `kind`: `registers·pairs·pointers·table·relations·rows·sweeps·swatches·pixels·rule` |
| `/registers` | GET | `{groups:[{name, registers:[{name, bits, raw, value}]}]}` (flag registers render `value` as a flag object) |
| `/memory/{hex}/{len}` | GET | `{address, length, bytes:[int], hex:["00"]}` — len decimal, ≤4096 |
| `/disassembly?at=&count=` | GET | `{at, lines:[{address, bytes, kind, length, text}]}` |
| `/graphics` | GET | `{graphics:{atlases:[{label, tile_width, tile_height, depth_bits, palettes, regions, tiles}], maps, objects}}` or `{graphics:null}`; auto-enables capture |
| `/graphics/capture` | POST | `{on}` — explicit capture gate |
| `/waveforms` | GET | `{waveforms:[{label, rate, depth_bits, active, levels}]}` or null; auto-enables |
| `/waveforms/capture` | POST | `{on}` |
| `/frame/bitmap` | GET | raw RGBA of the resolved frame |
| `/step` | POST | `{pc, frame, stop}` after one instruction |
| `/step-over` | POST | likewise, stepping over |
| `/step-frame` | POST | `{pc, frame, stop}` — `stop.reason`: `completed·breakpoint·watch` (+ `stop.watch.terms` on a watch hit) |
| `/step-tick?count=` | POST | `{pc, ran, tick, video:{label, summary}}`; **404 when the core has no sub-instruction tick** |
| `/reset` | POST | reset the console |
| `/breakpoints` (+`/{hex}`) | GET, PUT/DELETE | list / set / clear |
| `/watchables` | GET | this core's watch keys + param shapes |
| `/watches` | GET, PUT, DELETE | list / add (term or `{terms:[…]}`) / clear |
| `/symbols` | GET | loaded symbol table |

### Watches

All terms of a conjunction must match simultaneously:
```bash
dbg_watch '{"terms":[{"key":"scanline","value":58},{"key":"ppu-mode","value":3}]}'
dbg_run_until_watch
```

GB watchable keys include `bus-read`/`bus-write`/`dma-read`/`dma-write` (address param), `scanline`, `pixel-counter` (matches during Mode 3 only), `ppu-mode`, and value watches on the PPU registers and CPU registers (`ppu-lcdc`, `ppu-stat`, `cpu-a`, …). Run `dbg_watchables` for the live list — it is per-core.

**Note on LY timing** (GB): LY increments a few dots before OAM scan begins, so a scanline-only watch stops in the previous line's hblank. To stop at rendering, use a compound watch: `scanline=N` AND `ppu-mode=2` (OAM scan) or `=3` (drawing).

## Understanding pixel values

Sections and graphics carry the hardware's own values: GB shade/palette indices run **0 = lightest to 3 = darkest**; the test harness maps them to greyscale as `0→0xFF … 3→0x00`. `/frame/bitmap` is post-palette resolved RGBA (the frontend's palette choice applied on GB; hardware palettes on CGB/VCS).

## Scope discipline

**You are an observation tool, not a problem-solver.** Follow the same reporting contract as `/instrument`: measurements, not interpretation. If you catch yourself writing "this means..." or "the fix should be..." — stop, delete it, and return to reporting observations.

**Never read source code.** This API reference is complete — endpoints, field names, helpers. Do not read `.rs` files or grep the codebase; if an endpoint isn't in the reference, it doesn't exist.

## Debugging strategy: use watches, not step loops

**Prefer targeted watches over stepping.** Jump directly to the state you need rather than stepping through hundreds of ticks.

### Anti-pattern: step loops and guess-stepping
```bash
# BAD — stepping in a loop looking for a condition
for i in $(seq 1 200); do dbg_step_ticks 1 | grep -q "ly 60" && break; done

# BAD — guess-stepping a estimated tick count toward a beam position
dbg_step_ticks 60   # "should be near the sprite..." — no, use a watch
```

### Correct pattern
```bash
. scripts/debugger.sh
dbg_start crates/missingno-gb/tests/accuracy/roms/dmg-acid2/dmg-acid2.gb
dbg_step_frame 10
dbg_watch '{"terms":[{"key":"scanline","value":60},{"key":"ppu-mode","value":3}]}'
dbg_run_until_watch          # lands at scanline 60, Mode 3
dbg_section PPU              # read the state there
dbg_step_ticks 5             # fine observation from that point
```

To land on a specific `pixel-counter` value, add it to the conjunction — never estimate dot counts (sprite fetches, window stalls, and fine scroll break the mapping):
```bash
dbg_watch '{"terms":[{"key":"scanline","value":40},{"key":"ppu-mode","value":3},{"key":"pixel-counter","value":88}]}'
dbg_run_until_watch
```

### When to use bus watches
The most powerful "when does X happen" tool:
- **When is a register written?** `{"key":"bus-write","address":"ff4b"}` — catches the writing instruction; read the section state at the stop.
- **When is VRAM read?** `bus-read`. **When does DMA touch an address?** `dma-read`/`dma-write`.

### When to step ticks
`dbg_step_ticks` is for observing how visible state changes across a small window AFTER navigating to it with watches. Before stepping, the observation plan must state: exactly how many ticks, which fields to record at each, and what transition would confirm/refute the hypothesis.
