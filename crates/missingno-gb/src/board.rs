//! What the console's board does to the chip's pads.
//!
//! The console emits SO1/SO2 unfiltered; the board's coupling sits between the
//! pad and the jack. These values state what is wired there, for whatever
//! assembles the console into a working machine — nothing here is applied to
//! the core's own output.

use missingno_core::HighPass;

/// Charge factor of the output coupling per T-cycle. The board couples each pad
/// through 1 µF into 510 Ω and the volume pot, which leaves the corner to the
/// amplifier's input impedance — undrawn, and the term that dominates it. So the
/// coupling is stated as the decay it produces rather than as components, which
/// would imply a corner the board does not fix. The drawn parts do bound it:
/// the 510 Ω alone holds the corner under 312 Hz however the amplifier loads it.
const COUPLING_DECAY_PER_TCYCLE: f32 = 0.999958;
const T_CYCLES_PER_SECOND: f32 = 4_194_304.0;

/// The coupling between the audio pads and the jack. One value for the family:
/// the pads' coupling network is drawn identically on every board that has a
/// schematic, and no board reaches a corner that would justify splitting it.
pub fn audio_coupling() -> HighPass {
    HighPass::from_decay_per_cycle(COUPLING_DECAY_PER_TCYCLE, T_CYCLES_PER_SECOND)
}
