use rendering::Mode;
use types::sprites::{Sprite, SpriteId};

use types::control::{Control, ControlFlags};
use memory::{Oam, OamAddress, Vram};
use types::palette::Palettes;
use rendering::Rendering;
use registers::BackgroundViewportPosition;

pub use dff::DffLatch;
pub use rendering::{
    PipelineSnapshot, PpuTraceSnapshot, SpriteFetchPhase, SpriteStoreEntrySnapshot,
    SpriteStoreSnapshot,
};
pub use registers::{PipelineRegisters, Window};
pub use video_control::{InterruptFlags, VideoControl};

/// A pixel pushed to the LCD — the PPU's primary output signal.
/// One pixel per SEMU clock edge during Mode 3.
#[derive(Clone, Copy, Debug)]
pub struct PixelOutput {
    /// LCD X position (0-159).
    pub x: u8,
    /// Scanline (0-143).
    pub y: u8,
    /// Post-palette shade (0-3).
    pub shade: u8,
}

pub struct PpuTickResult {
    /// A pixel pushed to the LCD, if any. The caller is responsible
    /// for writing this into a framebuffer or capturing it in a trace.
    pub pixel: Option<PixelOutput>,
    /// A completed frame is ready to present. Fires at VBlank (line 144)
    /// or when the LCD is turned off. The caller should swap/present
    /// its back buffer and clear for the next frame.
    pub new_frame: bool,
    pub request_vblank: bool,
}

pub mod types;
mod dff;
pub mod memory;
mod oam_corruption;
pub mod rendering;
pub mod registers;
pub mod screen;
mod draw;
mod scan;
pub mod video_control;

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
    /// `video.vblank` (POPU DFF latch).
    pixel_pipeline: Option<Rendering>,
    pub registers: PipelineRegisters,
    pub video: VideoControl,
    pub oam: Oam,
    /// Frame counter for gbtrace output. Incremented each time a
    /// completed frame is extracted from the rendering pipeline.
    pub frame_number: u16,
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
            // Post-boot PPU state: internal line 153, LX=98, VBlank.
            // ly() returns 0 (MYTA early reset), matching DMG post-boot LY=0.
            // WUVU/VENA phase: TALU rises at dot 1 (phase C), matching hardware.
            video: VideoControl {
                dot_position: 98,
                dot_divider: false,
                mcycle_divider: false,
                ly: 153,
                lyc: 0,
                ly_comparison_pending: true,
                ly_comparison_latched: true,
                // The first bit is unused, but is set at boot time
                stat_flags: InterruptFlags::DUMMY,
                stat_line_was_high: false,
                delayed_line_end: false,
                line_end_pending: false,
                line_end_detected: false,
                line_end_active: false,
                frame_end_reset: true,
                vblank: true,
            },
            oam: Oam::default(),
            // Pipeline persists through VBlank — video.ly=153 means
            // popu is true, so pipeline ticking is gated off.
            pixel_pipeline: Some(Rendering::new()),
            frame_number: 0,
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
                dot_position: 0,
                dot_divider: false,
                mcycle_divider: false,
                ly: 0,
                lyc: 0,
                ly_comparison_pending: false,
                ly_comparison_latched: true,
                stat_flags: InterruptFlags::empty(),
                stat_line_was_high: false,
                delayed_line_end: false,
                line_end_pending: false,
                line_end_detected: false,
                line_end_active: false,
                frame_end_reset: false,
                vblank: false,
            },
            oam: Oam::default(),
            pixel_pipeline: None, // LCD off at power-on
            frame_number: 0,
        }
    }

    pub fn lx(&self) -> u8 {
        self.video.dot_position
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
                    Some(rendering) if !self.video.vblank => rendering.stat_mode(&self.video) as u8,
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
            Register::InterruptOnScanline => {
                self.video.lyc = value;
                self.video.update_ly_comparison();
            }
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
    /// On hardware, VID_RST deasserts at G→H (XOTA falling). All
    /// dividers start at qp=0 (async reset). We initialize wuvu=false
    /// to model this reset state. The first rise() call will toggle
    /// WUVU 0→1 (phase A), and the second rise() call will toggle
    /// WUVU 1→0 (phase C), triggering the first TALU rise and LX
    /// increment. This gives LX=0 the correct 3-half-phase duration.
    fn initialize_lcd_on(&mut self) {
        self.video.dot_position = 0;
        self.video.dot_divider = false;
        self.video.mcycle_divider = false;
        self.video.write_ly(0);
        self.video.delayed_line_end = false;
        self.video.line_end_pending = false;
        self.video.line_end_active = false;
        self.video.line_end_detected = false;
        self.video.frame_end_reset = false;
        self.video.vblank = false;
        self.video.ly_comparison_latched = false;
        self.video.update_ly_comparison();

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
                // Palette DFF8 registers settle combinationally within a single
                // dot (~10 gates), well before CLKPIPE fires (~16 gates). No
                // deferred latch needed — always write immediately.
                self.write_register_immediate(&register, value)
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

    pub fn mode(&self) -> rendering::Mode {
        match &self.pixel_pipeline {
            Some(rendering) if !self.video.vblank => rendering.mode(&self.video),
            Some(_) => Mode::VerticalBlank,
            None => Mode::VerticalBlank,
        }
    }

    pub fn oam_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.vblank => r.oam_locked(),
            _ => false,
        }
    }

    pub fn vram_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.vblank => r.vram_locked(),
            _ => false,
        }
    }

    pub fn oam_write_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.vblank => r.oam_write_locked(),
            _ => false,
        }
    }

    pub fn vram_write_locked(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.vblank => r.vram_write_locked(),
            _ => false,
        }
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    pub fn is_rendering(&self) -> bool {
        match &self.pixel_pipeline {
            Some(r) if !self.video.vblank => r.xymu(),
            _ => false,
        }
    }

    fn stat_line_active(&self) -> bool {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return false,
        };

        // Mode 2 interrupt active: during VBlank, never.
        // Otherwise delegate to the rendering pipeline's TAPA signal.
        let mode2_active = if self.video.vblank {
            false
        } else {
            rendering.mode2_interrupt_active(&self.video)
        };

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts. With POPU, this is only true at
        // LX=0 of line 144 (the first M-cycle where POPU is high).
        let vblank_line_144 = self.video.vblank && self.video.ly() == 144 && self.video.line_end_active();

        // TARU = AND(WODU-or-VOGA, NOT(VBlank)). WODU is combinational
        // (true for 1 rising-phase window), VOGA latches on the falling
        // edge and stays true through HBlank. Together they cover the
        // full HBlank period.
        (self
            .video
            .stat_flags
            .contains(InterruptFlags::HORIZONTAL_BLANK)
            && !self.video.vblank
            && (rendering.voga() || rendering.wodu()))
            || (self
                .video
                .stat_flags
                .contains(InterruptFlags::VERTICAL_BLANK)
                && self.video.vblank)
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
    pub fn rise(&mut self, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            pixel: None,
            new_frame: false,
            request_vblank: false,
        };

        if !self.control().video_enabled() {
            return result;
        }

        if self.pixel_pipeline.is_none() {
            return result;
        }

        // Save POPU state before divider chain updates it.
        let popu_was = self.video.vblank;

        // XOTA rising edge: toggle WUVU (dot-rate clock).
        self.video.tick_dot();

        // VENA/TALU cascade: only fires when WUVU falls.
        let mut scanline_boundary = false;
        if self.video.dot_divider_fell() {
            let talu_was = self.video.tick_mcycle_divider();

            if talu_was && !self.video.mcycle_divider {
                // TALU falling edge: RUTU fire, LY increment.
                scanline_boundary = self.video.tick_talu_fall();
                // PALY is combinational — recompute after any LY change
                // so the next ROPO latch (TALU rising) sees the fresh value.
                self.video.update_ly_comparison();
            }

            if !talu_was && self.video.mcycle_divider {
                // TALU rising edge: LX increment, SANU detect, ROPO latch.
                self.video.tick_talu_rise();
                self.video.latch_ly_comparison();
            }
        }

        if scanline_boundary {
            // Scanline boundary — RUTU fired, LY incremented. LX was
            // reset to 0 in tick_talu_fall().
            if let Some(rendering) = self.pixel_pipeline.as_mut() {
                let ly = self.video.ly();
                if ly == screen::NUM_SCANLINES {
                    // Line 144: frame complete, enter VBlank.
                    // Increment frame counter here (VSYNC start) to match
                    // GateBoy's MEDA_VSYNC_OUTn rising edge timing.
                    self.frame_number = self.frame_number.wrapping_add(1);
                    result.new_frame = true;
                } else if self.video.ly == 0 {
                    // Line 0: VBlank → Active Display. Reset for new frame.
                    rendering.reset_frame();
                } else if !self.video.vblank {
                    // Lines 1-143: per-scanline reset.
                    rendering.reset_scanline(ly);
                }
            }
        }

        // Pixel output, SACU, pipe shift — only during active display.
        if let Some(rendering) = self.pixel_pipeline.as_mut().filter(|_| !self.video.vblank) {
            result.pixel = rendering.rise(&self.registers, &self.video, &self.oam, vram);
        }

        // POPU rising edge → VYPU → LOPE: VBlank IF fires when POPU
        // transitions from low to high. POPU latches at NYPE rising edge,
        // so this detects the combinational cascade within one tick.
        if self.video.vblank && !popu_was && self.pixel_pipeline.is_some() {
            result.request_vblank = true;
        }

        result
    }

    /// Falling half-phase (DELTA_EVEN): fetcher pipeline (advance,
    /// cascade DFFs, TYFA), DFF8/DFF9 latches, LCD-off handling.
    pub fn fall(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            pixel: None,
            new_frame: false,
            request_vblank: false,
        };

        if self.control().video_enabled() {
            // Fetcher advance, cascade DFFs (NYKA/PORY/PYGO), TYFA.
            // Only during active display — pipeline is idle in VBlank.
            if let Some(rendering) = self.pixel_pipeline.as_mut().filter(|_| !self.video.vblank) {
                result.pixel = rendering.fall(&self.registers, &self.video, &self.oam, vram);
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
                result.new_frame = true;
            }
            // ly_comparison_latched is intentionally NOT updated — comparison clock
            // stops when the PPU is off, freezing the last result.
            return result;
        }

        result
    }

    /// Settle ALET-clocked DFFs (VOGA/XYMU) before CPU bus read.
    /// On hardware, ALET falls at F->G before BUKE opens at G-H --
    /// this models that ordering so the CPU sees post-XYMU state.
    pub fn settle_alet(&mut self) {
        if !self.control().video_enabled() {
            return;
        }
        if let Some(rendering) = self.pixel_pipeline.as_mut().filter(|_| !self.video.vblank) {
            rendering.settle_alet();
        }
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
            Some(rendering) if !self.video.vblank => Some(rendering.pipeline_state(&self.video)),
            _ => None,
        }
    }

    pub fn trace_snapshot(&self) -> Option<PpuTraceSnapshot> {
        self.pixel_pipeline.as_ref().map(|r| {
            let mut snap = r.trace_snapshot(&self.oam);
            snap.frame_num = self.frame_number;
            snap
        })
    }

    pub fn sprite_store(&self) -> Option<SpriteStoreSnapshot> {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.sprite_store_snapshot())
    }

    /// Construct a PPU from save state data.
    ///
    /// Builds all internal state from the snapshot values. The rendering
    /// pipeline is created if the LCD is enabled (LCDC bit 7), with
    /// VBlank derived from LY >= 144.
    pub fn from_save_state(
        lcdc: u8, stat: u8, ly: u8, lyc: u8,
        scy: u8, scx: u8, wy: u8, wx: u8,
        bgp: u8, obp0: u8, obp1: u8,
        dot_position: u8, stat_line_was_high: bool,
        oam: Oam,
    ) -> Self {
        let control = Control::new(ControlFlags::from_bits_retain(lcdc));
        let lcd_on = control.video_enabled();
        let stat_flags = InterruptFlags::from_bits_truncate(stat);

        let video = VideoControl {
            dot_position,
            dot_divider: false,
            mcycle_divider: false,
            ly,
            lyc,
            ly_comparison_pending: ly == lyc,
            ly_comparison_latched: ly == lyc,
            stat_flags,
            stat_line_was_high,
            delayed_line_end: false,
            line_end_pending: false,
            line_end_detected: false,
            line_end_active: false,
            frame_end_reset: false,
            vblank: ly >= 144,
        };

        let registers = PipelineRegisters {
            control,
            control_latch: DffLatch::new(lcdc),
            background_viewport: BackgroundViewportPosition {
                x: DffLatch::new(scx),
                y: DffLatch::new(scy),
            },
            window: Window {
                y: wy,
                x_plus_7: DffLatch::new(wx),
            },
            palettes: Palettes {
                background: DffLatch::new(bgp),
                sprite0: DffLatch::new(obp0),
                sprite1: DffLatch::new(obp1),
            },
        };

        Ppu {
            pixel_pipeline: if lcd_on { Some(Rendering::new()) } else { None },
            registers,
            video,
            oam,
            frame_number: 0,
        }
    }
}
