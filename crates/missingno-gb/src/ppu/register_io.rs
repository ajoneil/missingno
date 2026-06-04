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
                self.apply_register_write(&register, value);
                self.registers.control_latch.write_immediate(value);

                // Arm the VYXE/sprites-enabled OLD-overlay so the next resolve uses pre-transition.
                // Gated on WUSA so prelude writes (off-LCD first cp_pad↑) are ignored.
                if is_drawing && self.lcd_pushing_active() {
                    let new_bg_window_enabled =
                        self.registers.control.background_and_window_enabled();
                    self.registers
                        .arm_bg_window_enabled_shadow(old_bg_window_enabled, new_bg_window_enabled);
                    let new_sprites_enabled = self.registers.control.sprites_enabled();
                    self.registers
                        .arm_sprites_enabled_shadow(old_sprites_enabled, new_sprites_enabled);
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
                    let glitch_edge = self.video.stat.detect_suko_edge(glitch_legs, false);

                    self.video.stat.write_stat_bits(value);
                    let final_legs = self.stat_legs();
                    let final_edge = self.video.stat.detect_suko_edge(final_legs, false);

                    return glitch_edge || final_edge;
                }

                // CGB: the SUKO line is the OR of written enabled-and-met legs; a STAT
                // write raises an IRQ only on its 0→1 edge (no DMG all-enables transient).
                let prev_low = self.video.stat.legs_was_high().is_empty();
                self.video.stat.write_stat_bits(value);
                let final_legs = self.stat_legs();
                self.video.stat.prime_legs(final_legs);
                return prev_low && !final_legs.is_empty();
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
                self.video.write_lyc(value);
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
