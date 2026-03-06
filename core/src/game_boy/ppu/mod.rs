use pixel_pipeline::Mode;
use screen::Screen;
use sprites::{Sprite, SpriteId};

use control::{Control, ControlFlags};
use dff::LatchState;
use memory::{Oam, OamAddress, Vram};
use palette::Palettes;
use pixel_pipeline::{FramePhase, Rendering};
use registers::BackgroundViewportPosition;

pub use dff::DffLatch;
pub use pixel_pipeline::{
    FetcherStep, FetcherTick, PipelineSnapshot, RenderPhase, SpriteFetchPhase,
};
pub use registers::{PipelineRegisters, Window};
pub use video_control::{InterruptFlags, VideoControl};

pub struct PpuTickResult {
    pub screen: Option<Screen>,
    pub request_vblank: bool,
    pub request_stat: bool,
}

pub mod control;
mod dff;
pub mod memory;
mod oam_corruption;
pub mod palette;
pub mod pixel_pipeline;
mod registers;
pub mod screen;
pub mod sprites;
pub mod tile_maps;
pub mod tiles;
mod video_control;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    Control,
    Status,
    BackgroundViewportY,
    BackgroundViewportX,
    WindowY,
    WindowX,
    CurrentScanline,
    InterruptOnScanline,
    BackgroundPalette,
    Sprite0Palette,
    Sprite1Palette,
}

pub struct Ppu {
    pixel_pipeline: Option<FramePhase>,
    registers: PipelineRegisters,
    video: VideoControl,
    pub(super) oam: Oam,
}

impl Ppu {
    pub fn new() -> Self {
        let control = Control::default();
        Self {
            registers: PipelineRegisters {
                control_bg_en: DffLatch::new(
                    control.bits() & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits(),
                ),
                control,
                background_viewport: BackgroundViewportPosition {
                    x: DffLatch::new(0),
                    y: DffLatch::new(0),
                },
                window: Window {
                    y: 0,
                    x_plus_7: DffLatch::new(0),
                },
                palettes: Palettes::default(),
            },
            video: VideoControl {
                dot: 0,
                ly: 0,
                lyc: 0,
                ly_eq_lyc: true,
                // The first bit is unused, but is set at boot time
                stat_flags: InterruptFlags::DUMMY,
                stat_line_was_high: false,
            },
            oam: Oam::new(),
            pixel_pipeline: Some(FramePhase::new()),
        }
    }

    pub fn dot(&self) -> u32 {
        self.video.dot()
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.registers.control.bits(),
            Register::Status => {
                let mode = if let Some(ppu) = &self.pixel_pipeline {
                    ppu.stat_mode(&self.video) as u8
                } else {
                    0
                };
                let line_compare = if self.video.ly_eq_lyc() {
                    0b00000100
                } else {
                    0
                };
                0x80 | (self.video.stat_flags.bits() & 0b01111000) | line_compare | mode
            }
            Register::BackgroundViewportY => self.registers.background_viewport.y.output(),
            Register::BackgroundViewportX => self.registers.background_viewport.x.output(),
            Register::WindowY => self.registers.window.y,
            Register::WindowX => self.registers.window.x_plus_7.output(),
            Register::CurrentScanline => self.video.ly(),
            Register::InterruptOnScanline => self.video.lyc,
            Register::BackgroundPalette => self.registers.palettes.background.output(),
            Register::Sprite0Palette => self.registers.palettes.sprite0.output(),
            Register::Sprite1Palette => self.registers.palettes.sprite1.output(),
        }
    }

    /// Write a value directly to the register backing store.
    ///
    /// Returns true if the write triggered a STAT interrupt request
    /// (DMG STAT write quirk: writing to FF41 briefly sets all enable
    /// bits high, which can produce a rising edge on the STAT line).
    fn write_register_immediate(&mut self, register: &Register, value: u8) -> bool {
        match register {
            Register::Control => {
                self.registers.control = Control::new(ControlFlags::from_bits_retain(value))
            }
            Register::Status => {
                // DMG STAT write quirk: briefly set all enable bits high.
                // If any condition is active, this produces a rising edge.
                self.video.stat_flags = InterruptFlags::all();
                let glitch_line = self.stat_line_active();
                let glitch_edge = glitch_line && !self.video.stat_line_was_high;
                self.video.stat_line_was_high = glitch_line;

                // Now apply the real value.
                self.video.stat_flags = InterruptFlags::from_bits_truncate(value);
                let final_line = self.stat_line_active();
                let final_edge = final_line && !self.video.stat_line_was_high;
                self.video.stat_line_was_high = final_line;

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
            Register::InterruptOnScanline => self.video.lyc = value,
            Register::BackgroundPalette => {
                self.registers.palettes.background.write_immediate(value)
            }
            Register::Sprite0Palette => self.registers.palettes.sprite0.write_immediate(value),
            Register::Sprite1Palette => self.registers.palettes.sprite1.write_immediate(value),
            Register::CurrentScanline => {} // writes to LY are ignored on DMG
        }
        false
    }

    pub fn write_register(&mut self, register: Register, value: u8, _vram: &Vram) -> bool {
        let is_drawing = self
            .pixel_pipeline
            .as_ref()
            .map_or(false, |p| p.is_rendering());

        match register {
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                if is_drawing {
                    let latch = match register {
                        Register::BackgroundPalette => &mut self.registers.palettes.background,
                        Register::Sprite0Palette => &mut self.registers.palettes.sprite0,
                        Register::Sprite1Palette => &mut self.registers.palettes.sprite1,
                        _ => unreachable!(),
                    };
                    latch.write_dff8(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::Control => {
                if is_drawing {
                    // LCDC is DFF9: bits 1-7 latch atomically. Only BG_EN
                    // (bit 0) has a transitional `old | new` phase.
                    let old_bg_en = self.registers.control.bits()
                        & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits();
                    let new_bg_en = value & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits();
                    let transitional_bg_en = old_bg_en | new_bg_en;
                    let immediate = (value & !ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits())
                        | transitional_bg_en;
                    self.write_register_immediate(&Register::Control, immediate);
                    self.registers.control_bg_en.output = immediate;
                    self.registers.control_bg_en.state =
                        Some(LatchState::Transitional { final_value: value });
                    false
                } else {
                    self.write_register_immediate(&register, value);
                    self.registers
                        .control_bg_en
                        .write_immediate(value & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits());
                    false
                }
            }
            Register::BackgroundViewportY => {
                if is_drawing {
                    self.registers
                        .background_viewport
                        .y
                        .write_propagating(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::BackgroundViewportX => {
                if is_drawing {
                    self.registers
                        .background_viewport
                        .x
                        .write_propagating(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::WindowX => {
                if is_drawing {
                    self.registers.window.x_plus_7.write_propagating(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            _ => {
                // Remaining DFF9 registers: no propagation delay, atomic
                // latch at the write point (G→H boundary).
                self.write_register_immediate(&register, value)
            }
        }
    }

    pub fn read_oam(&self, address: OamAddress) -> u8 {
        self.oam.read(address)
    }

    pub fn write_oam(&mut self, address: OamAddress, value: u8) {
        self.oam.write(address, value);
    }

    pub fn mode(&self) -> pixel_pipeline::Mode {
        if let Some(ppu) = &self.pixel_pipeline {
            ppu.mode(&self.video)
        } else {
            pixel_pipeline::Mode::VerticalBlank
        }
    }

    pub fn oam_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.oam_locked())
    }

    pub fn vram_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.vram_locked())
    }

    pub fn oam_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.oam_write_locked())
    }

    pub fn vram_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.vram_write_locked())
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    pub fn is_rendering(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |p| p.is_rendering())
    }

    fn stat_line_active(&self) -> bool {
        let ppu = match &self.pixel_pipeline {
            Some(ppu) => ppu,
            None => return false,
        };

        let mode = ppu.interrupt_mode(&self.video);

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts.
        let vblank_line_144 = matches!(ppu, FramePhase::VerticalBlank)
            && self.video.ly() == 144
            && self.video.dot() < 4;

        // Mode 0 interrupt fires on the actual mode transition, not the
        // early stat_mode prediction (which is only for STAT register reads).
        (self
            .video
            .stat_flags
            .contains(InterruptFlags::HORIZONTAL_BLANK)
            && mode == Mode::HorizontalBlank)
            || (self
                .video
                .stat_flags
                .contains(InterruptFlags::VERTICAL_BLANK)
                && mode == Mode::VerticalBlank)
            || (self.video.stat_flags.contains(InterruptFlags::OAM_SCAN)
                && (ppu.mode2_interrupt_active(&self.video) || vblank_line_144))
            || (self
                .video
                .stat_flags
                .contains(InterruptFlags::CURRENT_LINE_COMPARE)
                && self.video.ly_eq_lyc())
    }

    /// DELTA_EVEN phase: DFF latch advance and pixel pipeline even-phase
    /// setup (fetcher control, mode transitions).
    pub fn tcycle_even(&mut self, vram: &Vram) {
        if !self.control().video_enabled() {
            return;
        }

        if self.pixel_pipeline.is_none() {
            self.video.dot = 0;
            self.video.write_ly(0);
            self.pixel_pipeline = Some(FramePhase::new_lcd_on());
        }

        // Advance DFF latches before pixel output.
        self.registers.tick_latches();

        self.pixel_pipeline
            .as_mut()
            .unwrap()
            .tcycle_even(&self.registers, &self.video, vram);
    }

    /// DELTA_ODD phase: pixel output, counter increment, M-cycle-rate
    /// interrupt edge detection and LYC comparison.
    pub fn tcycle_odd(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            screen: None,
            request_vblank: false,
            request_stat: false,
        };

        if self.control().video_enabled() {
            // When video is enabled but the pipeline hasn't been created yet
            // (LCDC was just written, tcycle_even hasn't run), skip all
            // M-cycle-rate work. The comparison clock and edge detector
            // don't start until the pipeline is initialized.
            if self.pixel_pipeline.is_none() {
                return result;
            }

            if let Some(pipeline) = self.pixel_pipeline.as_mut() {
                pipeline.tcycle_odd(&self.registers, &self.video, &self.oam, vram);
            }

            if self.video.advance_dot() {
                // Scanline boundary — dot counter wrapped to 0.
                match self.pixel_pipeline.as_mut() {
                    Some(FramePhase::ActiveDisplay(rendering)) => {
                        if self.video.ly() == screen::NUM_SCANLINES {
                            result.screen = Some(rendering.screen.clone());
                            result.request_vblank = true;
                            self.pixel_pipeline = Some(FramePhase::VerticalBlank);
                        } else {
                            rendering.reset_scanline();
                        }
                    }
                    Some(FramePhase::VerticalBlank) => {
                        if self.video.ly() == 0 {
                            self.pixel_pipeline = Some(FramePhase::ActiveDisplay(Rendering::new()));
                        }
                    }
                    None => {}
                }
            }

            // Detect rising edge of STAT interrupt line (runs every dot,
            // matching hardware's SUKO-clocked DFF which has phase granularity)
            let stat_line_high = self.stat_line_active();
            if stat_line_high && !self.video.stat_line_was_high {
                result.request_stat = true;
            }
            self.video.stat_line_was_high = stat_line_high;

            if !is_mcycle {
                return result;
            }

            // Update comparison clock (runs while PPU is on, M-cycle rate)
            self.video.latch_ly_comparison();
        } else {
            if !is_mcycle {
                return result;
            }
            if self.pixel_pipeline.is_some() {
                self.pixel_pipeline = None;
                self.registers.clear_latches();
                result.screen = Some(Screen::new());
            }
            // ly_eq_lyc is intentionally NOT updated — comparison clock
            // stops when the PPU is off, freezing the last result.
            return result;
        }

        result
    }

    /// Advance PPU by one dot. Call once per T-cycle.
    ///
    /// STAT interrupt edge detection runs every dot. LYC comparison
    /// only runs on M-cycle boundaries (when `is_mcycle` is true).
    pub fn tcycle(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        self.tcycle_even(vram);
        self.tcycle_odd(is_mcycle, vram)
    }

    pub fn palettes(&self) -> &Palettes {
        &self.registers.palettes
    }

    pub fn sprite(&self, sprite: SpriteId) -> &Sprite {
        self.oam.sprite(sprite)
    }

    pub fn pipeline_state(&self) -> Option<PipelineSnapshot> {
        self.pixel_pipeline
            .as_ref()
            .and_then(|p| p.pipeline_state())
    }
}
