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

/// CPU → pixel pipeline register file (DFF bank). DFF8/DFF9 write-conflict behaviour during Mode 3 is specific to this group.
pub struct PipelineRegisters {
    pub control: Control,
    /// DFF9 latch for full LCDC byte.
    pub control_latch: DffLatch,
    pub background_viewport: BackgroundViewportPosition,
    pub window: Window,
    pub palettes: Palettes,
    /// VYXE OLD-overlay for mid-Mode-3 LCDC.0 transitions. `just_set` keeps it alive across the same-fall tick.
    pub(in crate::ppu) bg_window_enabled_shadow: Option<bool>,
    pub(in crate::ppu) bg_window_enabled_shadow_just_set: bool,
    /// XYLO popper-side OLD-overlay for mid-Mode-3 LCDC.1 transitions. Sprite-fetch trigger chain sees live XYLO.
    pub(in crate::ppu) sprites_enabled_shadow: Option<bool>,
    pub(in crate::ppu) sprites_enabled_shadow_just_set: bool,
    /// LCDC.1 snapshot taken at start of rise() before staged write applies; consumed by FEPO-for-TEKY (SOBU/CUPA race).
    pub(in crate::ppu) sprites_enabled_pre_cupa: bool,
}

impl PipelineRegisters {
    /// Per-fall work: palette ticks, DFF9 ticks, BESU edge, OLD-overlay shadow ticks.
    pub fn tick_on_master_clock_fall(&mut self, besu: bool) {
        // BGP via wrapper for NURA overlay; OBP0/OBP1 direct.
        self.palettes.tick_background();
        self.palettes.sprite0.tick();
        self.palettes.sprite1.tick();

        // Pipeline reads reg_old; ticks fire after.
        self.background_viewport.x.tick();
        self.background_viewport.y.tick();
        self.window.x_plus_7.tick();
        if self.control_latch.tick() {
            self.control = Control::new(ControlFlags::from_bits_retain(self.control_latch.output));
        }

        // BESU↑ at scanline start releases BGP dlatch post-write recovery.
        self.palettes.tick_besu(besu);

        self.tick_bg_window_enabled_shadow();
        self.tick_sprites_enabled_shadow();
    }

    /// Freeze latches at their current output (LCD off).
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

    /// VYXE state for the BG plane gate (RAJY/TADE), with OLD-overlay applied.
    pub fn bg_window_enabled_for_resolve(&self) -> bool {
        self.bg_window_enabled_shadow
            .unwrap_or_else(|| self.control.background_and_window_enabled())
    }

    /// Capture pre-write VYXE if LCDC.0 transitions during Mode 3.
    pub fn arm_bg_window_enabled_shadow(&mut self, old_value: bool, new_value: bool) {
        if old_value != new_value {
            self.bg_window_enabled_shadow = Some(old_value);
            self.bg_window_enabled_shadow_just_set = true;
        }
    }

    /// CPU write site sets shadow before this fall runs; consume `just_set`, clear on next fall.
    fn tick_bg_window_enabled_shadow(&mut self) {
        if self.bg_window_enabled_shadow_just_set {
            self.bg_window_enabled_shadow_just_set = false;
        } else {
            self.bg_window_enabled_shadow = None;
        }
    }

    /// XYLO state for the OBJ-mux popper, with OLD-overlay applied. Sprite-fetch trigger does NOT use this.
    pub fn sprites_enabled_for_resolve(&self) -> bool {
        self.sprites_enabled_shadow
            .unwrap_or_else(|| self.control.sprites_enabled())
    }

    /// Capture pre-write XYLO if LCDC.1 transitions during Mode 3.
    pub fn arm_sprites_enabled_shadow(&mut self, old_value: bool, new_value: bool) {
        if old_value != new_value {
            self.sprites_enabled_shadow = Some(old_value);
            self.sprites_enabled_shadow_just_set = true;
        }
    }

    fn tick_sprites_enabled_shadow(&mut self) {
        if self.sprites_enabled_shadow_just_set {
            self.sprites_enabled_shadow_just_set = false;
        } else {
            self.sprites_enabled_shadow = None;
        }
    }
}
