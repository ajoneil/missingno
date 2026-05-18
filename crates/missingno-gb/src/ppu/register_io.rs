//! Memory-mapped register read/write — the CPU's interface to the PPU.

use super::Ppu;
use super::Register;
use super::stat_interrupt::InterruptFlags;
use super::types::control::{Control, ControlFlags};

impl Ppu {
    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.registers.control.bits(),
            Register::Status => {
                let mode = match &self.pixel_pipeline {
                    Some(_) => self.mode() as u8,
                    None => 0,
                };
                let line_compare = if self.video.stat.ly_eq_lyc_stat() {
                    0b00000100
                } else {
                    0
                };
                0x80 | (self.video.stat.enables().bits() & 0b01111000) | line_compare | mode
            }
            Register::BackgroundViewportY => self.registers.background_viewport.y.output(),
            Register::BackgroundViewportX => self.registers.background_viewport.x.output(),
            Register::WindowY => self.registers.window.y,
            Register::WindowX => self.registers.window.x_plus_7.output(),
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
                // BGP CUPA from a HALT-wake handler lands several LCD
                // columns later than running-CPU dispatch. Park the
                // value; tick_background commits it after the countdown.
                self.registers
                    .palettes
                    .write_background_halt_wake_deferred(value);
                false
            }
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                // DFF8 staging inside DffLatch — apply_register_write
                // sets pending, tick_palette_latches commits next fall.
                self.apply_register_write(&register, value)
            }
            Register::Control => {
                let was_enabled = self.registers.control.video_enabled();
                let old_bg_window_enabled = self.registers.control.background_and_window_enabled();
                let old_sprites_enabled = self.registers.control.sprites_enabled();
                self.apply_register_write(&register, value);
                self.registers.control_latch.write_immediate(value);

                // VYXE / sprites_enabled mid-Mode-3 first-cp_pad↑-samples-
                // OLD overlay: arm the shadow so the next BG resolve uses
                // the pre-transition value. Gated on WUSA so prelude writes
                // (where the first cp_pad↑ lands off-LCD) are ignored.
                if is_drawing && self.lcd_pushing_active() {
                    let new_bg_window_enabled =
                        self.registers.control.background_and_window_enabled();
                    self.registers
                        .arm_bg_window_enabled_shadow(old_bg_window_enabled, new_bg_window_enabled);
                    let new_sprites_enabled = self.registers.control.sprites_enabled();
                    self.registers
                        .arm_sprites_enabled_shadow(old_sprites_enabled, new_sprites_enabled);
                }

                // CUPA↑ → XODO↓ is combinational; schedule the matching
                // divider/scanner reset for this fall.
                if !was_enabled && self.registers.control.video_enabled() {
                    self.lcd_on_init_pending = true;
                }
                false
            }
            Register::WindowX if is_drawing => {
                self.registers.window.x_plus_7.write(value);
                false
            }
            _ => self.apply_register_write(&register, value),
        }
    }

    /// Apply a register write to its backing store. Returns true if
    /// the write produced a STAT rising edge (the DMG STAT write
    /// glitch can transiently raise all enable bits).
    fn apply_register_write(&mut self, register: &Register, value: u8) -> bool {
        match register {
            Register::Control => {
                self.registers.control = Control::new(ControlFlags::from_bits_retain(value))
            }
            Register::Status => {
                // DMG STAT write glitch: briefly raise all enables, then
                // settle to the real value. Either transition can produce
                // a STAT rising edge.
                self.video.stat.set_enables(InterruptFlags::all());
                let glitch_line = self.stat_line();
                let glitch_edge = self.video.stat.detect_line_edge(glitch_line);

                self.video.stat.write_stat_bits(value);
                let final_line = self.stat_line();
                let final_edge = self.video.stat.detect_line_edge(final_line);

                return glitch_edge || final_edge;
            }
            Register::BackgroundViewportY => {
                self.registers.background_viewport.y.write_immediate(value)
            }
            Register::BackgroundViewportX => {
                self.registers.background_viewport.x.write_immediate(value)
            }
            Register::WindowY => self.registers.window.y = value,
            Register::WindowX => self.registers.window.x_plus_7.write_immediate(value),
            Register::InterruptOnScanline => {
                self.video.write_lyc(value);
            }
            Register::BackgroundPalette => self.registers.palettes.background.write(value),
            Register::Sprite0Palette => self.registers.palettes.sprite0.write(value),
            Register::Sprite1Palette => self.registers.palettes.sprite1.write(value),
            Register::CurrentScanline => {}
        }
        false
    }
}
