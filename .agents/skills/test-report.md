# Test Report

Generate a comprehensive test failure report: run the test suite, cross-reference against gbtrace manifests for other emulators' pass/fail status, and categorise failures by root cause cluster and difficulty.

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

The got/expected values are critical for clustering — tests that fail with the same offset (e.g. all off by +1) likely share a root cause.

### 2. Fetch gbtrace manifests

Download all suite manifests:

```bash
for suite in age blargg bully dmg-acid2 gambatte-tests gbmicrotest little-things mbc3-tester mealybug-tearoom mooneye mooneye-wilbertpol samesuite scribbltests strikethrough turtle-tests; do
  curl -s "https://ajoneil.github.io/gbtrace/tests/${suite}/manifest.json" \
    -o "/tmp/gbtrace_manifest_${suite}.json"
done
```

Tracked emulators: `gambatte`, `gateboy`, `mgba`, `missingno`, `sameboy`.

### 3. Cross-reference each failing test

For every test that missingno fails, look up its status in the manifest for the matching suite. Record which other emulators pass and which fail.

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

**Suite name mapping** (Rust test module → manifest suite):
- `age::*` → `age`
- `blargg::*` → `blargg`
- `bully::*` → `bully`
- `dmg_acid2::*` → `dmg-acid2`
- `gambatte::*` → `gambatte-tests`
- `gbmicrotest::*` → `gbmicrotest`
- `little_things::*` → `little-things`
- `mbc3_tester::*` → `mbc3-tester`
- `mealybug_tearoom::*` → `mealybug-tearoom`
- `mooneye::*` → `mooneye`
- `mooneye_wilbertpol::*` → `mooneye-wilbertpol`
- `samesuite::*` → `samesuite`
- `scribbltests::*` → `scribbltests`
- `strikethrough::*` → `strikethrough`
- `turtle_tests::*` → `turtle-tests`

**Test name matching strategy:**

The manifest uses ROM filename stems. The Rust test name strips the module prefix. To match:

1. Strip the Rust module prefix (e.g. `gbmicrotest::ppu_sprite0_scx3_b` → `ppu_sprite0_scx3_b`).
2. Try exact match in the manifest.
3. If no match, try replacing underscores with hyphens and vice versa.
4. For mooneye/mooneye-wilbertpol, manifest names use `__` for path separators and may have `-GS`/`-dmgABCmgb` suffixes (e.g. `ppu__hblank_ly_scx_timing-GS`). Try matching with the `_gs` suffix stripped, or with `gpu_` mapped to `ppu__`.
5. For gambatte-tests, manifest names may use path separators (e.g. `dmgpalette_during_m3/scx3/1`). Try matching with `/` replaced by `_`.
6. If still no match after fuzzy attempts, mark as "no manifest match" in the report (but still try to categorise based on the suite's other tests).

### 4. Categorise by reference emulator status

Assign each failing test to one of these categories:

#### Category A: GateBoy passes
GateBoy (gate-level simulation) passes this test. This means the correct behaviour is capturable from the logic gate model. **Primary reference**: GateBoy traces and source code. These are the most approachable fixes — we can study exactly what the hardware does by comparing traces.

#### Category B: SameBoy/Gambatte pass, GateBoy fails
High-accuracy behavioural emulators pass but the gate-level simulation does not. This points to behaviours that require modelling propagation delays, signal races, or sub-dot timing that GateBoy's simulation model doesn't capture. **Primary reference**: PPU propagation delay analysis (`../gmb-ppu-analysis/`), SameBoy/Gambatte behaviour. These are harder fixes — the hardware behaviour must be inferred from race pair analysis and other emulators' implementations.

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

| Test | GateBoy | SameBoy | Gambatte | mGBA | Detail |
|------|---------|---------|----------|------|--------|
| test_name | pass | pass | pass | pass | got X, exp Y |

**Likely root cause**: <brief hypothesis based on test names and patterns>
**Difficulty**: <easy / medium / hard> — <why>

### Cluster: ...

## Category B: SameBoy/Gambatte pass, GateBoy fails (harder)

### Cluster: ...

## Category C: All reference emulators fail (low priority)

| Test | GateBoy | SameBoy | Gambatte | mGBA | Detail |
|------|---------|---------|----------|------|--------|
| ... | fail | fail | fail | fail | |

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
