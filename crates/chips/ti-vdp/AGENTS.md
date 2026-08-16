# missingno-ti-vdp — chip methodology

Chip-specific methodology for the Texas Instruments TMS9918A-family Video
Display Processor (TMS9918A NTSC / TMS9929A PAL; the TMS9928A shares the
digital core). The shared skill-system rules and workflow discipline live in
the repository-root `AGENTS.md`. This crate is *silicon without a board*: it
models the VDP — registers, VRAM interface, status flags, sprite machinery,
the four documented modes and the undocumented combinations, the interrupt
line — and leaves board wiring (I/O decode, the CPU, RAM) to its consumers
(SG-1000 first; reusable for ColecoVision and MSX1). Sega's later 315-5124
(the Master System VDP, with TMS legacy modes) is a different chip and stays
SMS-owned.

**A system doc outranks this one in-system.** When the chip is inside a
console, that console's ground-truth hierarchy adjudicates.

## Ground-truth hierarchy (the chip in isolation)

1. **The hardware-endorsed test corpus** — the crate's conformance oracle.
   Self-checking pass/fail ROMs from `missingno-ti-vdp-tests`
   (`ajoneil/missingno-ti-vdp-tests-wip`; this crate commits the built `.sg`
   binaries), adjudicated on a **Japanese NTSC SC-3000 (TMS9918A)**. Where a
   test header carries a hardware-endorsement citation, its assertion is
   measured silicon truth — including several findings no document or
   emulator agrees on. The suite in `tests/accuracy/` runs these under the
   crate's own testbench; staged invocations carry their blocker in the
   test's reason string. TMS9929A PAL behaviour stays
   documentary-provisional until PAL hardware or trustworthy measurements
   appear.
2. **Named documentation** — the TI *TMS9918A/9928A/9929A Data Manual* and
   *Video Display Processors Programmer's Guide* for documented behaviour;
   the community corpus (Sean Young's TMS9918A documentation, Nouspikel,
   Urbanus) cross-checked against them. Where silicon refuted a document,
   the test corpus records it — the corpus wins.
3. **Other emulators** — reference material only, attributed by name
   (openMSX, MAME, ares, Gearsystem), never ground truth. Every one of them
   is refuted by at least one hardware-endorsed test in the corpus.

## The testbench (not a system)

`tests/accuracy/` wires a Z80 (`missingno-zilog-z80`), 1 KB RAM, a 32 KB
cartridge image and this VDP into the SG-1000's common-subset envelope —
VDP data port `$BE` / control `$BF`, RAM window `$C000-$C3FF`, IM 1 with the
VDP interrupt on `/INT`. It is a dev-dependency fixture, not a console: the
`systems/sg1000` crate owns real board behaviour when it exists. Each ROM
latches its verdict to the RESULT block at `$C000` (`$A5` PASS / `$5A`
FAIL, then CODE/OBSERVED/EXPECTED) before rendering anything, so the
harness asserts on the block only.

## Stated abstractions

- **Per-T CPU↔VDP interleaving.** The testbench advances the VDP one Z80
  T-state at a time, ahead of each CPU tick, so a port access lands at its
  true T offset within the instruction.
- **CPU-access schedule.** The rendering-line access schedule and its
  service rule are pinned by the SC-3000's two canonical burst maps; the
  code's lattice constants state them, in the model's natural gauge. The
  schedule's rotation against hsync and its sub-cycle instants are free
  conventions adopted within the maps' measured freedom; only Graphics I
  with display on is map-constrained. Non-rendering time is modelled as
  every cycle claimable, except the last frame lines, where the measured
  turn-on seam sits earlier than the modelled line-boundary wake.
- **Sprite pre-processing: live counter, boundary-latched effects.** Status
  bits 0-4 present the scanner's progress live, and the fifth-sprite
  effects — the halt at the match's own entry, the hold on the presented
  field and its release, 5S's boundary-latched set instant — are
  corpus-pinned; the code's scan lattice states them, its base offset and
  sub-cycle instants adopted within the maps' measured freedom. One stated
  divergence: the corpus measured C live at the generating pixel, while
  the model latches it at the line boundary — awaiting an asserting test.
- **Sub-line rendering: the incremental raster pipeline.** Each character
  cell latches its tables from the live registers and VRAM at the cell's
  instant, and each dot resolves transparency against the live backdrop —
  mid-line writes land at their silicon-measured granularities (R7 per
  pixel; table bases and mode bits per cell; sprites and M1's schedule
  coupling line-latched). The raster placement is a calibrated convention
  pinned to a silicon-measured band, like the schedule rotation. Open: the
  midframe-m2 mode seam lands two cells late, and midframe-m1's disturbed
  transition row is unattributed on silicon. The text-mode side borders
  follow the Data Manual's asymmetric split, silicon-confirmed against the
  community's symmetric reading. Graphics II pattern fetches follow R3's
  AND mask with R4 contributing only the half select — silicon-adjudicated
  against the emulator consensus, so consensus cannot bless scenes that
  exercise it.
- **The emitted frame is the whole visible raster**, border painted from
  the live backdrop — the only plane that reaches it, so a mid-line R7
  write splits the border where the raster stands. F's instant is a
  separate hardware fact and stays at the end of the display area. **No
  capture has adjudicated a border pixel** — the screenshot harness crops
  to the display area, so the border geometry outside the crop is the Data
  Manual's, and the 9929A's border split is **derived, not documented**.
- **The shrunken-table sprite anomaly is unmodelled.** In TI's shrunken
  Graphics II configuration silicon degrades late sprites once more than
  eight are on screen; the corpus's captures pin the shape (damage attached
  to SAT index, not screen position, with duplicate images no emulator
  renders) but the mechanism is unattributed. The affected captures are
  hardware-PRIMARY, so no consensus reference can cover them and their
  subjects stay staged. The model renders the documented sprite path and
  states this gap rather than guessing a mechanism.
- **Digital core only.** Colour output stops at the TI colour indices; a
  frame pixel of 0 means every plane was transparent (the external-video
  pass-through) and presents as black. Composite encoding, analog levels,
  and the 9929A's Y/R-Y/B-Y outputs are presentation/frontend territory —
  the canonical datasheet RGB palette lives in the test harness, not the
  chip.
- **Inspection reads the renderer's own fetch.** `vram_cell` (a logical
  pointer value through the 4K/16K permutation), `vram` (the DRAM in
  physical order) and the table bases R2-R6 select are public and
  side-effect free: nothing latches, no flag clears, no port request is
  raised. A consumer decoding VRAM sees exactly what the raster sees,
  because it calls the same fetch the raster does.
