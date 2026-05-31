# Test Report

Generate a comprehensive test-failure report: run the suite, partition failures, cluster them by root cause, and label each cluster with cross-emulator status from the gbtrace manifests. Built to scale to the thousands of failures the CGB core currently has — **cluster first, look up references per cluster, never per test**.

## Invocation

This skill runs as a **Task subagent** (`subagent_type: "general-purpose"`). The suite output, raw failure list, and manifest cross-referencing produce a large amount of intermediate data that must stay out of the caller's context window. The subagent receives this skill file in the Task prompt, runs the workflow below, writes the report to `receipts/test-reports/`, and returns only the receipt path plus a short summary (pass/fail/ignored counts, partition census, and a one-line per top cluster).

The caller reads the receipt and decides what to do next — the subagent does not interpret results or recommend fixes.

**Which core**: the brief must name the core. Use `missingno-gb` / `test-report-gb.sh` for DMG, `missingno-gbc` / `test-report-gbc.sh` for CGB. The two have very different scales (DMG: a handful of failures; CGB: thousands) and the CGB path leans on the in-house core-diff described below.

## Scope discipline

**You are a fact-finder, not a problem-solver.** Run the suite, partition and cluster failures, label clusters against manifest data, and write the report. Do not propose fixes, design changes, or open a separate investigation — interpretation and triage belong to the caller.

If the suite produces unexpected output (build failure, panic before any test runs, missing manifest), capture what happened in the report and stop. Do not pivot to debugging.

## Reference model (read before clustering)

There is **no gate-level reference**. The tracked gbtrace emulators are **SameBoy, DocBoy, gambatte, missingno** — all behavioural. "Emulator X passes" tells you the expected behaviour is *reproducible*, not *how the hardware does it*; it does not, on its own, sort failures by approachability. Use it only as a secondary per-cluster label. Prefer SameBoy when references disagree.

The approachability signal comes from two cheaper places:

1. **(CGB only) The in-house core-diff.** A CGB failure whose test name also exists in the DMG suite and **passes there** is shared behaviour broken in the CGB core — the DMG core and DMG PPU timing spec are the reference, and these are the most approachable CGB fixes. A failure with no DMG counterpart is genuinely CGB-specific. See *AGENTS.md → CGB Core — Methodology Deltas*.
2. **Cluster size and structure.** Families that fail together (`sprite4_0_a`..`sprite4_7_a`), `_a`/`_b` boundary pairs, identical got/expected deltas — these point at one shared cause and one fix.

## Workflow

### 1. Capture baseline + (CGB) the core-diff partition

Run `./scripts/test-report-gb.sh --save-baseline` then `--diff` (use the `-gbc` variant for CGB) to record pass/fail/ignored counts. The `-gbc` script **also emits the core-diff partition automatically** (counts of: passes-gb/fails-gbc "integration suspects", cgb-only, fails-both) — record those numbers; they are the spine of the CGB report.

Also capture raw output with actual/expected values:

```bash
cargo test -p missingno-gbc 2>&1 | tee /tmp/test_output.txt   # or -gb
```

Test lines are `test module::test_name ... FAILED` (no `accuracy::` prefix). Extract the failing set:

```bash
grep "^test .* FAILED$" /tmp/test_output.txt | sed 's/^test //; s/ \.\.\. FAILED$//' | sort > /tmp/failed.txt
```

For register tests (gbmicrotest, gambatte), also grep got/expected — same delta across tests is a strong cluster signal:

```bash
grep -E "FAILED|assertion.*failed" /tmp/test_output.txt
```

### 2. (CGB) Partition by the in-house core-diff

This is the first cut for CGB and replaces blind manifest matching as the organizing step. Join the gbc failing set against the **gb baseline** (`scripts/.test-baseline-gb`):

```python
def load(p):
    d = {}
    for line in open(p):
        line = line.strip()
        if '=' in line:
            k, v = line.rsplit('=', 1); d[k] = v
    return d
gb = load('scripts/.test-baseline-gb')
failed = [l.strip() for l in open('/tmp/failed.txt') if l.strip()]
P1 = [t for t in failed if gb.get(t) == 'ok']        # passes gb / fails gbc — integration suspects
P3 = [t for t in failed if gb.get(t) == 'FAILED']    # fails both cores — DMG bug surfacing
P2 = [t for t in failed if t not in gb]              # no DMG counterpart — CGB-specific
```

- **P1 (shared regression)** — most approachable; DMG core + DMG PPU spec are the reference.
- **P2 (CGB-specific)** — genuinely new behaviour; needs the CGB ground-truth hierarchy (test-ROM > docs > behavioural refs).
- **P3 (shared, fails both)** — underlying DMG bug; usually fix in the DMG core first.

For DMG reports, skip this step (there is no second core to diff against) and go straight to clustering.

### 3. Cluster by subsystem and root cause (before any per-test lookup)

Within each partition, group tests that likely share a cause. Cluster *first* — with thousands of failures you cannot, and should not, look up references for each test individually.

Cluster indicators:
- **Same subsystem** — for CGB: double-speed/KEY1, HDMA/GDMA, CRAM/palettes (BCPS/BCPD/OCPS/OCPD), VRAM-bank/tile-attributes, object-priority (OPRI), DMG-compat palettes, plus the shared subsystems (STAT/mode timing, window, sprites, OAM, timer, APU, MBC/RTC).
- **Same test family** (`sprite4_0_a`..`sprite4_7_a` — same mechanism, different parameter).
- **Same `_a`/`_b` boundary pair** (if `_a` fails and `_b` passes, a timing boundary is one dot off).
- **Same got/expected delta** (all off by +N → one N-dot offset).

Name each cluster descriptively. **Each test appears in exactly one cluster.** Separate clusters that are *features not yet implemented* (roadmap items, not modelling bugs) — label them clearly so the caller doesn't triage them as bugs.

**gambatte filename encoding** carries the target models and expected output: `dir__name_dmg08_cgb04c_out1`. The trailing `_dmgXX` / `_cgbXX` and `_outN` tell you which models the ROM targets and what each expects — use it to cluster (and to tell "expects a different CGB result we don't produce" from "CGB-mode regression of shared logic") without any manifest lookup.

### 4. Label each cluster from the gbtrace manifests

Only now, and **per cluster** (sample a few representative tests, not all), look up cross-emulator status.

Fetch manifests (17 suites):

```bash
for suite in age blargg bully cgb-acid2 cgb-acid-hell dmg-acid2 gambatte gbmicrotest \
             mbc3-tester mealybug-tearoom mooneye mooneye-wilbertpol rtc3test samesuite \
             scribbltests strikethrough turtle-tests; do
  curl -s "https://ajoneil.github.io/gbtrace/tests/${suite}/manifest.json" \
    -o "/tmp/gbtrace_manifest_${suite}.json"
done
```

**Manifest schema** (current): each entry carries per-*system* status —

```json
{"name": "...", "rom": "...", "systems": {"dmg": {"sameboy": "pass", "docboy": "pass", "gambatte": "fail", "missingno": "fail"},
                                          "cgb": {"sameboy": "pass", "docboy": "fail", "gambatte": "fail", "missingno": "fail"}}}
```

Select the system that matches the core under report: `cgb` for `missingno-gbc`, `dmg` for `missingno-gb`. A suite may carry only one system (e.g. cgb-acid2 → cgb; gbmicrotest → dmg). Access as `t['systems'].get('cgb', {}).get('sameboy', 'N/A')`. The tracked emulators are `sameboy`, `docboy`, `gambatte`, `missingno` — **there is no gateboy**.

Load into a dict keyed by suite, then map Rust module → suite:

```python
MODULE_TO_SUITE = {
  'age':'age','blargg':'blargg','bully':'bully','cgb_acid2':'cgb-acid2','cgb_acid_hell':'cgb-acid-hell',
  'dmg_acid2':'dmg-acid2','gambatte':'gambatte','gbmicrotest':'gbmicrotest','little_things':'little-things',
  'mbc3_tester':'mbc3-tester','mealybug_tearoom':'mealybug-tearoom','mooneye':'mooneye',
  'mooneye_wilbertpol':'mooneye-wilbertpol','rtc3test':'rtc3test','samesuite':'samesuite',
  'scribbltests':'scribbltests','strikethrough':'strikethrough','turtle_tests':'turtle-tests',
}
```

**Name matching** (manifest stems use `__` for path separators and hardware-variant suffixes; Rust names use `_`). Try in order: (1) exact, (2) `_`↔`-` swap, then suite-specific patterns:
- **age**: variant suffixes (`-dmgC-cgbBCE`, `-nocgb`, `-GS`); convert `_`→`-` and match manifest keys that START WITH the prefix.
- **gambatte**: `__` directory separators; the test is usually the LAST `__` segment, minus the `_dmgXX_cgbXX_outN` suffix. `scx3` variants insert a `__scx3__` segment. Hex like `spx1A` is case-insensitive.
- **mooneye / mooneye-wilbertpol**: `acceptance__ppu__` / `acceptance__gpu__` / `acceptance__serial__` prefixes, `-GS` / `-dmgABCmgb` suffixes. Strip the Rust subsystem prefix, map `_gs`→`-GS`, `_dmg`→`-dmgABCmgb`.
- **samesuite**: paths like `apu__channel_1__channel_1_freq_change_timing-cgb0BC`; many carry a `-cgbXX` suffix.
- **blargg**: not tracked — mark "no manifest".
- Fallback: lowercase, strip `_ - /`, compare. Still nothing → "no manifest".

Use the per-cluster labels to characterise the cluster, not to re-sort it:
- **refs agree + pass** → behaviour is well-defined and reproducible; the reference disagreement is ours alone. Study refs (SameBoy first) + docs.
- **refs split** → contentious; weight the hardware test-ROM expected value and gb-ctr / Pan Docs over any single emulator.
- **all refs fail** → likely a test-ROM edge case or a shared bug; low priority.

### 5. Write the report

Write to `receipts/test-reports/` (or the `gb`/`gbc` subdir) with a timestamped name: `YYYY-MM-DD-HHMM-analysis.md` (`date +%Y-%m-%d-%H%M`).

#### Report format

```markdown
# Test Failure Analysis (<core>) — YYYY-MM-DD

## Summary
- **Passed**: N / **Failed**: N / **Ignored**: N
- (CGB) **Partition**: P1 shared-regression N · P2 CGB-specific N · P3 fails-both N

## P1 — Shared regression (passes DMG core, fails CGB core)   [CGB only; most approachable]

### Cluster: <descriptive name> — <subsystem>
<1-2 sentences on the shared mechanism>

| Test | SameBoy | DocBoy | gambatte | Detail (got/exp or pixel-mismatch) |
|------|---------|--------|----------|------------------------------------|
| ... | pass | pass | fail | got X, exp Y |

**Cluster reference status**: refs agree+pass / refs split / all-fail
**Likely root cause**: <hypothesis from names + deltas>
**Difficulty**: easy / medium / hard — <why>

## P2 — CGB-specific (no DMG counterpart)
### Cluster: ... — <subsystem; mark "feature not yet implemented" where applicable>

## P3 — Shared, fails both cores
| Test | SameBoy | DocBoy | gambatte | Detail |
|------|---------|--------|----------|--------|

## Not yet implemented (triaged out of the bug clusters)
<clusters that are missing features, not modelling errors>

## Difficulty ranking
Rank P1 and P2 clusters easiest→hardest, considering:
1. Cluster size (more tests = higher-impact fix)
2. P1 (DMG model is the reference) ranks above P2 (no gate-level reference)
3. Whether the failure pattern is a simple offset vs a structural model change
4. Whether closely-related tests already pass (model is close)
```

For a **DMG report** (`missingno-gb`), drop the P1/P2/P3 partition (single core) and organise straight into subsystem clusters with the same per-cluster reference labels.

## Notes

- Run `cargo test -p missingno-gb` / `-p missingno-gbc`, not bare `cargo test` (which pulls in GUI tests).
- The report scripts handle formatting and baseline diffing; the `-gbc` script emits the core-diff partition.
- Manifest data may lag the suite — a test can exist in the suite but not the manifest, or vice versa. Try the fuzzy strategies before giving up; otherwise mark "no manifest".
- **Detail column**: got/expected for register tests (gbmicrotest, gambatte, mooneye); pixel-mismatch counts for screenshot tests (cgb-acid2, cgb-acid-hell, mealybug-tearoom, age, scribbltests).
- **DocBoy** provides T-cycle granularity traces — most useful when a cluster needs cycle-level localisation; note it for the caller.
- **Hardware timing data** (`receipts/resources/gb-timing-data/`): when clustering, check whether a measurement campaign covers the cluster's subsystem — it can confirm a hypothesised cause. Data collection is in progress; check what's available.
- **Slowpeek** (`receipts/resources/slowpeek/`): when a cluster's timing behaviour is covered by no existing data source, note that a Slowpeek sweep could give the definitive hardware measurement. Hardware serial path not yet complete.
