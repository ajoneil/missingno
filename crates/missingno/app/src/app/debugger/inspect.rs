//! The debugger panes' inspection surface. The GUI-free data views live in
//! `missingno_gb::debugger::inspection` (and the CGB register view in
//! `missingno_gbc`); this module re-exports them and layers the pane-facing
//! [`InspectSource`] surface over the capture the seam publishes.

use std::any::Any;

use missingno_gb::ppu::types::palette::Palette;

use crate::app::console::{ConsoleColors, colors_from_snapshot};

pub use missingno_core::system::DebugView;
pub use missingno_gb::debugger::inspection::GbSnapshot;
pub use missingno_gbc::CgbSnapshot;

// --- Inspection source -------------------------------------------------------

/// The Game Boy family's colour resolution, behind one model-erased surface.
/// The graphics panes read their structure from the decoded [`GraphicsView`] on
/// the context; this resolves the DMG/CGB palettes those indices colour
/// through.
///
/// [`GraphicsView`]: missingno_core::graphics::GraphicsView
pub trait InspectSource {
    fn colors(&self, user_palette: &Palette) -> ConsoleColors;
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

/// Recover the Game Boy inspection surface from a family-erased state: the
/// capture the seam holds, whether it was taken for a paused readout or for a
/// published snapshot. `None` for any other family, whose own panes downcast
/// their typed state.
pub fn as_inspect_source(state: &dyn Any) -> Option<&dyn InspectSource> {
    if let Some(snapshot) = state.downcast_ref::<GbSnapshot>() {
        Some(snapshot as &dyn InspectSource)
    } else {
        state
            .downcast_ref::<CgbSnapshot>()
            .map(|snapshot| snapshot as &dyn InspectSource)
    }
}
