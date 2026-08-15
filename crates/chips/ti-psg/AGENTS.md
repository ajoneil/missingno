# missingno-ti-psg — chip methodology

Chip-specific methodology for the Texas Instruments SN76489-family
programmable tone/noise generator (the discrete SN76489AN as fitted to the
SG-1000, SG-1000 II, SC-3000 and Othello Multivision; the SN76494/SN76496
siblings share the datasheet). The shared skill-system rules and workflow
discipline live in the repository-root `AGENTS.md`. This crate is *silicon
without a board*: it models the chip — the write port and its latched register
address, three tone generators, the noise generator and its shift register,
the four attenuators, and the summing stage — and leaves board wiring (I/O
decode, the READY→/WAIT tie, the output RC network, the sample rate) to its
consumers (SG-1000 first; the Sega-integrated clone serves the Master System
until that core grows its own).

**A system doc outranks this one in-system.** When the chip is inside a
console, that console's ground-truth hierarchy adjudicates.

## Ground-truth hierarchy (the chip in isolation)

There is **no test corpus yet** — the ROM tier is future work, and until it
exists this crate has no conformance oracle of its own. Every reading below is
documentary, and the ones that are contested are listed as such further down.

1. **TI manufacturer documentation** — the *SN76489AN* data sheet/technical
   manual and the *SN76494/SN76494A/SN76496/SN76496A* application manual
   (D2801). Register map, data formats, divider ratios and frequency
   formulae, the attenuator weights, the output-buffer topology, and the full
   READY specification come from here. The two manuals disagree in places
   (the SN76489AN manual's data-format diagrams reverse three field orders
   against its own tables; its shift-rate table has a printing error; its
   maximum attenuation reads 28 dB where the weights sum to 30) — the D2801
   readings are the self-consistent ones and this crate follows them.
2. **Die-level material** — the emu-russia *SEGAChips* netlist reconstructed
   from a decapped Mega Drive VDP, which grounds the **Sega-integrated
   variant's** structure (16 shift-register cells; no READY or /CE pin; a
   reset input). Its cell bodies are stubs, so it grounds structure, not
   behaviour. The `76489A-analysis` annotated die photograph of a decapped
   SN76489A carries no prose, so nothing about the **discrete** part's
   internals is die-grounded — including its shift-register taps.
3. **Hardware measurement** — scarybeast's oscilloscope captures of a BBC
   Micro SN76489 (pin 7, one part, one board); the SMS Power shift-register
   width/tap rows attributed to named people sampling named machines (Charles
   MacDonald: SMS1, Game Gear, Genesis, SC-3000H; John Kortink: BBC Micro;
   Daniel Bienvenu: ColecoVision); Enri's hand-traced SC-3000 and SG-1000
   board sheets. One board each, protocols unpublished — treat as measurement
   of *that* machine.
4. **Community documentation** — the SMS Power SN76489 page's prose, where it
   is not attached to one of those samplings. Its write-port semantics are
   corroborated by the TI manuals; its power-on and period-0 claims are not.
5. **Emulators** — MAME's `sn76496.cpp`, attributed by name, never ground
   truth. Its header cites hardware tests (PlgDavid, ValleyBell, Nemesis,
   "Verified on SMS hardware") whose captures are unpublished, so what is on
   record is a maintainer's summary.

Resources live in `receipts/resources/ti-psg/`; the consolidated behaviour
research, with every claim tiered and every conflict recorded, is
`receipts/research/sn76489-behaviour-checklist.md`, and the observability
survey is `receipts/research/sn76489-test-observability.md`.

## The variant seam

`Variant` is the whole of the discrete-vs-Sega split, consulted at the single
point each difference manifests, so a refuted constant is a constant change:

| Difference | `DiscreteTi` | `SegaIntegrated` |
|---|---|---|
| Shift register | 15 bits, white taps 0 and 1 | 16 bits, white taps 0 and 3 |
| Zero period register | counts 0x400 | holds the channel |
| Power-on | attenuations 0, tone 1's frequency addressed | attenuations $F, tone 2's attenuation addressed |
| READY | open-collector pin, low while a byte loads | no such pin |

The clock-divider seam (the ÷2 SN76494 parts, whose byte load is 4 clocks
rather than 32) is documented but unmodelled — this crate is the ÷16 part.
The Game Gear's stereo mask is a later addition to the integrated variant and
is likewise unmodelled.

## Stated abstractions

Each of these is a documentary reading standing in for a measurement nobody
has published. All are **pending hardware**.

- **Zero period register: 0x400 on the TI part, held on the Sega part.**
  The split is MAME's, whose Sega half is annotated "verified on SMS
  hardware" and whose TI half is not verified anywhere. SMS Power states the
  opposite for the discrete part (constant output for register value 0 *or*
  1), without a variant qualification. Both TI manuals are silent. This is
  the highest-stakes reading in the crate for the SG-1000 target: the two
  candidates differ audibly (a 109 Hz tone versus DC). A held channel keeps
  its flip-flop rather than being forced high, so from power-on it presents
  the constant high level SMS Power describes.
- **Deterministic power-on.** MAME's values (TI: registers zero, attenuation
  0, first register addressed, hence its reported cold-boot beep; Sega:
  attenuations $F and register 3 addressed, attributed to hardware tests by
  Nemesis and ValleyBell). SMS Power says discrete parts start *random*, and
  reports an SC-3000 sounding a tone before software writes to the chip — the
  two agree that the discrete part makes noise at power-on and disagree on
  whether the state is defined. No measured power-on register dump exists for
  either part.
- **Half-period = N internal clocks.** The counter borrows at zero, reloading
  and toggling; TI's "the period is twice the value of the period register"
  is the only statement, and no source says whether the borrow tick is
  consumed (N versus N+1). The difference is one internal clock per half
  period — audible only at very small N.
- **READY low for exactly 32 input clocks.** The SN76489AN manual says
  *approximately* 32, and the D2801 figure numbers them on the CLOCK
  waveform. Nobody has measured the actual release instant, and the model
  restarts the count on a second write while busy rather than modelling the
  datasheet's board-level repeat-write hazard (which is about how long /WE is
  held, not about the die's response to a byte).
- **Registers latch at write acceptance.** The write's effect is immediate in
  this model, while the real part is still loading for the ~32 clocks READY
  is low. Where inside that window the register actually changes is
  unmeasured.
- **The shift register clears on any noise-register write.** Both TI manuals
  say "whenever the noise control register is changed, the shift register is
  cleared", and MAME clears on any write to that register for non-NCR parts.
  Whether a write of the *same* value clears is untested. The noise counter
  is documented nowhere as resetting with it, so it keeps counting.
- **Non-inverted output sense.** MAME distinguishes the SN76489 from the
  SN76489A by output stage and polarity; SMS Power does not distinguish them
  and groups the SG-1000 part with the BBC Micro and ColecoVision;
  scarybeast measured his BBC Micro part as inverted (a silent channel
  sitting at ~0.8 V, a loud one alternating 0 V and ~0.8 V). The SG-1000
  part is the A suffix and no consulted source measures an SN76489AN
  distinguished from a non-A SN76489. This crate emits the non-inverted
  sense.
- **Linear mix, normalised to all four channels wide open.** The D2801
  output-buffer description is an operational-amplifier summing circuit fed
  by a current-mode DAC, so the four channels add — deliberately unlike the
  VCS TIA, whose saturating summing node is a *board* fact about tied pads.
  The 2 dB per step ladder is the datasheet's nominal (±1 dB per weight in
  its own electrical characteristics); **no measured per-step DAC curve
  exists for either variant**, and how the Sega-integrated part's four
  transistor-pile DACs sum on their common node is unresolved even at die
  level. The normalisation point is a convention, not a measurement.
- **`dac_codes()` reports amplitude, not the attenuation register.** Per
  channel it is the attenuation complemented (`$F` mute reads 0, no
  attenuation reads 15) while the generator conducts, and 0 while it does
  not — the code the current-mode DAC is handed, sharing `level()`'s
  conduction predicate so the two cannot disagree. Turning a code into a
  voltage is the 2 dB per step ladder's job, and stays with the DAC and the
  frontend.
- **Shift-register cycle lengths follow from the taps.** With the tap
  constants above, white noise repeats after 32767 shifts on `DiscreteTi`
  (the 15-bit register's maximal length) and **57337** on `SegaIntegrated` —
  the 16-bit tap pair is not a maximal-length choice, so no seed reaches
  65535. "Periodic" noise recirculates the register, giving the documented
  1/15 and 1/16 duty cycles. The cycle lengths are arithmetic consequences of
  the taps; the taps themselves are community-attributed sampling (Sega side
  corroborated at die level only for the register's *width*).

## Not modelled

- **The analog stage.** `level()` stops at a normalised linear sum. Output
  polarity and offset (the measured ~3 V centre on one BBC Micro), the
  10 Ω/0.1 µF decoupling the datasheet requires, and any board network
  (the SC-3000's 2.2 kΩ/1000 pF/6.2 kΩ/10 µF path to its audio connector) are
  frontend and console territory.
- **Pin 9 audio input** (SN76494/96 only; N.C. on the SN76489AN).
- **The /CE and /WE pins.** The crate takes accepted bytes, not pin edges;
  READY is exposed as a predicate so a board can stall its CPU. The
  datasheet's unclocked function table (READY following /CE) has no analogue
  here.
- **Test and debug lines** on the Sega-integrated part (the VDP debug
  register's channel-replacement mode).
