# missingno-vcs — Atari VCS methodology

Core-specific accuracy methodology for `missingno-vcs`. The shared skill-system
rules, agent infrastructure, and workflow discipline live in the repository-root
`AGENTS.md` — read that first; this file is the VCS *delta*. Load it before any
VCS accuracy work.

## Ground-truth hierarchy

The DMG methodology assumes **gate-level ground truth** — dmg-sim, the die
netlist, and the DMG Timing Specification collated from them. The Atari VCS has
that footing for **two of its three chips and loses it for the third**, so the
DMG discipline holds but the ground-truth tier is *split by chip* — and topped
by real-console captures. **Hardware captures are the arbiter of WHAT; the
netlist and the simulators are the primary resource for WHY. The behavioural
VCS emulators sit below both and only corroborate.**

- **Real-console captures: the arbiter of observable behaviour.** The test-suite
  repo (see the vcs-tests row below) holds calibrated 1080p captures of every
  screenshot test from a real PAL console (multiple boots), a calibration-rig
  profile, and `scripts/hwcompare.py` to register captures onto the TIA grid
  and score them. The committed suite references are endorsed by these captures
  cell-by-cell, so a screenshot regression is a divergence from measured
  silicon. When a reference and a sim run disagree, the capture wins — never
  refute a capture with a sim run. Sub-pixel capture reads have conventions
  (plateau-centre scanlines, local dx bias correction against agreed-content
  controls, PSF predictions per hypothesis; a lit far-right dash peaks ~1 px
  left of nominal); tooling-level verdicts (region NCC, window centroids) have
  known artifact modes — arbitrate contested cells with a raw bias-corrected
  read, not a tool verdict alone.

- **Gate-level truth for the CPU and TIA: Sim2600.** `receipts/resources/Sim2600/`
  is a transistor-level simulation of the 6507 and TIA, built from the visual6502
  decapped netlists (`chips/net_6502.pkl`, `chips/net_TIA.pkl`) and run against
  real cartridge ROMs. It is the VCS analog of dmg-sim: the runnable oracle for
  *why* the CPU and the beam do what they do — undocumented opcodes, RESP/HMOVE
  landing, RSYNC, WSYNC release, mid-line register races. For CPU and TIA
  mechanism questions it is authoritative, **with one measured limit: it can
  mis-resolve ±1-CLK sub-clock races that real silicon settles the other way**
  (its edges are sharp and it has no analog propagation delays; confirmed on
  the reset-vs-redraw window edge and stuck-train serialiser edge cells). For
  any behaviour that hinges on such a race, ground the WHAT in a hardware
  capture and use the sim for the mechanism skeleton. Run it headless with
  `scripts/sim2600-observe.sh <rom> [half_clocks] [output_dir] [extra_wires]`
  (the `dmg-sim-observe.sh` analog) — it clones + Py3-ports Sim2600 on first use,
  runs the ROM, and dumps a VCD of the CPU/TIA wires per half-clock for GTKWave
  (CLK0, RDY, R/W, SYNC, address/data buses, PC/A/X/Y/S/P; TIA
  VSYNC/VBLANK/WSYNC/RSYNC, colour, luminance). Add targeted probes with
  `cpu:NAME,tia:NAME` (e.g. `tia:BL_lowCtrl`). It is slow (transistor-level,
  ~500 half-clocks/s; ~40000 to the first pixel), so like a dmg-sim run, name the
  specific wire and half-clock rather than running long from a dispatch — and
  don't spin one up for a question the schematics already answer.

- **Sim2600 measurement conventions — read before porting any sim value.** The
  probe frame anchored at a `BL_lowCtrl` 1→0 fall IS the screen frame (playfield
  cells land at their exact programmed screen clocks in it), but the sim performs
  no power-on alignment: its CPU-grid-vs-line phase is an arbitrary netlist
  settle, and it measures 2 CLK left of the reference convention the emulator
  models (`PHI0_GRID_PHASE` in `src/console.rs`). That phase is a machine
  constant — RSYNC requantises the φ0 divider together with the horizontal
  counter, so no program walks it — which means a capture's ABSOLUTE landings are
  valid only at the sim's own phase. Measure and port RELATIVE to the triggering
  write's beam position: its bus cycle sits at half-integer probe x, the φ2-fall
  is the cycle's end, and a strobe's address-decoded level rises 1 CLK before
  that fall and releases 0.5 CLK after it (SEC-like latches respond to the rise;
  counter grounds to the release). Sample once per half-clock
  (advance-then-sample), verify the 456-hc line period and the spin PC before
  extracting, and report raw x alongside write-relative offsets so the caller can
  re-phase. Two probe hazards: `tia_lum` is inverted (`get3BitLuminance` = 7 −
  the `L*_lowCtrl` bits, so on a dark field the object is raw `== 0`, the
  background `== 7`); and the netlist settles registers to ARBITRARY power-on
  state — `CLEAN_START` clears only RAM, so a probe ROM must write every TIA
  register it depends on (an unwritten NUSIZ settled to double-size in one run
  and silently invalidated it). Prefer running the actual suite test ROMs over
  hand-built minimal repros; a hand repro's jam/park state can differ
  sub-phase-for-sub-phase from the shipped construction.

- **The RIOT has no gate-level oracle.** The 6532 (RAM + I/O + timer) was never
  fully reverse-engineered — visual6502's RIOT netlist is ~⅓ complete, so Sim2600
  *emulates* the PIA behaviourally (`emuPIA.py`) rather than simulating a netlist.
  For RIOT timing (timer prescaler and underflow, port and interrupt edges) VCS is
  in the **CGB regime — no die-derived truth**. Ground it, in order, on the **MOS
  6532 datasheet**, the **console schematics**, and **hardware-confirmed timer
  torture ROMs** (TimerTest, the diagnostic cartridge), with behavioural emulators
  only to localise. **Never treat Sim2600's PIA as ground truth.**

- **Static-analysis layer (the gb-propagation-delay-analysis analog): schematics +
  TIA_HW_Notes.** Chip and console schematics and Andrew Towers'
  `TIA_HW_Notes.txt` explain which signal gates which — and races and timing —
  *without* running a sim. Reach for them first to frame a question, then use
  Sim2600 to confirm the specific edge. Where a schematic reading and a Sim2600
  measurement disagree, the sim wins (it is the decapped die; schematics are
  design drawings, and hand-scan reads are fallible).

- **Documented hardware behaviour.** The **Stella Programmer's Guide** is the
  canonical TIA/RIOT programming reference; the **TIA Technical Manual** and the
  **6532 datasheet** are the chip-level references. Treat these as the VCS
  equivalent of gb-ctr — reliable written hardware documentation, below the
  netlist/sim tier.

- **Behavioural cross-emulator traces (morepork) — corroborate only.** VCS morepork
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
| Local reference library | `receipts/resources/` | Additional core reference material — schematics, chip documentation, test cartridges. Inventory: `receipts/resources/vcs-library.md`. |
| vcs-tests suite | `crates/missingno-vcs/tests/accuracy/` | The in-repo accuracy suite (RESULT RAM convention; NTSC + PAL + SECAM), fully green — the gate for any change. Baseline/diff via `scripts/test-report-vcs.sh`. |
| vcs-tests source repo | `~/Projects/missingno-vcs-tests` | The suite's source: test .asm + Makefile, the blessed references, the real-console captures (`*_pal_capture.png` + 16-bit `_luma`/`_std` sidecars), the calibration rig profile, and `scripts/hwcompare.py`. Rebuild ROMs there and re-import on suite updates; treat it read-only otherwise. |

## Core shape

`missingno-vcs` is a standalone core crate (not a `Console<M>` model): 6507 + TIA +
RIOT + cartridge on one colour-clock master. `src/console.rs` steps each colour
clock as two half-clocks — CPU φ2/bus access on the high half, TIA MOTCK and render
on the low half — with one CPU cycle per three colour clocks on a grid phase-locked
to the line (`PHI0_GRID_PHASE`: cycle boundaries where position ≡ 2 mod 3; RSYNC's
divider requantisation is emergent because the grid derives from the position).
TIA register writes defer through a two-slot pipe with die-measured per-class
commit instants: colour registers are transparent, most registers latch at the φ2
fall, playfield registers reach the serialiser's cell latch one clock later, HMOVE
commits at the strobe's rise and the RESxx resets at its release — ground any new
write-timing number the same way before adding it. `src/tia/` holds the pixel
pipeline: per-object ÷4 divider rings with wrap-grid decode and a one-wrap pending
START latch, the HSync counter spine, and the HMOVE engine (three-stage SEC
two-phase shift; a live stuff absorbs into a firing MOTCK with a one-clock seam
lookahead on the following render, phase-gated per object — the player previews
only at its scan clock's source class, the missile at every class but the ring's
pulse class, the ball at every class; all console-measured, and the missile and
ball genuinely differ despite sharing the width gate). `src/riot.rs` has the
timer/ports. The
frontend drives it through the `app/system/` seam described in
`docs/adding-a-system.md`.
