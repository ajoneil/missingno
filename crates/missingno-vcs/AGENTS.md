# missingno-vcs — Atari 2600 (VCS) methodology

Core-specific accuracy methodology for `missingno-vcs`. The shared skill-system
rules, agent infrastructure, and workflow discipline live in the repository-root
`AGENTS.md` — read that first; this file is the VCS *delta*. Load it before any
VCS accuracy work.

## Ground-truth hierarchy

The DMG methodology assumes **gate-level ground truth** — dmg-sim, the die
netlist, and the DMG Timing Specification collated from them. The Atari 2600 has
that footing for **two of its three chips and loses it for the third**, so the
DMG discipline holds but the ground-truth tier is *split by chip*. **The netlist
and the simulators are the primary resource for solving accuracy issues — they
explain *why* the hardware behaves as it does. The behavioural VCS emulators sit
below them and only corroborate.**

- **Gate-level truth for the CPU and TIA: Sim2600.** `receipts/resources/Sim2600/`
  is a transistor-level simulation of the 6507 and TIA, built from the visual6502
  decapped netlists (`chips/net_6502.pkl`, `chips/net_TIA.pkl`) and run against
  real cartridge ROMs. It is the VCS analog of dmg-sim: the runnable oracle for
  *why* the CPU and the beam do what they do — undocumented opcodes, RESP/HMOVE
  landing, RSYNC, WSYNC release, mid-line register races. For any CPU or TIA
  gate-level question it is authoritative. Run it headless with
  `scripts/sim2600-observe.sh <rom> [half_clocks] [output_dir] [extra_wires]`
  (the `dmg-sim-observe.sh` analog) — it clones + Py3-ports Sim2600 on first use,
  runs the ROM, and dumps a VCD of the CPU/TIA wires per half-clock for GTKWave
  (CLK0, RDY, R/W, SYNC, address/data buses, PC/A/X/Y/S/P; TIA
  VSYNC/VBLANK/WSYNC/RSYNC, colour, luminance). Add targeted probes with
  `cpu:NAME,tia:NAME` (e.g. `tia:BL_lowCtrl`). It is slow (transistor-level,
  ~500 half-clocks/s; ~40000 to the first pixel), so like a dmg-sim run, name the
  specific wire and half-clock rather than running long from a dispatch — and
  don't spin one up for a question the schematics already answer.

- **The RIOT has no gate-level oracle.** The 6532 (RAM + I/O + timer) was never
  fully reverse-engineered — visual6502's RIOT netlist is ~⅓ complete, so Sim2600
  *emulates* the PIA behaviourally (`emuPIA.py`) rather than simulating a netlist.
  For RIOT timing (timer prescaler and underflow, port and interrupt edges) VCS is
  in the **CGB regime — no die-derived truth**. Ground it, in order, on the **MOS
  6532 datasheet**, the **console schematics**, and **hardware-confirmed timer
  torture ROMs** (TimerTest, the diagnostic cartridge), with behavioural emulators
  only to localise. **Never treat Sim2600's PIA as ground truth.**

- **Static-analysis layer (the gb-propagation-delay-analysis analog): schematics +
  TIA_HW_Notes.** The reverse-engineered TIA/RIOT/console schematics and Andrew
  Towers' `TIA_HW_Notes.txt` explain which signal gates which — and races and
  timing — *without* running a sim. Reach for them first to frame a question, then
  use Sim2600 to confirm the specific edge. (These live in the user's Atari 2600
  reference library, not in-repo; ask for a copy in `receipts/resources/` when one
  is needed.)

- **Documented hardware behaviour.** The **Stella Programmer's Guide** is the
  canonical TIA/RIOT programming reference; the **TIA Technical Manual** and the
  **6532 datasheet** are the chip-level references. Treat these as the VCS
  equivalent of gb-ctr — reliable written hardware documentation, below the
  netlist/sim tier.

- **Behavioural cross-emulator traces (gbtrace) — corroborate only.** VCS gbtrace
  adapters already exist for **Stella, Gopher2600, and MAME** (plus missingno), all
  behavioural. Preference order: **Stella → Gopher2600 → MAME**. They *localise* a
  divergence and *suggest* a mechanism; they never ground one. Never "Stella does
  X, so we do X" — the mechanism must trace back to Sim2600 (CPU/TIA) or the
  datasheet + schematics (RIOT). The vcs-tests suite's own verdicts ride the RESULT
  RAM convention (`$80` PASS/FAIL, `$81` code, `$82`/`$83` observed/expected).

- **No silent fallback.** As on CGB: when Sim2600 (CPU/TIA) or the
  datasheet/schematics (RIOT) don't settle a question, escalate — isolate with a
  hardware/torture test ROM → 6532 datasheet / Stella Programmer's Guide → Sim2600
  signal observation (CPU/TIA only) → **ask the user**. Do not substitute "Stella
  does X" for "the hardware does X" and move on.

## Resources

| Resource | Location / URL | Description |
|----------|----------------|-------------|
| Sim2600 | `receipts/resources/Sim2600/` (https://github.com/gregjames/Sim2600) | Transistor-level sim of the 6507 + TIA from the visual6502 decapped netlists — the gate-level oracle. RIOT is emulated (`emuPIA.py`), not netlist-simulated. Run headless via `scripts/sim2600-observe.sh`. |
| sim2600-observe | `scripts/sim2600-observe.sh` (+ `scripts/sim2600_observe.py`) | Headless harness: runs a ROM through Sim2600 and dumps a per-half-clock VCD of the CPU/TIA wires for GTKWave (the `dmg-sim-observe.sh` analog). |
| Stella Programmer's Guide | https://atarihq.com/danb/files/stella.pdf | Canonical TIA/RIOT programming reference (VCS equivalent of gb-ctr). |
| TIA_HW_Notes | https://www.atarihq.com/danb/files/TIA_HW_Notes.txt | Andrew Towers' TIA hardware timing notes — the static-analysis layer. |
| MOS 6532 datasheet | https://6502.org/documents/datasheets/mos/mos_6532_riot.pdf | RIOT chip reference — the primary RIOT source (no gate-level sim exists). |
| vcs-tests suite | `crates/missingno-vcs/tests/accuracy/` | The in-repo accuracy suite (RESULT RAM convention; NTSC + PAL). Baseline/diff via `scripts/test-report-vcs.sh`. |

## Core shape

`missingno-vcs` is a standalone core crate (not a `Console<M>` model): 6507 + TIA +
RIOT + cartridge on one colour-clock master, one CPU cycle = three colour clocks.
See `src/console.rs` for the step loop and bus map, `src/tia/` for the pixel
pipeline, `src/riot.rs` for the timer/ports. The frontend drives it through the
`app/system/` seam described in `docs/adding-a-system.md`.
