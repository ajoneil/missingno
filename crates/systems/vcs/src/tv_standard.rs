//! The broadcast standard the console is wired to. VCS cartridges carry no
//! region header, so the standard is supplied by the caller, not detected —
//! it selects the colour decode and the master-clock-derived audio rate.

use missingno_core::ClockRatio;
pub use missingno_core::TvStandard;

/// The console's audio output tap.
const SAMPLE_RATE: u64 = 44_100;

/// Display aspect of a TIA pixel on a 525-line (NTSC) raster: 12/7.
const NTSC_PIXEL_ASPECT: f32 = 12.0 / 7.0;

/// Display aspect of a TIA pixel on the cart's standard. The 228-clock line
/// spans the screen identically everywhere, but a 625-line raster paints
/// 312.5 lines into the height a 525-line raster fills with 262.5, so
/// PAL/SECAM pixels are 25/21 wider relative to their height.
pub fn pixel_aspect(standard: TvStandard) -> f32 {
    match standard {
        TvStandard::Ntsc => NTSC_PIXEL_ASPECT,
        TvStandard::Pal | TvStandard::Secam => NTSC_PIXEL_ASPECT * 25.0 / 21.0,
    }
}

/// TIA colour-clock (pixel-clock) frequency; the CPU runs at a third of it.
/// The PAL crystal is not PAL's colour carrier, so the rate is the console's
/// property rather than the standard's.
pub fn master_clock_hz(standard: TvStandard) -> u32 {
    match standard {
        TvStandard::Ntsc => 3_579_545,
        TvStandard::Pal | TvStandard::Secam => 3_546_894,
    }
}

/// The 44.1 kHz output tap, divided from the region's master clock.
pub fn sample_clock(standard: TvStandard) -> ClockRatio {
    ClockRatio::new(SAMPLE_RATE, master_clock_hz(standard) as u64)
}
