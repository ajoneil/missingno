//! PPU timing is measured in **dots** — one dot = one master clock
//! period (`ck1_ck2`). The PPU clock (ALET) is the 4 MHz subsystem
//! clock derived from `ck1_ck2`; ALET-clocked DFFs capture on one
//! edge, MYVO-clocked DFFs on the other. 1 dot = 1 T-cycle = 2 atal
//! half-cycles.

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
mod oam_corruption;
mod register_io;
pub mod registers;
pub mod rendering;
mod scan;
pub mod screen;
pub mod stat_interrupt;
pub mod types;
pub mod video_control;

// ── PPU output ──────────────────────────────────────────────────────────

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

#[derive(Default)]
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

/// Internal PPU signals exposed for gbtrace capture. These mirror
/// specific DFFs/NOR-latches and are otherwise of no interest to
/// callers.
#[derive(Clone, Copy, Debug)]
pub struct TraceSignals {
    pub wuvu: bool,
    pub vena: bool,
    pub xupy: bool,
    pub besu: bool,
    pub wodu: bool,
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

// ── PPU state ───────────────────────────────────────────────────────────

pub struct Ppu {
    /// Pixel pipeline state. `None` = LCD off (VID_RST asserted). The
    /// pipeline persists through both active display and VBlank;
    /// VBlank is derived from `video.vblank` (POPU).
    pub(super) pixel_pipeline: Option<Rendering>,
    pub registers: PipelineRegisters,
    pub video: VideoControl,
    pub oam: Oam,
    /// Frame counter for gbtrace output.
    pub frame_number: u16,
    /// CUPA↑ → XODO↓ scheduling: set on LCDC.7 0→1 in the rise-edge
    /// staged write; consumed in the same fall.
    pub(super) lcd_on_init_pending: bool,
    /// OAM-bug arming (BOWA → MOPA window).
    pub(super) oam_corruption: oam_corruption::OamCorruption,
}

// ── Construction ────────────────────────────────────────────────────────

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

// ── Position / state accessors ──────────────────────────────────────────

impl Ppu {
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

    /// True when the WUSA NOR-latch is open — LCD is actively shifting
    /// pixels. Gates LCDC.0/.1 overlay arming during prelude.
    pub(super) fn lcd_pushing_active(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.lcd_pushing_active())
    }
}

// ── Mode + memory locks ─────────────────────────────────────────────────

impl Ppu {
    /// STAT mode bits — independent NOR gates on the rendering /
    /// scanning / vblank lines (schematic page 21):
    ///   bit 0 = XYMU OR POPU   (rendering OR vblank)
    ///   bit 1 = ACYL OR XYMU   (scanning OR rendering)
    /// CPU STAT reads use the cpu_port_d bus model to sample at dot 2.
    pub fn mode(&self) -> Mode {
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
            .is_some_and(|r| r.oam_locked())
    }

    /// VRAM read-locked: XYMU asserted.
    pub fn vram_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.vram_locked())
    }

    pub fn oam_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.oam_write_locked())
    }

    pub fn vram_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.vram_write_locked())
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

    pub(crate) fn read_oam(&self, address: OamAddress) -> u8 {
        self.oam.read(address)
    }

    pub(crate) fn write_oam(&mut self, address: OamAddress, value: u8) {
        self.oam.write(address, value);
    }
}

// ── STAT interrupt ──────────────────────────────────────────────────────

impl Ppu {
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
}

// ── Inspection / trace ──────────────────────────────────────────────────

impl Ppu {
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

    /// LALU edge-detector state (STAT line previous value). Exposed
    /// for gbtrace snapshot capture.
    pub fn stat_line_was_high(&self) -> bool {
        self.video.stat.line_was_high()
    }

    /// Snapshot the per-DFF signals that gbtrace exposes as individual
    /// trace columns. Hot path: called once per row.
    pub fn trace_signals(&self) -> TraceSignals {
        let sprites_enabled = self.registers.control.sprites_enabled();
        let (besu, wodu) = self
            .pixel_pipeline
            .as_ref()
            .map(|r| (r.scan_besu(), r.wodu(sprites_enabled)))
            .unwrap_or((false, false));
        TraceSignals {
            wuvu: self.video.dividers.half_mcycle,
            vena: self.video.dividers.mcycle(),
            xupy: self.video.dividers.xupy(),
            besu,
            wodu,
            stat_line: self.stat_line(),
        }
    }

    /// Single-signal accessor for the BGP recovery edge detector. Not
    /// part of the public trace surface; used internally by the
    /// master-clock-fall path.
    pub(super) fn besu(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .is_some_and(|r| r.scan_besu())
    }
}
