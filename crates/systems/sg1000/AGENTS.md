# missingno-sg1000 — system methodology

System methodology for the Sega SG-1000. The shared skill-system rules and
workflow discipline live in the repository-root `AGENTS.md`. This crate is a
*board, not silicon*: the chips are finished elsewhere — the Z80 in
`missingno-zilog-z80`, the TMS9918A in `missingno-ti-vdp`, the SN76489AN in
`missingno-ti-psg` — and what lives here is the wiring between them. Decode
(two halves of one 74LS139), a kilobyte of mirrored work RAM, the two
joystick multiplexers, the pause switch on /NMI, and the crystal that ties
the three chips to one grid.

**This doc outranks the chip docs in-system.** When a question is about the
board, adjudicate here; when it is about a chip's internals, that chip's
`AGENTS.md` leads and this one defers.

## Ground-truth hierarchy

1. **Enri's traced schematics** — hand-drawn from a real board, the primary
   source for every wiring fact this crate encodes: the 74LS139 halves and
   what they decode, the TMM2009's brought-out address lines, the joystick
   bit assignments and their pull-ups, VDP INT → Z80 /INT with IM 1, the
   pause switch on /NMI, and the cartridge edge.
2. **The `barbeque/sg1000` ("Soggy-1000") KiCad recreation** — an
   independent re-trace, built and tested against real cartridges. It is the
   citable topology where Enri's scan resolution defeats a junction (the /M1
   transistor on the PSG select), and a corroborating second opinion
   elsewhere. Its own additions (a reset button, a memory mapper in the
   unused `$00-$3F` block) are Soggy's, not the SG-1000's.
3. **TI device documentation** — the TMS9918A data manual and the SN76489AN
   manual, for pin-level facts the board sheets assume.
4. **MAME** — reference material only, attributed by name, never hardware
   fact. Everything sourced only to MAME is flagged as such below.

There is **no gate-level oracle for this board** and no hardware captures of
a running SG-1000 in the tree. Where a board question has no answer in the
tiers above, escalate to the user rather than adopting an emulator's
behaviour as fact.

## Resources

Where each source above lives.

| Source | Tier | Location / URL |
|--------|------|----------------|
| Enri, *Enri's Home PAGE (SG-1000)* — the traced schematic sheets (CPU, VDP, PSG, I/O decode, joystick wiring, connectors, cartridge boards) | 1 | `http://www43.tok2.com/home/cmpslv/Sg1000/EnrSG.htm` — host dead, retrieved through the Internet Archive (Shift-JIS original). Enri's companion SC-3000 and Mark III pages sit under the same host path. |
| `barbeque/sg1000` ("Soggy-1000") KiCad recreation — `Schematic-V0.4.pdf`, `sg1000.kicad_sch` | 2 | https://github.com/barbeque/sg1000 |
| TI, *TMS9918A/TMS9928A/TMS9929A Video Display Processors Data Manual* (Nov 1982) | 3 | http://www.bitsavers.org/components/ti/TMS9900/TMS9918A_TMS9928A_TMS9929A_Video_Display_Processors_Data_Manual_Nov82.pdf |
| TI, *SN76489AN Digital Complex Sound Generator* data manual (undated) | 3 | No publisher URL located; the copy in hand ships in the `docs/` directory of https://github.com/rejunity/tt05-psg-sn76489 |
| MAME — SG-1000 driver, Sega-8 slot and per-cart handlers, software list | 4 | https://github.com/mamedev/mame (`src/mame/sega/sg1000.cpp`, `src/devices/bus/sega8/`, `hash/sg1000.xml`) |

## The conformance tier

The chip crate's hardware-endorsed `.sg` corpus — adjudicated on a Japanese
NTSC SC-3000 — is the only measured oracle this core can reach, and
`tests/board.rs` runs a slice of it through the whole console instead of
through the chip crate's testbench. The real board map satisfies the ROMs'
assumptions (1 KB work RAM, the RESULT block at `$C000`, VDP data `$BE` /
control `$BF`, IM 1 on the VDP interrupt) by construction, so a corpus ROM
that passes in the testbench and fails here is a statement about the board
model. The corpus does not exercise the PSG or the joystick ports at all, so
the crate's own smoke tests pin those two board behaviours instead — the
multiplexer pair and the PSG's READY→/WAIT stall.

The slice is deliberately a smoke selection, not a second run of the corpus:
the two harness mirror ROMs (work RAM through the top window, the VDP ports
across their block), one frame-flag-against-interrupt-line race, and one
Graphics I scene carried through to a non-blank frame.

`cargo test -p missingno-sg1000` is the whole gate for this crate — the board
suite in `tests/board.rs` plus the crate's unit tests. The VDP screenshot
corpus and its references belong to `missingno-ti-vdp` and run there. There is
no test-report script for this core; run the suite directly.

## The timing model

The 10.738635 MHz crystal is the grid, exactly as in the chip crate's
testbench: **the VDP advances three crystal periods and the PSG one CLOCK
per Z80 T-state, with the VDP ahead of the CPU's tick**, so a port access
lands against a VDP that has already reached the instant it fires on. /INT
is sampled from the VDP after each T-state. Nothing here batches per
instruction — the interleave is per-T from day one, and the earlier
first-pass cores' instruction-granular loops are explicitly not precedent.

The PSG's READY sits on the /WAIT net and is answered through the Z80's
`Bus::wait_requested`, so an `OUT` to the PSG stretches the very cycle that
strobed it — and moves the /INT sample point along with it.

## Stated abstractions

- **An undriven bus reads `0xFF`.** No consulted source says what the data
  bus settles to when nothing drives it: `$8000-$BFFF` with no second ROM,
  the whole `$00-$3F` I/O block, and reads from the PSG's block. The value
  is **MAME's** empty-slot behaviour, adopted as a modelling choice.
- **A PSG read does not stall.** `/CS PSG` is qualified by nothing but
  /IORQ, so an `IN` from `$40-$7F` asserts the select on real hardware; the
  SN76489AN has no data outputs, and whether READY falls for a read /CE is
  a **documented silence**. This model leaves reads unstalled and returns an
  undriven bus.
- **The /M1 transistor needs no code.** A PNP transistor forces the PSG
  select inactive whenever /M1 is low — the only Z80 cycle with /M1 and
  /IORQ both low being an interrupt acknowledge. The modelled acknowledge
  never touches port space and READY can only be low inside the stretched
  `OUT` itself, so the protection holds vacuously. If the acknowledge cycle
  ever grows a bus surface, this becomes real wiring.
- **Flat cartridges only.** The cart is a ROM image: a power-of-two image
  inside `/EXM2` repeats through `$0000-$7FFF` (the documented no-decoder
  multi-ROM boards mirror exactly this way), a larger image runs flat into
  `$8000-$BFFF`, and anything past the two windows is rejected. Not
  supported, and named: **Othello** and **The Castle** cart RAM at `$8000`,
  the Taiwanese **Dahjee** RAM expanders, **Terebi Oekaki**'s tablet, the
  SC-3000 Survivors **multicarts**, and the `/DSRAM` route by which a cart
  disables the console's own RAM. A game probing `$8000-$BFFF` for cart RAM
  reads `0xFF` and will misbehave. The selector for a board model is the
  gamedb `cart_type` field: both loaders already carry it — the session
  factory's `LoadOptions` (`missingno-session`) and the app's `MediaLoad`
  (`missingno`'s `app/system/`) — and this core's constructors ignore it. A
  combined 16 KB dump of an 8 K + 8 K board is a known limitation: the halves
  must be dumped per region to mirror correctly.
- **1 KB of work RAM — the SG-1000 proper.** The SG-1000 II and SC-3000
  fit 2 KB, which the documented machine-detection routine distinguishes by
  reading the mirror. The size is fixed here, not a variant axis; modelling
  those machines is what would make it one.
- **`CON` is held released.** `$DD` bit 4 is a real line on the cartridge
  edge and the keyboard connector whose **function no source explains**; it
  reads 1, along with the three unconnected multiplexer inputs above it.
  MAME takes those four bits from its expansion slot instead — tertiary.
- **The picture is the VDP's visible raster.** The console hands out what
  the chip emits — the 256×192 display area inside its live backdrop
  border, 284×243 on NTSC — so the presented picture is 4:3.
- **Colour indices resolved through the datasheet palette.** The VDP stops
  at TI colour indices; the 16-entry RGB table this crate presents them
  through is the canonical datasheet palette, the same one the chip crate's
  screenshot harness stamps. Index 0 — every plane transparent, the
  external-video input — presents as black.
- **The debugger's graphics decode states the pattern, not the raster.** The
  surfaces the panes read are decoded from the live registers and VRAM at the
  instant the decode runs — a mid-frame mode or table change shows as the
  state after it, exactly as the vocabulary says of a sampled register.
  Within that:
  - **Colour is resolved into the pattern atlas.** The colour table fixes a
    pattern's two colours at the pattern's own index (per group of eight in
    Graphics I, per pattern row in Graphics II), so the atlas ships TI colour
    indices rather than pattern bits, and index 0 keeps its transparent
    meaning.
  - **A sprite's colour rides its object entry.** The attribute owns the
    colour, not the pattern, so sprite patterns stay one bit deep and each
    object's palette selector is its colour nibble, over a two-entry palette
    per TI colour.
  - **MAG is not modelled in the panes.** Magnification is a display-time
    doubling; the object table states the pattern size R1's SIZE bit selects
    and the panes draw that.
  - **Multicolor patterns show whole.** The display takes only the byte pair
    a cell's map row selects, each byte painting two 4×4 blocks; the atlas
    shows all eight bytes instead, one two-colour row each, so the map
    composite in that mode is not the screen.
  - **Undocumented mode combinations fall back to the Graphics I layout.**
    Nothing states their table geometry; the sidebar still names the mode.
- **Both joystick sites are modelled as ports.** Player 1's stick is wired
  directly to the board rather than to a connector, but it reads through the
  same multiplexer pair as player 2's, so both appear as control-pad ports.
- **Deterministic power-on.** The chips seed themselves; the board adds
  zeroed work RAM and released joystick lines. Real hardware's RAM contents
  at power-on are unmeasured.

## Out of scope

- **SG-1000 II and SC-3000.** The II is documented only for its 2 KB of RAM;
  its pause control and whether its I/O block is this multiplexer pair or
  the SC-3000's 8255 are **not established** by anything consulted. The
  SC-3000 replaces the joystick block with an 8255 PPI and a keyboard
  matrix, and splits `$C0-$FF` to make room for expansion.
- **The SK-1100 keyboard, the SF-7000 disk unit, and the arcade variant of
  the joystick ports** — the arcade board's `$DC`/`$DD` map is a different
  machine's and must not be applied here.
- **The analog stages.** The PSG's output network and the VDP's composite
  encoding are frontend territory; the console states its board and stops.
