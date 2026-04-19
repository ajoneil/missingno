//! Video timing and control orchestrator.
//!
//! After the Stage 1-4 decomposition, VideoControl is a thin composer
//! of the §1.2 Dividers, §2.14 StatInterrupt, and §2.6+§2.7 LineCounter
//! containers. The only state it still owns directly is the §6.4 NYPE
//! pipeline (`line_end_pending` / `delayed_line_end`), awaiting Stage 5
//! extraction after the §6.4 review.
//!
//! Dispatcher methods sequence the subsystems' per-edge work:
//!   on_lx_counter_clock_rise: §6.4 NYPE capture → §2.7 POPU/MYTA →
//!                             §2.6 LX advance → §2.6 SANU decode
//!   on_lx_counter_clock_fall: §2.6 RUTU fire → §2.7 LY advance+wrap →
//!                             §2.7 POPU holdover → §6.4 NYPE feed
//!
//! Pass-through accessors preserve the external call-site interface
//! per OQ.6 (`ly()`, `xupy()`, `line_end_active()`, etc.).

use crate::ppu::dividers::Dividers;
use crate::ppu::line_counter::LineCounter;
use crate::ppu::stat_interrupt::StatInterrupt;

/// Video timing and control (schematic page 21). Composes the extracted
/// subsystem containers; retains §6.4 NYPE state inline until Stage 5.
pub struct VideoControl {
    /// §1.2 WUVU + VENA clock dividers.
    pub dividers: Dividers,

    /// §2.6 + §2.7 LX and LY line counters, composed per hardware cascade.
    pub lines: LineCounter,

    /// §2.14 STAT Interrupt Generation (LYC register, enable bits,
    /// LYC-match pipeline, LALU edge-detection state).
    pub stat: StatInterrupt,

    /// §6.4 NYPE DFF output (delayed line-end). Clocked by TALU rising;
    /// captures the pending line-end state. Stays in VideoControl until
    /// Stage 5 extracts the LineEndPipeline container after §6.4 review.
    pub delayed_line_end: bool,

    /// §6.4 NYPE D input — pending line-end feed. Set when RUTU fires;
    /// consumed by NYPE on the next LX counter clock rise.
    pub line_end_pending: bool,
}

impl VideoControl {
    /// VID_RST: reset all subsystems. Used when the LCD is turned off
    /// and when LCD turns on (VID_RST released after initialization).
    pub fn vid_rst(&mut self) {
        self.dividers.vid_rst();
        self.lines.vid_rst();
        self.delayed_line_end = false;
        self.line_end_pending = false;
    }

    // ── Clock pass-throughs (Stage 3) ─────────────────────────

    /// TALU = VENA.Q — 1 MHz LX counter clock. Pass-through to Dividers.
    pub fn talu(&self) -> bool {
        self.dividers.talu()
    }

    /// XUPY — scan-counter / OAM-pipeline clock. Pass-through to Dividers.
    pub fn xupy(&self) -> bool {
        self.dividers.xupy()
    }

    // ── Line pass-throughs (Stage 4; OQ.6 high-traffic stability) ──

    /// CPU-visible LY value ($FF44). On line 153, frame-end reset (MYTA)
    /// drives LAMA low, making LY read as 0 while the internal counter
    /// is still 153. See `LineCounterY` two-level partition.
    pub fn ly(&self) -> u8 {
        self.lines.ly()
    }

    /// Hardware-internal LY (0-153); bypasses MYTA smoothing. Use for
    /// hardware-level checks (e.g., detecting the 153→0 internal wrap).
    pub fn ly_hardware(&self) -> u8 {
        self.lines.ly_hardware()
    }

    pub fn vblank(&self) -> bool {
        self.lines.vblank()
    }

    pub fn popu_active(&self) -> bool {
        self.lines.popu_active()
    }

    pub fn popu_holdover(&self) -> bool {
        self.lines.popu_holdover()
    }

    pub fn line_end_active(&self) -> bool {
        self.lines.line_end_active()
    }

    pub fn dot_position(&self) -> u8 {
        self.lines.dot_position()
    }

    pub fn write_ly(&mut self, value: u8) {
        self.lines.y.write_ly(value);
    }

    /// §6.4 NYPE output accessor. Still owned by VideoControl until
    /// Stage 5.
    pub fn delayed_line_end(&self) -> bool {
        self.delayed_line_end
    }

    // ── Cross-subsystem orchestration ─────────────────────────

    /// Triggers a PALY recompute against register-visible LY; consumes
    /// the MYTA propagation-race suppression flag from LineCounterY (see
    /// myta-investigation.md).
    pub fn update_ly_comparison(&mut self) {
        let ly = self.lines.ly();
        let suppress_onset = self.lines.y.take_myta_fired();
        self.stat.update_comparison(ly, suppress_onset);
    }

    /// LYC register write — CPU path. Updates LYC then recomputes PALY
    /// with MYTA-suppression consumed.
    pub fn write_lyc(&mut self, value: u8) {
        let ly = self.lines.ly();
        let suppress_onset = self.lines.y.take_myta_fired();
        self.stat.write_lyc(value, ly, suppress_onset);
    }

    // ── Per-dot tick ─────────────────────────────────────────

    /// XOTA rising edge: toggle WUVU (via Dividers) and clear POPU
    /// holdover (§2.7). Called every dot.
    pub fn tick_dot(&mut self) {
        self.dividers.tick_dot();
        self.lines.y.clear_popu_holdover();
    }

    // ── LX counter clock edges ───────────────────────────────

    /// LX counter clock rising edge (TALU rising). §6.4 NYPE captures
    /// line-end-pending; if NYPE rose, LineCounter.y captures POPU+MYTA;
    /// LineCounter.x advances and decodes SANU.
    pub fn on_lx_counter_clock_rise(&mut self) {
        let nype_rising = self.capture_nype_dff();
        self.lines.on_lx_counter_clock_rise(nype_rising);
    }

    /// LX counter clock falling edge (TALU falling). LineCounter fires
    /// the scanline boundary (RUTU + LY advance) atomically; if the
    /// boundary fired, §6.4 NYPE feed is set for the next rise.
    pub fn on_lx_counter_clock_fall(&mut self) -> bool {
        let scanline_boundary = self.lines.on_lx_counter_clock_fall();
        if scanline_boundary {
            self.feed_nype();
        }
        scanline_boundary
    }

    // ── §6.4 NYPE pipeline (stays until Stage 5) ─────────────

    /// §6.4 NYPE DFF captures `line_end_pending` on LX counter clock
    /// rising; returns true on NYPE rising edge (0→1 transition).
    fn capture_nype_dff(&mut self) -> bool {
        let nype_was = self.delayed_line_end;
        self.delayed_line_end = self.line_end_pending;
        self.line_end_pending = false;
        !nype_was && self.delayed_line_end
    }

    /// §6.4 NYPE D input: set `line_end_pending` so NYPE captures RUTU
    /// on the subsequent LX counter clock rising.
    fn feed_nype(&mut self) {
        self.line_end_pending = true;
    }
}
