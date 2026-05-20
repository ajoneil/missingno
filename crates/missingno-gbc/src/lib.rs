//! Game Boy Color emulation.
//!
//! This crate models the CGB as a distinct system that reuses the
//! shared SM83-based hardware modules from `missingno-gb`. CGB-specific
//! behaviour (palette memory, VRAM/WRAM banking, double-speed, HDMA,
//! object priority) lives here.

use missingno_gb::{GameBoy, cartridge::Cartridge, cpu::Cpu, execute::StepResult};

/// A Game Boy Color console.
///
/// Initial scaffold: composes a `GameBoy` for execution while CGB-only
/// state and PPU divergence are still empty. As CGB features land,
/// fields move out of the inner `GameBoy` into `GameBoyColor` directly.
pub struct GameBoyColor {
    gb: GameBoy,
}

impl GameBoyColor {
    pub fn new(cartridge: Cartridge, boot_rom: Option<Box<[u8; 256]>>) -> Self {
        Self {
            gb: GameBoy::new(cartridge, boot_rom),
        }
    }

    pub fn step(&mut self) -> StepResult {
        self.gb.step()
    }

    pub fn cpu(&self) -> &Cpu {
        self.gb.cpu()
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        self.gb.cpu_mut()
    }

    pub fn read(&self, address: u16) -> u8 {
        self.gb.read(address)
    }

    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        self.gb.drain_serial_output()
    }

    pub fn screen(&self) -> &missingno_gb::ppu::screen::Screen {
        self.gb.screen()
    }

    pub fn interrupts(&self) -> &missingno_gb::interrupts::Registers {
        self.gb.interrupts()
    }

    /// Access the underlying `GameBoy`. Will go away once CGB state
    /// lives directly on `GameBoyColor`.
    pub fn inner(&self) -> &GameBoy {
        &self.gb
    }

    /// Mutable access to the underlying `GameBoy`. Will go away once
    /// CGB state lives directly on `GameBoyColor`.
    pub fn inner_mut(&mut self) -> &mut GameBoy {
        &mut self.gb
    }
}

#[cfg(feature = "test-support")]
impl missingno_gb::test_support::System for GameBoyColor {
    fn step(&mut self) -> StepResult {
        GameBoyColor::step(self)
    }
    fn read(&self, address: u16) -> u8 {
        GameBoyColor::read(self, address)
    }
    fn cpu(&self) -> &Cpu {
        GameBoyColor::cpu(self)
    }
    fn cpu_mut(&mut self) -> &mut Cpu {
        GameBoyColor::cpu_mut(self)
    }
    fn screen(&self) -> &missingno_gb::ppu::screen::Screen {
        GameBoyColor::screen(self)
    }
    fn drain_serial_output(&mut self) -> Vec<u8> {
        GameBoyColor::drain_serial_output(self)
    }
    fn interrupts(&self) -> &missingno_gb::interrupts::Registers {
        GameBoyColor::interrupts(self)
    }
}
