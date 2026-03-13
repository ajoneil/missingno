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
    /// Advance DFF8 palette latches by one dot. Runs before the pipeline
    /// (in tcycle_falling) so the pipeline sees the transitional old|new
    /// value on the write dot, matching DFF8 master-slave transparency.
    pub(super) fn tick_palette_latches(&mut self) {
        self.palettes.background.tick();
        self.palettes.sprite0.tick();
        self.palettes.sprite1.tick();
    }

    /// Advance DFF9 register latches by one dot. Runs after the pipeline
    /// (in tcycle_rising) so the pipeline reads pre-tick values (reg_old),
    /// matching hardware's combinational read-from-old behavior.
    pub(super) fn tick_register_latches(&mut self) {
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
