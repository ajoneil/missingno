//! Memory-mapped register read/write.

use super::Ppu;
use super::PpuModel;
use super::Register;
use super::stat_interrupt::InterruptFlags;
use super::types::control::{Control, ControlFlags};

impl<P: PpuModel> Ppu<P> {
    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.registers.control.bits(),
            Register::Status => {
                let mode = match &self.pixel_pipeline {
                    Some(_) => self.mode() as u8,
                    None => 0,
                };
                let line_compare = if self.video.stat.ly_eq_lyc() {
                    0b00000100
                } else {
                    0
                };
                0x80 | (self.video.stat.enables().bits() & 0b01111000) | line_compare | mode
            }
            Register::BackgroundViewportY => self.registers.background_viewport.y.output(),
            Register::BackgroundViewportX => self.registers.background_viewport.x.output(),
            Register::WindowY => self.registers.window.y,
            Register::WindowX => self.registers.window.x.output(),
            Register::CurrentScanline => self.video.ly(),
            Register::InterruptOnScanline => self.video.stat.lyc(),
            Register::BackgroundPalette => self.registers.palettes.background.output(),
            Register::Sprite0Palette => self.registers.palettes.sprite0.output(),
            Register::Sprite1Palette => self.registers.palettes.sprite1.output(),
        }
    }

    pub fn write_register(
        &mut self,
        register: Register,
        value: u8,
        halt_wake_active: bool,
    ) -> bool {
        let is_drawing = self.is_rendering();

        match register {
            Register::BackgroundPalette if halt_wake_active => {
                // BGP write from a HALT-wake handler lands later than running-CPU dispatch — park.
                self.registers
                    .palettes
                    .write_background_halt_wake_deferred(value);
                false
            }
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                self.apply_register_write(&register, value)
            }
            Register::Control => {
                let was_enabled = self.registers.control.video_enabled();
                let old_bg_window_enabled = self.registers.control.background_and_window_enabled();
                let old_sprites_enabled = self.registers.control.sprites_enabled();
                let old_block0_tiles =
                    self.registers.control.bits() & ControlFlags::TILE_ADDRESS_MODE.bits() != 0;
                self.apply_register_write(&register, value);
                self.registers.control_latch.write_immediate(value);

                // The tile-map-select fetch samples LCDC live on DMG; the CGB
                // latches a mid-Mode-3 write onto its own clock and the fetch
                // reads it the crossing's falls late.
                let tile_map_falls = if is_drawing {
                    P::TILE_MAP_CROSSING.write_delayed_falls()
                } else {
                    0
                };
                self.registers.write_tile_map_select(value, tile_map_falls);

                // LCDC.4 (tile-data select) follows the same crossing: live on DMG,
                // the crossing's falls late on the CGB clock.
                let tile_data_falls = if is_drawing {
                    P::TILE_DATA_CROSSING.write_delayed_falls()
                } else {
                    0
                };
                self.registers
                    .write_tile_data_select(value, tile_data_falls);

                // LCDC.2 (OBJ size) follows the same crossing for the sprite fetch:
                // live on DMG, the crossing's falls late on the CGB clock.
                let obj_size_falls = if is_drawing {
                    P::OBJ_SIZE_CROSSING.write_delayed_falls()
                } else {
                    0
                };
                self.registers.write_obj_size_select(value, obj_size_falls);

                if P::TILE_SEL_RESET_GLITCH
                    && old_block0_tiles
                    && value & ControlFlags::TILE_ADDRESS_MODE.bits() == 0
                {
                    self.registers.tile_sel_reset_glitch.arm();
                }

                // Arm the VYXE/sprites-enabled OLD-overlays so the next resolve uses pre-transition.
                // is_drawing already excludes the off-LCD prelude (first cp_pad↑). A CGB enable-lag
                // (RAJY for bg, XYLO for obj) also covers the boundary write before the first pixel
                // is pushed (WUSA still low), holding OLD across the left edge; DMG's combinational
                // paths apply at once, so they keep the WUSA gate.
                let pushing = self.lcd_pushing_active();
                let bg_enable_extra_hold = P::BG_ENABLE_CROSSING.write_delayed_falls();
                if is_drawing && (pushing || bg_enable_extra_hold > 0) {
                    let new_bg_window_enabled =
                        self.registers.control.background_and_window_enabled();
                    self.registers.arm_bg_window_enabled_shadow(
                        old_bg_window_enabled,
                        new_bg_window_enabled,
                        bg_enable_extra_hold,
                    );
                }
                let obj_enable_extra_hold = P::OBJ_ENABLE_CROSSING.write_delayed_falls();
                if is_drawing && (pushing || obj_enable_extra_hold > 0) {
                    let new_sprites_enabled = self.registers.control.sprites_enabled();
                    self.registers.arm_sprites_enabled_shadow(
                        old_sprites_enabled,
                        new_sprites_enabled,
                        obj_enable_extra_hold,
                    );
                }

                // CUPA↑ → XODO↓: schedule divider/scanner reset for this fall.
                if !was_enabled && self.registers.control.video_enabled() {
                    self.lcd_on_init_pending = true;
                }
                false
            }
            Register::WindowX if is_drawing => {
                self.registers.window.x.write(value);
                false
            }
            Register::BackgroundViewportY
                if is_drawing && P::SCY_CROSSING.write_delayed_falls() > 0 =>
            {
                // CGB latches the mid-Mode-3 SCY write onto its own clock; the BG
                // fetch samples it the crossing's falls late. On DMG the crossing
                // is combinational — this guard folds to false and the write takes
                // the immediate path below.
                self.registers
                    .background_viewport
                    .y
                    .write_delayed(value, P::SCY_CROSSING.write_delayed_falls());
                false
            }
            Register::BackgroundViewportX if is_drawing => {
                self.registers.background_viewport.x.write(value);
                false
            }
            _ => self.apply_register_write(&register, value),
        }
    }

    /// Returns true only on the STAT-write DMG glitch path — momentarily all enables go high
    /// before settling to `value`, which may raise the STAT line and request an interrupt.
    /// All other registers return false (writes never produce a same-tick STAT edge).
    fn apply_register_write(&mut self, register: &Register, value: u8) -> bool {
        match register {
            Register::Control => {
                self.registers.control = Control::new(ControlFlags::from_bits_retain(value))
            }
            Register::Status => {
                if P::STAT_WRITE_ALL_ENABLES_GLITCH {
                    // DMG STAT write glitch: all enables briefly high, then settle.
                    self.video.stat.set_enables(InterruptFlags::all());
                    let glitch_legs = self.stat_legs();
                    let glitch_edge = self.video.stat.detect_suko_edge(glitch_legs);

                    self.video
                        .stat
                        .write_stat_bits(value, self.model.stat_shadow_mut());
                    let final_legs = self.stat_legs();
                    let final_edge = self.video.stat.detect_suko_edge(final_legs);

                    return glitch_edge || final_edge;
                }

                // CGB: the cells update now (readback is write-time); the
                // STAT-IRQ block sees them at the next M-cycle-clock capture —
                // a write never produces a same-tick edge.
                self.video.stat.write_stat_bits_cell(value);
                return false;
            }
            Register::BackgroundViewportY => {
                self.registers.background_viewport.y.write_immediate(value)
            }
            Register::BackgroundViewportX => {
                self.registers.background_viewport.x.write_immediate(value)
            }
            Register::WindowY => self.registers.window.y = value,
            Register::WindowX => self.registers.window.x.write_immediate(value),
            Register::InterruptOnScanline => {
                if P::LYC_CROSSING.is_synced() {
                    self.video.stat.write_lyc_cell(value);
                } else {
                    self.video.write_lyc(value, self.model.stat_shadow_mut());
                }
            }
            Register::BackgroundPalette => {
                if self.registers.control.video_enabled() {
                    self.registers.palettes.background.write(value)
                } else {
                    self.registers.palettes.background.write_immediate(value)
                }
            }
            Register::Sprite0Palette => {
                if self.registers.control.video_enabled() {
                    self.registers.palettes.sprite0.write(value)
                } else {
                    self.registers.palettes.sprite0.write_immediate(value)
                }
            }
            Register::Sprite1Palette => {
                if self.registers.control.video_enabled() {
                    self.registers.palettes.sprite1.write(value)
                } else {
                    self.registers.palettes.sprite1.write_immediate(value)
                }
            }
            Register::CurrentScanline => {}
        }
        false
    }
}
