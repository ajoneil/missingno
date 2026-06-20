//! CGB register-path timing data: the speed-independent `cgb_extra_falls` each
//! [`CaptureSpec`] carries — a real CGB-vs-DMG silicon delta present at every CGB
//! speed, authored behind the wall and never derived from the CPU:dot ratio.
//!
//! [`CaptureSpec`]: missingno_gb::ppu::CaptureSpec

use missingno_gb::ppu::{CaptureEdge, CaptureSpec};

/// The mid-Mode-3 SCY ($FF42) write → BG-fetch crossing on the CGB: the write
/// crosses on the M-cycle-last-fall edge and the BG fetch samples it two falls
/// late — the documented CGB 2-T-cycle register-write lag (mealybug
/// `m3_scy_change`). `cgb_extra_falls` is the *total* carried fall count, matching
/// the historical `SCY_WRITE_LAG_FALLS = 2`.
pub const SCY_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 2,
};

/// The FF45 (LYC) → STAT-IRQ-block crossing on the CGB: the cell crosses into
/// the IRQ block on the resolved capture edge with no register-path lag on top
/// — pure (ii) clock phase. The phase arrives from the resolver; `cgb_extra_falls`
/// stays 0.
pub const LYC_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 0,
};

/// The mid-Mode-3 LCDC tile-map-select (LCDC.3/.6) write → BG-fetch crossing on
/// the CGB: the write crosses on the M-cycle-last-fall edge and the BG fetch
/// samples the select bit two falls late — the documented CGB resync lag
/// (mealybug `m3_lcdc_bg_map_change`). Like SCY, the fetch ticks the cell on the
/// dot-fall grid, so this is the (iv) register-path offset alone; `cgb_extra_falls`
/// is the *total* carried fall count.
pub const TILE_MAP_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 2,
};

/// The FF43 (SCX) → fine-scroll-match crossing on the CGB: the cell crosses
/// into the pixel pipeline on the resolved capture edge with no register-path
/// lag on top — pure (ii) clock phase, like LYC. The phase arrives from the
/// resolver; `cgb_extra_falls` stays 0.
pub const SCX_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 0,
};

/// The FF41 (STAT enables) → STAT-IRQ-block crossing on the CGB: the enables
/// cell crosses into the IRQ block on the resolved capture edge (the M-boundary
/// fall) with no register-path lag on top — pure (ii) clock phase, like LYC.
/// The intra-evaluation register arrival that races the SUKO waveform is the
/// separate `REGISTER_PATH_ARRIVAL_PS` constant, not this offset; `cgb_extra_falls`
/// stays 0.
pub const STAT_ENABLES_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 0,
};

/// The window register file (WY/WX/LCDC.5/LCDC.2) crossing on the CGB: the
/// cells cross into the window decode and scan Y-comparator on the resolved
/// capture edge with no register-path lag on top — pure (ii) clock phase, like
/// LYC and SCX. The phase arrives from the resolver; `cgb_extra_falls` stays 0.
pub const WINDOW_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 0,
};

/// The mid-Mode-3 LCDC.0 (VYXE) write → BG-plane-blank crossing on the CGB: the
/// write crosses on the M-cycle-last-fall edge and the OLD-overlay holds the
/// pre-write value one extra fall — RAJY lands one dot later than the DMG's
/// combinational path. The OLD-overlay carries its own same-fall base hold, so
/// `cgb_extra_falls` is the extra-falls offset on top, matching the historical
/// `BG_ENABLE_WRITE_LAG`'s `extra_hold = 1`.
pub const BG_ENABLE_CROSSING: CaptureSpec = CaptureSpec {
    capture: CaptureEdge::MCycleLastFall,
    cgb_extra_falls: 1,
};

/// The mid-Mode-3 LCDC.1 (XYLO) write → OBJ-mux crossing on the CGB: no
/// register-path lag — the OLD-overlay's same-fall base hold is the whole story
/// on both cores, so this stays combinational.
pub const OBJ_ENABLE_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

#[cfg(test)]
mod tests {
    use super::*;

    /// The CGB SCY crossing must hand `DffLatch::write_delayed` a total of 2
    /// falls — bit-identical to the pre-migration `SCY_WRITE_LAG_FALLS = 2`.
    #[test]
    fn scy_crossing_carries_two_falls() {
        assert_eq!(SCY_CROSSING.write_delayed_falls(), 2);
    }

    /// The CGB LYC crossing carries no register-path lag — its phase rides the
    /// capture edge alone.
    #[test]
    fn lyc_crossing_carries_no_extra_falls() {
        assert_eq!(LYC_CROSSING.write_delayed_falls(), 0);
    }

    /// The CGB tile-map-select crossing must hand `DffLatch::write_delayed` a
    /// total of 2 falls — bit-identical to the pre-migration
    /// `TILE_MAP_READ_STALE_FALLS = 2` ring depth at the fetch sample point.
    #[test]
    fn tile_map_crossing_carries_two_falls() {
        assert_eq!(TILE_MAP_CROSSING.write_delayed_falls(), 2);
    }

    /// The CGB BG-enable crossing must hand the OLD-overlay one extra hold fall
    /// — bit-identical to the pre-migration `BG_ENABLE_WRITE_LAG`'s
    /// `extra_hold = 1`.
    #[test]
    fn bg_enable_crossing_carries_one_extra_fall() {
        assert_eq!(BG_ENABLE_CROSSING.write_delayed_falls(), 1);
    }

    /// The CGB OBJ-enable crossing carries no register-path lag — combinational
    /// on both cores, `extra_hold = 0`.
    #[test]
    fn obj_enable_crossing_carries_no_extra_falls() {
        assert_eq!(OBJ_ENABLE_CROSSING.write_delayed_falls(), 0);
    }

    /// The CGB SCX crossing carries no register-path lag — its phase rides the
    /// capture edge alone, like LYC.
    #[test]
    fn scx_crossing_carries_no_extra_falls() {
        assert_eq!(SCX_CROSSING.write_delayed_falls(), 0);
        assert_eq!(SCX_CROSSING.capture, CaptureEdge::MCycleLastFall);
    }

    /// The CGB window register-file crossing carries no register-path lag — its
    /// phase rides the capture edge alone, like LYC and SCX.
    #[test]
    fn window_crossing_carries_no_extra_falls() {
        assert_eq!(WINDOW_CROSSING.write_delayed_falls(), 0);
        assert_eq!(WINDOW_CROSSING.capture, CaptureEdge::MCycleLastFall);
    }

    /// The CGB STAT-enables crossing carries no register-path lag — its phase
    /// rides the capture edge alone (the SUKO-waveform register arrival is the
    /// separate `REGISTER_PATH_ARRIVAL_PS` constant).
    #[test]
    fn stat_enables_crossing_carries_no_extra_falls() {
        assert_eq!(STAT_ENABLES_CROSSING.write_delayed_falls(), 0);
        assert_eq!(STAT_ENABLES_CROSSING.capture, CaptureEdge::MCycleLastFall);
    }
}
