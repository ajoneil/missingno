//! PPU timing is measured in **dots** — one dot = one master clock
//! period (`ck1_ck2`). The PPU clock (ALET) is the 4 MHz subsystem
//! clock derived from `ck1_ck2`; ALET-clocked DFFs capture on one
//! edge, MYVO-clocked DFFs on the other. 1 dot = 1 T-cycle = 2 atal
//! half-cycles.

use crate::dma::OamBusOwner;
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

pub use dff::{DffLatch, NorLatch};
pub use registers::{PipelineRegisters, Window};
pub use rendering::{
    PipelineSnapshot, PpuTraceSnapshot, SpriteFetchPhase, SpriteStoreEntrySnapshot,
    SpriteStoreSnapshot,
};
pub use stat_interrupt::{InterruptFlags, StatInterrupt};
pub use video_control::VideoControl;

/// A pixel pushed to the LCD — one per SACU edge during Mode 3.
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
    /// A pixel pushed to the LCD, if any.
    pub pixel: Option<PixelOutput>,
    /// VSYNC pulse — LY wrapped at the end of line 153. Not set on
    /// LCD-off (MEDA stops pulsing).
    pub new_frame: bool,
    /// LCDC.7 just went 1→0 while the pipeline was active; the caller
    /// should blank the screen. Not a hardware frame boundary.
    pub lcd_disabled: bool,
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
    /// Pixel pipeline state. `None` = LCD off (VID_RST asserted). The
    /// pipeline persists through both active display and VBlank;
    /// VBlank is derived from `video.vblank` (POPU).
    pixel_pipeline: Option<Rendering>,
    pub registers: PipelineRegisters,
    pub video: VideoControl,
    pub oam: Oam,
    /// Frame counter for gbtrace output.
    pub frame_number: u16,
    /// CUPA↑ → XODO↓ scheduling: set on LCDC.7 0→1 in the rise-edge
    /// staged write; consumed in the same fall.
    lcd_on_init_pending: bool,
    /// OAM-bug arming (BOWA → MOPA window).
    oam_corruption: oam_corruption::OamCorruption,
}

impl Ppu {
    /// Power-on state: LCD off, all registers zeroed.
    pub fn new() -> Self {
        Self {
            registers: PipelineRegisters {
                control_latch: DffLatch::new(0),
                control: Control::new(ControlFlags::empty()),
                background_viewport: BackgroundViewportPosition {
                    x: DffLatch::new(0),
                    y: DffLatch::new(0),
                },
                window: Window {
                    y: 0,
                    x_plus_7: DffLatch::new(0),
                },
                palettes: Palettes::default(),
                bg_window_enabled_shadow: None,
                bg_window_enabled_shadow_just_set: false,
                sprites_enabled_shadow: None,
                sprites_enabled_shadow_just_set: false,
                sprites_enabled_pre_cupa: false,
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
                    meda: false,
                    vsync_committed: false,
                },
            },
            oam: Oam::default(),
            pixel_pipeline: None,
            frame_number: 0,
            lcd_on_init_pending: false,
            oam_corruption: oam_corruption::OamCorruption::default(),
        }
    }

    /// Post-boot state — equivalent to what the DMG boot ROM leaves
    /// behind at first PC=$0100 detection.
    pub fn post_boot() -> Self {
        let control = Control::default();
        let mut ppu = Self::new();
        ppu.registers.control = control;
        ppu.registers.control_latch = DffLatch::new(control.bits());
        ppu.video = VideoControl {
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
                meda: false,
                vsync_committed: true,
            },
        };
        ppu.pixel_pipeline = Some(Rendering::post_boot());
        ppu.registers.sprites_enabled_pre_cupa = true;
        ppu
    }

    pub fn lx(&self) -> u8 {
        self.video.dot_position()
    }

    /// True once MEDA has gone 0→1 since the most recent VID_RST
    /// deassertion — the LCD's first VSYNC has fired and frames may
    /// be committed.
    pub fn vsync_committed(&self) -> bool {
        self.video.line_end.vsync_committed
    }

    /// Current OAM scan counter entry (0-39). None when not rendering.
    pub fn scan_counter(&self) -> Option<u8> {
        self.pixel_pipeline.as_ref().map(|r| r.scan_counter_entry())
    }

    /// True when the WUSA NOR-latch is open — LCD is actively shifting
    /// pixels. Gates LCDC.0/.1 overlay arming during prelude.
    fn lcd_pushing_active(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.lcd_pushing_active())
            .unwrap_or(false)
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

    /// VID_RST deasserts at XOTA rising (= our fall). Toggle DFFs
    /// async-reset to q=0; the divider cascade then ramps WUVU then
    /// VENA. The first RUTU-capturing edge is VENA's first rise.
    fn initialize_lcd_on(&mut self) {
        self.video.vid_rst();
        // ROPO is not VID_RST-reset; PALY is combinational so recompute
        // here. ROPO latches this value at the first TALU rising edge.
        self.video.update_ly_comparison();

        self.pixel_pipeline = Some(Rendering::new());
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.start_scanning();
        }

        // STAT line and its edge detector reach steady state together
        // when VID_RST deasserts — sync to avoid a spurious first edge.
        let stat_line = self.stat_line();
        self.video.stat.set_line_was_high(stat_line);
    }

    pub fn write_register(
        &mut self,
        register: Register,
        value: u8,
        _vram: &Vram,
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

    pub fn read_oam(&self, address: OamAddress) -> u8 {
        self.oam.read(address)
    }

    pub fn write_oam(&mut self, address: OamAddress, value: u8) {
        self.oam.write(address, value);
    }

    /// STAT mode bits — independent NOR gates on the rendering /
    /// scanning / vblank lines (schematic page 21):
    ///   bit 0 = XYMU OR POPU   (rendering OR vblank)
    ///   bit 1 = ACYL OR XYMU   (scanning OR rendering)
    /// CPU STAT reads use the cpu_port_d bus model to sample at dot 2.
    pub fn mode(&self) -> rendering::Mode {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return Mode::VerticalBlank,
        };
        let rendering_active = rendering.rendering_active();
        let bit0 = rendering_active || self.video.vblank();
        let bit1 = rendering_active || rendering.scan_besu();
        match (bit1, bit0) {
            (false, false) => Mode::HorizontalBlank,
            (false, true) => Mode::VerticalBlank,
            (true, false) => Mode::OamScan,
            (true, true) => Mode::Drawing,
        }
    }

    /// OAM read-locked: ACYL (BESU) or XYMU asserted.
    pub fn oam_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.oam_locked())
            .unwrap_or(false)
    }

    /// VRAM read-locked: XYMU asserted.
    pub fn vram_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.vram_locked())
            .unwrap_or(false)
    }

    pub fn oam_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.oam_write_locked())
            .unwrap_or(false)
    }

    pub fn vram_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.vram_write_locked())
            .unwrap_or(false)
    }

    /// Lock state for a CPU write to `address`. `None` for non-PPU
    /// memory.
    pub fn write_lock(&self, address: u16) -> Option<bool> {
        match address {
            0xFE00..=0xFE9F => Some(self.oam_write_locked()),
            0x8000..=0x9FFF => Some(self.vram_write_locked()),
            _ => None,
        }
    }

    /// Whether a CPU read at `address` is blocked by PPU mode gating.
    pub fn read_locked(&self, address: u16) -> bool {
        match address {
            0xFE00..=0xFE9F => self.oam_locked(),
            0x8000..=0x9FFF => self.vram_locked(),
            _ => false,
        }
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    /// Latched LY==LYC (ROPO output).
    pub fn ly_eq_lyc(&self) -> bool {
        self.video.stat.ly_eq_lyc()
    }

    /// LALU edge-detector state (STAT line previous value). Exposed
    /// for gbtrace snapshot capture.
    pub fn stat_line_was_high(&self) -> bool {
        self.video.stat.line_was_high()
    }

    pub fn is_rendering(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.rendering_active())
            .unwrap_or(false)
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
        let sprites_enabled = self.control().sprites_enabled();
        self.pixel_pipeline
            .as_ref()
            .map(|r| r.wodu(sprites_enabled))
            .unwrap_or(false)
    }

    /// Combinational STAT interrupt line.
    pub fn stat_line(&self) -> bool {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return false,
        };

        // popu_active includes the NYPE→POPU DFF holdover at the 153→0
        // boundary so Mode-2-during-VBlank suppression covers it.
        let popu = self.video.popu_active();
        let mode2_active = if popu {
            false
        } else {
            rendering.mode2_interrupt_active(&self.video)
        };

        // Mode 2 STAT also fires at LX=0 of line 144 (first M-cycle
        // where POPU is high).
        let vblank_line_144 = popu && self.video.ly() == 144 && self.video.line_end_active();

        let enables = self.video.stat.enables();
        let sprites_enabled = self.registers.control.sprites_enabled();
        (enables.contains(InterruptFlags::HORIZONTAL_BLANK)
            && !popu
            && rendering.wodu(sprites_enabled))
            || (enables.contains(InterruptFlags::VERTICAL_BLANK) && popu)
            || (enables.contains(InterruptFlags::OAM_SCAN) && (mode2_active || vblank_line_144))
            || (enables.contains(InterruptFlags::CURRENT_LINE_COMPARE)
                && self.video.stat.ly_eq_lyc())
    }

    /// Snapshot LCDC.1 (XYLO) BEFORE the CPU's staged bus write applies
    /// this rise. The captured value is consumed in mode3_rising to
    /// model the SOBU vs CUPA gate-delay race.
    pub fn snapshot_pre_cupa_lcdc(&mut self) {
        self.registers.sprites_enabled_pre_cupa = self.registers.control.sprites_enabled();
    }

    /// Master clock rise — PPU clock (ALET) rises. ALET-clocked DFFs
    /// capture: NYKA, LYZU, PYGO, RENE, DOBA, NOPA, VOGA.
    pub fn on_master_clock_rise(&mut self, vram: &Vram, oam_bus: OamBusOwner) -> PpuTickResult {
        let mut result = PpuTickResult {
            pixel: None,
            new_frame: false,
            lcd_disabled: false,
            request_vblank: false,
        };

        if !self.control().video_enabled() {
            return result;
        }

        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            let sprites_enabled_pre_cupa = self.registers.sprites_enabled_pre_cupa;
            result.pixel = rendering.on_ppu_clock_rise(
                &self.registers,
                &self.video,
                &self.oam,
                oam_bus,
                vram,
                sprites_enabled_pre_cupa,
            );
        }

        result
    }

    /// Master clock fall — PPU clock (ALET) falls. XOTA rises here,
    /// toggling the divider chain (WUVU/VENA/TALU); MYVO-clocked DFFs
    /// (PORY) capture; SACU fires and drives pixel output.
    pub fn on_master_clock_fall(
        &mut self,
        is_mcycle: bool,
        oam_bus: OamBusOwner,
    ) -> PpuTickResult {
        let mut result = PpuTickResult {
            pixel: None,
            new_frame: false,
            lcd_disabled: false,
            request_vblank: false,
        };

        // XODO↓ collapses to this fall; subsequent tick_dot is WUVU's
        // first toggle.
        if self.lcd_on_init_pending {
            self.initialize_lcd_on();
            self.lcd_on_init_pending = false;
        }

        if !self.control().video_enabled() {
            return self.handle_lcd_off(is_mcycle, result);
        }
        if self.pixel_pipeline.is_none() {
            return result;
        }

        // tick_dot toggles WUVU; the returned previous WUVU.Q value
        // determines this fall's XUPY edge (XUPY = WUVU.Q).
        let xupy_rising = !self.video.tick_dot();

        self.advance_dividers(&mut result);
        self.registers.palettes.tick_besu(self.besu());
        self.tick_register_latches();
        self.run_ppu_clock_fall(oam_bus, xupy_rising, &mut result);

        result
    }

    /// VENA rising/falling drives scanline-boundary handling and frame
    /// completion (new_frame / request_vblank / reset_frame).
    fn advance_dividers(&mut self, result: &mut PpuTickResult) {
        if !self.video.dividers.half_mcycle_fell() {
            return;
        }

        let vena_was = self.video.dividers.tick_mcycle();
        let vena_now = self.video.dividers.mcycle();
        let popu_was = self.video.vblank();

        let mut scanline_boundary = false;
        if !vena_was && vena_now {
            // VENA↑ = TALU↑. ROPO captures pre-reset PALY (4-stage
            // capture beats 6-stage MYTA→LY-reset). NYPE captures
            // POPU/MYTA and LX advances.
            self.video.update_ly_comparison();
            self.video.stat.latch_comparison();
            self.video.on_lx_counter_clock_rise();
            self.video.update_ly_comparison();
        }
        if vena_was && !vena_now {
            // VENA↓ = SONO↑ = TALU↓. RUTU captures SANU; LY advances.
            scanline_boundary = self.video.on_lx_counter_clock_fall();
            self.video.update_ly_comparison();
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

        // POPU↑ → VYPU → LOPE: VBlank IF.
        if self.video.vblank() && !popu_was {
            result.request_vblank = true;
        }
    }

    /// Resolve DFF8/DFF9 latches BEFORE the pipeline reads them — the
    /// tick models the boundary entering this dot: a CPU write from
    /// the previous dot (pending) transfers to output here, giving the
    /// pipeline a 1-dot delay.
    fn tick_register_latches(&mut self) {
        self.registers.tick_palette_latches();
        self.registers.tick_register_latches();
        // LCDC.0/.1 first-cp_pad↑-samples-OLD shadows live for this
        // fall's BG resolve and clear on the next fall.
        self.registers.tick_bg_window_enabled_shadow();
        self.registers.tick_sprites_enabled_shadow();
    }

    /// PPU clock falling work: pixel emit + CATU pipeline.
    fn run_ppu_clock_fall(
        &mut self,
        oam_bus: OamBusOwner,
        xupy_rising: bool,
        result: &mut PpuTickResult,
    ) {
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel = rendering.on_ppu_clock_fall(
                &self.registers,
                &self.video,
                &self.oam,
                oam_bus,
                xupy_rising,
            );
            if result.pixel.is_some() {
                self.registers.palettes.note_bg_pixel_emit();
            }
        }

        // CATU runs AFTER on_ppu_clock_fall so advance_scan reads
        // pre-tick_catu state. On a scanline-boundary +1 fall,
        // advance_scan sees scanning=false; tick_catu then captures
        // CATU, sets scanning=true and counter=0.
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.tick_catu(&self.video);
        }
    }

    /// LCD off (or just disabled): tear down the pipeline on the next
    /// M-cycle, hold counters in VID_RST.
    fn handle_lcd_off(&mut self, is_mcycle: bool, mut result: PpuTickResult) -> PpuTickResult {
        if !is_mcycle {
            return result;
        }
        if self.pixel_pipeline.is_some() {
            self.pixel_pipeline = None;
            self.registers.clear_latches();
            result.lcd_disabled = true;
        }
        // Hardware holds counters at 0 continuously while LCD is off;
        // we reset each M-cycle to match. comparison_latched is not
        // updated — the comparison clock stops with the PPU.
        self.video.vid_rst();
        result
    }

    /// SUKO edge detect — combinational on hardware, fires on any
    /// phase where an enabled condition transitions inactive → active.
    /// LCD off: inputs hold static, latch freezes.
    pub fn check_stat_edge(&mut self) -> bool {
        if !self.control().video_enabled() {
            return false;
        }
        let stat_line_high = self.stat_line();
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
            Some(rendering) if !self.video.vblank() => {
                Some(rendering.pipeline_state(&self.video, &self.registers))
            }
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

    /// Construct a PPU from a gbtrace snapshot. Pipeline is created
    /// when LCD is enabled; VBlank derived from `LY >= 144`.
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
                meda: false,
                vsync_committed: lcd_on,
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
                background_or_overlay: None,
                bgp_recovery_active: false,
                bgp_visible_emit_since_tick: false,
                bgp_halt_wake_deferred: None,
                prev_besu: false,
            },
            bg_window_enabled_shadow: None,
            bg_window_enabled_shadow_just_set: false,
            sprites_enabled_shadow: None,
            sprites_enabled_shadow_just_set: false,
            sprites_enabled_pre_cupa: lcd_on,
        };

        Ppu {
            pixel_pipeline: if lcd_on { Some(Rendering::new()) } else { None },
            registers,
            video,
            oam,
            frame_number: 0,
            lcd_on_init_pending: false,
            oam_corruption: oam_corruption::OamCorruption::default(),
        }
    }
}
