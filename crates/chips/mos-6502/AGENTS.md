# missingno-mos-6502 — chip methodology

Chip-specific methodology for the shared NMOS 6502-family core. The shared
skill-system rules and workflow discipline live in the repository-root
`AGENTS.md`. This crate is *silicon without a board*: it models the CPU die's
behaviour (cycle-stepped, one bus access per cycle, RDY freeze semantics) and
leaves packaging and wiring to its consumers — the VCS's 6507 (masked address
bus, no interrupt pins) and the NES's 2A03 core (no decimal mode), each applied
in the console crate, not here.

**A system doc outranks this one in-system** — for the VCS that means Sim2600
gate-level lockstep and hardware captures sit *above* everything below. This
doc governs only the chip in isolation.

## Ground-truth hierarchy (the chip in isolation)

1. **The SingleStepTests 65x02 oracle**
   (https://github.com/SingleStepTests/65x02) — the crate's conformance floor.
   Per-cycle bus activity and final state for all 256 opcodes (documented and
   undocumented) in each of the `6502` (decimal) and `nes6502` (decimal-less)
   variants. The sweep in `tests/single_step.rs` runs in plain `cargo test`,
   fetches the data over the network on first run into
   `tests/single-step-tests/` (gitignored; `SINGLE_STEP_TESTS_DIR` overrides),
   and records the fetched oracle commit in a marker file beside it. Tier
   honesty: the oracle is **behavioural, not silicon-derived** — its README
   describes generation from a reference implementation conforming to available
   documentation and prior published test sets. It is the floor, not the
   ceiling; a gate-level source (Sim2600, visual6502 lineage) outranks it where
   one applies.
2. **Hardware documentation** — the MOS MCS6500 family programming and hardware
   manuals and datasheets; for undocumented opcodes, the modern collations of
   measured NMOS behaviour (e.g. *NMOS 6510 Unintended Opcodes*). Use these to
   adjudicate anything the oracle is silent on, and attribute findings to the
   named document.
3. **Other emulators** — reference material only, attributed by name, never
   ground truth.

## Known oracle-silent edges

- **Interrupt dispatch timing.** Dispatch is implemented with documented
  acceptance semantics, but the oracle contains no interrupt cases and no VCS
  software can observe the poll points. Cycle-level interrupt timing is
  therefore **unverified**; NES work that depends on it (IRQ/NMI poll points,
  branch-boundary quirks) must bring its own evidence before trusting the
  current model, and flag the gap to the user rather than tuning to a test.
- **RDY on write cycles.** The model honours RDY on reads only (writes always
  complete), matching the NMOS behaviour the VCS relies on; treat any consumer
  that needs finer RDY semantics as new evidence work.
