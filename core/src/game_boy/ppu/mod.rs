use pixel_pipeline::Mode;
use screen::Screen;
use sprites::{Sprite, SpriteId};

use control::{Control, ControlFlags};
use memory::{Oam, OamAddress, Vram};
use palette::Palettes;
use pixel_pipeline::Rendering;
use registers::BackgroundViewportPosition;

pub use dff::DffLatch;
pub use pixel_pipeline::{FetcherStep, PipelineSnapshot, SpriteFetchPhase};
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
    /// Pixel pipeline state. None = LCD off (VID_RST asserted, circuits
    /// held in reset). Some = LCD on — the pipeline persists through both
    /// active display and VBlank, matching hardware where these circuits
    /// are always present. VBlank vs active display is derived from
    /// `video.in_vblank()`.
    pixel_pipeline: Option<Rendering>,
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
            // WUVU/VENA phase: TALU rises at dot 1 (phase C), matching hardware.
            video: VideoControl {
                lx: 100,
                wuvu: false,
                vena: false,
                ly: 153,
                lyc: 0,
                ly_match_pending: true,
                ly_eq_lyc: true,
                // The first bit is unused, but is set at boot time
                stat_flags: InterruptFlags::DUMMY,
                stat_line_was_high: false,
                nype: false,
                rutu_old: false,
            },
            oam: Oam::new(),
            // Pipeline persists through VBlank — video.ly=153 means
            // in_vblank() is true, so pipeline ticking is gated off.
            pixel_pipeline: Some(Rendering::new()),
        }
    }

    /// Power-on state: LCD off, all registers zeroed.
    pub fn power_on() -> Self {
        let control = Control::new(ControlFlags::empty());
        Self {
            registers: PipelineRegisters {
                control_latch: DffLatch::new(0),
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
                lx: 0,
                wuvu: false,
                vena: false,
                ly: 0,
                lyc: 0,
                ly_match_pending: false,
                ly_eq_lyc: true,
                stat_flags: InterruptFlags::empty(),
                stat_line_was_high: false,
                nype: false,
                rutu_old: false,
            },
            oam: Oam::new(),
            pixel_pipeline: None, // LCD off at power-on
        }
    }

    pub fn lx(&self) -> u8 {
        self.video.lx
    }

    /// Current OAM scan counter entry (0-39). Returns None when not rendering.
    pub fn scan_counter(&self) -> Option<u8> {
        self.pixel_pipeline.as_ref().map(|r| r.scan_counter_entry())
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.registers.control.bits(),
            Register::Status => {
                let mode = match &self.pixel_pipeline {
                    Some(rendering) if !self.video.in_vblank() => {
                        rendering.stat_mode(&self.video) as u8
                    }
                    Some(_) => Mode::VerticalBlank as u8,
                    None => 0,
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

    /// Initialize the PPU when LCDC bit 7 transitions from 0 to 1.
    ///
    /// On hardware, VID_RST deasserts at G→H (XOTA falling). The very
    /// next XOTA rising edge (H→A) is only 0.5 dots later — within the
    /// same falling half-phase boundary. All dividers start at qp=0
    /// (async reset). WUVU toggles to 1 on that first H→A edge before
    /// any emulator-visible work happens, so we initialize wuvu=true
    /// to capture that sub-dot toggle's net effect.
    fn initialize_lcd_on(&mut self) {
        self.video.lx = 0;
        self.video.wuvu = true;
        self.video.vena = false;
        self.video.write_ly(0);
        self.video.nype = false;
        self.video.rutu_old = false;

        // Create the pixel pipeline (VID_RST released).
        self.pixel_pipeline = Some(Rendering::new());
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.start_scanning();
        }

        // Sync the STAT edge detector: the STAT line and its edge detector
        // reach their new steady state simultaneously when VID_RST deasserts.
        // No false edge on the first evaluation.
        self.video.stat_line_was_high = self.stat_line_active();
    }

    pub fn write_register(&mut self, register: Register, value: u8, _vram: &Vram) -> bool {
        let is_drawing = self.is_rendering();

        match register {
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                if is_drawing {
                    let latch = match register {
                        Register::BackgroundPalette => &mut self.registers.palettes.background,
                        Register::Sprite0Palette => &mut self.registers.palettes.sprite0,
                        Register::Sprite1Palette => &mut self.registers.palettes.sprite1,
                        _ => unreachable!(),
                    };
                    latch.write(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::Control => {
                let was_enabled = self.registers.control.video_enabled();
                // LCDC uses combinational reads on hardware — the fetcher's
                // VRAM address logic reads reg_new.reg_lcdc with zero delay
                // after the DFF9 latches. No propagation delay needed.
                self.write_register_immediate(&register, value);
                self.registers.control_latch.write_immediate(value);

                // VID_RST deasserts when bit 7 transitions 0→1.
                if !was_enabled && self.registers.control.video_enabled() {
                    self.initialize_lcd_on();
                }
                false
            }
            Register::BackgroundViewportY => {
                if is_drawing {
                    self.registers.background_viewport.y.write(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::BackgroundViewportX => {
                if is_drawing {
                    self.registers.background_viewport.x.write(value);
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::WindowX => {
                if is_drawing {
                    self.registers.window.x_plus_7.write(value);
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
        match &self.pixel_pipeline {
            Some(rendering) if !self.video.in_vblank() => rendering.mode(&self.video),
            Some(_) => Mode::VerticalBlank,
            None => Mode::VerticalBlank,
        }
    }

    pub fn oam_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.in_vblank() => r.oam_locked(),
            _ => false,
        }
    }

    pub fn vram_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.in_vblank() => r.vram_locked(),
            _ => false,
        }
    }

    pub fn oam_write_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.in_vblank() => r.oam_write_locked(),
            _ => false,
        }
    }

    pub fn vram_write_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.in_vblank() => r.vram_write_locked(),
            _ => false,
        }
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    pub fn is_rendering(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.in_vblank() => r.xymu,
            _ => false,
        }
    }

    fn stat_line_active(&self) -> bool {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return false,
        };

        let in_vblank = self.video.in_vblank();

        // On hardware, Mode 1 STAT fires at clock 4 of line 144, not clock 0.
        let mode = if in_vblank && self.video.ly() == 144 && self.video.lx == 0 {
            Mode::HorizontalBlank
        } else {
            self.mode()
        };

        // Mode 2 interrupt active: during VBlank, never.
        // Otherwise delegate to the rendering pipeline's TAPA signal.
        let mode2_active = if in_vblank {
            false
        } else {
            rendering.mode2_interrupt_active(&self.video)
        };

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts.
        let vblank_line_144 = in_vblank && self.video.ly() == 144 && self.video.lx == 0;

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
                && (mode2_active || vblank_line_144))
            || (self
                .video
                .stat_flags
                .contains(InterruptFlags::CURRENT_LINE_COMPARE)
                && self.video.ly_eq_lyc())
    }

    /// Rising half-phase (DELTA_ODD, H→A boundary): XOTA divider chain
    /// toggle, scanline boundary handling, pixel output pipeline, VBlank
    /// IF, and LYC comparison. All rising-phase work in a single method.
    pub fn rise(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            screen: None,
            request_vblank: false,
        };

        if !self.control().video_enabled() {
            return result;
        }

        if self.pixel_pipeline.is_none() {
            return result;
        }

        // Save NYPE state before tick_xota updates it.
        let nype_was = self.video.nype;

        // XOTA rising edge (H→A): toggle WUVU/VENA divider chain,
        // increment LX, detect scanline boundary.
        if self.video.tick_xota() {
            // Scanline boundary — LX wrapped to 0.
            if let Some(rendering) = self.pixel_pipeline.as_mut() {
                let ly = self.video.ly();
                if ly == screen::NUM_SCANLINES {
                    // Line 144: extract completed frame, enter VBlank.
                    // Pipeline persists — circuits are idle, not destroyed.
                    result.screen = Some(rendering.screen.clone());
                } else if self.video.ly == 0 {
                    // Line 0: VBlank → Active Display. Reset for new frame.
                    rendering.reset_frame();
                } else if !self.video.in_vblank() {
                    // Lines 1-143: per-scanline reset.
                    rendering.reset_scanline(ly);
                }
            }
        }

        // Pixel output, SACU, pipe shift — only during active display.
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            if !self.video.in_vblank() {
                rendering.rise(&self.registers, &self.video, &self.oam, vram);
            }
        }

        // NYPE→POPU→VYPU→LOPE pipeline: VBlank IF fires on NYPE
        // rising edge when LY=144. POPU latches XYVO_y144p_old
        // (LY was incremented to 144 at the scanline boundary,
        // 2 dots ago). The cascade is combinational within one tick.
        let nype_rose = !nype_was && self.video.nype;
        if nype_rose
            && self.video.ly() == 144
            && self.video.in_vblank()
            && self.pixel_pipeline.is_some()
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

    /// Falling half-phase (DELTA_EVEN): fetcher pipeline (advance,
    /// cascade DFFs, TYFA), DFF8/DFF9 latches, LCD-off handling.
    pub fn fall(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            screen: None,
            request_vblank: false,
        };

        if self.control().video_enabled() {
            // Fetcher advance, cascade DFFs (NYKA/PORY/PYGO), TYFA.
            // Only during active display — pipeline is idle in VBlank.
            if let Some(rendering) = self.pixel_pipeline.as_mut() {
                if !self.video.in_vblank() {
                    rendering.fall(&self.registers, &self.video, &self.oam, vram);
                }
            }

            // DFF8 palette capture (TEPO rising, phase H). On hardware,
            // palette capture happens on the falling phase — after pixel
            // output (rising) has already read the old palette value.
            self.registers.tick_palette_latches();

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
        match &self.pixel_pipeline {
            Some(rendering) if !self.video.in_vblank() => {
                Some(rendering.pipeline_state(&self.video))
            }
            _ => None,
        }
    }
}
