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
  measured, cause open); the burst presents cleanly. While 5S is set the
  latched index masks the live counter at every intra-line phase —
  silicon-corroborated (cadence-8match sweeps its read mid-active across
  all 228 phases), not an adopted convention. The scan's match effects —
  5S, C, the stop latch — still apply at the line boundary: an
  abstraction the race tests currently endorse, to be adjudicated by the
  corpus's queued set-instant probes (coinciding low/high 5S bands are
  this abstraction's signature; separated bands refute it). The lattice's
  base offset inside the run period and its sub-cycle instants are
  adopted within the maps' measured freedom, like the schedule rotation.
- **Line-granular rendering.** The frame renderer composites each display
  line at its line boundary from the registers and VRAM as they then stand —
  the pattern plane by mode, then the line's displayed sprites front to
  back. Mid-line and mid-frame register seams are therefore quantised to
  the line; every corpus subject that would pin them finer (the midline-*
  and midframe-* scenes) is still awaiting its hardware capture, and the
  renderer refines when those land. The text-mode side borders are the
  Data Manual's asymmetric 6/10 split, measured on silicon (the 2026-08-13
  text capture: 5.84/9.93/240.22; the community's symmetric 8/8 refuted by
  ~2 px per side). Graphics II pattern fetches follow R3's AND
  mask with R4 contributing only the half select — the silicon-adjudicated
  reading (gii-mask-pattern/gii-mask-colour), which mame and gearsystem
  get wrong (they collapse the thirds under a masking R4), so consensus
  cannot bless scenes that exercise it.
- **The shrunken-table sprite anomaly is unmodelled.** In TI's shrunken
  Graphics II configuration silicon duplicates and displaces sprites once
  more than eight are on screen (TI documents the symptom; the 2026-08-14
  SC-3000 capture of gii-shrunken observed it — sprites absent from design
  positions, apparitions at fixed columns). No emulator models it and its
  mechanism is unattributed pending the corpus's dedicated probe scene; the
  affected cells of that scene are hardware-PRIMARY and no consensus
  reference can cover them. The model renders the documented sprite path
  and states this gap rather than guessing a mechanism.
- **Digital core only.** Colour output stops at the TI colour indices; a
  frame pixel of 0 means every plane was transparent (the external-video
  pass-through) and presents as black. Composite encoding, analog levels,
  and the 9929A's Y/R-Y/B-Y outputs are presentation/frontend territory —
  the canonical datasheet RGB palette lives in the test harness, not the
  chip.
