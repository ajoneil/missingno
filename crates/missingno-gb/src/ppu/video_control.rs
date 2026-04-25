//! Video timing and control orchestrator.
//!
//! After the Stage 1-4 decomposition, VideoControl is a thin composer
//! of the §1.2 Dividers, §8 StatInterrupt, and §2.1+§2.2 LineCounter
//! containers. The only state it still owns directly is the §7.3 NYPE
//! pipeline (`line_end_pending` / `delayed_line_end`), awaiting Stage 5
//! extraction after the §7.3 review.
//!
//! Dispatcher methods sequence the subsystems' per-edge work:
//!   on_lx_counter_clock_rise: §7.3 NYPE capture → §2.2 POPU/MYTA →
//!                             §2.1 LX advance → §2.1 SANU decode
//!   on_lx_counter_clock_fall: §2.1 RUTU fire → §2.2 LY advance+wrap →
//!                             §2.2 POPU holdover → §7.3 NYPE feed
//!
//! Pass-through accessors preserve the external call-site interface
//! per OQ.6 (`ly()`, `xupy()`, `line_end_active()`, etc.).

use crate::ppu::dividers::Dividers;
use crate::ppu::line_counter::LineCounter;
use crate::ppu::line_end_pipeline::LineEndPipeline;
use crate::ppu::stat_interrupt::StatInterrupt;

/// Video timing and control (schematic page 21). Composes the extracted
/// subsystem containers; retains §7.3 NYPE state inline until Stage 5.
pub struct VideoControl {
    /// §1.2 WUVU + VENA clock dividers.
    pub dividers: Dividers,

    /// §2.1 + §2.2 LX and LY line counters, composed per hardware cascade.
    pub lines: LineCounter,

    /// §8 STAT Interrupt Generation (LYC register, enable bits,
    /// LYC-match pipeline, LALU edge-detection state).
    pub stat: StatInterrupt,

    /// NYPE LINE_END redistribution DFF — produces NypeEdge
    /// (rising / falling / none) for POPU / MYTA dispatch per hardware.
    pub line_end: LineEndPipeline,
}

impl VideoControl {
    /// VID_RST: reset all subsystems. Used when the LCD is turned off
    /// and when LCD turns on (VID_RST released after initialization).
    pub fn vid_rst(&mut self) {
        self.dividers.vid_rst();
        self.lines.vid_rst();
        self.line_end.vid_rst();
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

    /// NYPE output accessor — pass-through to LineEndPipeline.
    pub fn delayed_line_end(&self) -> bool {
        self.line_end.delayed_line_end()
    }

    // ── Cross-subsystem orchestration ─────────────────────────

    /// Triggers a PALY recompute against register-visible LY.
    pub fn update_ly_comparison(&mut self) {
        let ly = self.lines.ly();
        self.stat.update_comparison(ly);
    }

    /// LYC register write — CPU path. Updates LYC then recomputes PALY.
    pub fn write_lyc(&mut self, value: u8) {
        let ly = self.lines.ly();
        self.stat.write_lyc(value, ly);
    }

    // ── Per-dot tick ─────────────────────────────────────────

    /// XOTA rising edge: toggle WUVU (via Dividers) and clear POPU
    /// holdover (§2.2). Called every dot.
    pub fn tick_dot(&mut self) {
        self.dividers.tick_dot();
        self.lines.y.clear_popu_holdover();
    }

    // ── LX counter clock edges ───────────────────────────────

    /// LX counter clock rising edge (TALU rising). NYPE captures on
    /// this edge and reports which Q-transition occurred (Rising /
    /// Falling / None); LineCounter dispatches POPU (Rising) or MYTA
    /// (Falling). LineCounter.x advances and decodes SANU regardless.
    pub fn on_lx_counter_clock_rise(&mut self) {
        let nype_edge = self.line_end.capture();
        self.lines.on_lx_counter_clock_rise(nype_edge);
    }

    /// LX counter clock falling edge (TALU falling). LineCounter fires
    /// the scanline boundary (RUTU + LY advance) atomically; if the
    /// boundary fired, NYPE feed is signalled for the next rise.
    pub fn on_lx_counter_clock_fall(&mut self) -> bool {
        let scanline_boundary = self.lines.on_lx_counter_clock_fall();
        if scanline_boundary {
            self.line_end.signal_line_end();
        }
        scanline_boundary
    }
}
