//! The broadcast standard a console is wired for.

/// The colour standard a console's video output is encoded for. It selects the
/// colour decode; the master clock a console derives from it is the console's
/// own property, not the standard's, and lives with that core.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum TvStandard {
    #[default]
    Ntsc,
    Pal,
    /// French SECAM: PAL's 50 Hz field timing, but the set drives fixed colours
    /// from the luma alone and ignores the hue, so it differs from PAL only in
    /// the colour decode.
    Secam,
}

impl TvStandard {
    pub fn name(self) -> &'static str {
        match self {
            TvStandard::Ntsc => "NTSC",
            TvStandard::Pal => "PAL",
            TvStandard::Secam => "SECAM",
        }
    }

    /// The name catalogues and launch options carry the standard under.
    pub fn code(self) -> &'static str {
        match self {
            TvStandard::Ntsc => "ntsc",
            TvStandard::Pal => "pal",
            TvStandard::Secam => "secam",
        }
    }

    /// The standard a code names, however it was cased.
    pub fn from_code(code: &str) -> Option<TvStandard> {
        match code.trim().to_ascii_lowercase().as_str() {
            "ntsc" => Some(TvStandard::Ntsc),
            "pal" => Some(TvStandard::Pal),
            "secam" => Some(TvStandard::Secam),
            _ => None,
        }
    }

    /// Every standard, in the order they are offered.
    pub fn all() -> [TvStandard; 3] {
        [TvStandard::Ntsc, TvStandard::Pal, TvStandard::Secam]
    }
}
