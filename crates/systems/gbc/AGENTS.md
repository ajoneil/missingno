# missingno-gbc — Game Boy Color methodology

Core-specific accuracy methodology for `missingno-gbc`. The shared skill-system
rules, agent infrastructure, and workflow discipline live in the repository-root
`AGENTS.md`; the Game Boy family base (DMG ground truth, clock model, instruction
execution) lives in `crates/systems/gb/AGENTS.md`. CGB is a superset of DMG, so
read the `missingno-gb` methodology first — this file is the CGB *delta*.

The DMG methodology assumes **gate-level ground truth** — dmg-sim, the DMG-CPU die
netlist analysis, and the DMG Timing Specification collated from them. **None of
that exists for the Game Boy Color**: there is no CGB die-sim or netlist. The rules
still hold in spirit ("hardware is the source of truth"), but their
*operationalisation* changes. Read this before any CGB investigation.

- **CGB is a superset of DMG.** SM83 is identical; the PPU pixel pipeline, OAM scan,
  mode state machine, and FIFO are the same silicon family. For every circuit shared
  with DMG, **the DMG Timing Specification and its gate names remain authoritative** —
  don't re-derive them. CGB work is the *delta*: double-speed (KEY1), VRAM banking +
  BG tile attributes, BG/OBJ palette RAM (CRAM: BCPS/BCPD/OCPS/OCPD), HDMA/GDMA,
  object priority (OPRI), DMG-compatibility palettes, and the extra WRAM/VRAM banks.

- **In-house first cut: diff our own two cores.** The cheapest, most informative CGB
  triage is the set-diff of the gb and gbc baselines. A test that **passes
  `missingno-gb` but fails `missingno-gbc`** is shared behaviour that's either a
  CGB-integration regression or an unimplemented CGB divergence — mechanism mostly
  already understood from the DMG model, and the DMG core is the reference. A test
  with **no DMG counterpart** is genuinely CGB-specific. Use this partition before any
  cross-emulator lookup; `test-report-gbc.sh` emits it automatically. The gate is a
  fully-passing suite — any failure is a regression, and this partition is how you
  triage one.

- **Ground-truth hierarchy for the CGB delta** (where no gate-level truth exists), in
  order:
  1. **Hardware test-ROM expected values** — measured on real CGB hardware; the only
     true ground truth left. Weight these *more* heavily than on DMG. (cgb-acid2,
     cgb-acid-hell, samesuite CGB, age CGB variants, mooneye CGB, gambatte CGB
     expected-output suffixes.)
  2. **Documented hardware behaviour** — gb-ctr (CGB registers, DMA, double-speed),
     Pan Docs CGB sections, and **SameBoy source comments** (extensively documents CGB
     hardware findings inline — treat as *documentation leads*, not code to copy).
  3. **Cross-emulator behavioural agreement** (SameBoy ≈ DocBoy ≈ gambatte) — a
     *consistency* signal, not a mechanism. All CGB references are behavioural peers;
     there is no die-derived tier to break ties.

- **Don't model SameBoy.** Without gate-level truth the pull toward copying the
  most-accurate emulator is strong, and it violates the "don't mimic emulator
  internals" rule. The disciplined loop: the hardware-test-ROM expected value is the
  target; behavioural references *localise* the divergence and *suggest* a mechanism;
  the mechanism must be consistent with documented hardware and with the shared DMG
  model. Never "SameBoy does X, so we do X."

- **No silent fallback when CGB behaviour is unknown.** DMG has an escape hatch
  (dmg-sim measurement → spec update). CGB has none. Escalation when the references
  don't settle it: isolate with a hardware test ROM → gb-ctr / Pan Docs → SameBoy-comment
  lead → **ask the user**. Do not substitute "SameBoy does X" for "the hardware does X"
  and move on.

- **CGB reference traces** (morepork): the traced emulators are **SameBoy, DocBoy,
  gambatte, missingno** — all behavioural. Preference order for CGB trace comparison:
  **SameBoy → DocBoy → gambatte**. The manifest schema is `systems.{dmg,cgb}.{emulator}`.

- **Double-speed is implemented via the clock divider.** `MasterClock` owns a ÷1/÷2
  divider between CPU edge and dot edge: at double speed (KEY1) the CPU takes two edges
  per dot and the dot edge lands on alternate CPU edges; the speed-switch blackout holds
  the CPU phase while the dot domain free-runs. The split is across crates: this crate
  owns the switch/blackout orchestration (`speed_switch.rs` and the `Model` hooks in
  `lib.rs` — arming, the blackout countdown, the wake), while the held-edge stepping it
  drives is generic and lives in the gb crate (`step_blackout_chunk`,
  `crates/systems/gb/src/execute/blackout.rs`). Extend this model, don't redesign it.

## Resources

**Tier-1 test ROMs.** Two roots, resolved by the helpers in
`crates/systems/gbc/tests/accuracy/common/`: ROMs that run on both models live in
the gb crate (`crates/systems/gb/tests/accuracy/roms/`, loaded with `load_rom`),
CGB-only ROMs in this crate (`crates/systems/gbc/tests/accuracy/roms/`,
`load_cgb_rom`). Each root has an `ATTRIBUTION.md` with per-suite author and
licence; the binaries come from the
[c-sp/game-boy-test-roms](https://github.com/c-sp/game-boy-test-roms) collection.

| Suite | Upstream | ROMs |
|-------|----------|------|
| cgb-acid2 | https://github.com/mattcurrie/cgb-acid2 | gbc `roms/cgb-acid2/` |
| cgb-acid-hell | https://github.com/mattcurrie/cgb-acid-hell | gbc `roms/cgb-acid-hell/` |
| SameSuite | https://github.com/LIJI32/SameSuite | gbc `roms/samesuite/`; the DMG-compatible ones in the gb root |
| age-test-roms | https://github.com/c-sp/age-test-roms | `roms/age-test-roms/` in both roots |
| mooneye-test-suite | https://github.com/Gekkio/mooneye-test-suite | gb root `roms/mooneye/`; the CGB subset is selected by filename suffix |
| gambatte | https://github.com/pokemon-speedrunning/gambatte-core | `roms/gambatte/` in both roots; the expected-output suffixes are the runner's (`test/testrunner.cpp`) |
| rtc3test | https://github.com/aaaaaa123456789/rtc3test | gbc `roms/rtc3test/` |

**Tier-2 references**: gb-ctr (https://gekkio.fi/files/gb-docs/gbctr.pdf),
Pan Docs (https://github.com/gbdev/pandocs), and the SameBoy source read for
its inline hardware comments (https://github.com/LIJI32/SameBoy).
