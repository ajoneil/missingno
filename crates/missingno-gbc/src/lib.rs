//! Game Boy Color emulation.
//!
//! The CGB reuses the shared SM83-based hardware modules from
//! `missingno-gb` through the generic [`Console`](missingno_gb::Console)
//! core; this crate supplies only the CGB-specific [`Model`] seams.
//! CGB behaviour (color palette memory, VRAM/WRAM banking, double-speed,
//! HDMA, object priority) attaches there.
//!
//! No SGB co-processor and no DMG OAM corruption bug — both are
//! DMG-only hardware quirks.
//!
//! ## Target SoC revision
//!
//! The CGB went through several CPU-SoC revisions (CPU-CGB-A through
//! CPU-CGB-E). Behaviour differs subtly between them — STOP/double-speed
//! wakeup timing, PPU mode-boundary alignment, STAT IRQ edges, APU
//! envelope retrigger, and so on. This crate targets **CPU-CGB-C**:
//! the most commonly-targeted revision across emulators (Gambatte's
//! `cgb04c`), the best-documented in test ROMs, and behaviourally
//! representative of the mainstream CGB hardware run.
//!
//! Test suites filter their ROM selection accordingly — CGB-E-only or
//! CGB-B-only ROMs are excluded from the CGB-C-passing set.

pub mod screen;

use missingno_gb::{Console, Model, PixelOutput, StopAction, cartridge::Cartridge, cpu::Cpu};

use crate::screen::{Color555, GREYSCALE, Screen};

/// The Game Boy Color [`Model`]. Remaining CGB features (VBK, CRAM, HDMA) and
/// the color pixel pipeline attach here as they land.
pub struct Cgb {
    /// 8 × 4 KiB work-RAM banks. C000-CFFF is fixed bank 0; D000-DFFF is the
    /// SVBK-selected bank.
    wram: Box<[u8; 0x8000]>,
    /// SVBK ($FF70) bits 0-2 as written; the effective D000 bank is `max(svbk, 1)`.
    svbk: u8,
    /// KEY1 ($FF4D) bit 0 — speed-switch arm.
    key1_armed: bool,
    /// KEY1 ($FF4D) bit 7 — current speed (false = normal, true = double).
    /// The switch toggles it; the 2× clock cadence itself lands later.
    double_speed: bool,
    /// OPRI ($FF6C) bit 0 — object priority mode (0 = by OAM index). The
    /// priority effect lands with the color PPU.
    opri: bool,
}

impl Default for Cgb {
    fn default() -> Self {
        Self {
            wram: Box::new([0; 0x8000]),
            svbk: 1,
            key1_armed: false,
            double_speed: false,
            opri: false,
        }
    }
}

impl Cgb {
    /// Index into `wram` for a work-RAM or echo-RAM address, else `None`.
    fn wram_index(&self, address: u16) -> Option<usize> {
        let bank = if self.svbk == 0 { 1 } else { self.svbk } as usize;
        let banked = |within: u16| bank * 0x1000 + within as usize;
        match address {
            0xC000..=0xCFFF => Some((address - 0xC000) as usize),
            0xD000..=0xDFFF => Some(banked(address - 0xD000)),
            0xE000..=0xEFFF => Some((address - 0xE000) as usize),
            0xF000..=0xFDFF => Some(banked(address - 0xF000)),
            _ => None,
        }
    }
}

impl Model for Cgb {
    type Screen = Screen;

    fn map_pixel(pixel: PixelOutput) -> Color555 {
        GREYSCALE[(pixel.shade & 0x3) as usize]
    }

    fn cpu_post_boot(_checksum: u8) -> Cpu {
        Cpu::post_boot_cgb()
    }

    fn resolve_stop(&mut self) -> StopAction {
        if self.key1_armed {
            self.double_speed = !self.double_speed;
            self.key1_armed = false;
            StopAction::SpeedSwitch
        } else {
            StopAction::Remain
        }
    }

    fn cpu_steps_per_dot(&self) -> u8 {
        if self.double_speed { 2 } else { 1 }
    }

    fn on_reset(&mut self, _cartridge: &Cartridge) {
        *self = Self::default();
    }

    fn map_read(&self, address: u16) -> Option<u8> {
        if let Some(i) = self.wram_index(address) {
            return Some(self.wram[i]);
        }
        match address {
            0xFF4C => Some(0xFF), // KEY0: boot-locked
            0xFF4D => Some(0x7E | ((self.double_speed as u8) << 7) | self.key1_armed as u8), // KEY1
            0xFF6C => Some(0xFE | self.opri as u8), // OPRI: bit0
            0xFF70 => Some(self.svbk | 0xF8), // SVBK: bits 0-2
            _ => None,
        }
    }

    fn map_write(&mut self, address: u16, value: u8) -> bool {
        if let Some(i) = self.wram_index(address) {
            self.wram[i] = value;
            return true;
        }
        match address {
            0xFF4C => true, // KEY0: boot-locked, ignore
            0xFF4D => {
                self.key1_armed = value & 0x01 != 0;
                true
            }
            0xFF6C => {
                self.opri = value & 0x01 != 0;
                true
            }
            0xFF70 => {
                self.svbk = value & 0x07;
                true
            }
            _ => false,
        }
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;
