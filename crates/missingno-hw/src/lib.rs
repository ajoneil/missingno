//! The vocabulary a console uses to describe itself and its board.
//!
//! A core models its chips up to their pads. What surrounds them — the board's
//! coupling network, the television, the speaker — is not the console, and is
//! shared across consoles that were never related: NTSC is NTSC whether the
//! signal came from a TIA or a VDP, and a resistor-capacitor coupling is the
//! same circuit on every board that has one.
//!
//! Cores depend on this crate to *state* what their hardware is. Applying any
//! of it belongs to whatever assembles a console into a working machine.

pub mod analog;
pub mod tv;

pub use analog::{HighPass, OnePoleHighPass, RcHighPass};
pub use tv::TvStandard;
