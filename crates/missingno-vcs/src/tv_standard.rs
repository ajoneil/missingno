//! The broadcast standard the console is wired to. VCS cartridges carry no
//! region header, so the standard is supplied by the caller, not detected —
//! it selects the colour decode and the master-clock-derived audio rate.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TvStandard {
    #[default]
    Ntsc,
    Pal,
}

impl TvStandard {
    /// TIA colour-clock (pixel-clock) frequency; the CPU runs at a third of it.
    pub fn master_clock_hz(self) -> f32 {
        match self {
            TvStandard::Ntsc => 3_579_545.0,
            TvStandard::Pal => 3_546_894.0,
        }
    }

    /// Colour clocks per 44.1 kHz output sample.
    pub fn clocks_per_sample(self) -> f32 {
        self.master_clock_hz() / 44_100.0
    }

    pub fn name(self) -> &'static str {
        match self {
            TvStandard::Ntsc => "NTSC",
            TvStandard::Pal => "PAL",
        }
    }
}
