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
   Self-checking pass/fail ROMs from `missingno-ti-vdp-tests` (sources live
   in that repo; this crate commits the built `.sg` binaries), adjudicated
   on a **Japanese NTSC SC-3000 (TMS9918A)**. Where a test
   header carries a hardware-endorsement citation, its assertion is measured
   silicon truth — including several findings no document or emulator agrees
   on. The suite in `tests/accuracy/` runs these under the crate's own
   testbench. TMS9929A PAL behaviour stays documentary-provisional until PAL
   hardware or trustworthy measurements appear.
2. **Named documentation** — the TI *TMS9918A/9928A/9929A Data Manual* and
   *Video Display Processors Programmer's Guide* for documented behaviour;
   the community corpus (Sean Young's TMS9918A documentation, Nouspikel,
   Urbanus) cross-checked against them. Resources in
   `receipts/resources/ti-vdp/`; the consolidated behaviour research is
   `receipts/research/tms9918-behaviour-checklist.md` and the tiered
   coverage list `receipts/sg1000/coverage-checklist.md`
   ([TI]/[COMM]/[GAP]/[CONFLICT]). Where silicon refuted a document, the
   test corpus records it — the corpus wins.
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
- **CPU-access schedule.** The rendering-line schedule (eleven runs,
  starts spaced 16,16,16,16,15,16,15,13,16,16,16 memory cycles, lengths
  1,1,1,1,1,2,5,4,1,1,1) and the service rule (claim at the first access
  cycle after the request; transfer locked +17 XTAL, flag released +15
  XTAL from that cycle's start) are pinned exactly by the SC-3000's two
  canonical maps. The lengths' apparent +t freedom is a reparametrisation,
  not a measurement gap (corpus desk answer 2026-08-14: lengths L+t with
  the reference instants shifted 4t XTAL is the same observable model,
  byte-identical on every burst map, and no CPU-side instrument can reach
  the gap that would separate them) — this is the model in its natural
  gauge. The schedule's rotation against
  hsync is unmeasured (free convention), only Graphics I with display on
  is map-constrained, and non-rendering time is modelled as every cycle
  claimable — except the last three frame lines, where the schedule is
  already running (the measured turn-on seam sits ~2.6 lines before
  display line 0; the model wakes at the line boundary, the sub-line
  position being free with the rotation convention).
- **Sprite pre-processing: live counter, boundary-latched effects.** Status
  bits 0-4 present the scanner's progress live — counter reset with entry 0
  at the length-4 run, entries 1-7 one per memory cycle behind it, entries
  8-31 three per 16-cycle run period across the eight regular length-1
  runs, held at the stop through the long runs and the border. Steady steps
  carry the measured boundary-window texture (bits 4/3 blank, low bits old
  → all-ones → new; the 7-to-8 carry's inverted cell reproduced as
  measured, cause open); the burst presents cleanly. The scan halts at
  the fifth match's own entry — measured directly (the entry-15 scene
  shows no field value above 15 in 456 cells) — as it does at a
  terminator's. The fifth-match event arms a hold on the presented field
  that neither the 5S-clearing read (the freeze does not belong to the
  flag: the field holds through the clear at all 228 phases) nor the
  next scan's reset releases; it drops at the release cycle — between
  the counter's 13th and 14th steps, pinned by the two live-ruler
  scenes' plateau lengths, the sub-cycle instant free — of the first
  scan with no event. A terminator halt arms nothing (its plateau ends
  at the reset). While 5S is set the flag-held capture presents above
  the hold (the first capture wins until a read). 5S's set instant is
  boundary-latched — corroborated on three fifth-match entry positions
  (4, 15, 31) whose visibility bands coincide at one corrected phase —
  but C's is not: the corpus measured C live at the generating pixel
  (c-instant/c-instant-x, recorders), so the model's line-boundary C is
  a stated divergence awaiting an asserting test. The lattice's base
  offset inside the run period and its sub-cycle instants are adopted
  within the maps' measured freedom, like the schedule rotation.
- **Sub-line rendering: the incremental raster pipeline.** The renderer
  follows the raster: each character cell latches its name, pattern and
  colours from the live registers and VRAM at the cell's instant, and
  each dot resolves transparency against the live backdrop — mid-line
  writes land at their silicon-measured granularities (R7 per pixel, the
  table bases and mode bits per cell; sprites and M1's schedule coupling
  stay line-latched). The raster placement — picture row N emits during
  counter line N−1, pixel 0 at a fixed XTAL offset — is a calibrated
  convention like the schedule rotation: midline-name's silicon seam
  (row 98, column 16 at that ROM's write phase) pins it to a 16-XTAL
  band, and the interrupt-anchored write instants ride the Z80's
  documented acceptance timing. Open: midframe-m2's seam lands two cells
  late (whether mode bits latch at the fetch stage, ahead of emission,
  needs an anchored M2 sweep); midframe-m1's disturbed transition row is
  unattributed on silicon. The text-mode side borders are the
  Data Manual's asymmetric 6/10 split, measured on silicon (the 2026-08-13
  text capture: 5.84/9.93/240.22; the community's symmetric 8/8 refuted by
  ~2 px per side). Graphics II pattern fetches follow R3's AND
  mask with R4 contributing only the half select — the silicon-adjudicated
  reading (gii-mask-pattern/gii-mask-colour), which mame and gearsystem
  get wrong (they collapse the thirds under a masking R4), so consensus
  cannot bless scenes that exercise it.
  The emitted frame is the whole visible raster rather than the display
  area: 13 dots of border left and 15 right, 27 lines above and 24 below
  the 256×192 window (NTSC, Data Manual Table 3-3), painted from the live
  backdrop — the only plane that reaches the border, so a mid-line R7 write
  splits it where the raster stands and no fetch belongs to a border dot.
  The frame counter increments as the raster leaves the bottom border; F's
  instant is a separate hardware fact and stays at the end of display line
  191. **No capture has adjudicated a border pixel**: the screenshot
  harness crops to the display area, and the blessed references and the
  dumps for adjudication are both stated in that crop. The 9929A's split is
  **derived, not documented** — no TI document breaks down its 313 lines,
  so holding NTSC's 19 non-visible lines leaves 102 border lines, halved
  51/51 for want of a measurement. Fitting the whole visible span inside
  one counter line needs the pixel-0 offset at 26 XTAL or more; the
  calibrated 32 leaves 6 XTAL of slack, and a recalibration to the bottom
  of the measured [24, 40) band would have to take the left border from the
  previous line's tail instead.
- **The shrunken-table sprite anomaly is unmodelled.** In TI's shrunken
  Graphics II configuration silicon degrades late sprites once more than
  eight are on screen. The corpus's ladder and reversal captures pin the
  shape: onset at the TENTH sprite; the damage is attached to SAT index 9
  (an eleventh renders full while 9 stays reduced) and graded across three
  consecutive indices — index 9 exactly one pale steady line, 10 all lines
  at reduced flickering duty, 11 nothing; the gradient follows the SAT
  walk, not screen position (a reversed table renders clean at the same
  screen positions); and reversal exposes duplicate images of the last
  four entries at +64 and +128 lines — the Graphics II third boundaries —
  that no emulator renders. The mechanism is unattributed (the corpus's
  full-table and compact controls are queued); the affected captures are
  hardware-PRIMARY and no consensus reference can cover them
  (sprites/shrunken-dup stays a staged bless-never subject). The model
  renders the documented sprite path and states this gap rather than
  guessing a mechanism.
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
