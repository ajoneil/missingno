//! The broadcast standard a console is wired for.

/// The colour standard a console's video output is encoded for. It selects the
/// colour decode; the master clock a console derives from it is the console's
/// own property, not the standard's, and lives with that core.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum TvStandard {
    #[default]
    Ntsc,
    Pal,
    /// Standard PAL colour on a 60 Hz/525-line raster: correct colours on a
    /// PAL set but NTSC-speed timing. Common as a second build of homebrew
    /// alongside the NTSC one; distinct from PAL-M.
    Pal60,
    /// NTSC colour on a 50 Hz/625-line raster — Pal60's mirror tag.
    Ntsc50,
    /// Brazil's PAL-M: PAL colour encoding on System M's 525-line, 59.94 Hz
    /// raster, so it runs at NTSC timing rather than PAL's.
    PalM,
    /// French SECAM: PAL's 50 Hz field timing, but the set drives fixed colours
    /// from the luma alone and ignores the hue, so it differs from PAL only in
    /// the colour decode.
    Secam,
}

impl TvStandard {
    pub fn display_name(self) -> &'static str {
        match self {
            TvStandard::Ntsc => "NTSC",
            TvStandard::Pal => "PAL",
            TvStandard::Pal60 => "PAL60",
            TvStandard::Ntsc50 => "NTSC50",
            TvStandard::PalM => "PAL-M",
            TvStandard::Secam => "SECAM",
        }
    }

    /// The name catalogues and launch options carry the standard under — the
    /// variant's own, as a board's is.
    pub fn name(self) -> &'static str {
        match self {
            TvStandard::Ntsc => "ntsc",
            TvStandard::Pal => "pal",
            TvStandard::Pal60 => "pal60",
            TvStandard::Ntsc50 => "ntsc50",
            TvStandard::PalM => "palm",
            TvStandard::Secam => "secam",
        }
    }

    /// The standard a name names, however it was cased.
    pub fn from_name(name: &str) -> Option<TvStandard> {
        match name.trim().to_ascii_lowercase().as_str() {
            "ntsc" => Some(TvStandard::Ntsc),
            "pal" => Some(TvStandard::Pal),
            "pal60" => Some(TvStandard::Pal60),
            "ntsc50" => Some(TvStandard::Ntsc50),
            "palm" => Some(TvStandard::PalM),
            "secam" => Some(TvStandard::Secam),
            _ => None,
        }
    }

    /// Every standard, in the order they are offered.
    pub fn all() -> [TvStandard; 6] {
        [
            TvStandard::Ntsc,
            TvStandard::Pal,
            TvStandard::Pal60,
            TvStandard::Ntsc50,
            TvStandard::PalM,
            TvStandard::Secam,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::TvStandard;

    #[test]
    fn every_standards_code_parses_back() {
        for standard in TvStandard::all() {
            assert_eq!(TvStandard::from_name(standard.name()), Some(standard));
        }
    }
}
