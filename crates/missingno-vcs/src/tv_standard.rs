//! The broadcast standard the console is wired to. VCS cartridges carry no
//! region header, so the standard is supplied by the caller, not detected —
//! it selects the colour decode and the master-clock-derived audio rate.

pub use missingno_hw::TvStandard;

/// Display aspect of a TIA pixel (12/7).
pub const PIXEL_ASPECT: f32 = 12.0 / 7.0;

/// TIA colour-clock (pixel-clock) frequency; the CPU runs at a third of it.
/// The PAL crystal is not PAL's colour carrier, so the rate is the console's
/// property rather than the standard's.
pub fn master_clock_hz(standard: TvStandard) -> f32 {
    match standard {
        TvStandard::Ntsc => 3_579_545.0,
        TvStandard::Pal | TvStandard::Secam => 3_546_894.0,
    }
}

/// Colour clocks per 44.1 kHz output sample.
pub fn clocks_per_sample(standard: TvStandard) -> f32 {
    master_clock_hz(standard) / 44_100.0
}
