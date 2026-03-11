use super::control::{Control, ControlFlags};
use super::dff::DffLatch;
use super::palette::Palettes;

pub(super) struct BackgroundViewportPosition {
    pub(super) x: DffLatch,
    pub(super) y: DffLatch,
}

pub struct Window {
    pub(super) y: u8,
    pub(super) x_plus_7: DffLatch,
}

/// Pipeline registers (schematic pages 23/36): the DFF register file
/// that the CPU writes and the pixel pipeline reads. One-directional —
/// CPU → pipeline. These cells sit together on the die as a register
/// bank, and their DFF8/DFF9 write-conflict behavior during Mode 3
/// is specific to this group.
pub struct PipelineRegisters {
    pub(super) control: Control,
    /// DFF9-style latch for full LCDC byte.
    pub(super) control_latch: DffLatch,
    pub(super) background_viewport: BackgroundViewportPosition,
    pub(super) window: Window,
    pub(super) palettes: Palettes,
}

impl PipelineRegisters {
    /// Advance all DFF latches by one dot. On hardware, the master-slave
    /// DFF cells resolve their pending values on the clock edge each dot.
    pub(super) fn tick_latches(&mut self) {
        self.palettes.background.tick();
        self.palettes.sprite0.tick();
        self.palettes.sprite1.tick();
        self.background_viewport.x.tick();
        self.background_viewport.y.tick();
        self.window.x_plus_7.tick();
        if self.control_latch.tick() {
            self.control = Control::new(ControlFlags::from_bits_retain(self.control_latch.output));
        }
    }

    /// Clear all pending DFF latch state without applying final values.
    /// Called when the PPU turns off — latches freeze at their current output.
    pub(super) fn clear_latches(&mut self) {
        self.palettes.background.clear();
        self.palettes.sprite0.clear();
        self.palettes.sprite1.clear();
        self.background_viewport.x.clear();
        self.background_viewport.y.clear();
        self.window.x_plus_7.clear();
        self.control_latch.clear();
    }
}
