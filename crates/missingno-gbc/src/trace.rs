//! gbtrace integration for the CGB.
//!
//! The tracer itself lives in `missingno-gb` and is generic over the
//! [`Traceable`](missingno_gb::trace::Traceable) trait. The CGB reuses
//! the shared `Cpu`/`Ppu`/`Audio`/`Cartridge` types, so capturing its
//! state is just a matter of forwarding the accessors — no duplicated
//! capture logic. `GameBoyColor` then traces through the same
//! `missingno_gb::trace::Tracer` as the DMG.

use missingno_gb::audio::Audio;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::cpu::Cpu;
use missingno_gb::ppu::Ppu;
use missingno_gb::trace::Traceable;

use crate::GameBoyColor;

impl Traceable for GameBoyColor {
    fn cpu(&self) -> &Cpu {
        GameBoyColor::cpu(self)
    }
    fn ppu(&self) -> &Ppu {
        GameBoyColor::ppu(self)
    }
    fn audio(&self) -> &Audio {
        GameBoyColor::audio(self)
    }
    fn peek(&self, address: u16) -> u8 {
        GameBoyColor::peek(self, address)
    }
    fn cartridge(&self) -> &Cartridge {
        GameBoyColor::cartridge(self)
    }
}
