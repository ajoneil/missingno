//! Shared console-core foundation: board/TV vocabulary, analog filters, and
//! debug sidecar formats.
//!
//! A core models its chips up to their pads. What surrounds them — the board's
//! coupling network, the television, the speaker — is not the console, and is
//! shared across consoles that were never related: NTSC is NTSC whether the
//! signal came from a TIA or a VDP, and a resistor-capacitor coupling is the
//! same circuit on every board that has one. Cores depend on this crate to
//! *state* what their hardware is; applying any of it belongs to whatever
//! assembles a console into a working machine.
//!
//! Symbol tables and code/data logs are conventions of the tools around an
//! emulator rather than of any one machine: the `.sym` grammar and the Mesen
//! CDL flag bits mean the same thing whichever CPU produced them. What differs
//! per console — how a CPU address maps onto a ROM offset, what state a
//! watchpoint can name — stays with that console.

pub mod analog;
pub mod cdl;
pub mod symbols;
pub mod tv;

pub use analog::{HighPass, OnePoleHighPass, RcHighPass};
pub use tv::TvStandard;
