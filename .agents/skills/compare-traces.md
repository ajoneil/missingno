# Compare Traces

Compare and inspect execution traces between missingno and reference emulators (GateBoy, Gambatte, SameBoy, DocBoy) to find the exact divergence point in a test failure, or inspect individual traces to understand emulator behavior.

## When to use

Use this skill when investigating a test failure where:
- A reference trace exists (check suite manifests at `https://ajoneil.github.io/gbtrace/tests/{suite}/manifest.json` or locally in `receipts/resources/gbtrace/test-suites/`)
- You need to find **where** execution diverges, not just **that** it fails
- The failure involves timing, register values, or execution path differences
- You need to understand what the emulator did during a test (individual trace inspection)

**Choose the right approach for the test:**
- **Small, focused tests** (gbmicrotest, small mooneye tests): Direct `gbtrace diff` between missingno and a reference trace is usually productive. The divergence point is close to the root cause.
- **Larger tests** (blargg, full mooneye suites, mealybug-tearoom): Direct diff becomes less useful — the first divergence may be far from the root cause, or initial state differences create noise. Use individual trace inspection (`gbtrace query`, `gbtrace render`, `gbtrace frames`) to understand what each emulator does, then compare specific regions of interest.
- **Visual tests** (mealybug-tearoom, dmg-acid2, scribbltests): Use `gbtrace render` to produce frame images and compare visually. Then use `gbtrace query` to examine the trace around the scanline/dot where the visual difference occurs.

Prefer this over `/inspect` (debugger) for initial diagnosis — traces show the full execution history and let you find the divergence without guessing where to set breakpoints. Use `/inspect` for follow-up once you know the area of interest.

## Prerequisites

1. **gbtrace CLI** built: `cd receipts/resources/gbtrace && cargo build -p gbtrace --features cli`
2. **gbtrace feature** on missingno: `cargo test -p missingno-gb --features gbtrace`
3. **GBTRACE_PROFILE** env var set to the suite name (e.g. `gbmicrotest`, `blargg`, `mooneye`). Profiles are per-suite TOML files in `receipts/resources/gbtrace/test-suites/*/profile.toml`.

## Generating traces

### Missingno trace
```bash
GBTRACE_PROFILE=gbmicrotest cargo test -p missingno-gb --features gbtrace -- <test_name>
# Writes to: receipts/traces/<rom_name>.gbtrace
```

The test runner captures state at every T-cycle (for tcycle profiles) or every instruction (for instruction profiles). Traces are written even when tests fail.

### GateBoy trace (fresh, with fixed adapter)
```bash
receipts/resources/gbtrace/adapters/gateboy/gbtrace-gateboy \
  --rom receipts/resources/gbtrace/test-suites/gbmicrotest/<test>.gb \
  --profile receipts/resources/gbtrace/test-suites/gbmicrotest/profile.toml \
  --output <output_path> \
  --stop-when FF82=01 --stop-when FF82=FF \
  --frames 5
```

The GateBoy adapter uses `bit_pack()` for IO registers and reconstructs STAT from gate-level state (mode from XYMU/ACYL/LY, LYC from RUPO, enables from reg_stat DFFs).

### Reference traces from manifests
Reference traces for ~700 tests across 6 suites are hosted at [ajoneil.github.io/gbtrace](https://ajoneil.github.io/gbtrace/). Each suite has a `manifest.json` listing tests with per-emulator pass/fail status.

**Fetch a manifest to find available traces:**
```bash
curl -s https://ajoneil.github.io/gbtrace/tests/gbmicrotest/manifest.json | jq '.[0]'
# → {"name": "div_inc_timing_a", "rom": "div_inc_timing_a.gb", "emulators": {"gambatte": "pass", "gateboy": "pass", ...}}
```

**Download a reference trace:**
```bash
curl -LO https://ajoneil.github.io/gbtrace/tests/gbmicrotest/div_inc_timing_a_gateboy_pass.gbtrace
```

URL pattern: `tests/{suite}/{test}_{emulator}_{status}.gbtrace`

Tracked emulators: gambatte, gateboy, docboy, missingno, sameboy. DocBoy provides T-cycle granularity traces. Suites: gbmicrotest, blargg, mooneye, gambatte-tests, mealybug-tearoom, dmg-acid2, age, mooneye-wilbertpol, samesuite, scribbltests, bully, little-things, mbc3-tester, strikethrough, turtle-tests.

Reference traces are also available locally in `receipts/resources/gbtrace/test-suites/` if the gbtrace repo has them built.

## Diffing traces

### Basic diff
```bash
gbtrace diff <missingno.gbtrace> <reference.gbtrace>
```

### Alignment gotchas

**Initial state differs between emulators.** Post-boot register values (LY, STAT, DIV, IF, TAC) differ between skip-boot emulators. Fields that diverge from entry 0 are noise, not bugs.

**Use `--sync` to align at a meaningful event.** The `--sync` flag skips entries in both traces until a condition is met, aligning them at the same logical point:

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

### Interpreting results

**`Classification: execution-path-split`** means PC diverges — the emulators take different code paths. Look at where PC first differs.

**`Classification: register-drift`** means PC matches but register values differ — same code, different results. Look at which register diverges first.

**Persistent PC offset (e.g. missingno=0x0150 vs reference=0x0151):** This is a 4-dot (1 M-cycle) timing offset, usually from initial state divergence. Not a bug in the code under test — it's the starting position within the frame being different.

**STAT divergence throughout:** GateBoy's STAT reconstruction from gate state is approximate (mode bits derived from XYMU/ACYL/LY, enable bits from reg_stat DFFs). Small STAT differences between GateBoy and missingno may be adapter artifacts, not real bugs.

## Visual comparison with `render`

For PPU tests, render frames from both traces and compare visually:
```bash
gbtrace render <missingno.gbtrace> -o receipts/traces/renders/missingno/
gbtrace render <reference.gbtrace> -o receipts/traces/renders/reference/
# Render specific frames only:
gbtrace render <trace> --frames 2,3
```

This is especially useful for mealybug-tearoom and dmg-acid2 tests where the failure is a visual difference in rendered output.

## Frame analysis

Use `frames` to understand frame boundaries and identify which frame to focus on:
```bash
gbtrace frames <trace>
```

## Useful queries

### Check test results
```bash
# What did the test produce?
gbtrace query <trace> --where "test_pass=1" --max 1    # passing
gbtrace query <trace> --where "test_pass=0xFF" --max 1  # failing
```

### Find specific events
```bash
# When does LY reach 144 (VBlank)?
gbtrace query <trace> --where "ly=144" --max 1 --context 5

# When does the ISR fire?
gbtrace query <trace> --where "pc=0x48" --max 1 --context 10

# When does a register change?
gbtrace query <trace> --where "scx=1" --max 1 --context 3

# Multiple conditions — use separate --where arguments (NOT comma-separated):
gbtrace query <trace> --where "ly=9" --where "stat&3=3" --max 5

# Show the last 5 entries (no condition needed):
gbtrace query <trace> --last 5
```

### Compare test results across SCX values
```bash
for scx in 0 1 2 3 4 5 6 7; do
  trace="receipts/traces/int_hblank_nops_scx${scx}.gbtrace"
  result=$(gbtrace query "$trace" --where "test_pass=1" --max 1 2>&1 | grep -oP 'test_result=\K\S+')
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
gbtrace frames <trace>

# What does the trace contain?
gbtrace info <trace>
```

### Find specific events
```bash
# When does a specific register value appear?
gbtrace query <trace> --where "scx=3" --max 5 --context 3

# When does mode 3 start on a specific line?
gbtrace query <trace> --where "ly=66" --fields ly,stat,pix_count --max 20

# What happens at the end of the test?
gbtrace query <trace> --last 30

# What's happening around a specific index?
gbtrace query <trace> --range 50000..50100
```

### Visual inspection
```bash
# Render all frames
gbtrace render <trace> -o receipts/traces/renders/

# Render specific frames for comparison
gbtrace render <missingno.gbtrace> -o receipts/traces/renders/missingno/ --frames 2
gbtrace render <reference.gbtrace> -o receipts/traces/renders/reference/ --frames 2
```

### Compare specific regions (not full diff)
When the full diff is too noisy, narrow the comparison to a specific region:
1. Use `gbtrace query` on both traces to find the same logical event (e.g., start of scanline 66)
2. Extract the index ranges around that event
3. Compare those ranges manually or use `--fields` to focus the diff on relevant fields

### Data sources for context

When interpreting trace data, cross-reference with:
- **Hardware timing data** (`receipts/resources/gb-timing-data/`): If a campaign exists for the behavior you're investigating, the CSV data provides ground-truth cycle measurements. Check `receipts/resources/gb-timing-data/campaigns/` for relevant TOML definitions. **Note: data collection is in progress — check what's available before assuming a campaign has results.**
- **PPU race pairs** (`receipts/resources/gb-propagation-delay-analysis/output/race_pairs_report.md`): For 1-dot timing discrepancies, check whether the divergence corresponds to a known signal race.
- **Slowpeek** (`receipts/resources/slowpeek/`): For behaviors where no existing data covers the question, note that a Slowpeek sweep test could provide definitive hardware measurements. **Note: hardware serial path not yet complete — emulator-only for now.**

## Limitations — suggest improvements

If you cannot answer the investigation question with the current gbtrace tooling, **do not silently fall back to the debugger**. Instead, report:

1. What you tried (which sync/filter/query)
2. What information was missing or ambiguous
3. What gbtrace feature would have helped (e.g. "a `--sync` on field transitions rather than values", "negative context before sync point", "DIV internal counter in trace fields")

This feedback helps improve gbtrace for future investigations.
