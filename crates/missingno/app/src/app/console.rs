use missingno_gb::debugger::inspection::ColorSnapshot;
use missingno_gb::ppu::types::palette::Palette;

/// The colours the debugger panes draw with: the user-selected palette on
/// DMG, the corrected CRAM palettes on CGB.
// One per pane render; boxing the CGB arrays would just add a hop.
#[allow(clippy::large_enum_variant)]
pub enum ConsoleColors {
    Dmg {
        palette: Palette,
    },
    Cgb {
        background: [Palette; 8],
        objects: [Palette; 8],
    },
}

impl ConsoleColors {
    /// CGB tile data has no palette of its own — show it in greyscale.
    pub fn tiles_palette(&self) -> &Palette {
        match self {
            Self::Dmg { palette } => palette,
            Self::Cgb { .. } => &Palette::CLASSIC,
        }
    }
}

/// Rebuild the render palettes from a running snapshot's colour data, applying
/// the live user palette (which can change mid-run on DMG).
pub fn colors_from_snapshot(colors: &ColorSnapshot, user_palette: &Palette) -> ConsoleColors {
    match colors {
        ColorSnapshot::Dmg { sgb } => ConsoleColors::Dmg {
            palette: if *sgb {
                Palette::CLASSIC
            } else {
                *user_palette
            },
        },
        ColorSnapshot::Cgb {
            background,
            objects,
        } => ConsoleColors::Cgb {
            background: *background,
            objects: *objects,
        },
    }
}
