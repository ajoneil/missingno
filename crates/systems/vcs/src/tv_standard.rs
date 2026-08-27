//! The broadcast standard the console is wired to. VCS cartridges carry no
//! region header, so the standard is supplied by the caller, not detected —
//! it selects the colour decode and the master-clock-derived audio rate.

use missingno_core::ClockRatio;
pub use missingno_core::TvStandard;

/// The console's audio output tap.
const SAMPLE_RATE: u64 = 44_100;

/// Display aspect of a TIA pixel on a 525-line (NTSC) raster: 12/7.
const NTSC_PIXEL_ASPECT: f32 = 12.0 / 7.0;

/// The scanning system the standard rides: System M's 525-line/60 Hz field or
/// the 625-line/50 Hz field. A VCS kernel nominally emits 262 or 312 lines per
/// field accordingly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Raster {
    System525,
    System625,
}

pub fn raster(standard: TvStandard) -> Raster {
    match standard {
        TvStandard::Ntsc | TvStandard::Pal60 | TvStandard::PalM => Raster::System525,
        TvStandard::Pal | TvStandard::Ntsc50 | TvStandard::Secam => Raster::System625,
    }
}

/// Display aspect of a TIA pixel on the cart's standard. The 228-clock line
/// spans the screen identically everywhere, but a 625-line raster paints
/// 312.5 lines into the height a 525-line raster fills with 262.5, so its
/// pixels are 25/21 wider relative to their height.
pub fn pixel_aspect(standard: TvStandard) -> f32 {
    match raster(standard) {
        Raster::System525 => NTSC_PIXEL_ASPECT,
        Raster::System625 => NTSC_PIXEL_ASPECT * 25.0 / 21.0,
    }
}

/// TIA colour-clock (pixel-clock) frequency; the CPU runs at a third of it.
/// The PAL crystal is not PAL's colour carrier, so the rate is the console's
/// property rather than the standard's.
pub fn master_clock_hz(standard: TvStandard) -> u32 {
    match standard {
        TvStandard::Ntsc | TvStandard::Ntsc50 => 3_579_545,
        TvStandard::Pal | TvStandard::Pal60 | TvStandard::Secam => 3_546_894,
        // PAL-M's subcarrier on the single-crystal NTSC board; no console
        // schematic located, Gopher2600 concurs.
        TvStandard::PalM => 3_575_611,
    }
}

/// The 44.1 kHz output tap, divided from the region's master clock.
pub fn sample_clock(standard: TvStandard) -> ClockRatio {
    ClockRatio::new(SAMPLE_RATE, master_clock_hz(standard) as u64)
}

#[cfg(test)]
mod tests {
    use super::{Raster, TvStandard, master_clock_hz, raster};

    #[test]
    fn hybrid_standards_ride_their_own_raster() {
        assert_eq!(raster(TvStandard::Pal60), Raster::System525);
        assert_eq!(raster(TvStandard::PalM), Raster::System525);
        assert_eq!(raster(TvStandard::Ntsc50), Raster::System625);
    }

    #[test]
    fn hybrid_standards_carry_their_own_crystal() {
        assert_eq!(
            master_clock_hz(TvStandard::Pal60),
            master_clock_hz(TvStandard::Pal)
        );
        assert_eq!(
            master_clock_hz(TvStandard::Ntsc50),
            master_clock_hz(TvStandard::Ntsc)
        );
        assert_eq!(master_clock_hz(TvStandard::PalM), 3_575_611);
    }
}
