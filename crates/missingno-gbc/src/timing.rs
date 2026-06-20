//! CGB register-path timing data.
//!
//! The home for the speed-independent CGB register-crossing offsets — the
//! `cgb_extra_falls` each [`CaptureSpec`] carries on top of its capture edge.
//! These are CGB *data*: a real CGB-vs-DMG silicon timing delta present at every
//! CGB speed, including single speed (ratio=1). They are **not** derived from the
//! CPU:dot ratio — the (ii) double-speed clock model supplies the phase skew at
//! ratio=2 and collapses to DMG at ratio=1, while these offsets ride on top per
//! crossing.
//!
//! The shared `missingno-gb` core names only the [`CaptureSpec`] *type* and its
//! [`CaptureSpec::COMBINATIONAL`] collapse; every non-zero value is authored here,
//! behind the wall.
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
}
