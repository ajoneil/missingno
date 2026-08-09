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

- **Instruction-granular CPU↔VDP interleaving.** The testbench advances the
  VDP between whole CPU instructions (the Z80 crate exposes no
  sub-instruction bus timing yet). The corpus's `timing/` tier asserts
  hardware-measured sub-instruction contention and race behaviour that an
  instruction-granular executor cannot represent — those tests are staged
  (`#[ignore]`) with their blocking granularity named, not tuned around.
  Un-staging them is CPU-membrane work, not VDP work.
- **Digital core only.** Colour output stops at the TI colour indices;
  composite encoding, analog levels, and the 9929A's Y/R-Y/B-Y outputs are
  presentation/frontend territory.
