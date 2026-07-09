# missingno-gbc — Game Boy Color methodology

Core-specific accuracy methodology for `missingno-gbc`. The shared skill-system
rules, agent infrastructure, and workflow discipline live in the repository-root
`AGENTS.md`; the Game Boy family base (DMG ground truth, clock model, instruction
execution) lives in `crates/missingno-gb/AGENTS.md`. CGB is a superset of DMG, so
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
  cross-emulator lookup; `test-report-gbc.sh` emits it automatically. (Both suites
  fully pass in the steady state — this triage applies when a regression or a newly
  added test breaks the green baseline.)

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

- **CGB reference traces** (gbtrace): the traced emulators are **SameBoy, DocBoy,
  gambatte, missingno** — all behavioural. Preference order for CGB trace comparison:
  **SameBoy → DocBoy → gambatte**. The manifest schema is `systems.{dmg,cgb}.{emulator}`.

- **Double-speed is implemented via the clock divider.** `MasterClock` owns a ÷1/÷2
  divider between CPU edge and dot edge: at double speed (KEY1) the CPU takes two edges
  per dot and the dot edge lands on alternate CPU edges; the speed-switch blackout holds
  the CPU phase while the dot domain free-runs (`step_blackout_chunk`). The
  blackout/switch orchestration lives in `missingno-gbc`. Both suites fully pass under
  this model — extend it, don't redesign it.
