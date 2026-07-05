//! Memory-mapped register read/write.

use super::Ppu;
use super::PpuModel;
use super::Register;
use super::TileSelGlitch;
use super::crossing::CaptureSpec;
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
        edge_carries_dot_fall: bool,
    ) -> bool {
        let is_drawing = self.is_rendering();

        match register {
            Register::BackgroundPalette if halt_wake_active => {
                // BGP write from a HALT-wake handler lands later than running-CPU dispatch — park.
                self.registers
                    .palettes
                    .write_background_halt_wake_deferred(value);
                self.registers.arm_halt_wake_bgp_settle();
                false
            }
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                self.registers.arm_register_write_settle();
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

                // Each LCDC-fetch view samples live on DMG; the CGB latches a
                // mid-Mode-3 write onto its own clock, read the crossing's falls
                // late (LCDC.3/.6 tile-map, LCDC.4 tile-data, LCDC.2 obj-size).
                // At double speed the write's CUPA-rising shares its master edge with
                // a PPU dot-fall (the crossing cell's capture edge), so the cell
                // captures the write on that coincident fall — one fall sooner than at
                // single speed, where the write lands on a dot-rise and the first
                // capture is the following dot's fall.
                let crossing_falls = |spec: CaptureSpec| {
                    let falls = if is_drawing {
                        spec.write_delayed_falls()
                    } else {
                        0
                    };
                    if falls > 0 && edge_carries_dot_fall {
                        falls - 1
                    } else {
                        falls
                    }
                };
                self.registers
                    .write_tile_map_select(value, crossing_falls(P::TILE_MAP_CROSSING));
                // ff40_d4 settles fast on a 0->1 SET (~1.4 ge), so it reaches the
                // counter-2 low read within the write dot — both planes take the new
                // block, as the DMG renders the whole band. On a sprite line the fetch
                // phase collides with the read (the set glitch below); keep the slow
                // crossing so the low read stays OLD for the glitch to substitute.
                let tile_data_set_fast = !old_block0_tiles
                    && value & ControlFlags::TILE_ADDRESS_MODE.bits() != 0
                    && !self.sprite_on_line();
                let tile_data_falls = if tile_data_set_fast {
                    crossing_falls(P::TILE_DATA_CROSSING).min(1)
                } else {
                    crossing_falls(P::TILE_DATA_CROSSING)
                };
                self.registers
                    .write_tile_data_select(value, tile_data_falls);
                self.registers
                    .write_obj_size_select(value, crossing_falls(P::OBJ_SIZE_CROSSING));

                // The reset-substitution is a same-edge bus race: the tile index
                // reaches the bitplane read only when the reset write's crossing
                // edge is the read's dot-rise sample. At double speed the crossing
                // lands on the dot-fall (mux settled before the next read) — no glitch.
                let tile_sel_reset = old_block0_tiles
                    && value & ControlFlags::TILE_ADDRESS_MODE.bits() == 0
                    && !edge_carries_dot_fall;
                if P::TILE_SEL_RESET_GLITCH && tile_sel_reset {
                    self.model.tile_sel_glitch_mut().arm();
                }

                // The SET-direction sibling substitutes the tile-data bus's frozen
                // glitch source into the bitplane read the SET lands on. The source
                // is refreshed by two events: a TILE_SEL reset (CLEAR) snapshots the
                // BG tile fetched next; a 0->1 SET at an odd BG fetch counter then
                // corrupts the following read (counter 1 → low, counter 3 → high).
                if P::TILE_SEL_SET_GLITCH && is_drawing {
                    if let Some(rendering) = self.pixel_pipeline.as_mut() {
                        if tile_sel_reset {
                            rendering.arm_bg_glitch_capture();
                        } else if !old_block0_tiles
                            && value & ControlFlags::TILE_ADDRESS_MODE.bits() != 0
                            && !edge_carries_dot_fall
                        {
                            rendering.arm_bg_set_glitch();
                        }
                    }
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
                self.registers.arm_register_write_settle();
                false
            }
            Register::WindowX if is_drawing => {
                self.registers.window.x.write(value);
                self.registers.arm_register_write_settle();
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
                self.registers.arm_register_write_settle();
                false
            }
            Register::BackgroundViewportX if is_drawing => {
                self.registers.background_viewport.x.write(value);
                self.registers.arm_register_write_settle();
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
