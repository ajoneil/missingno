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
    /// NURA-combiner OR overlay: a same-tick BGP write presents OR(prior, new) on the cp_pad sample.
    pub(in crate::ppu) background_or_overlay: Option<u8>,
    /// A BGP write has resolved during this scanline's active period; cleared at next BESU↑.
    pub(in crate::ppu) bgp_recovery_active: bool,
    /// LCD has emitted a visible pixel since the last tick_background commit (primes OR-overlap).
    pub(in crate::ppu) bgp_visible_emit_since_tick: bool,
    /// BGP write parked while CPU is in a HALT-wake handler; countdown shifts the visible transition 4-5 columns later.
    pub(in crate::ppu) bgp_halt_wake_deferred: Option<DeferredBgpWrite>,
    /// Prior fall's BESU; feeds the BESU↑ edge detector that releases the NURA-overlay recovery.
    pub(in crate::ppu) prev_besu: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DeferredBgpWrite {
    pub value: u8,
    pub ticks_remaining: u8,
}

impl Default for Palettes {
    fn default() -> Self {
        Self {
            background: DffLatch::new(0xfc),
            sprite0: DffLatch::new(0xFF),
            sprite1: DffLatch::new(0xFF),
            background_or_overlay: None,
            bgp_recovery_active: false,
            bgp_visible_emit_since_tick: false,
            bgp_halt_wake_deferred: None,
            prev_besu: false,
        }
    }
}

impl Palettes {
    /// 5-tick countdown if recovery is active (NURA adds +1 column), else 6, so HALT-wake and running-CPU writes land at the same wall-clock.
    pub fn write_background_halt_wake_deferred(&mut self, value: u8) {
        let ticks_remaining = if self.bgp_recovery_active { 5 } else { 6 };
        self.bgp_halt_wake_deferred = Some(DeferredBgpWrite {
            value,
            ticks_remaining,
        });
    }

    pub fn tick_background(&mut self) -> bool {
        // Commit a HALT-wake-deferred write into `pending` when its countdown expires.
        if let Some(deferred) = self.bgp_halt_wake_deferred.as_mut() {
            if deferred.ticks_remaining > 0 {
                deferred.ticks_remaining -= 1;
            }
            if deferred.ticks_remaining == 0 {
                let value = deferred.value;
                self.bgp_halt_wake_deferred = None;
                self.background.write(value);
            }
        }

        let prior = self.background.output();
        let ticked = self.background.tick();
        if ticked {
            self.background_or_overlay =
                if self.bgp_recovery_active && self.bgp_visible_emit_since_tick {
                    Some(prior | self.background.output())
                } else {
                    None
                };
            self.bgp_recovery_active = true;
            self.bgp_visible_emit_since_tick = false;
        } else {
            self.background_or_overlay = None;
        }
        ticked
    }

    /// A visible cp_pad↑ has emitted a pixel; subsequent BGP CUPAs satisfy the recovery-engaged precondition.
    pub fn note_bg_pixel_emit(&mut self) {
        self.bgp_visible_emit_since_tick = true;
    }

    pub fn background_for_bg_resolve(&self) -> u8 {
        if let Some(overlay) = self.background_or_overlay {
            return overlay;
        }
        // dlatch_ee transparency: a pixel emit between drive_ppu_bus (rise) and tick_palette_latches (fall)
        // sees OR(prior, pending) — extends the NURA overlay one emulator edge backwards.
        if self.bgp_recovery_active
            && self.bgp_visible_emit_since_tick
            && let Some(pending) = self.background.pending()
        {
            return self.background.output() | pending;
        }
        self.background.output()
    }

    /// BESU↑ at Mode 2 entry releases the BGP NURA-overlay recovery (dlatch has settled through HBlank).
    pub(in crate::ppu) fn tick_besu(&mut self, besu: bool) {
        if besu && !self.prev_besu {
            self.background_or_overlay = None;
            self.bgp_recovery_active = false;
            self.bgp_visible_emit_since_tick = false;
        }
        self.prev_besu = besu;
    }

    pub fn clear_background_overlay(&mut self) {
        self.background_or_overlay = None;
        self.bgp_recovery_active = false;
        self.bgp_visible_emit_since_tick = false;
    }
}
