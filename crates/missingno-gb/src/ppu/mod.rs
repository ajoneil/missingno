//! Timing in this module is measured in **dots**. One dot is one
//! full `ck1_ck2` cycle (one master clock period), driven by the
//! `ck1_ck2` → ANOS/AVET → ATAL/ADEH → AZOF → ZAXY → ZEME → PPU
//! clock (ALET) cascade.
//!
//! "PPU clock" is the subsystem-idiomatic name for the 4 MHz main
//! clock distributed across the PPU's per-dot DFFs; ALET is its
//! gate name in the netlist. DFFs clocked by ALET capture on one
//! edge of the PPU clock; DFFs clocked by its complement MYVO
//! capture on the other. Subsystem-level dispatcher methods use
//! PPU-clock vocabulary (`on_ppu_clock_rise` / `on_ppu_clock_fall`);
//! gate-level signal names (ALET, MYVO) appear where cross-
//! referencing spec sections or explaining hardware derivations
//! (e.g., LEBO = NAND(ALET, MOCE)).
//!
//! Vocabulary equivalences: 1 dot = 1 T-cycle = 2 atal half-cycles.
//! "Dot" is primary in PPU code; "atal half-cycle" and "T-cycle"
//! appear in definitional contexts and where comments bridge to
//! CPU-subsystem timing (register-write strobes, M-cycle phasing).

use rendering::Mode;
use types::sprites::{Sprite, SpriteId};

use dividers::Dividers;
use line_counter::{LineCounter, LineCounterX, LineCounterY};
use line_end_pipeline::LineEndPipeline;
use memory::{Oam, OamAddress, Vram};
use registers::BackgroundViewportPosition;
use rendering::Rendering;
use types::control::{Control, ControlFlags};
use types::palette::Palettes;

pub use dff::DffLatch;
pub use registers::{PipelineRegisters, Window};
pub use rendering::{
    PipelineSnapshot, PpuTraceSnapshot, SpriteFetchPhase, SpriteStoreEntrySnapshot,
    SpriteStoreSnapshot,
};
pub use stat_interrupt::{InterruptFlags, StatInterrupt};
pub use video_control::VideoControl;

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

mod dff;
mod dividers;
mod draw;
mod line_counter;
mod line_end_pipeline;
pub mod memory;
mod oam_corruption;
pub mod registers;
pub mod rendering;
mod scan;
pub mod screen;
pub mod stat_interrupt;
pub mod types;
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
    /// CUPA↑ → XODO↓ scheduling. Set on LCDC.7 0→1 in the staged-write
    /// apply (dot-2 rise); consumed in the same fall to run
    /// `initialize_lcd_on`.
    pending_lcd_on_init: bool,
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
            // Post-boot PPU state: matches what real DMG boot ROM
            // produces at first PC=$0100 detection in missingno.
            // Empirically equivalent to running the real boot ROM —
            // skip-boot anchored at §11.1's literal table (lx=98,
            // WUVU.q=0, XUPY=0) lands one master-clock edge earlier
            // than missingno's executor first observes PC=$0100,
            // producing the cluster-ROM dispatch failure pattern.
            // Anchoring at §11.1 + 1 sub-edge matches the first
            // observable state and passes the cluster tests.
            video: VideoControl {
                dividers: Dividers {
                    half_mcycle: true,
                    mcycle: true,
                },
                lines: LineCounter {
                    x: LineCounterX {
                        value: 99,
                        line_end_detected: false,
                        line_end_active: false,
                    },
                    y: LineCounterY::post_boot(),
                },
                stat: StatInterrupt::post_boot(),
                line_end: LineEndPipeline {
                    delayed_line_end: false,
                    line_end_pending: false,
                },
            },
            oam: Oam::default(),
            pixel_pipeline: Some(Rendering::post_boot()),
            frame_number: 0,
            pending_lcd_on_init: false,
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
                dividers: Dividers {
                    half_mcycle: false,
                    mcycle: false,
                },
                lines: LineCounter {
                    x: LineCounterX {
                        value: 0,
                        line_end_detected: false,
                        line_end_active: false,
                    },
                    y: LineCounterY {
                        value: 0,
                        vblank: false,
                        popu_holdover: false,
                        frame_end_reset: false,
                    },
                },
                stat: StatInterrupt {
                    lyc: 0,
                    comparison_pending: false,
                    comparison_latched: true,
                    enables: InterruptFlags::empty(),
                    line_was_high: false,
                },
                line_end: LineEndPipeline {
                    delayed_line_end: false,
                    line_end_pending: false,
                },
            },
            oam: Oam::default(),
            pixel_pipeline: None, // LCD off at power-on
            frame_number: 0,
            pending_lcd_on_init: false,
        }
    }

    pub fn lx(&self) -> u8 {
        self.video.dot_position()
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

    /// Apply a register write to its backing store.
    ///
    /// Per-register dispatch; the backing store's own semantics govern
    /// whether the new value is visible immediately or after a staging
    /// tick:
    ///
    /// - **Palette registers (BGP, OBP0, OBP1)**: `DffLatch::write`
    ///   — sets pending; new value visible after the next
    ///   `tick_palette_latches` (dlatch_ee + CUPA staging).
    /// - **Viewport / WindowX / control_latch**: `DffLatch::write_immediate`
    ///   — updates the latch output directly (DFF9 register read is
    ///   combinational).
    /// - **STAT (FF41)**: runs the DMG write-glitch (briefly sets all
    ///   enable bits high, then writes the final value); returns true
    ///   if any STAT rising edge was produced.
    /// - **LY**: ignored on DMG.
    ///
    /// Returns true if the write triggered a STAT interrupt request
    /// (DMG STAT write quirk produces a glitch edge when enable bits
    /// transition).
    fn apply_register_write(&mut self, register: &Register, value: u8) -> bool {
        match register {
            Register::Control => {
                self.registers.control = Control::new(ControlFlags::from_bits_retain(value))
            }
            Register::Status => {
                // DMG STAT write quirk: briefly set all enable bits high.
                // If any condition is active, this produces a rising edge.
                // Glitch orchestration stays on Ppu per PW.2; StatInterrupt
                // provides primitives (set_enables / write_stat_bits /
                // detect_line_edge).
                self.video.stat.set_enables(InterruptFlags::all());
                let glitch_line = self.stat_line_active();
                let glitch_edge = self.video.stat.detect_line_edge(glitch_line);

                // Now apply the real value.
                self.video.stat.write_stat_bits(value);
                let final_line = self.stat_line_active();
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
            Register::CurrentScanline => {} // writes to LY are ignored on DMG
        }
        false
    }

    /// Initialize the PPU when LCDC bit 7 transitions from 0 to 1.
    ///
    /// VID_RST deasserts at XOTA rising (= master clock falls = our
    /// fall()). All toggle-DFF dividers async-reset to q=0; with
    /// VENA.q=0, TALU (= VENA.q) is also 0. Hardware's divider
    /// cascade then ramps: WUVU toggles first, then VENA. The first
    /// RUTU-capturing edge after vid_rst is VENA's first rise
    /// (= SONO rising = TALU falling).
    fn initialize_lcd_on(&mut self) {
        self.video.vid_rst();
        // ROPO is NOT reset by VID_RST — the DFF retains its last value.
        // PALY is combinational and settles immediately when LY resets to 0,
        // so recompute the pending comparison here. The ROPO DFF will latch
        // this value at the first TALU rising edge after dividers start.
        self.video.update_ly_comparison();

        // Create the pixel pipeline (VID_RST released).
        self.pixel_pipeline = Some(Rendering::new());
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.start_scanning();
        }

        // Sync the STAT edge detector: the STAT line and its edge detector
        // reach their new steady state simultaneously when VID_RST deasserts.
        // No false edge on the first evaluation.
        let stat_line = self.stat_line_active();
        self.video.stat.set_line_was_high(stat_line);
    }

    pub fn write_register(&mut self, register: Register, value: u8, _vram: &Vram) -> bool {
        let is_drawing = self.is_rendering();

        match register {
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                // Palette registers use DFF8 staging inside DffLatch —
                // `apply_register_write` calls `DffLatch::write` (sets
                // pending), and `tick_palette_latches` applies
                // pending → output on the next PPU clock fall. This
                // models the dlatch_ee + CUPA transparency → next-
                // SACU-rising visibility window. No orchestration
                // branch here (unlike WindowX's Mode-3-dependent
                // staging); DffLatch handles the staging uniformly.
                self.apply_register_write(&register, value)
            }
            Register::Control => {
                let was_enabled = self.registers.control.video_enabled();
                // LCDC uses combinational reads on hardware — the fetcher's
                // VRAM address logic reads reg_new.reg_lcdc with zero delay
                // after the DFF9 latches. No propagation delay needed.
                self.apply_register_write(&register, value);
                self.registers.control_latch.write_immediate(value);

                // CUPA↑ → XODO↓ is combinational; schedule the matching
                // divider / scanner reset for this fall.
                if !was_enabled && self.registers.control.video_enabled() {
                    self.pending_lcd_on_init = true;
                }
                false
            }
            Register::BackgroundViewportY | Register::BackgroundViewportX => {
                // SCY, SCX use DFF9 cells identical to LCDC on hardware.
                // The fetcher reads them combinationally — no propagation delay
                // needed. Always write immediately, matching LCDC behavior.
                self.apply_register_write(&register, value)
            }
            Register::WindowX => {
                if is_drawing {
                    self.registers.window.x_plus_7.write(value);
                    false
                } else {
                    self.apply_register_write(&register, value)
                }
            }
            _ => {
                // Remaining DFF9 registers: no propagation delay, atomic
                // latch at the write point (G→H boundary).
                self.apply_register_write(&register, value)
            }
        }
    }

    pub fn read_oam(&self, address: OamAddress) -> u8 {
        self.oam.read(address)
    }

    pub fn write_oam(&mut self, address: OamAddress, value: u8) {
        self.oam.write(address, value);
    }

    /// STAT mode bits, computed as two independent NOR gates matching hardware.
    ///
    /// Hardware (GateBoyInterrupts.cpp:53-55):
    ///   bit 0 = XYMU OR POPU (rendering OR vblank)
    ///   bit 1 = ACYL OR XYMU (scanning OR rendering)
    ///
    /// No priority logic — each bit is an independent OR of its input signals.
    /// During the POPU+BESU overlap at the 153->0 boundary, this produces
    /// mode 3 (both bits set) instead of the old priority-based mode 1.
    ///
    /// XYMU and BESU inputs are read from the STAT-readout mirrors
    /// (`rendering_active_stat`, `besu_stat`), which lag the internal
    /// NOR-latch state by one PPU-clock-fall. This models the CPU's
    /// T-cycle STAT sampling phase — GateBoy's adapter samples where
    /// BESU/XYMU's pre-transition values are still on the bus during
    /// the AVAP integer dot, so Mode 2's observable duration is 80
    /// dots. The NOR-gate formula is unchanged; only the inputs'
    /// observation window shifts.
    pub fn mode(&self) -> rendering::Mode {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return Mode::VerticalBlank,
        };
        // Hardware (schematic page 21): mode bits are independent NOR gates.
        //   bit 0 = XYMU OR POPU (rendering OR vblank)
        //   bit 1 = ACYL OR XYMU (scanning OR rendering)
        //
        // XYMU is cleared by WEGO = OR2(VID_RST, VOGA). Only VOGA is
        // clocked (ALET rising DFF capture); WEGO and XYMU's set path
        // are combinational. In the emulator, VOGA capture and the
        // WEGO→XYMU chain all fire within the same master-clock edge
        // (capture_voga() clears rendering_active when VOGA is set).
        let rendering_active = rendering.rendering_active_stat();
        let bit0 = rendering_active || self.video.vblank();
        let bit1 = rendering_active || rendering.is_scanning_stat();
        match (bit1, bit0) {
            (false, false) => Mode::HorizontalBlank,
            (false, true) => Mode::VerticalBlank,
            (true, false) => Mode::OamScan,
            (true, true) => Mode::Drawing,
        }
    }

    pub fn oam_locked(&self) -> bool {
        match &self.pixel_pipeline {
            // Hardware: OAM locked by ACYL (BESU-driven) or XYMU (rendering).
            // During VBlank, both BESU and XYMU are low, so this returns false
            // without needing a POPU guard.
            Some(r) => r.oam_locked(),
            None => false,
        }
    }

    pub fn vram_locked(&self) -> bool {
        match &self.pixel_pipeline {
            // Hardware: VRAM locked by XYMU_RENDERINGp only.
            // During VBlank, XYMU is low, so this returns false.
            Some(r) => r.vram_locked(),
            None => false,
        }
    }

    pub fn oam_write_locked(&self) -> bool {
        match &self.pixel_pipeline {
            // Hardware: OAM writes blocked by ACYL (BESU) or XYMU.
            Some(r) => r.oam_write_locked(),
            None => false,
        }
    }

    pub fn vram_write_locked(&self) -> bool {
        match &self.pixel_pipeline {
            // Hardware: XYMU gates reads and writes identically.
            Some(r) => r.vram_write_locked(),
            None => false,
        }
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    /// Whether the latched LY==LYC comparison is currently true (ROPO output).
    pub fn ly_eq_lyc(&self) -> bool {
        self.video.stat.ly_eq_lyc()
    }

    /// LALU edge-detection state (STAT line previous value). Exposed for
    /// gbtrace snapshot capture.
    pub fn stat_line_was_high(&self) -> bool {
        self.video.stat.line_was_high()
    }

    pub fn is_rendering(&self) -> bool {
        match &self.pixel_pipeline {
            // Hardware: XYMU is the rendering gate (Mode 3 active).
            // During VBlank, XYMU is always low — no POPU guard needed.
            Some(r) => r.rendering_active(),
            _ => false,
        }
    }

    pub fn wuvu(&self) -> bool {
        self.video.dividers.half_mcycle
    }

    pub fn vena(&self) -> bool {
        self.video.dividers.mcycle()
    }

    pub fn xupy(&self) -> bool {
        self.video.dividers.xupy()
    }

    pub fn besu(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.scan_besu())
            .unwrap_or(false)
    }

    pub fn wodu(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.wodu())
            .unwrap_or(false)
    }

    pub fn stat_line(&self) -> bool {
        self.stat_line_active()
    }

    fn stat_line_active(&self) -> bool {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return false,
        };

        // Mode 2 interrupt active: during VBlank, never.
        // Otherwise delegate to the rendering pipeline's TAPA signal.
        // Use popu_active() to include the NYPE→POPU DFF holdover period
        // at the 153→0 boundary for STAT interrupt suppression.
        let popu = self.video.popu_active();
        let mode2_active = if popu {
            false
        } else {
            rendering.mode2_interrupt_active(&self.video)
        };

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts. With POPU, this is only true at
        // LX=0 of line 144 (the first M-cycle where POPU is high).
        let vblank_line_144 = popu && self.video.ly() == 144 && self.video.line_end_active();

        let enables = self.video.stat.enables();
        (enables.contains(InterruptFlags::HORIZONTAL_BLANK) && !popu && rendering.wodu())
            || (enables.contains(InterruptFlags::VERTICAL_BLANK) && popu)
            || (enables.contains(InterruptFlags::OAM_SCAN) && (mode2_active || vblank_line_144))
            || (enables.contains(InterruptFlags::CURRENT_LINE_COMPARE)
                && self.video.stat.ly_eq_lyc())
    }

    /// Master clock rising edge — one of the two edges of `ck1_ck2`
    /// that bound a single dot. The master clock is the DMG's 4.194304
    /// MHz crystal oscillator input; all on-chip clocks derive from it.
    ///
    /// Clock mapping on this edge: PPU clock **rises** (ALET rises in-
    /// phase with ck1_ck2). ALET-clocked DFFs capture here: NYKA, LYZU,
    /// PYGO (cascade), RENE, DOBA, NOPA, VOGA.
    /// Sprite fetch counter advances on SABE (opposite edge from BG
    /// fetcher's LEBO).
    ///
    /// The complementary edge (`on_master_clock_fall`) handles the XOTA
    /// divider chain toggle (XOTA rises when ck1_ck2 falls, toggling
    /// WUVU/VENA/TALU), CATU pipeline, MYVO-clocked DFFs (PORY), SACU
    /// pixel shift, scanline boundary handling, VBlank IF, and LYC.
    ///
    /// Collapsed cascade: ck1_ck2 → ANOS/AVET → ATAL/ADEH → AZOF → ZAXY → ZEME → PPU clock (ALET).
    pub fn on_master_clock_rise(&mut self, vram: &Vram) -> PpuTickResult {
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

        // Divider chain (WUVU/VENA/TALU) and CATU now run in
        // on_master_clock_fall() — confirmed by dmg-sim: XOTA rises when
        // master clock falls, and WUVU/XUPY/CATU all toggle on XOTA rising.

        // ALET rises in-phase with ck1_ck2 (ATAL → AZOF → ZAXY → ZEME
        // → ALET is an even number of inversions from ck1_ck2, so ALET
        // rising aligns with master-clock rising at 16.3 ge buffer
        // delay). ALET-clocked DFFs capture here: NYKA, PYGO (cascade),
        // VOGA (hblank), LYZU. Also XUPY-derived logic read at this
        // edge.
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel =
                rendering.on_ppu_clock_rise(&self.registers, &self.video, &self.oam, vram);
        }

        result
    }

    /// Master clock falling edge — the complementary edge to
    /// `on_master_clock_rise`. Together they bound one dot = one full
    /// cycle of `ck1_ck2`.
    ///
    /// Clock mapping on this edge: PPU clock **falls** (ALET falls in-
    /// phase with ck1_ck2). XOTA rises (XOTA = NOT(ck1_ck2), so XOTA
    /// rises when ck1_ck2 falls), toggling the divider chain
    /// (WUVU/VENA/TALU); XUPY transitions, clocking the OAM-scan
    /// subsystem (CATU, BYBA, CENO). MYVO-clocked DFFs (PORY) capture
    /// here; SACU fires (delayed via the VYBO/TYFA/SEGU chain) and
    /// drives the pixel shift / output.
    ///
    /// Also: DFF8/DFF9 register latches, scanline-boundary handling,
    /// and LCD-off state management.
    pub fn on_master_clock_fall(&mut self, is_mcycle: bool) -> PpuTickResult {
        let mut result = PpuTickResult {
            pixel: None,
            new_frame: false,
            request_vblank: false,
        };

        // XODO↓ collapses to this fall; subsequent tick_dot is WUVU's
        // first toggle.
        if self.pending_lcd_on_init {
            self.initialize_lcd_on();
            self.pending_lcd_on_init = false;
        }

        // XOTA rising edge (= master clock falls = our fall()): toggle
        // WUVU, cascade VENA/TALU, handle scanline boundaries. Confirmed
        // by dmg-sim gate-level simulation.
        // tick_dot toggles WUVU on every fall when the LCD is on; the
        // returned previous WUVU.Q value determines the XUPY edge for
        // this fall (XUPY = WUVU.Q, so WUVU 0→1 = XUPY rising).
        let xupy_rising = if self.control().video_enabled() && self.pixel_pipeline.is_some() {
            !self.video.tick_dot()
        } else {
            false
        };

        if self.control().video_enabled() && self.pixel_pipeline.is_some() {
            let popu_was = self.video.vblank();

            let mut scanline_boundary = false;
            if self.video.dividers.half_mcycle_fell() {
                let vena_was = self.video.dividers.tick_mcycle();
                let vena_now = self.video.dividers.mcycle();

                if !vena_was && vena_now {
                    // VENA rising = TALU rising. ROPO captures PALY
                    // before MYTA fires — at LY=153 the MYTA→LAMA→LY
                    // DFF reset race favours ROPO (4-stage capture vs
                    // 6-stage MYTA propagation), so it captures
                    // pre-reset PALY=(153==LYC). Then NYPE captures
                    // (POPU/MYTA) and LX advances.
                    self.video.update_ly_comparison();
                    self.video.stat.latch_comparison();
                    self.video.on_lx_counter_clock_rise();
                    self.video.update_ly_comparison();
                }

                if vena_was && !vena_now {
                    // VENA falling = SONO rising = TALU falling. RUTU
                    // captures SANU on SONO rising; LY advances/wraps
                    // on the RUTU pulse.
                    scanline_boundary = self.video.on_lx_counter_clock_fall();
                    self.video.update_ly_comparison();
                }
            }

            if scanline_boundary {
                if let Some(rendering) = self.pixel_pipeline.as_mut() {
                    let ly = self.video.ly();
                    if ly == screen::NUM_SCANLINES {
                        self.frame_number = self.frame_number.wrapping_add(1);
                        result.new_frame = true;
                    } else if self.video.ly_hardware() == 0 {
                        rendering.reset_frame();
                    } else if self.video.ly() < 144 {
                        rendering.reset_scanline(ly);
                    }
                }
            }

            // POPU rising edge → VYPU → LOPE: VBlank IF.
            if self.video.vblank() && !popu_was {
                result.request_vblank = true;
            }
        }

        if self.control().video_enabled() {
            // Resolve DFF8/DFF9 latches BEFORE the pipeline reads them.
            // The tick models the clock boundary *entering* this dot:
            // any CPU write from the previous dot (stored as pending)
            // transfers to output here, so the pipeline sees a 1-dot
            // delay — matching hardware's reg_new → reg_old copy at the
            // tick boundary followed by combinational read of reg_old.
            self.registers.tick_palette_latches();
            self.registers.tick_register_latches();

            // ALET falls in-phase with ck1_ck2 falling. MYVO-clocked DFFs
            // capture here (PORY), SACU fires (delayed via TYFA), pixel
            // output advances, pixel counter / fine counter increment.
            // Gated by XYMU/BESU on hardware, not POPU. During VBlank,
            // XYMU and BESU are low, making this effectively a no-op.
            if let Some(rendering) = self.pixel_pipeline.as_mut() {
                result.pixel = rendering.on_ppu_clock_fall(
                    &self.registers,
                    &self.video,
                    &self.oam,
                    xupy_rising,
                );
            }

            // CATU DFF pipeline runs AFTER on_ppu_clock_fall so that
            // advance_scan reads pre-tick_catu state. On the +1 fall
            // of a scanline boundary, advance_scan sees scanning=false
            // (still false from the prior scanline's AVAP) and does
            // not tick the counter; tick_catu then captures CATU,
            // sets scanning=true and counter=0. The next XUPY-rising
            // fall's advance_scan ticks counter 0→1 — matching the
            // "1 XUPY cycle between CATU capture and the first counter
            // tick" rule.
            //
            // tick_rutu runs after tick_catu so the CATU reader sees
            // the pre-promotion RUTU value; the scanline-boundary
            // write promoted by tick_rutu is only visible on the next
            // XUPY rising edge.
            if let Some(rendering) = self.pixel_pipeline.as_mut() {
                rendering.tick_catu(&self.video);
                rendering.tick_rutu();
            }

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

            // VID_RST: async-reset all counters while LCD is off.
            // Hardware holds these at 0 continuously; we reset on each
            // M-cycle to match.
            self.video.vid_rst();

            // stat.comparison_latched is intentionally NOT updated —
            // comparison clock stops when the PPU is off, freezing the
            // last result.
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
        self.video.stat.detect_line_edge(stat_line_high)
    }

    pub fn palettes(&self) -> &Palettes {
        &self.registers.palettes
    }

    pub fn sprite(&self, sprite: SpriteId) -> &Sprite {
        self.oam.sprite(sprite)
    }

    pub fn pipeline_state(&self) -> Option<PipelineSnapshot> {
        match &self.pixel_pipeline {
            Some(rendering) if !self.video.vblank() => Some(rendering.pipeline_state(&self.video)),
            _ => None, // VBlank or LCD off: no pipeline to snapshot.
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

    /// Construct a PPU from a gbtrace snapshot.
    ///
    /// The rendering pipeline is created if the LCD is enabled (LCDC bit 7),
    /// with VBlank derived from LY >= 144.
    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::PpuSnapshot, oam: Oam) -> Self {
        let control = Control::new(ControlFlags::from_bits_retain(snap.lcdc));
        let lcd_on = control.video_enabled();
        let enables = InterruptFlags::from_bits_truncate(snap.stat);

        let video = VideoControl {
            dividers: Dividers {
                half_mcycle: false,
                mcycle: false,
            },
            lines: LineCounter {
                x: LineCounterX {
                    value: snap.dot_position,
                    line_end_detected: false,
                    line_end_active: false,
                },
                y: LineCounterY {
                    value: snap.ly,
                    vblank: snap.ly >= 144,
                    popu_holdover: false,
                    frame_end_reset: false,
                },
            },
            stat: StatInterrupt {
                lyc: snap.lyc,
                comparison_pending: snap.ly == snap.lyc,
                comparison_latched: snap.ly == snap.lyc,
                enables,
                line_was_high: snap.stat_line_was_high,
            },
            line_end: LineEndPipeline {
                delayed_line_end: false,
                line_end_pending: false,
            },
        };

        let registers = PipelineRegisters {
            control,
            control_latch: DffLatch::new(snap.lcdc),
            background_viewport: BackgroundViewportPosition {
                x: DffLatch::new(snap.scx),
                y: DffLatch::new(snap.scy),
            },
            window: Window {
                y: snap.wy,
                x_plus_7: DffLatch::new(snap.wx),
            },
            palettes: Palettes {
                background: DffLatch::new(snap.bgp),
                sprite0: DffLatch::new(snap.obp0),
                sprite1: DffLatch::new(snap.obp1),
            },
        };

        Ppu {
            pixel_pipeline: if lcd_on { Some(Rendering::new()) } else { None },
            registers,
            video,
            oam,
            frame_number: 0,
            pending_lcd_on_init: false,
        }
    }
}
