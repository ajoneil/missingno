//! PPU timing measured in dots (master clock periods, ck1_ck2). 1 dot = 1 T-cycle.

use dividers::Dividers;
use line_counter::{LineCounter, LineCounterX, LineCounterY};
use line_end_pipeline::LineEndPipeline;
use memory::{Oam, OamAddress};
use registers::{BackgroundViewportPosition, Window};
use rendering::Rendering;
use types::control::{Control, ControlFlags};
use types::palette::Palettes;
use types::sprites::{Sprite, SpriteId};

pub use dff::{DffLatch, NorLatch};
pub use model::{
    CartridgeBootHeader, ColorRegister, DmgPixel, DomainSamples, PixelMux, PpuModel,
    resolve_dmg_pixel, resolve_shade,
};
pub use registers::PipelineRegisters;
pub use rendering::{
    Mode, PipelineSnapshot, PpuTraceSnapshot, SpriteFetchPhase, SpriteStoreEntrySnapshot,
    SpriteStoreSnapshot,
};
pub use stat_interrupt::{InterruptFlags, StatInterrupt};
pub use video_control::VideoControl;

mod clock_edges;
mod dff;
mod dividers;
mod draw;
mod line_counter;
mod line_end_pipeline;
pub mod memory;
pub mod model;
mod oam_corruption;
mod register_io;
pub mod registers;
pub mod rendering;
mod scan;
pub mod screen;
pub mod stat_interrupt;
pub mod types;
pub mod video_control;

/// A finished pixel pushed to the LCD on a SACU edge, carrying the console's
/// resolved framebuffer pixel (DMG shade / CGB RGB555).
#[derive(Clone, Copy, Debug)]
pub struct DrawnPixel<Pix> {
    pub x: u8,
    pub y: u8,
    pub color: Pix,
}

/// A pixel pushed to the LCD, as a 2-bit shade for the gbtrace pixel stream.
#[derive(Clone, Copy, Debug)]
pub struct PixelOutput {
    pub x: u8,
    pub y: u8,
    pub shade: u8,
}

pub struct PpuTickResult<Pix> {
    pub pixel: Option<DrawnPixel<Pix>>,
    /// MEDA VSYNC pulse — LY wrapped at end of line 153.
    pub new_frame: bool,
    /// LCDC.7 went 1→0 mid-pipeline; caller should blank the screen.
    pub lcd_disabled: bool,
    pub request_vblank: bool,
    /// SUKO 0→1 detected by the two-phase rule: post-fast snapshot uses pre-advance vblank
    /// for Mode 0 / Mode 2 legs (TOLU 1-gate lag); final snapshot uses live vblank. Fires
    /// only if SUKO actually transitions through zero across the two snapshots.
    pub request_stat: bool,
}

impl<Pix> Default for PpuTickResult<Pix> {
    fn default() -> Self {
        Self {
            pixel: None,
            new_frame: false,
            lcd_disabled: false,
            request_vblank: false,
            request_stat: false,
        }
    }
}

/// Internal PPU DFF/latch signals exposed for gbtrace capture.
#[derive(Clone, Copy, Debug)]
pub struct TraceSignals {
    /// WUVU.Q — 2-dot divider.
    pub half_mcycle: bool,
    /// VENA.Q — 4-dot divider.
    pub mcycle: bool,
    /// XUPY = WUVU.Q — scan-counter / OAM-pipeline clock.
    pub scan_clock: bool,
    /// BESU — Mode 2 OAM-scan + locks asserted.
    pub mode2_active: bool,
    /// WODU = AND2(XUGU, !FEPO) — HBlank STAT contributor.
    pub end_of_visible_line: bool,
    pub stat_line: bool,
}

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

pub struct Ppu<P: PpuModel> {
    /// `None` while LCD is off (VID_RST asserted).
    pub(super) pixel_pipeline: Option<Rendering<P>>,
    pub registers: PipelineRegisters,
    pub video: VideoControl,
    pub oam: Oam,
    pub frame_number: u16,
    /// CUPA↑ → XODO↓: set on LCDC.7 0→1 in the rise-edge staged write, consumed in the same fall.
    pub(super) lcd_on_init_pending: bool,
    pub(super) oam_corruption: oam_corruption::OamCorruption,
    /// The console's colour hardware (CRAM, OPRI, …); the DMG impl is a unit.
    pub(super) model: P,
}

impl<P: PpuModel> Ppu<P> {
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
                    x: DffLatch::new(0),
                },
                palettes: Palettes::default(),
                bg_window_enabled_overlay: registers::OldOverlay::default(),
                sprites_enabled_overlay: registers::OldOverlay::default(),
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
                        frame_end_reset: false,
                    },
                },
                stat: StatInterrupt::power_on(),
                line_end: LineEndPipeline {
                    delayed_line_end: false,
                    line_end_pending: false,
                    vsync_active: false,
                    vsync_committed: false,
                },
            },
            oam: Oam::default(),
            pixel_pipeline: None,
            frame_number: 0,
            lcd_on_init_pending: false,
            oam_corruption: oam_corruption::OamCorruption::default(),
            model: P::default(),
        }
    }

    /// State equivalent to what the DMG boot ROM leaves at first PC=$0100.
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
                vsync_active: false,
                vsync_committed: true,
            },
        };
        ppu.pixel_pipeline = Some(Rendering::post_boot());
        ppu.registers.sprites_enabled_pre_cupa = true;
        ppu
    }

    /// Boot-ROM residue in the object palette latches (the CGB boot ROM
    /// zeroes OBP0/OBP1; the DMG boot ROM leaves them at $FF).
    pub fn set_post_boot_object_palettes(&mut self, value: u8) {
        self.registers.palettes.sprite0 = DffLatch::new(value);
        self.registers.palettes.sprite1 = DffLatch::new(value);
    }

    /// Post-boot state with a model-specific handoff phase: mid-VBlank
    /// on line `ly` at horizontal counter `lx`.
    pub fn post_boot_vblank_handoff(ly: u8, lx: u8) -> Self {
        let mut ppu = Self::post_boot();
        ppu.video.lines.x.value = lx;
        ppu.video.lines.y = LineCounterY::vblank_handoff(ly);
        ppu.video.stat = StatInterrupt::post_boot_at_line(ly);
        ppu
    }

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
                    frame_end_reset: false,
                },
            },
            stat: StatInterrupt {
                lyc: snap.lyc,
                comparison_pending: snap.ly == snap.lyc,
                comparison_latched: snap.ly == snap.lyc,
                enables,
                legs_was_high: InterruptFlags::empty(),
                conditions_was: InterruptFlags::empty(),
                irq_domain: stat_interrupt::StatIrqDomain::mirroring(enables, snap.lyc),
            },
            line_end: LineEndPipeline {
                delayed_line_end: false,
                line_end_pending: false,
                vsync_active: false,
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
                x: DffLatch::new(snap.wx),
            },
            palettes: Palettes {
                background: DffLatch::new(snap.bgp),
                sprite0: DffLatch::new(snap.obp0),
                sprite1: DffLatch::new(snap.obp1),
                recovery: types::palette::BgpRecovery::default(),
                bgp_halt_wake_deferred: None,
            },
            bg_window_enabled_overlay: registers::OldOverlay::default(),
            sprites_enabled_overlay: registers::OldOverlay::default(),
            sprites_enabled_pre_cupa: lcd_on,
        };

        let mut ppu = Ppu {
            pixel_pipeline: if lcd_on { Some(Rendering::new()) } else { None },
            registers,
            video,
            oam,
            frame_number: 0,
            lcd_on_init_pending: false,
            oam_corruption: oam_corruption::OamCorruption::default(),
            model: P::default(),
        };
        if let Some(rendering) = ppu.pixel_pipeline.as_mut() {
            rendering.capture_window_register_sync(&ppu.registers);
        }
        ppu
    }
}

impl<P: PpuModel> Ppu<P> {
    pub fn model(&self) -> &P {
        &self.model
    }

    pub fn lx(&self) -> u8 {
        self.video.dot_position()
    }

    /// MEDA has gone 0→1 since the most recent VID_RST deassertion — first VSYNC has fired.
    pub fn vsync_committed(&self) -> bool {
        self.video.line_end.vsync_committed
    }

    pub fn scan_counter(&self) -> Option<u8> {
        self.pixel_pipeline.as_ref().map(|r| r.scan_counter_entry())
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    /// Latched LY==LYC (ROPO output).
    pub fn ly_eq_lyc(&self) -> bool {
        self.video.stat.ly_eq_lyc()
    }

    pub fn is_rendering(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.rendering_active())
    }

    /// WUSA NOR-latch open — LCD shifting pixels. Gates LCDC.0/.1 overlay arming during prelude.
    pub(super) fn lcd_pushing_active(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.lcd_pushing_active())
    }
}

impl<P: PpuModel> Ppu<P> {
    /// STAT mode bits: bit0 = XYMU OR POPU, bit1 = ACYL OR XYMU.
    pub fn mode(&self) -> Mode {
        let rendering = match &self.pixel_pipeline {
            Some(r) => r,
            None => return Mode::VerticalBlank,
        };
        let rendering_active = rendering.rendering_active();
        let bit0 = rendering_active || self.video.vblank();
        let bit1 = rendering_active || rendering.scan_mode2_active();
        match (bit1, bit0) {
            (false, false) => Mode::HorizontalBlank,
            (false, true) => Mode::VerticalBlank,
            (true, false) => Mode::OamScan,
            (true, true) => Mode::Drawing,
        }
    }

    /// Configure the PPU model from the cartridge at post-boot (DMG-compat on
    /// the CGB). DMG is a no-op.
    pub fn init_model_post_boot(&mut self, header: &model::CartridgeBootHeader) {
        self.model.init_post_boot(header);
    }

    /// CPU read/write of OPRI ($FF6C). DMG has no such register (reads 0xFF).
    pub fn read_object_priority(&self) -> u8 {
        self.model.object_priority_register()
    }

    pub fn write_object_priority(&mut self, value: u8) {
        self.model.set_object_priority_register(value);
    }

    /// M-cycle-boundary rise: the CGB palette block's clock-domain sample
    /// (CRAM mode-3 lock). The STAT register-file synchroniser captures on
    /// the M-boundary fall instead — see `eval_synced` in the fall path.
    pub fn tick_clock_domain_capture(&mut self) {
        if P::HAS_CLOCK_DOMAIN_SYNC {
            let drawing = self.mode() == Mode::Drawing;
            self.model
                .tick_clock_domain(model::DomainSamples { drawing });
        }
    }

    /// Double-speed M-boundary fall on the half-dot edge that carries no PPU
    /// fall: the CPU-clocked register synchroniser still captures (the PPU
    /// condition inputs are unchanged since the last dot's evaluation, so
    /// only register-path edges can race here).
    pub fn capture_register_sync_standalone(&mut self) -> bool {
        if !P::HAS_CLOCK_DOMAIN_SYNC {
            return false;
        }
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.capture_window_register_sync(&self.registers);
        }
        let conditions = self.stat_conditions();
        let ly = self.video.ly();
        self.video.stat.eval_synced(conditions, false, true, ly)
    }

    /// CPU read of a CGB colour-palette register; the model's clock-domain
    /// sample supplies the data-port lock.
    pub fn read_color_register(&self, register: ColorRegister) -> u8 {
        self.model.read_color_register(register)
    }

    /// CPU write of a CGB colour-palette register.
    pub fn write_color_register(&mut self, register: ColorRegister, value: u8) {
        self.model.write_color_register(register, value);
    }

    pub fn oam_locked(&self) -> bool {
        self.pixel_pipeline.as_ref().is_some_and(|r| r.oam_locked())
    }

    pub fn vram_locked(&self) -> bool {
        let live = self
            .pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.vram_locked());
        self.model.vram_cpu_lock(live)
    }

    pub fn oam_write_locked(&self) -> bool {
        self.pixel_pipeline.as_ref().is_some_and(|r| {
            if P::REVISED_OAM_LOCK {
                r.oam_locked()
            } else {
                r.oam_write_locked()
            }
        })
    }

    pub fn vram_write_locked(&self) -> bool {
        if P::REVISED_OAM_LOCK {
            self.vram_locked()
        } else {
            self.pixel_pipeline
                .as_ref()
                .is_some_and(|r| r.vram_write_locked())
        }
    }

    pub fn write_lock(&self, address: u16) -> Option<bool> {
        match address {
            // The OAM access gates cover the whole $FE page, extra rows included.
            0xFE00..=0xFEFF => Some(self.oam_write_locked()),
            0x8000..=0x9FFF => Some(self.vram_write_locked()),
            _ => None,
        }
    }

    pub fn read_locked(&self, address: u16) -> bool {
        match address {
            0xFE00..=0xFEFF => self.oam_locked(),
            0x8000..=0x9FFF => self.vram_locked(),
            _ => false,
        }
    }

    /// `Some(locked)` for the OAM/VRAM read-lock ranges, `None` elsewhere — the
    /// pre-grid lock view the double-speed read latch resolves against.
    pub fn read_lock(&self, address: u16) -> Option<bool> {
        match address {
            0xFE00..=0xFEFF => Some(self.oam_locked()),
            0x8000..=0x9FFF => Some(self.vram_locked()),
            _ => None,
        }
    }

    pub fn read_oam(&self, address: OamAddress) -> u8 {
        self.oam.read(address)
    }

    pub fn write_oam(&mut self, address: OamAddress, value: u8) {
        self.oam.write(address, value);
    }
}

impl<P: PpuModel> Ppu<P> {
    /// SUKO condition vector — the per-source condition inputs (ROPO and the
    /// TARU/PARU/TAPA terms), pre-enable. Shared DMG silicon on both
    /// consoles. The mode-2 term is TAPA = TOLU AND SELA: the line-144 mode-2
    /// IRQ comes from RUTU's line-end pulse landing before POPU rises (NYPE
    /// lag), not from a vblank extension of the leg.
    fn stat_conditions(&self) -> InterruptFlags {
        let mut conditions = InterruptFlags::empty();

        // ROPO is frozen across LCD-off, so the LYC condition stays live
        // while the LCD is off — only the mode terms go quiet.
        if self.video.stat.ly_eq_lyc() {
            conditions |= InterruptFlags::CURRENT_LINE_COMPARE;
        }

        // Mode terms need the pixel pipeline, which is held in reset while LCD off.
        let Some(rendering) = &self.pixel_pipeline else {
            return conditions;
        };

        let vblank = self.video.vblank();
        let sprites_enabled = self.registers.control.sprites_enabled();

        if !vblank
            && (rendering.end_of_line_signal(sprites_enabled) || rendering.terminal_wodu_pulse())
        {
            conditions |= InterruptFlags::HORIZONTAL_BLANK;
        }
        if vblank {
            conditions |= InterruptFlags::VERTICAL_BLANK;
        }
        if !vblank && rendering.mode2_interrupt_active(&self.video) {
            conditions |= InterruptFlags::OAM_SCAN;
        }
        conditions
    }

    /// SUKO source-leg vector — one bit per enabled-source AND-term (matches AO2222 structure).
    /// The CGB reads the enables through the register-file synchroniser.
    pub fn stat_legs(&self) -> InterruptFlags {
        let enables = if P::HAS_CLOCK_DOMAIN_SYNC {
            self.video.stat.synced_enables()
        } else {
            self.video.stat.enables()
        };
        self.stat_conditions() & enables
    }

    /// SUKO combined output (= any leg active).
    pub fn stat_line(&self) -> bool {
        !self.stat_legs().is_empty()
    }

    /// LALU edge detect: fires on SUKO 0→1, with the pulse-width filter applied on
    /// TALU↑ evaluations (callable off-TALU; pulse-width filter is skipped).
    pub fn check_stat_edge(&mut self) -> bool {
        if !self.control().video_enabled() {
            return false;
        }
        let conditions = self.stat_conditions();
        if P::HAS_CLOCK_DOMAIN_SYNC {
            let ly = self.video.ly();
            return self.video.stat.eval_synced(conditions, false, false, ly);
        }
        self.video.stat.eval_conditions(conditions, false)
    }
}

impl<P: PpuModel> Ppu<P> {
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

    pub fn stat_line_was_high(&self) -> bool {
        !self.video.stat.legs_was_high().is_empty()
    }

    pub fn trace_signals(&self) -> TraceSignals {
        let sprites_enabled = self.registers.control.sprites_enabled();
        let (mode2_active, end_of_visible_line) = self
            .pixel_pipeline
            .as_ref()
            .map(|r| (r.scan_mode2_active(), r.end_of_line_signal(sprites_enabled)))
            .unwrap_or((false, false));
        TraceSignals {
            half_mcycle: self.video.dividers.half_mcycle,
            mcycle: self.video.dividers.mcycle(),
            scan_clock: self.video.dividers.scan_clock(),
            mode2_active,
            end_of_visible_line,
            stat_line: self.stat_line(),
        }
    }

    /// Used internally by the master-clock-fall path for the BGP recovery edge detector.
    pub(super) fn mode2_active(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.scan_mode2_active())
    }
}
