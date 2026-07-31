# Compare Traces

Compare and inspect execution traces between missingno and reference emulators (SameBoy, DocBoy, Gambatte) to find the exact divergence point in a test failure, or inspect individual traces to understand emulator behavior.

## Adapter preference

The reference emulators are all **behavioural** — none is die-derived, so none is ground truth. They show *where* execution diverges; *why* comes from the hardware, never from matching another emulator. The preferred order is per-core (see the target core's `crates/missingno-<core>/AGENTS.md`): **Game Boy (DMG/CGB) — SameBoy → DocBoy → gambatte** (why-source: the DMG Timing Specification, gb-ctr, race pairs); **Atari 2600 (VCS) — Stella → Gopher2600 → MAME** (why-source: Sim2600 for CPU/TIA, the schematics + 6532 datasheet for RIOT). Fall back down the order when the preferred emulator has no passing trace for the test, and state the reason in the receipt (e.g. "no SameBoy pass trace for blargg/halt_bug; using DocBoy"). Don't silently treat any single emulator's behaviour as the hardware's.

## When to use

Use this skill when investigating a test failure where:
- The test belongs to a suite morepork covers (`receipts/resources/morepork/test-suites/` — reference traces are generated locally, see below)
- You need to find **where** execution diverges, not just **that** it fails
- The failure involves timing, register values, or execution path differences
- You need to understand what the emulator did during a test (individual trace inspection)

**Choose the right approach for the test:**
- **Small, focused tests** (gbmicrotest, small mooneye tests): Direct `morepork diff` between missingno and a reference trace is usually productive. The divergence point is close to the root cause.
- **Larger tests** (blargg, full mooneye suites, mealybug-tearoom): Direct diff becomes less useful — the first divergence may be far from the root cause, or initial state differences create noise. Use individual trace inspection (`morepork query`, `morepork render`, `morepork frames`) to understand what each emulator does, then compare specific regions of interest.
- **Visual tests** (mealybug-tearoom, dmg-acid2, scribbltests): Use `morepork render` to produce frame images and compare visually. Then use `morepork query` to examine the trace around the scanline/dot where the visual difference occurs.

Prefer this over `/inspect` (debugger) for initial diagnosis — traces show the full execution history and let you find the divergence without guessing where to set breakpoints. Use `/inspect` for follow-up once you know the area of interest.

## Prerequisites

1. **morepork CLI** built: `cd receipts/resources/morepork && make cli` (binary at `target/release/morepork`; equivalently `cargo build --release --features cli`)
2. **morepork feature** on missingno: `cargo test -p missingno-gb --features morepork`
3. **MOREPORK_PROFILE** env var set (any value enables capture). The column set comes from the core's hardware state schema, not a named profile; `MOREPORK_TRIGGER` (`tcycle`/`instruction`, default instruction) sets the cadence and `MOREPORK_SCOPE` (`observable`/`full`, default observable) the tier depth.

The format is **version 3**, re-founded on hardware-named system state schemas. Traces from older morepork versions (the v2 corpus) don't open in v3 tools — regenerate rather than reuse old files.

## Generating traces

### Missingno trace
```bash
MOREPORK_PROFILE=1 MOREPORK_TRIGGER=tcycle cargo test -p missingno-gb --features morepork -- <test_name>
# Writes to: receipts/traces/<rom_name>.morepork
```

The test runner captures at every T-cycle (`MOREPORK_TRIGGER=tcycle`) or every instruction (the default). `MOREPORK_SCOPE=full` adds the schema's Tier-2a deep state. Traces are written even when tests fail.

### Missingno CGB trace
The CGB core has its own `morepork` feature (`missingno-gbc`), captured through a dedicated `morepork_capture` test driven by an env var rather than a test-name filter. The test is `#[ignore]`d (it isn't pass/fail), so pass `--ignored`:
```bash
MOREPORK_PROFILE=1 MOREPORK_CAPTURE_ROM=<rom-relative-to-gbc-roms-dir> \
  cargo test -p missingno-gbc --features morepork -- morepork_capture --ignored
# Uses load_cgb_rom_traced; writes to receipts/traces/<rom_name>.morepork
```

### Reference traces — generate locally

**The published trace library is legacy.** The old CDN corpus was written in format v2 and is orphaned by the v3 re-founding — do not download traces from the web manifests or the CDN. Generate the reference traces you need locally in `receipts/resources/morepork/`:

```bash
cd receipts/resources/morepork
make adapters                              # build the adapter binaries once
make traces-gbmicrotest EMUS=sameboy       # one suite, one emulator
make traces-blargg EMUS=sameboy,docboy -j$(nproc)
```

There is a `traces-<suite>` target per suite (see the Makefile's `.PHONY` list); `EMUS=` restricts which emulators run (default `gambatte,sameboy,missingno,docboy`). Generate only what the investigation needs — a full `make traces` run is hours of work.

Reference emulators: **sameboy, docboy, gambatte** (GB/CGB), **stella, gopher2600, mame** (VCS) — all behavioural; follow the per-core adapter preference above. Generated traces land in `build/traces/<suite>/`, named `<test>_<emu>_<model>_<status>.morepork`.

## Diffing traces

### Basic diff
```bash
morepork diff <missingno.morepork> <reference.morepork>
morepork diff <a> <b> --summary        # one line per divergent field, with first-divergence values
```

### Alignment gotchas

**Initial state differs between emulators.** Post-boot register values (LY, STAT, DIV, IF, TAC) differ between skip-boot emulators. Fields that diverge from entry 0 are noise, not bugs.

**Sync defaults to `auto`** — skip to the family's program entry when both traces start there, else first-common-address. That usually suffices. Other modes: `--sync cartridge`, `--sync pc`, `--sync none`, or an explicit condition that skips entries in both traces until it's met (hex values):

```bash
# Sync at LCD-on (PPU enable)
--sync "lcdc&0x80"

# Sync at a specific PC (e.g. test entry point)
--sync "pc=0x0150"

# Sync at a specific register write (e.g. SCX set)
--sync "scx=1"
```

**Choose the right sync point.** The best sync point is the last setup action before the behavior under test. For PPU timing tests, `--sync "lcdc&0x80"` (LCD-on) works when the test turns LCD off then on. For tests that don't toggle LCD, sync on a register write that the test makes during setup (e.g. `--sync "scx=1"`, `--sync "ie=2"`).

**If sync doesn't help (field already has the sync value from boot):** Use a later sync point. If LCDC is 0x91 from boot, `--sync "lcdc&0x80"` syncs at entry 0 — useless. The test ROM likely turns LCD off then on; sync on a register written after the LCD toggle.

### Filtering fields

**Use `--exclude` to drop noisy initial-state fields:**
```bash
--exclude div,tac,if_
```

Common noise fields: `div` (phase-dependent), `tac` (init differs), `if_` (upper bits differ), `tima`, `tma`.

**Use `--fields` to focus on what matters:**
```bash
# Execution path only
--fields pc,a,f,sp

# PPU timing
--fields pc,ly,stat,lcdc

# Just the test result
--fields test_result,test_expect,test_pass
```

### Mixed granularities — downsample

When one trace is T-cycle and the other instruction/M-cycle cadence, downsample the finer one first:
```bash
morepork downsample <tcycle.morepork>            # → mcycle view; missingno traces keep the M-boundary phase
morepork downsample <trace> --keep "<condition>" # custom keep filter
```

### Interpreting results

The diff reports per-field divergence counts plus the first few divergent entries (`field:a|b` pairs). **PC diverging** means the emulators take different code paths — look at where PC first differs. **PC matching while other fields differ** means same code, different results — look at which register diverges first.

**Persistent PC offset (e.g. missingno=0x0150 vs reference=0x0151):** This is a 4-dot (1 M-cycle) timing offset, usually from initial state divergence. Not a bug in the code under test — it's the starting position within the frame being different.

**STAT divergence throughout:** Adapters reconstruct STAT differently and may sample its mode/enable bits at slightly different points. Small persistent STAT differences between a reference and missingno may be adapter sampling artifacts, not real bugs — verify against the DMG Timing Specification before treating one as a divergence.

## Visual comparison with `render`

For PPU tests, render frames from both traces and compare visually:
```bash
morepork render <missingno.morepork> -o receipts/traces/renders/missingno/
morepork render <reference.morepork> -o receipts/traces/renders/reference/
# Render specific frames only:
morepork render <trace> --frames 2,3
```

This is especially useful for mealybug-tearoom and dmg-acid2 tests where the failure is a visual difference in rendered output.

## Frame analysis

Use `frames` to understand frame boundaries and identify which frame to focus on:
```bash
morepork frames <trace>
```

## Useful queries

### Check test results
```bash
# What did the test produce?
morepork query <trace> --where "test_pass=1" --max 1    # passing
morepork query <trace> --where "test_pass=0xFF" --max 1  # failing
```

### Find specific events
```bash
# When does LY reach 144 (VBlank)?
morepork query <trace> --where "ly=144" --max 1 --context 5

# When does the ISR fire?
morepork query <trace> --where "pc=0x48" --max 1 --context 10

# When does a register change?
morepork query <trace> --where "scx=1" --max 1 --context 3

# Multiple conditions — use separate --where arguments (NOT comma-separated):
morepork query <trace> --where "ly=9" --where "stat&3=3" --max 5

# Show the last 5 entries (no condition needed):
morepork query <trace> --last 5
```

### Compare test results across SCX values
```bash
for scx in 0 1 2 3 4 5 6 7; do
  trace="receipts/traces/int_hblank_nops_scx${scx}.morepork"
  result=$(morepork query "$trace" --where "test_pass=1" --max 1 2>&1 | grep -oP 'test_result=\K\S+')
  echo "SCX=${scx}: ${result:-FAIL}"
done
```

## Reporting results

Write a measurement receipt to the investigation's `measurements/` folder with:

```markdown
# Measurement: <title>

## Test result
<pass/fail, what values differed>

## Trace comparison
<sync point used, fields compared, first divergence>

## Raw data
<key entries from both traces around the divergence point>

## Also observed
<unexpected findings>
```

## Individual trace inspection

When direct diff is impractical or insufficient, inspect traces individually to build understanding.

### Understand the test structure
```bash
# How many frames? Where are the frame boundaries?
morepork frames <trace>

# What does the trace contain?
morepork info <trace>
```

### Find specific events
```bash
# When does a specific register value appear?
morepork query <trace> --where "scx=3" --max 5 --context 3

# When does mode 3 start on a specific line?
morepork query <trace> --where "ly=66" --fields ly,stat,pix_count --max 20

# What happens at the end of the test?
morepork query <trace> --last 30

# What's happening around a specific index?
morepork query <trace> --range 50000..50100
```

### Visual inspection
```bash
# Render all frames
morepork render <trace> -o receipts/traces/renders/

# Render specific frames for comparison
morepork render <missingno.morepork> -o receipts/traces/renders/missingno/ --frames 2
morepork render <reference.morepork> -o receipts/traces/renders/reference/ --frames 2
```

### Compare specific regions (not full diff)
When the full diff is too noisy, narrow the comparison to a specific region:
1. Use `morepork query` on both traces to find the same logical event (e.g., start of scanline 66)
2. Extract the index ranges around that event
3. Compare those ranges manually or use `--fields` to focus the diff on relevant fields

### Data sources for context

When interpreting trace data, cross-reference the core's other data sources (hierarchy: `crates/missingno-<core>/AGENTS.md`) — notably hardware timing campaigns (`receipts/resources/gb-timing-data/campaigns/`; check what's available before assuming results exist) and, for 1-dot discrepancies, the race-pairs report (`receipts/resources/gb-propagation-delay-analysis/output/race_pairs_report.md`). Note when a Slowpeek sweep would provide a definitive measurement no existing source covers.

## Limitations — suggest improvements

If you cannot answer the investigation question with the current morepork tooling, **do not silently fall back to the debugger**. Instead, report:

1. What you tried (which sync/filter/query)
2. What information was missing or ambiguous
3. What morepork feature would have helped (e.g. "a `--sync` on field transitions rather than values", "negative context before sync point", "DIV internal counter in trace fields")

This feedback helps improve morepork for future investigations.
