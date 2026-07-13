//! Nintendo Entertainment System / Famicom hardware model.

pub mod apu;
pub mod cartridge;
pub mod console;
pub mod ppu;

#[cfg(feature = "morepork")]
pub mod trace;
