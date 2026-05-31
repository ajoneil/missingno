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

use missingno_gb::{Console, Model, PixelOutput};

use crate::screen::{Color555, GREYSCALE, Screen};

/// The Game Boy Color [`Model`]. Holds no extra state yet; CGB registers
/// (KEY1, VBK, SVBK, CRAM, HDMA, OPRI) and the color pixel pipeline attach
/// here as features land.
#[derive(Default)]
pub struct Cgb;

impl Model for Cgb {
    type Screen = Screen;

    fn map_pixel(pixel: PixelOutput) -> Color555 {
        GREYSCALE[(pixel.shade & 0x3) as usize]
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;
