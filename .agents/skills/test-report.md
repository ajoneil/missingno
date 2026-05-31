# Test Report

Run the suite, categorise the failures by root cause, and label each cluster with cross-emulator status from the gbtrace manifests. **Effort scales to the size of the failure set** — a handful of DMG failures is a quick inline job; the thousands on the CGB core need fan-out and the core-diff partition. Match the effort to the count; don't run the heavy machinery for six failures.

## Two rules that always hold

1. **Scratch never touches the repo.** Put every intermediate file in a `mktemp -d` and `trap` it clean on exit. Never write files or directories under the working tree — no `.tmp_testreport/`, no stray `failed.txt`, no per-suite manifests dumped in the repo. The *only* thing you write into the repo is the final report, under `receipts/test-reports/{gb,gbc}/` (gitignored).
2. **You are a fact-finder, not a problem-solver.** Categorise failures; do not fix, design, or open an investigation. If the suite won't build, or panics before any test runs, record what happened and stop.

## Which core

| core | crate | wrapper |
|------|-------|---------|
| DMG | `missingno-gb` | `./scripts/test-report-gb.sh` |
| CGB | `missingno-gbc` | `./scripts/test-report-gbc.sh` |

The two are wildly different scales (DMG: a few failures; CGB: thousands), which is why effort is gated on the count rather than fixed.

## Step 1 — Census + capture (one suite run)

Make a scratch dir and run the wrapper once, asking it to also dump the raw cargo output for got/expected detail:

```bash
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
TEST_REPORT_RAW_OUT="$tmp/raw.txt" ./scripts/test-report-gb.sh --diff    # or -gbc
```

The wrapper prints **counts**, the **failing-test list**, baseline **regressions / newly-passing**, and (gbc only) the **P1/P2/P3 core-diff partition** — and writes `$tmp/raw.txt` with the full per-test panic / got-expected output, so you don't run the suite twice. It overwrites a single `latest.md`; it does not accumulate files.

**The failure count from this step decides everything below.**

## Step 2 — Pick effort by count

- **0 failures** → report "all green" and stop.
- **≤ 25 failures** (typical DMG) → **Inline path**. The whole set fits in context; no subagent.
- **> 25 failures** (typical CGB) → **Fan-out path**: dispatch ONE `general-purpose` subagent so the large intermediate data (raw output, manifest joins) stays out of the caller's context. Hand it this skill file and the core name; it returns the receipt path + a one-line-per-cluster summary.

---

## Inline path (small sets)

You already have the failing list and `$tmp/raw.txt`. No subagent, no heavy clustering ceremony.

1. **Group** failures by module → subsystem (the module prefix is the suite: `mealybug_tearoom::…`, `gbmicrotest::…`, `mooneye::…`). A handful usually collapses to 1–3 groups. Note `_a`/`_b` boundary pairs and shared got/expected deltas — both signal one cause.
2. **Detail** each failure from `$tmp/raw.txt`: got/expected for register suites; pixel-mismatch for screenshot suites (mealybug, dmg-acid2, scribbltests).
3. **Manifest label** — fetch *only the suites that actually have failures*, into `$tmp`:
   ```bash
   curl -s "https://ajoneil.github.io/gbtrace/tests/mealybug-tearoom/manifest.json" -o "$tmp/m.json"
   ```
   Read the system matching the core (`dmg` for gb, `cgb` for gbc). See *Manifest reference* for schema + name-matching.
4. **Write** a short report to `receipts/test-reports/<core>/latest-analysis.md` in the format below.

## Fan-out path (large sets, CGB)

One subagent, same scratch rule (`mktemp -d`, repo stays clean). Organise by the **core-diff partition**, then **cluster-first** labelling — never per-test lookups at this scale.

1. **Partition** (gbc): the wrapper already printed P1/P2/P3 counts. Reconstruct the sets by joining the failing list against `scripts/.test-baseline-gb` (in `$tmp`):
   - **P1** passes DMG / fails CGB — shared regression; most approachable, the DMG core + DMG PPU spec are the reference.
   - **P2** no DMG counterpart — genuinely CGB-specific; needs the CGB ground-truth hierarchy (test-ROM > docs > behavioural refs).
   - **P3** fails both — underlying DMG bug; usually fix in the DMG core first.
2. **Cluster within each partition** by subsystem + root cause *before any reference lookup*. Indicators: same subsystem (CGB: double-speed/KEY1, HDMA/GDMA, CRAM/palettes BCPS/BCPD/OCPS/OCPD, VRAM-bank/tile-attributes, OPRI, DMG-compat palettes; shared: STAT/mode timing, window, sprites, OAM, timer, APU, MBC/RTC), same test family (`sprite4_0_a`..`sprite4_7_a`), `_a`/`_b` pairs, shared got/expected delta. The gambatte filename suffix `_dmgXX_cgbXX_outN` encodes target models + expected output — cluster on it with no manifest lookup. Each test in exactly one cluster. Separate *not-yet-implemented features* from *modelling bugs*.
3. **Label per cluster** — sample a few representative tests (not all), fetching only the suites present in the failure set.
4. **Write** the report (P1/P2/P3 sections + difficulty ranking) to `receipts/test-reports/gbc/latest-analysis.md`.

---

## Manifest reference

Schema — each entry carries per-*system* status:
```json
{"name":"…","systems":{"dmg":{"sameboy":"pass","docboy":"pass","gambatte":"fail","missingno":"fail"},
                       "cgb":{"sameboy":"pass","docboy":"fail","gambatte":"fail","missingno":"fail"}}}
```
Pick the system matching the core — `t['systems'].get('dmg',{}).get('sameboy','N/A')` (gb) or `…get('cgb',{})…` (gbc). Tracked emulators: `sameboy`, `docboy`, `gambatte`, `missingno` (no gateboy). A suite may carry only one system (gbmicrotest→dmg, cgb-acid2→cgb). Suite manifest URL: `https://ajoneil.github.io/gbtrace/tests/<suite>/manifest.json`.

Module→suite: dashes for underscores, with exceptions `cgb_acid2`→`cgb-acid2`, `cgb_acid_hell`→`cgb-acid-hell`, `mbc3_tester`→`mbc3-tester`, `mealybug_tearoom`→`mealybug-tearoom`, `mooneye_wilbertpol`→`mooneye-wilbertpol`, `turtle_tests`→`turtle-tests`.

Name matching (manifest stems use `__` for path separators + variant suffixes; Rust uses `_`): try exact, then `_`↔`-`, then per-suite:
- **age** — variant suffixes (`-dmgC-cgbBCE`, `-nocgb`, `-GS`); `_`→`-`, match keys that START WITH the prefix.
- **gambatte** — `__` directory separators; test = last `__` segment minus `_dmgXX_cgbXX_outN`; `scx3` inserts `__scx3__`; hex (`spx1A`) case-insensitive.
- **mooneye / -wilbertpol** — strip `acceptance__ppu__` / `gpu__` / `serial__` prefix; `_gs`→`-GS`, `_dmg`→`-dmgABCmgb`.
- **samesuite** — paths like `apu__channel_1__…-cgb0BC`; many carry a `-cgbXX` suffix.
- **blargg** — not tracked → "no manifest".
- Fallback: lowercase, strip `_ - /`, compare. Still nothing → "no manifest".

Per-cluster label meaning:
- **refs agree + pass** → behaviour well-defined and reproducible; the divergence is ours alone.
- **refs split** → contentious; weight the hardware test-ROM expected value + gb-ctr / Pan Docs over any single emulator.
- **all refs fail** → likely a test-ROM edge case or shared bug; low priority.

References are behavioural peers — they show the behaviour is *reproducible*, not *how the hardware does it*. Prefer SameBoy when they disagree. There is no gate-level reference.

## Report format

```markdown
# Test Failure Analysis (<core>) — <date>

## Summary
- Passed N / Failed N / Ignored N
- (gbc) Partition: P1 N · P2 N · P3 N

## <Cluster name> — <subsystem>   (×N tests)
<one line: the shared mechanism>

| Test | SameBoy | DocBoy | gambatte | Detail |
|------|---------|--------|----------|--------|
| … | pass | pass | fail | pixel mismatch / got X exp Y |

Refs: agree+pass / split / all-fail · Likely cause: <hypothesis> · Difficulty: easy/med/hard

## Not yet implemented (not bugs)
<feature-gap clusters>

## Difficulty ranking
<clusters easiest→hardest: size, simple-offset vs structural, whether neighbours pass; gbc: P1 above P2>
```

For DMG drop the partition line and the P1/P2/P3 framing — just subsystem clusters.

## Notes

- `cargo test -p missingno-gb` / `-gbc`, never bare `cargo test` (pulls in GUI tests).
- Manifests may lag the suite — try the fuzzy matches, else mark "no manifest".
- DocBoy traces at T-cycle granularity — note it when a cluster needs cycle-level localisation.
- Hardware timing data (`receipts/resources/gb-timing-data/`) can confirm a hypothesised timing cause; check which campaigns exist. A Slowpeek sweep (`receipts/resources/slowpeek/`) is the definitive hardware measurement when no data source covers a cluster (hardware serial path not yet complete).
