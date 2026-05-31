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

use missingno_gb::{Console, Model, PixelOutput, cpu::Cpu};

use crate::screen::{Color555, GREYSCALE, Screen};

/// The Game Boy Color [`Model`]. Remaining CGB registers (VBK, SVBK, CRAM,
/// HDMA) and the color pixel pipeline attach here as features land.
#[derive(Default)]
pub struct Cgb {
    /// KEY1 ($FF4D) bit 0 — speed-switch arm. The switch itself lands with
    /// double-speed support.
    key1_armed: bool,
    /// OPRI ($FF6C) bit 0 — object priority mode (0 = by OAM index). The
    /// priority effect lands with the color PPU.
    opri: bool,
}

impl Model for Cgb {
    type Screen = Screen;

    fn map_pixel(pixel: PixelOutput) -> Color555 {
        GREYSCALE[(pixel.shade & 0x3) as usize]
    }

    fn cpu_post_boot(_checksum: u8) -> Cpu {
        Cpu::post_boot_cgb()
    }

    fn speed_switch_armed(&self) -> bool {
        self.key1_armed
    }

    fn map_read(&self, address: u16) -> Option<u8> {
        match address {
            0xFF4C => Some(0xFF),                         // KEY0: boot-locked
            0xFF4D => Some(0x7E | self.key1_armed as u8), // KEY1: bit7 speed=0, bits1-6=1, bit0 arm
            0xFF6C => Some(0xFE | self.opri as u8),       // OPRI: bit0
            _ => None,
        }
    }

    fn map_write(&mut self, address: u16, value: u8) -> bool {
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
            _ => false,
        }
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;
