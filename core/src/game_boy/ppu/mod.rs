use pixel_pipeline::Mode;
use screen::Screen;
use sprites::{Sprite, SpriteId};

use control::{Control, ControlFlags};
use memory::{Oam, OamAddress, Vram};
use palette::Palettes;
use pixel_pipeline::{FramePhase, Rendering};
use registers::BackgroundViewportPosition;

pub use dff::DffLatch;
pub use pixel_pipeline::{FetcherStep, PipelineSnapshot, RenderPhase, SpriteFetchPhase};
pub use registers::{PipelineRegisters, Window};
pub use video_control::{InterruptFlags, VideoControl};

pub struct PpuTickResult {
    pub screen: Option<Screen>,
    pub request_vblank: bool,
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
                control_latch: DffLatch::new(control.bits()),
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
            // Post-boot PPU state: internal line 153, LX=100, VBlank.
            // ly() returns 0 (MYTA early reset), matching DMG post-boot LY=0.
            // WUVU/VENA at TALU-rising state (LX just incremented).
            video: VideoControl {
                lx: 100,
                wuvu: false,
                vena: true,
                ly: 153,
                lyc: 0,
                ly_match_pending: true,
                ly_eq_lyc: true,
                // The first bit is unused, but is set at boot time
                stat_flags: InterruptFlags::DUMMY,
                stat_line_was_high: false,
            },
            oam: Oam::new(),
            pixel_pipeline: Some(FramePhase::VerticalBlank),
        }
    }

    pub fn lx(&self) -> u8 {
        self.video.lx
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
                // LCDC uses combinational reads on hardware — the fetcher's
                // VRAM address logic reads reg_new.reg_lcdc with zero delay
                // after the DFF9 latches. No propagation delay needed.
                self.write_register_immediate(&register, value);
                self.registers.control_latch.write_immediate(value);
                false
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
            && self.video.lx == 0;

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

    /// Rising edge (DELTA_ODD): DFF8 palette latch advance, LCD
    /// initialization, pixel output pipeline (SACU, pipe shift).
    ///
    /// DFF8 palette latches tick first so the pipeline sees the
    /// transitional old|new value on the write dot, matching DFF8
    /// master-slave transparency.
    pub fn tcycle_rising(&mut self, vram: &Vram) {
        if !self.control().video_enabled() {
            return;
        }

        if self.pixel_pipeline.is_none() {
            // WUVU/VENA come out of async reset at qp=0. The CPU write to LCDC
            // takes effect at DELTA_GH. The combination of divider phase offset
            // (4 dots) and write timing within the M-cycle (4 dots) produces an
            // 8-dot shortening of the first scanline (448 dots instead of 456).
            //
            // Model: start LX at 2 with phase 0. This skips 8 dots (2 LX values
            // x 4 dots/LX), so LX=113 is reached after 444 dots from LX=2's
            // perspective, giving 444 + 8 skipped = ~448 total dots for the first
            // scanline as seen by the CPU.
            self.video.lx = 2;
            // WUVU/VENA start at qp=false after VID_RST deasserts.
            self.video.wuvu = false;
            self.video.vena = false;
            self.video.write_ly(0);
            self.pixel_pipeline = Some(FramePhase::new_lcd_on());
        }

        // Advance DFF8 palette latches before pixel output.
        self.registers.tick_palette_latches();

        // Pixel output, SACU, pipe shift.
        if let Some(pipeline) = self.pixel_pipeline.as_mut() {
            pipeline.tcycle_rising(&self.registers, &self.video, &self.oam, vram);
        }
    }

    /// Master clock tick (XOTA rising edge). Runs the WUVU/VENA divider
    /// chain, LX counter, scanline boundary logic, VBlank IF, and LYC
    /// comparison. Called once per dot by the executor, independent of
    /// the rising/falling half-phase split — on hardware, the master
    /// clock is not part of either half-phase.
    pub fn tick_xota(&mut self, is_mcycle: bool) -> PpuTickResult {
        let mut result = PpuTickResult {
            screen: None,
            request_vblank: false,
        };

        if !self.control().video_enabled() {
            // WUVU/VENA are held in async reset by VID_RST while LCD
            // is disabled. Nothing to tick.
            return result;
        }

        if self.pixel_pipeline.is_none() {
            return result;
        }

        if self.video.tick_xota() {
            // Scanline boundary — LX wrapped to 0.
            match self.pixel_pipeline.as_mut() {
                Some(FramePhase::ActiveDisplay(rendering)) => {
                    if self.video.ly() == screen::NUM_SCANLINES {
                        result.screen = Some(rendering.screen.clone());
                        self.pixel_pipeline = Some(FramePhase::VerticalBlank);
                    } else {
                        rendering.reset_scanline(self.video.ly());
                    }
                }
                Some(FramePhase::VerticalBlank) => {
                    if self.video.ly == 0 {
                        self.pixel_pipeline = Some(FramePhase::ActiveDisplay(Rendering::new()));
                    }
                }
                None => {}
            }
        }

        // NYPE→POPU pipeline: VBlank IF fires at dot 4 of line 144,
        // not at the scanline boundary (dot 0).
        if self.video.lx == 1
            && self.video.talu()
            && !self.video.wuvu
            && self.video.ly() == 144
            && matches!(self.pixel_pipeline, Some(FramePhase::VerticalBlank))
        {
            result.request_vblank = true;
        }

        // M-cycle-rate LYC comparison — AFTER tick_xota so the
        // comparison sees post-increment LY, BEFORE STAT edge detection
        // so interrupts see the freshly promoted ly_eq_lyc.
        if is_mcycle {
            self.video.latch_ly_comparison();
        }

        result
    }

    /// Falling edge (DELTA_EVEN): fetcher pipeline (advance, cascade DFFs,
    /// TYFA), DFF9 resolve, LCD-off handling.
    pub fn tcycle_falling(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            screen: None,
            request_vblank: false,
        };

        if self.control().video_enabled() {
            // When video is enabled but the pipeline hasn't been created yet
            // (LCDC was just written, rising phase hasn't run), skip all
            // work. The pipeline is initialized on the next rising phase.
            if self.pixel_pipeline.is_none() {
                return result;
            }

            // Fetcher advance, cascade DFFs (NYKA/PORY/PYGO), TYFA.
            if let Some(pipeline) = self.pixel_pipeline.as_mut() {
                pipeline.tcycle_falling(&self.registers, &self.video, &self.oam, vram);
            }

            // Advance DFF9 register latches after the pipeline so it reads
            // pre-tick values (reg_old), matching hardware.
            self.registers.tick_register_latches();

            // STAT edge detection moved to check_stat_edge() — called
            // after each phase by the executor, matching hardware's
            // combinational SUKO which fires on any phase.
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

    /// Detect a rising edge on the STAT interrupt line (SUKO).
    /// On hardware, SUKO is purely combinational — it can fire on
    /// any phase where an enabled condition transitions from inactive
    /// to active. The caller invokes this after each phase tick so
    /// that edges from the rising phase (e.g. WODU/Mode 0) are not
    /// deferred to the next falling phase.
    ///
    /// Only evaluates when the LCD is enabled. When LCD is off, SUKO's
    /// inputs (TARU, TAPA, PARU, ROPO) retain their static values and
    /// the latch state freezes — matching hardware where the DFF outputs
    /// persist without a clock.
    pub fn check_stat_edge(&mut self) -> bool {
        if !self.control().video_enabled() {
            return false;
        }
        let stat_line_high = self.stat_line_active();
        let edge = stat_line_high && !self.video.stat_line_was_high;
        self.video.stat_line_was_high = stat_line_high;
        edge
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
