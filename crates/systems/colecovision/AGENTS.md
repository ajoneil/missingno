# missingno-colecovision — system methodology

System methodology for the ColecoVision. The shared skill-system rules and
workflow discipline live in the repository-root `AGENTS.md`. Like the
SG-1000, this crate is *a board, not silicon*: the Z80 lives in
`missingno-zilog-z80`, the TMS9928A/9929A in `missingno-ti-vdp`, the
SN76489AN in `missingno-ti-psg`. What lives here is the wiring — two '138
decoders, the 8 KB BIOS ahead of the cartridge, a kilobyte of mirrored RAM,
the 74LS74 wait latch on /M1, the VDP interrupt on /NMI, and the two hand
controllers behind one mode latch.

**This doc outranks the chip docs in-system.** A board question is
adjudicated here; a chip's internals belong to that chip's `AGENTS.md`.

## Ground-truth hierarchy

1. **The `.col` conformance corpus run on hardware** — the same
   self-checking ROMs the chip crate runs as `.sg`, built for this board
   (32 KB cartridge with the BIOS header, RESULT block at `$7000`, the VDP
   at `$BE`/`$BF`, the interrupt as NMI, the M1 wait in every ledger) and
   flown on a **PAL CBS ColecoVision (TMS9929A)** across six sittings. Every
   class byte matched the SC-3000's; the 313-line frame measured exactly
   71364 T. The 262-line body has not run on hardware, and no ColecoVision
   colour or scene capture exists yet.
2. **Coleco's own documents** — the OS 7 PRIME listing (the BIOS source),
   the *ColecoVision Programmers' Manual* Rev. 5, and the *ADAM Technical
   Reference Manual* (the ADAM's ColecoVision-compatible board). These
   settle the header, the vector routing, the controller protocol and the
   clock chain's description.
3. **TI and Zilog device documentation** — the TMS9918A-family data manual,
   the SN76489AN manual, the Z80 user manual, for pin-level facts the board
   sheets assume.
4. **Dan Boris's traced schematic** ("Colecovision Main PCB Rev H2-1", seven
   sheets, "traced from actual board, accuracy can not be guaranteed") — the
   only schematic located; no Coleco-drawn schematic has surfaced. It is
   the source for every decode, the wait latch, the reset chain and the
   controller buffers, corroborated where the repair notes name the same
   parts.
5. **MAME** — reference material only, attributed by name.

There is **no gate-level oracle for this board**. Where a question has no
answer above, escalate to the user rather than adopting an emulator's
behaviour.

## Resources

| Source | Tier | Location / URL |
|--------|------|----------------|
| `missingno-ti-vdp-tests` — the corpus, `COLECOVISION.md` its `.col` contract | 1 | https://github.com/ajoneil/missingno-ti-vdp-tests-wip |
| Coleco, *ColecoVision Programmers' Manual* Rev. 5 (1982) | 2 | https://archive.org/details/colecovision-programmers-guide-revision-5 |
| Coleco, *ADAM Technical Reference Manual* (preliminary) | 2 | https://archive.org/details/coleco-adam-technical-reference-manual |
| Coleco, OS 7 PRIME absolute listing (in the *ADAM Technical Manual EOS6 OS7* scan; re-OCR by FozzTexx) | 2 | https://archive.org/details/coleco-adam-technical-manual-eos6-os7 ; https://github.com/FozzTexx/Coleco-Adam-Source |
| TI, *TMS9918A/TMS9928A/TMS9929A Data Manual* (Nov 1982) | 3 | http://www.bitsavers.org/components/ti/TMS9900/TMS9918A_TMS9928A_TMS9929A_Video_Display_Processors_Data_Manual_Nov82.pdf |
| Dan Boris, *Colecovision Main PCB Rev H2-1* schematic V1.0 (2002); *CV-Tech.txt* | 4 | https://atarihq.com/danb/files/colecovision.pdf ; https://atarihq.com/danb/files/CV-Tech.txt |
| Ole Nielsen, *Technical Tips/Repair For ColecoVision* | 4 | http://www.colecovision.dk/technical.htm |
| MAME — `src/mame/coleco/coleco.cpp`, `coleco_m.cpp`, `src/devices/bus/coleco/` | 5 | https://github.com/mamedev/mame |

## The conformance tier

`tests/accuracy/` runs the corpus's `.col` builds through the whole console,
on a 262-line body and a 313-line body. The BIOS is proprietary and never
committed: the tier reads it from `COLECOVISION_BIOS`, and with the variable
unset every corpus test prints a skip line and returns — so the crate's gate
is

```
COLECOVISION_BIOS=<path> cargo test -p missingno-colecovision
```

A ROM's skip sentinel (`$FD` "SG-1000 BUS ONLY" for the phase-anchor family)
is asserted where the corpus states it and never counted as a pass.
Screenshot subjects compare the display area against the SC-3000-endorsed
references the chip crate holds — no ColecoVision capture exists — and stay
staged where the chip crate stages them; `registers/midline-name`, whose
seam is placed from the F edge through this board's longer ledger, lands nine
cells right of the SC-3000's and is staged on both bodies until a
ColecoVision capture places it. `tests/board.rs` pins what the
corpus never exercises: the controller mode latch and both segments, the
PSG's READY on /WAIT, and the reset button leaving the VDP alone.

The `morepork` feature adds the same trace bridge the SG-1000 carries, under
morepork's `coleco` id; with `MOREPORK_PROFILE` set the corpus tier writes one
trace per ROM.

## The timing model

One 7.15909 MHz crystal. Divided by two it is the 3.579545 MHz system clock
the Z80 and the PSG take; a tuned tank on that clock's third harmonic is the
VDP's 10.738635 MHz. So the grid is the SG-1000's: **the VDP advances three
periods and the PSG one CLOCK per Z80 T-state, the VDP ahead of the CPU's
tick**, and the frame is 262 or 313 lines of 228 T. The sub-T phase between
the Z80's strobe and the VDP's clock — what the corpus calls the strobe
residue, which wanders between three alignments on real boards — is modelled
at residue 0, the SC-3000's alignment: the variant set is board tolerance,
not chip behaviour.

**One wait state per M1 cycle.** U8A, one half of a 74LS74 with D tied to
/Q, is held clear while /M1 is high and toggles on the system clock while
it is low: Q rises on the first edge inside the cycle and falls on the next,
pulling /WAIT low through an open-collector inverter exactly across the
T2 sample. Every opcode fetch, prefix fetch, halt refetch and interrupt
acknowledge costs one extra T. The PSG's READY shares the pulled-up line.

**The VDP interrupt is the NMI.** /INT reaches the Z80's /NMI, whose own
edge detector latches one request per falling edge; a frame flag left
uncleared re-delivers nothing. Nothing on the console drives /INT.

## Stated abstractions

- **An undriven bus reads `0xFF`.** The expansion windows `$2000-$5FFF`, a
  cartridge window the image does not hold, and the I/O ranges with no read
  select (`$00-$7F`, `$80-$9F`, `$C0-$DF`). No source states the value; the
  choice is the SG-1000's.
- **The controller buffers' idle bits read 1.** Bits 4, 5 and 7 have no line
  on a standard hand controller and the '541 inputs are pulled up; no
  primary source states their level. **MAME** reads bit 7 low.
- **The keypad is the diode matrix.** Two keys held together read the AND of
  their codes, which is what 28 diodes onto four lines do; the codes are
  OS 7's decode table read backwards.
- **The Super Action Controller, the roller controller and the spinner
  interrupt are unmodelled.** The traced board routes the spinner pulse only
  to the expansion connector; the ADAM manual says it reaches the maskable
  interrupt. Unresolved, and out of this cut.
- **Flat cartridges only.** An image up to 32 KB answers the windows it
  holds; MegaCart banking and the Super Game Module are refused, named.
- **The PAL board is MAME's.** No PAL/CBS schematic or crystal inventory has
  been located. The corpus's measured 71364 T frame on Andrew's unit fixes
  the ratio — three VDP periods per T over 313 lines — and the same clocks
  are assumed; the PAL BIOS image is listed once its SHA-256 is in hand.
- **The reset button resets the CPU, the wait latch and the mode latch, not
  the VDP.** The traced board leaves the VDP's /RESET on the expansion
  connector alone; RAM keeps its contents.
- **Colour indices resolved through the datasheet palette.** The TMS9928A's
  colour-difference output goes to an external encoder; no calibrated
  ColecoVision capture exists, so the SC-3000's table stands.
- **The mode latch powers on in joystick mode.** A cross-coupled pair has
  no documented power-on state; the BIOS's controller initialisation writes
  `$C0` on the logo path, and a test cartridge selects its own segment before
  reading.
- **Deterministic power-on**: zeroed RAM, released controls. Real hardware's
  RAM and VRAM at power-on are unmeasured.

## Out of scope

The ADAM and its memory map; Expansion Module #1 (the VCS adapter) and #2
(the driving controller); the Super Game Module; the Bit90/Dina/Onyx clones
and their BIOSes.
