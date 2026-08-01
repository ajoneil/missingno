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
}
