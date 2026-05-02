# Test Report

Generate a comprehensive test failure report: run the test suite, cross-reference against gbtrace manifests for other emulators' pass/fail status, and categorise failures by root cause cluster and difficulty.

## Invocation

This skill runs as a **Task subagent** (`subagent_type: "general-purpose"`). The test suite output, raw failure list, and manifest cross-referencing produce a large amount of intermediate data that must stay out of the caller's context window. The subagent receives this skill file in the Task prompt, runs the workflow below, writes the report to `receipts/test-reports/`, and returns only the receipt path plus a short summary (pass/fail/ignored counts and a one-line per category).

The caller is responsible for reading the receipt and deciding what to do next — the subagent does not interpret results or recommend fixes.

## Scope discipline

**You are a fact-finder, not a problem-solver.** Run the suite, classify failures against manifest data, cluster by shared symptoms, and write the report. Do not propose fixes, design changes, or open a separate investigation — interpretation and triage belong to the caller.

If the test suite produces unexpected output (build failure, panic before any test runs, missing manifest), capture what happened in the report and stop. Do not pivot to debugging.

## Workflow

### 1. Capture baseline

Run `./scripts/test-report.sh --save-baseline` to save the current state, then `./scripts/test-report.sh --diff` to generate the report. Record pass/fail/ignored counts.

Also capture the raw test output with actual/expected values:

```bash
cargo test -p missingno-gb 2>&1 | tee /tmp/test_output.txt
```

Extract failing test details (name + got/expected for gbmicrotest register tests):

```bash
grep -E "FAILED|assertion.*failed" /tmp/test_output.txt
```

**Test output line format**: Test lines are `test module::test_name ... FAILED` (no `accuracy::` prefix). Parse with:
```bash
grep "^test .* FAILED$" /tmp/test_output.txt | sed 's/^test //' | sed 's/ \.\.\. FAILED$//'
```

The got/expected values are critical for clustering — tests that fail with the same offset (e.g. all off by +1) likely share a root cause.

### 2. Fetch gbtrace manifests

Download all suite manifests:

```bash
for suite in age blargg bully dmg-acid2 gambatte-tests gbmicrotest little-things mbc3-tester mealybug-tearoom mooneye mooneye-wilbertpol samesuite scribbltests strikethrough turtle-tests; do
  curl -s "https://ajoneil.github.io/gbtrace/tests/${suite}/manifest.json" \
    -o "/tmp/gbtrace_manifest_${suite}.json"
done
```

Tracked emulators: `gambatte`, `gateboy`, `docboy`, `missingno`, `sameboy`. (DocBoy provides T-cycle granularity traces.)

### 3. Cross-reference each failing test

For every test that missingno fails, look up its status in the manifest for the matching suite. Record which other emulators pass and which fail.

**Manifest JSON format**: Each manifest is an array of test objects:
```json
[{"name": "test_name", "rom": "test_name.gb", "emulators": {"gambatte": "pass", "gateboy": "fail", ...}}]
```
The `emulators` field maps emulator names directly to status strings (`"pass"` or `"fail"`), NOT to objects. Access as `test['emulators'].get('gateboy', 'N/A')`.

**Use python3 for cross-referencing.** Load all manifests into a dict keyed by suite, then for each failing test: extract the module name, map to the manifest suite, and look up the test name. Example:

```python
import json, os

# Load all manifests
manifests = {}
for suite in ['age', 'blargg', 'bully', 'dmg-acid2', 'gambatte-tests', 'gbmicrotest',
              'little-things', 'mbc3-tester', 'mealybug-tearoom', 'mooneye',
              'mooneye-wilbertpol', 'samesuite', 'scribbltests', 'strikethrough', 'turtle-tests']:
    path = f'/tmp/gbtrace_manifest_{suite}.json'
    if os.path.exists(path):
        with open(path) as f:
            manifests[suite] = {t['name']: t.get('emulators', {}) for t in json.load(f)}

# Map Rust module to manifest suite
MODULE_TO_SUITE = {
    'age': 'age', 'blargg': 'blargg', 'bully': 'bully', 'dmg_acid2': 'dmg-acid2',
    'gambatte': 'gambatte-tests', 'gbmicrotest': 'gbmicrotest', 'little_things': 'little-things',
    'mbc3_tester': 'mbc3-tester', 'mealybug_tearoom': 'mealybug-tearoom', 'mooneye': 'mooneye',
    'mooneye_wilbertpol': 'mooneye-wilbertpol', 'samesuite': 'samesuite',
    'scribbltests': 'scribbltests', 'strikethrough': 'strikethrough', 'turtle_tests': 'turtle-tests',
}
```

**Test name matching strategy:**

The manifest uses ROM filename stems with path separators encoded as `__`. The Rust test name uses `_` for everything. Matching requires suite-specific transformations.

**General approach**: Strip the Rust module prefix, then try suite-specific patterns:

1. **Exact match** in the manifest.
2. **Underscore/hyphen swap**: Replace `_` with `-` and try again.

**Suite-specific patterns** (these are the actual manifest naming conventions):

- **age**: Manifest names include hardware variant suffixes: `halt-m0-interrupt-dmgC-cgbBCE`, `stat-mode-dmgC-cgbBC`, `m3-bg-scx-nocgb`. Match by converting test name underscores to hyphens and checking if any manifest key STARTS WITH that prefix (the suffix varies).

- **gambatte-tests**: Manifest names use `__` for directory separators: `dmgpalette_during_m3__dmgpalette_during_m3_2`, `halt__ime_noie_nolcdirq_readstat_dmg08_cgb_blank`, `sprites__sprite_late_disable_spx1A_1_dmg08_out0`. The test name is usually the LAST `__`-separated segment (without `_dmg08_outN` suffix). For `scx3` variants: `dmgpalette_during_m3__scx3__dmgpalette_during_m3_1`. Match by checking if the manifest key's last `__` segment contains the test name (case-insensitive for hex characters like `spx1A`).

- **mooneye-wilbertpol**: Manifest names have `acceptance__gpu__` prefix and `-GS`/`-dmgABCmgb` suffixes: `acceptance__gpu__ly_lyc-GS`, `acceptance__gpu__intr_2_mode0_scx3_timing_nops`. Strip `gpu_` from the Rust test name, replace `_gs` suffix with `-GS`, and prepend `acceptance__gpu__`.

- **mooneye**: Similar to wilbertpol but with `acceptance__ppu__` or `acceptance__serial__` prefixes: `acceptance__ppu__lcdon_timing-GS`, `serial__boot_sclk_align-dmgABCmgb`. Strip `ppu_`/`serial_` from Rust name, replace `_gs`→`-GS`, `_dmg`→`-dmgABCmgb`.

- **blargg**: Not tracked in manifests. Mark as Category D.

- **gbmicrotest, mealybug-tearoom, scribbltests, strikethrough, bully, little-things, mbc3-tester**: Usually exact match or simple underscore-to-hyphen conversion.

3. **Fallback normalization**: Lowercase both sides, strip all `_`, `-`, `/`, and compare. This catches remaining edge cases.
4. If still no match, mark as "no manifest match" (Category D).

### 4. Categorise by reference emulator status

Assign each failing test to one of these categories:

#### Category A: GateBoy passes
GateBoy (gate-level simulation) passes this test. This means the correct behaviour is capturable from the logic gate model. **Primary reference**: GateBoy traces and source code. These are the most approachable fixes — we can study exactly what the hardware does by comparing traces.

#### Category B: SameBoy/Gambatte/DocBoy pass, GateBoy fails
High-accuracy behavioural emulators pass but the gate-level simulation does not. This points to behaviours that require modelling propagation delays, signal races, or sub-dot timing that GateBoy's simulation model doesn't capture. **Primary reference**: propagation delay analysis (`receipts/resources/gb-propagation-delay-analysis/`), hardware timing data (`receipts/resources/gb-timing-data/`), DocBoy/SameBoy/Gambatte traces. DocBoy is particularly valuable here as it provides T-cycle granularity traces. These are harder fixes — the hardware behaviour must be inferred from race pair analysis, hardware measurements, and cross-emulator trace comparison.

#### Category C: All reference emulators fail
No tracked emulator passes this test. These may be tests with incorrect expected values, very obscure hardware edge cases, or bugs in the test ROMs themselves. **Low priority** — not worth investigating until categories A and B are resolved.

#### Category D: No manifest data
Test is not tracked in any gbtrace manifest AND could not be fuzzy-matched. Cannot compare against other emulators. Note what subsystem the test covers based on the test name/module.

### 5. Cluster by root cause

Within each category, group tests that likely share the same root cause. Indicators of shared cause:

- **Same test family** (e.g. `sprite4_0_a` through `sprite4_7_a` — same mechanism, different parameters)
- **Same subsystem** (e.g. all OAM bug tests, all timer tests, all window tests)
- **Same failure pattern** (e.g. all fail with result off by exactly 1, all show same pixel mismatch pattern)
- **Same pair structure** (`_a`/`_b` pairs that bracket a timing boundary — if `_a` fails but `_b` passes, the timing is 1 dot off in one direction)
- **Same got/expected delta** (e.g. all gbmicrotest failures show got=expected+3, suggesting a 3-dot offset)

Name each cluster descriptively (e.g. "OAM scan timing", "sprite fetch duration", "window reactivation").

**Each test appears in exactly one cluster.** If a test could fit multiple clusters, assign it to the one with the most specific shared cause. Do not duplicate tests across clusters.

### 6. Write the report

Write to `receipts/test-reports/` with a timestamped name: `YYYY-MM-DD-HHMM-analysis.md`.

Get the current timestamp: `date +%Y-%m-%d-%H%M`.

#### Report format

```markdown
# Test Failure Analysis — YYYY-MM-DD

## Summary
- **Passed**: N / **Failed**: N / **Ignored**: N
- **Category A** (GateBoy passes): N tests
- **Category B** (SameBoy/Gambatte pass, GateBoy fails): N tests
- **Category C** (all reference emulators fail): N tests
- **Category D** (no manifest data): N tests

## Category A: GateBoy passes (approachable)

### Cluster: <descriptive name>
<1-2 sentence description of the shared mechanism>

| Test | GateBoy | SameBoy | Gambatte | DocBoy | Detail |
|------|---------|---------|----------|--------|--------|
| test_name | pass | pass | pass | pass | got X, exp Y |

**Likely root cause**: <brief hypothesis based on test names and patterns>
**Difficulty**: <easy / medium / hard> — <why>

### Cluster: ...

## Category B: SameBoy/Gambatte pass, GateBoy fails (harder)

### Cluster: ...

## Category C: All reference emulators fail (low priority)

| Test | GateBoy | SameBoy | Gambatte | DocBoy | Detail |
|------|---------|---------|----------|--------|--------|
| ... | fail | fail | fail | fail  | |

## Category D: No manifest data

| Test | Module | Subsystem guess |
|------|--------|----------------|
| ... | age | PPU timing |

## Difficulty ranking

Rank the Category A and B clusters from easiest to hardest to fix, considering:
1. Number of tests in the cluster (more tests = higher impact fix)
2. Whether GateBoy passes (gate-level reference available)
3. Whether the failure pattern suggests a simple timing offset vs a structural model change
4. Whether related tests already pass (suggesting the model is close)
```

## Notes

- Run `cargo test -p missingno-gb` for the test suite, not `cargo test` (which includes GUI tests).
- The test report script handles formatting and diffing against baselines.
- Manifest data may be stale relative to the current test suite — some tests may exist in the suite but not in the manifest, or vice versa.
- When a test name doesn't match any manifest entry, always try the fuzzy matching strategies in step 3 before giving up.
- The Detail column should include got/expected values for register-based tests (gbmicrotest, gambatte) and pixel mismatch counts for screenshot tests (mealybug_tearoom, age, scribbltests).
- **DocBoy provides T-cycle granularity traces** — when a test passes on DocBoy but fails on missingno, DocBoy traces are especially valuable for pinpointing the exact T-cycle where behavior diverges.
- **Hardware timing data** (`receipts/resources/gb-timing-data/`): When clustering failures by root cause, check whether relevant measurement campaigns exist. Campaign data can confirm hypothesized root causes (e.g., "sprite penalty offset" cluster matches the `stat-mode-sprite-penalty` campaign). **Note: data collection is in progress — check what's available.**
- **Slowpeek** (`receipts/resources/slowpeek/`): When a cluster of failures points to a timing behavior that no existing data source covers, note that a Slowpeek sweep test could provide definitive hardware measurements for that cluster. **Note: hardware serial path not yet complete.**
