# missingno-zilog-z80 — chip methodology

Chip-specific methodology for the shared Zilog NMOS Z80 core. The shared
skill-system rules and workflow discipline live in the repository-root
`AGENTS.md`. This crate is *silicon without a board*: it models the CPU
(T-state-stepped, with bus activity recorded at T-state resolution,
including WZ/MEMPTR and the Q flag latch) and leaves board wiring — memory
maps, I/O decoding, driving /WAIT — to its consumers (the SG-1000 and SMS
cores).

**A system doc outranks this one in-system.** No Z80 consumer has a
gate-level source — per the root rules, where no gate-level oracle exists,
escalate to the user rather than substituting an emulator's behaviour for
hardware fact.

## Ground-truth hierarchy (the chip in isolation)

1. **The SingleStepTests Z80 oracle**
   (https://github.com/SingleStepTests/z80) — the crate's conformance floor.
   Per-T-state bus activity, port transactions, and final state for every
   opcode including the CB/ED/DD/FD/DDCB/FDCB prefixes. The sweep in
   `tests/single_step.rs` runs in plain `cargo test`, fetches the data over
   the network on first run into `tests/single-step-tests/` (gitignored;
   `SINGLE_STEP_TESTS_Z80_DIR` overrides), and records the fetched oracle
   commit in a marker file beside it. Tier
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
  first, not worked around. `tests/bus_timing.rs` pins the emitted per-cycle
  shapes.
- **The /WAIT input, and where it is sampled.** The board answers for the pin
  through `Bus::wait_requested` (defaulted to a released line), read at each
  sample edge — so a device whose not-ready condition begins with the access
  itself, like a PSG whose READY falls at its own select strobe, stretches
  the very cycle that strobed it. Following the Zilog Z80 User Manual,
  the line is sampled at the falling edge of a transfer cycle's access T-state
  — T2 for an opcode fetch or a memory read/write, the automatically inserted
  TW for an I/O cycle — and an active sample makes the following T-state a wait
  state, which holds the last driven address with the data pins off, samples
  again at its own falling edge, and hands back to the cycle's schedule on
  release. The manual states the sample points and the "another WAIT state is
  entered during the following cycle" rule; it gives no text for what the pins
  do during a TW, so the held snapshot follows its figures (address and control
  lines continuous across the inserted state) and this crate's single-pulse
  waveform. Three limits:
  - **M1's refresh T-states are not sampled.** The manual names only T2 and the
    subsequent automatic wait states, and its fetch figure draws no sample
    pulse in T3/T4 — a silence, not a stated absence. A wait chain entered at
    T2 delays refresh wholesale.
  - **The interrupt-acknowledge cycle is not gated.** Acceptance is modelled as
    an internal cycle (see the interrupt bullet below), and gating an
    abstracted cycle would invent structure the model does not have. The
    manual's two automatic wait states there are unmodelled as such.
  - **A waited read returns its data at the access T-state**, before the wait
    chain, where hardware samples the data bus after release. Fine for a stall
    that only delays; a consumer whose waited *reads* carry time-sensitive data
    (SMS VDP slot contention) must bring evidence and extend the read model
    first.

  The oracle drives no wait states, so the whole waited path is
  **oracle-unverified**; `tests/wait_states.rs` pins the T counts and held
  snapshots, and the sweep — riding the trait's released default — pins that
  a board that never answers changes nothing.
- **The acknowledge cycle never reaches the bus.** Acceptance is an internal
  cycle: no device sees an M1+IORQ strobe and no device supplies a byte. In
  consequence IM 0 executes as IM 1 (the unsupplied opcode is RST $38), and
  IM 2 forms its vector with a fixed $FF low byte where hardware reads the
  device's. A board whose interrupting devices drive the bus (a Z80
  peripheral daisy chain, ColecoVision-style IORQ+M1 decode) needs the
  acknowledge surfaced first, not worked around.
- **Interrupt entry timing.** The oracle contains no interrupt cases, so the
  cycle-level acceptance sequence is **unverified**. The /INT sample point
  follows the Zilog UM: the line is sampled at the rising edge of an
  instruction's final T-state, so a halted CPU wakes on its 4 T refetch
  grid and the wake phase follows the halt's entry phase
  (`tests/halt_wake.rs`). Console work that depends on finer acceptance
  timing (the SMS/SG-1000 VDP line interrupt) must bring its own
  evidence — test ROMs measured on hardware — and flag the gap to the user
  rather than tuning to a test.
