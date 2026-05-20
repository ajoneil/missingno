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

/// BGP NURA-combiner recovery state. While `active`, a BGP CUPA on a
/// dot where the LCD has already emitted a pixel produces the OR
/// overlay on the cp_pad sample; otherwise the new value lands clean.
#[derive(Default)]
pub(in crate::ppu) struct BgpRecovery {
    /// OR(prior, new) presented on the cp_pad sample when a same-tick
    /// BGP write engages the recovery overlap.
    or_overlay: Option<u8>,
    /// A BGP write has resolved during the current scanline's active
    /// period; cleared at the next BESU↑.
    active: bool,
    /// LCD has emitted a visible pixel since the last tick commit —
    /// primes the OR-overlap precondition.
    visible_emit_since_tick: bool,
    /// Prior fall's mode2_active; feeds the BESU↑ edge detector that releases
    /// the recovery at Mode 2 entry.
    prev_mode2_active: bool,
}

impl BgpRecovery {
    fn note_pixel_emit(&mut self) {
        self.visible_emit_since_tick = true;
    }

    /// Apply a BGP DffLatch tick: present OR(prior, new) if the
    /// overlap is engaged, then arm `active` for the next CUPA.
    fn commit_tick(&mut self, prior: u8, new: u8) {
        self.or_overlay = (self.active && self.visible_emit_since_tick).then_some(prior | new);
        self.active = true;
        self.visible_emit_since_tick = false;
    }

    /// No-op tick (no pending write committed) — overlay clears.
    fn clear_overlay(&mut self) {
        self.or_overlay = None;
    }

    /// BESU↑ at Mode 2 entry releases the recovery (dlatch has
    /// settled through HBlank). No-op on other edges.
    fn tick_mode2_active(&mut self, mode2_active: bool) {
        let rising = mode2_active && !self.prev_mode2_active;
        self.prev_mode2_active = mode2_active;
        if rising {
            self.reset();
        }
    }

    /// Whole-cell reset: LCD off, or via BESU↑.
    fn reset(&mut self) {
        self.or_overlay = None;
        self.active = false;
        self.visible_emit_since_tick = false;
    }

    fn overlay(&self) -> Option<u8> {
        self.or_overlay
    }

    /// True when a pending DffLatch write would present OR(output, pending)
    /// at the cp_pad sample — the dlatch_ee transparency window.
    fn pending_or_engaged(&self) -> bool {
        self.active && self.visible_emit_since_tick
    }

    fn active(&self) -> bool {
        self.active
    }
}

pub struct Palettes {
    pub background: DffLatch,
    pub sprite0: DffLatch,
    pub sprite1: DffLatch,
    pub(in crate::ppu) recovery: BgpRecovery,
    /// BGP write parked while CPU is in a HALT-wake handler; countdown shifts the visible transition 4-5 columns later.
    pub(in crate::ppu) bgp_halt_wake_deferred: Option<DeferredBgpWrite>,
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
            recovery: BgpRecovery::default(),
            bgp_halt_wake_deferred: None,
        }
    }
}

impl Palettes {
    /// 5-tick countdown if recovery is active (NURA adds +1 column), else 6, so HALT-wake and running-CPU writes land at the same wall-clock.
    pub fn write_background_halt_wake_deferred(&mut self, value: u8) {
        let ticks_remaining = if self.recovery.active() { 5 } else { 6 };
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
            self.recovery.commit_tick(prior, self.background.output());
        } else {
            self.recovery.clear_overlay();
        }
        ticked
    }

    /// A visible cp_pad↑ has emitted a pixel; subsequent BGP CUPAs satisfy the recovery-engaged precondition.
    pub fn note_bg_pixel_emit(&mut self) {
        self.recovery.note_pixel_emit();
    }

    pub fn background_for_bg_resolve(&self) -> u8 {
        if let Some(overlay) = self.recovery.overlay() {
            return overlay;
        }
        // dlatch_ee transparency: a pixel emit between drive_ppu_bus (rise) and tick_palette_latches (fall)
        // sees OR(prior, pending) — extends the NURA overlay one emulator edge backwards.
        if self.recovery.pending_or_engaged()
            && let Some(pending) = self.background.pending()
        {
            return self.background.output() | pending;
        }
        self.background.output()
    }

    /// BESU↑ at Mode 2 entry releases the BGP NURA-overlay recovery (dlatch has settled through HBlank).
    pub(in crate::ppu) fn tick_mode2_active(&mut self, mode2_active: bool) {
        self.recovery.tick_mode2_active(mode2_active);
    }

    pub fn clear_background_overlay(&mut self) {
        self.recovery.reset();
    }
}
