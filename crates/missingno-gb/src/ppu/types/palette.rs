use std::fmt;

use rgb::RGB8;

pub struct Palette {
    colors: [RGB8; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct PaletteIndex(pub u8);

impl Palette {
    pub const MONOCHROME_GREEN: Self = Self {
        colors: [
            RGB8::new(0x7b, 0x82, 0x10),
            RGB8::new(0x5a, 0x79, 0x42),
            RGB8::new(0x39, 0x59, 0x4a),
            RGB8::new(0x2f, 0x41, 0x39),
        ],
    };

    pub const POCKET: Self = Self {
        colors: [
            RGB8::new(0xc4, 0xcf, 0xa1),
            RGB8::new(0x8b, 0x95, 0x6d),
            RGB8::new(0x4d, 0x53, 0x3c),
            RGB8::new(0x1b, 0x1b, 0x1b),
        ],
    };

    pub const CLASSIC: Self = Self {
        colors: [
            RGB8::new(0xff, 0xff, 0xff),
            RGB8::new(0xaa, 0xaa, 0xaa),
            RGB8::new(0x55, 0x55, 0x55),
            RGB8::new(0x00, 0x00, 0x00),
        ],
    };

    pub fn color(&self, index: PaletteIndex) -> RGB8 {
        self.colors[index.0 as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PaletteChoice {
    #[default]
    Green,
    Pocket,
    Classic,
}

impl PaletteChoice {
    pub const ALL: &[Self] = &[Self::Green, Self::Pocket, Self::Classic];

    pub fn palette(&self) -> &Palette {
        match self {
            Self::Green => &Palette::MONOCHROME_GREEN,
            Self::Pocket => &Palette::POCKET,
            Self::Classic => &Palette::CLASSIC,
        }
    }
}

impl fmt::Display for PaletteChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Green => write!(f, "Original"),
            Self::Pocket => write!(f, "Pocket"),
            Self::Classic => write!(f, "Greyscale"),
        }
    }
}

pub struct PaletteMap(pub u8);

impl PaletteMap {
    pub fn color(&self, index: PaletteIndex, palette: &Palette) -> RGB8 {
        palette.color(self.map(index))
    }

    pub fn map(&self, index: PaletteIndex) -> PaletteIndex {
        PaletteIndex((self.0 >> (index.0 * 2)) & 0b11)
    }
}

use super::super::DffLatch;

pub struct Palettes {
    pub background: DffLatch,
    pub sprite0: DffLatch,
    pub sprite1: DffLatch,
    /// NURA-combiner OR overlay. On the cp_pad sample of a BGP write
    /// while the dlatch_ee cell is still in its post-write recovery
    /// state, the cell presents OR(prior, new) instead of settled new.
    /// Held for one tick after a qualifying write.
    pub(crate) background_or_overlay: Option<u8>,
    /// True once a BGP write has resolved during the current scanline's
    /// active period (Mode 2 onward). Cleared at the next scanline's
    /// Mode 2 entry (BESU↑) when the cell can finish settling.
    pub(crate) bgp_recovery_active: bool,
}

impl Default for Palettes {
    fn default() -> Self {
        Self {
            background: DffLatch::new(0xfc),
            sprite0: DffLatch::new(0xFF),
            sprite1: DffLatch::new(0xFF),
            background_or_overlay: None,
            bgp_recovery_active: false,
        }
    }
}

impl Palettes {
    pub fn tick_background(&mut self) -> bool {
        let prior = self.background.output();
        let ticked = self.background.tick();
        if ticked {
            self.background_or_overlay = if self.bgp_recovery_active {
                Some(prior | self.background.output())
            } else {
                None
            };
            self.bgp_recovery_active = true;
        } else {
            self.background_or_overlay = None;
        }
        ticked
    }

    pub fn background_for_bg_resolve(&self) -> u8 {
        if let Some(overlay) = self.background_or_overlay {
            return overlay;
        }
        // dlatch_ee transparency: while CUPA is high, a BGP write that
        // has set DffLatch.pending but not yet been committed by
        // tick_background presents OR(prior, new) on the cp_pad sample.
        // Extends the NURA overlay window backwards by one emulator
        // edge so a pixel emit between drive_ppu_bus (rise) and
        // tick_palette_latches (fall) sees the in-flight value.
        if self.bgp_recovery_active
            && let Some(pending) = self.background.pending()
        {
            return self.background.output() | pending;
        }
        self.background.output()
    }

    /// Mode 2 entry (BESU↑) at scanline start. The pixel pipe is idle
    /// during the prior HBlank, so the BGP dlatch has settled by now —
    /// a new BGP write will start a fresh recovery window.
    pub fn reset_on_mode_2_entry(&mut self) {
        self.background_or_overlay = None;
        self.bgp_recovery_active = false;
    }

    pub fn clear_background_overlay(&mut self) {
        self.background_or_overlay = None;
        self.bgp_recovery_active = false;
    }
}
