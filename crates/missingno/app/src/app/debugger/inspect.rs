//! The debugger panes' inspection surface. The GUI-free data views live in
//! `missingno_gb::debugger::inspection` (and the CGB register view in
//! `missingno_gbc`); this module re-exports them and layers the pane-facing
//! [`InspectSource`] surface — the live console while paused, or a per-vblank
//! snapshot while the machine free-runs — on top.

use std::any::Any;

use missingno_gb::ppu::types::palette::Palette;
use missingno_gb::{Console, Dmg, GameBoy, Model};
use missingno_gbc::{Cgb, GameBoyColor, cram_palettes};

use crate::app::console::{ConsoleColors, colors_from_snapshot};

pub use missingno_core::system::DebugView;
pub use missingno_gb::debugger::inspection::GbSnapshot;
pub use missingno_gbc::CgbSnapshot;

// --- Model colour hooks ------------------------------------------------------

/// How each model resolves the debugger's render palettes and its CGB
/// register view — the model-specific, iced-adjacent slice of inspection that
/// stays app-side.
pub trait GbColors: Model {
    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors;
}

impl GbColors for Dmg {
    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors {
        ConsoleColors::Dmg {
            palette: if console.sgb().is_some() {
                Palette::CLASSIC
            } else {
                *user_palette
            },
        }
    }
}

impl GbColors for Cgb {
    fn colors(console: &Console<Self>, _user_palette: &Palette) -> ConsoleColors {
        let ppu = console.ppu().model();
        ConsoleColors::Cgb {
            background: cram_palettes(|palette, index| ppu.bg_color(palette, index)),
            objects: cram_palettes(|palette, index| ppu.obj_color(palette, index)),
        }
    }
}

// --- Inspection source -------------------------------------------------------

/// The Game Boy family's colour resolution, behind one model-erased surface:
/// the live console while paused, or the per-vblank snapshot while the machine
/// free-runs. The graphics panes read their structure from
/// the decoded [`GraphicsView`] on the context; this resolves the DMG/CGB
/// palettes those indices colour through.
///
/// [`GraphicsView`]: missingno_core::graphics::GraphicsView
pub trait InspectSource {
    fn colors(&self, user_palette: &Palette) -> ConsoleColors;
}

impl<M: GbColors> InspectSource for Console<M> {
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        M::colors(self, user_palette)
    }
}

impl InspectSource for GbSnapshot {
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        colors_from_snapshot(&self.colors, user_palette)
    }
}

impl InspectSource for CgbSnapshot {
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        self.base.colors(user_palette)
    }
}

/// Recover the Game Boy inspection surface from a family-erased state — the
/// live console while paused, or its snapshot while running. `None` for any
/// other family, whose own panes downcast their typed state.
pub fn as_inspect_source(state: &dyn Any) -> Option<&dyn InspectSource> {
    if let Some(console) = state.downcast_ref::<GameBoy>() {
        Some(console as &dyn InspectSource)
    } else if let Some(console) = state.downcast_ref::<GameBoyColor>() {
        Some(console as &dyn InspectSource)
    } else if let Some(snapshot) = state.downcast_ref::<GbSnapshot>() {
        Some(snapshot as &dyn InspectSource)
    } else {
        state
            .downcast_ref::<CgbSnapshot>()
            .map(|snapshot| snapshot as &dyn InspectSource)
    }
}
