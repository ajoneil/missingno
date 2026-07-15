//! What the console's board does to the TIA's pads.
//!
//! The core drives its pads and stops there. These values state what the board
//! wires to them, for whatever assembles the console into a working machine —
//! nothing here is applied to the core's own output.

use missingno_hw::RcHighPass;

/// The audio pads' coupling into the RF modulator's amplifier: both channels'
/// shared summing node feeds a 0.1 µF series capacitor into 18K. It blocks the
/// node's resting level and sets the low end of the console's audio.
pub const AUDIO_COUPLING: RcHighPass = RcHighPass {
    resistance_ohms: 18_000.0,
    capacitance_farads: 100e-9,
};
