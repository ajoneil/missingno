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
    /// VYXE first-cp_pad↑-samples-OLD overlay. When a mid-Mode-3 CUPA
    /// transitions LCDC.0 (either direction), the LCD column emitted on
    /// the next cp_pad↑ resolves with the OLD VYXE state. Cleared on
    /// the next fall via `tick_bg_window_enabled_shadow`. `just_set`
    /// keeps the shadow alive for the same-fall resolve after the CPU
    /// write site sets it.
    pub(crate) bg_window_enabled_shadow: Option<bool>,
    pub(crate) bg_window_enabled_shadow_just_set: bool,
    /// XYLO popper-side OLD overlay. When a mid-Mode-3 CUPA transitions
    /// LCDC.1, the OBJ-mux popper (XULA/WOXA → NULY) at the next cp_pad↑
    /// resolves with the OLD XYLO state. The sprite-fetch trigger chain
    /// (AROR/FEPO/TEKY/SOBU) sees live XYLO — only the popper-side read
    /// consumes the shadow.
    pub(crate) sprites_enabled_shadow: Option<bool>,
    pub(crate) sprites_enabled_shadow_just_set: bool,
}

impl PipelineRegisters {
    /// Advance DFF8 palette latches by one dot. BGP captures via the
    /// Palettes wrapper so the NURA-combiner OR overlay updates with
    /// the tick. OBP0/OBP1 capture directly — the sprite combiners
    /// (WUFU/MOKA) read the settled output only.
    pub fn tick_palette_latches(&mut self) {
        self.palettes.tick_background();
        self.palettes.sprite0.tick();
        self.palettes.sprite1.tick();
    }

    /// Advance DFF9 register latches by one dot. Runs after the pipeline
    /// (in the PPU-clock-rise phase) so the pipeline reads pre-tick values
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
        self.palettes.clear_background_overlay();
        self.background_viewport.x.clear();
        self.background_viewport.y.clear();
        self.window.x_plus_7.clear();
        self.control_latch.clear();
        self.bg_window_enabled_shadow = None;
        self.bg_window_enabled_shadow_just_set = false;
        self.sprites_enabled_shadow = None;
        self.sprites_enabled_shadow_just_set = false;
    }

    /// Live VYXE state for the BG plane gate (RAJY / TADE), with the
    /// §6.15 first-cp_pad↑-samples-OLD overlay applied. When the shadow
    /// is set, the BG resolve sees the pre-transition LCDC.0 value;
    /// otherwise it sees the live `control` bit.
    pub fn bg_window_enabled_for_resolve(&self) -> bool {
        self.bg_window_enabled_shadow
            .unwrap_or_else(|| self.control.background_and_window_enabled())
    }

    /// CPU-write site: capture the pre-write VYXE state into the overlay
    /// shadow if LCDC.0 transitions during Mode 3. `just_set` keeps the
    /// shadow alive across the same-fall `tick_bg_window_enabled_shadow`.
    pub fn arm_bg_window_enabled_shadow(&mut self, old_value: bool, new_value: bool) {
        if old_value != new_value {
            self.bg_window_enabled_shadow = Some(old_value);
            self.bg_window_enabled_shadow_just_set = true;
        }
    }

    /// Once-per-fall tick. The CPU bus write fires before
    /// `on_master_clock_fall`, so the shadow is set with `just_set=true`
    /// before this tick runs. The tick consumes `just_set` — keeping the
    /// shadow alive for the same-fall BG resolve. On any subsequent fall
    /// without a fresh CPU write that toggles LCDC.0, the shadow clears,
    /// reverting the BG resolve to the live LCDC.0.
    pub fn tick_bg_window_enabled_shadow(&mut self) {
        if self.bg_window_enabled_shadow_just_set {
            self.bg_window_enabled_shadow_just_set = false;
        } else {
            self.bg_window_enabled_shadow = None;
        }
    }

    /// Live XYLO state for the OBJ-mux popper (XULA/WOXA → NULY), with
    /// the popper-side OLD overlay applied. When the shadow is set, the
    /// OBJ pixel resolve sees the pre-transition LCDC.1 value; otherwise
    /// it sees the live `control` bit. The sprite-fetch trigger path
    /// (FEPO/TEKY/SOBU) does NOT go through this accessor.
    pub fn sprites_enabled_for_resolve(&self) -> bool {
        self.sprites_enabled_shadow
            .unwrap_or_else(|| self.control.sprites_enabled())
    }

    /// CPU-write site: capture the pre-write XYLO state into the overlay
    /// shadow if LCDC.1 transitions during Mode 3. `just_set` keeps the
    /// shadow alive across the same-fall `tick_sprites_enabled_shadow`.
    pub fn arm_sprites_enabled_shadow(&mut self, old_value: bool, new_value: bool) {
        if old_value != new_value {
            self.sprites_enabled_shadow = Some(old_value);
            self.sprites_enabled_shadow_just_set = true;
        }
    }

    /// Once-per-fall tick, mirroring `tick_bg_window_enabled_shadow`.
    /// Keeps the shadow alive for the same-fall OBJ-mux resolve, then
    /// clears on the next fall without a fresh LCDC.1 transition.
    pub fn tick_sprites_enabled_shadow(&mut self) {
        if self.sprites_enabled_shadow_just_set {
            self.sprites_enabled_shadow_just_set = false;
        } else {
            self.sprites_enabled_shadow = None;
        }
    }
}
