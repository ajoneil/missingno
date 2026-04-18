use super::dff::DffLatch;
use super::types::control::{Control, ControlFlags};
use super::types::palette::Palettes;

pub struct BackgroundViewportPosition {
    pub x: DffLatch,
    pub y: DffLatch,
}

pub struct Window {
    pub y: u8,
    pub x_plus_7: DffLatch,
}

/// Pipeline registers (schematic pages 23/36): the DFF register file
/// that the CPU writes and the pixel pipeline reads. One-directional —
/// CPU → pipeline. These cells sit together on the die as a register
/// bank, and their DFF8/DFF9 write-conflict behavior during Mode 3
/// is specific to this group.
pub struct PipelineRegisters {
    pub control: Control,
    /// DFF9-style latch for full LCDC byte.
    pub control_latch: DffLatch,
    pub background_viewport: BackgroundViewportPosition,
    pub window: Window,
    pub palettes: Palettes,
}

impl PipelineRegisters {
    /// Advance DFF8 palette latches by one dot. The old value persists
    /// through the write dot; the new value appears atomically on the
    /// capture tick (one dot after write).
    pub fn tick_palette_latches(&mut self) -> bool {
        let bg = self.palettes.background.tick();
        let sp0 = self.palettes.sprite0.tick();
        let sp1 = self.palettes.sprite1.tick();
        bg || sp0 || sp1
    }

    /// Advance DFF9 register latches by one dot. Runs after the pipeline
    /// (in the alet-rising phase) so the pipeline reads pre-tick values
    /// (reg_old), matching hardware's combinational read-from-old behavior.
    pub fn tick_register_latches(&mut self) {
        self.background_viewport.x.tick();
        self.background_viewport.y.tick();
        self.window.x_plus_7.tick();
        if self.control_latch.tick() {
            self.control = Control::new(ControlFlags::from_bits_retain(self.control_latch.output));
        }
    }

    /// Clear all pending DFF latch state without applying final values.
    /// Called when the PPU turns off — latches freeze at their current output.
    pub fn clear_latches(&mut self) {
        self.palettes.background.clear();
        self.palettes.sprite0.clear();
        self.palettes.sprite1.clear();
        self.background_viewport.x.clear();
        self.background_viewport.y.clear();
        self.window.x_plus_7.clear();
        self.control_latch.clear();
    }
}
