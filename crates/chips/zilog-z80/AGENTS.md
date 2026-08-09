# missingno-zilog-z80 — chip methodology

Chip-specific methodology for the shared Zilog NMOS Z80 core. The shared
skill-system rules and workflow discipline live in the repository-root
`AGENTS.md`. This crate is *silicon without a board*: it models the CPU
(T-state-stepped, with bus activity recorded at T-state resolution,
including WZ/MEMPTR and the Q flag latch) and leaves board wiring — memory
maps, I/O decoding, wait states — to its consumers (the SMS core today, SG-1000
next).

**A system doc outranks this one in-system.** When the chip is inside a
console, that console's ground-truth hierarchy adjudicates. No Z80 consumer
currently has a gate-level source wired up — per the root rules, where no
gate-level oracle exists, escalate to the user rather than substituting an
emulator's behaviour for hardware fact.

## Ground-truth hierarchy (the chip in isolation)

1. **The SingleStepTests Z80 oracle** — the crate's conformance floor.
   Per-T-state bus activity, port transactions, and final state for every
   opcode including the CB/ED/DD/FD/DDCB/FDCB prefixes: 1,604 files,
   ~1.6M cases, all passing. The sweep in `tests/single_step.rs` runs in plain
   `cargo test`, fetches the data on first run into `tests/single-step-tests/`
   (gitignored), and records the oracle commit in `FETCHED_COMMIT`. Tier
   honesty: the oracle is **behavioural, not silicon-derived** (the
   SingleStepTests project generates sets from a reference implementation
   conforming to available documentation and prior test suites). It is the
   floor, not the ceiling.
2. **Hardware documentation** — the Zilog Z80 User Manual for documented
   behaviour; *The Undocumented Z80 Documented* (Sean Young) for undocumented
   opcodes, X/Y flag results, and MEMPTR/Q semantics. Use these to adjudicate
   anything the oracle is silent on, and attribute findings to the named
   document.
3. **Other emulators** — reference material only, attributed by name, never
   ground truth.

## Stated abstractions and oracle-silent edges

- **Simplified memory-cycle waveform.** Bus snapshots follow the oracle's
  simplified timing (MREQ/RD/WR pulsed for a single T-state) rather than the
  real multi-T-state pin waveform. This is an honest abstraction with its
  limit stated: consumers that decode timing finer than whole machine cycles
  (wait-state insertion, refresh-cycle observation) need the model extended
  first, not worked around.
- **No WAIT input.** The core has no `/WAIT` pin: every machine cycle runs its
  nominal T-states, so a consumer whose board inserts wait states (the SMS VDP
  slot contention, a slow cartridge) must model that outside the chip until the
  pin exists.
- **Interrupt entry timing.** NMI and IM 0/1/2 acceptance implement the
  documented semantics, but the oracle contains no interrupt cases, so
  cycle-level interrupt timing is **unverified**. Console work that depends on
  it (the SMS/SG-1000 VDP line interrupt) must bring its own evidence — test
  ROMs measured on hardware — and flag the gap to the user rather than tuning
  to a test.
