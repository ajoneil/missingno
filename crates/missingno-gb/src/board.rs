//! What the console's board does to the chip's pads.
//!
//! The console emits SO1/SO2 unfiltered; the board's coupling sits between the
//! pad and the jack. These values state what is wired there, for whatever
//! assembles the console into a working machine — nothing here is applied to
//! the core's own output.

use missingno_hw::HighPass;

/// Charge factor of the output coupling caps per T-cycle. Fitted against
/// hardware rather than read off the board, so the capacitor and its load are
/// not known separately — only the decay they produce together.
const COUPLING_DECAY_PER_TCYCLE: f32 = 0.999958;
const T_CYCLES_PER_SECOND: f32 = 4_194_304.0;

/// The coupling between the audio pads and the jack.
pub fn audio_coupling() -> HighPass {
    HighPass::from_decay_per_cycle(COUPLING_DECAY_PER_TCYCLE, T_CYCLES_PER_SECOND)
}
