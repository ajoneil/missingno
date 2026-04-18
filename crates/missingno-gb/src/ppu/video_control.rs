//! Video timing and control.
//!
//! The "LX counter clock" is the subsystem-idiomatic name for the 1 MHz
//! M-cycle-cadence clock that advances the LX scanline-position counter
//! and drives LY/LYC/NYPE/POPU/MYTA updates; TALU is its gate name in
//! the netlist. Edge methods use the role-based name
//! (`on_lx_counter_clock_rise` / `on_lx_counter_clock_fall`); gate-level
//! comments reference TALU directly where explaining hardware
//! derivations (e.g., `TALU = NOT(VENA)`, "TALU cascade" named-phenomenon
//! framing).

use bitflags::bitflags;

bitflags! {
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const OAM_SCAN             = 0b00100000;
        const VERTICAL_BLANK       = 0b00010000;
        const HORIZONTAL_BLANK     = 0b00001000;
    }
}

/// Video timing and control (schematic page 21).
///
/// Owns the dot clock dividers (WUVU/VENA), the scanline position
/// counter (LX), the scanline number counter (LY), the LYC comparator,
/// and the STAT interrupt enable flags. These signals sit together on
/// the die's video control section.
pub struct VideoControl {
    /// Dot position within the current scanline (SAXO-TYRY ripple counter).
    /// Counts 0-113, clocked by TALU (every 4 dots). When it reaches 113,
    /// the line-end sequence fires (SANU → RUTU).
    pub dot_position: u8,

    /// 2-dot clock divider (WUVU DFF). Toggles every dot on XOTA rising.
    /// Period = 2 dots. XUPY (= WUVU.qp) clocks the OAM scan counter.
    pub dot_divider: bool,

    /// 4-dot clock divider (VENA DFF). Toggles on WUVU falling edge.
    /// Period = 4 dots = 1 M-cycle. TALU (= VENA.qp) clocks LX, ROPO,
    /// and NYPE.
    pub mcycle_divider: bool,

    /// Internal scanline counter (MUWY-LAFO ripple counter). Counts
    /// 0-153, incrementing at each RUTU line-end pulse. On line 153,
    /// MYTA fires to reset the CPU-visible value to 0 early — see
    /// `ly()`. The internal counter stays at 153 until RUTU wraps it.
    pub ly: u8,

    /// LYC register (FF45). CPU-writable comparison target.
    pub lyc: u8,

    /// Pending LY==LYC comparison result (PALY combinational comparator).
    /// Recomputed at LX counter clock fall (via `update_ly_comparison()`)
    /// and on CPU writes to LYC. Promoted to `ly_comparison_latched` at
    /// LX counter clock rise by `latch_ly_comparison()`.
    pub ly_comparison_pending: bool,

    /// Latched LY==LYC result (ROPO DFF output). Unconditionally latched
    /// from `ly_comparison_pending` at each LX counter clock rise. Drives
    /// the LYC STAT interrupt source.
    /// Reset only by SYS_RST, NOT by VID_RST (LCD off/on).
    pub ly_comparison_latched: bool,

    /// STAT bit 2 visible value (RUPO NOR latch). Asymmetric update:
    /// match clearing is immediate (PAGO always drives "no match"),
    /// match onset requires ROPO DFF to latch. Also updated at LX counter
    /// clock rise (normal ROPO cadence). Frame-wrap follows ROPO only.
    pub ly_comparison_stat_visible: bool,

    /// STAT interrupt enable flags (FF41 bits 3-6).
    pub stat_flags: InterruptFlags,

    /// Previous STAT line state for rising-edge detection.
    pub stat_line_was_high: bool,

    /// Delayed line-end signal (NYPE DFF). Clocked by TALU rising edge,
    /// captures the previous RUTU state. Goes high 2 dots after RUTU,
    /// providing the delayed clock for POPU (VBlank) and MYTA (frame-end).
    pub delayed_line_end: bool,

    /// Pending line-end state for NYPE's D input. Set true when RUTU
    /// fires (LX counter clock fall of the LX=113 M-cycle), consumed by
    /// NYPE on the next LX counter clock rise.
    pub line_end_pending: bool,

    /// Line-end detected flag (SANU combinational gate). True when
    /// dot_position == 113. Set on LX counter clock rise, consumed by
    /// RUTU on the next LX counter clock fall (2 dots later, same M-cycle).
    pub line_end_detected: bool,

    /// Line-end pulse active (RUTU). Set at LX counter clock fall when
    /// the scanline boundary fires, cleared at the next LX counter clock
    /// rise. Duration: 2 dots. Drives TAPA (Mode 2 interrupt) and the
    /// line-144 VBlank STAT condition.
    pub line_end_active: bool,

    /// Frame-end reset flag (MYTA DFF). Set when NYPE rises while
    /// LY==153, causing `ly()` to return 0 (LAMA async-resets all LY
    /// DFFs). Cleared when the internal counter wraps 153→0 at RUTU.
    pub frame_end_reset: bool,

    /// MYTA new-match suppression. When MYTA fires, the PALY comparator
    /// runs normally at the next LX counter clock fall but any false-to-true
    /// transition (new match onset) is suppressed. This models the
    /// hardware's reg_old lag: PALY reads the registered LY value which
    /// doesn't reflect the MYTA async reset until one M-cycle later.
    /// True-to-false transitions (match clearing) are NOT suppressed,
    /// so LYC=153 clears in 1 M-cycle while LYC=0 onset is delayed.
    pub myta_suppress_new_match: bool,

    /// VBlank latch (POPU DFF). Clocked by NYPE rising edge, captures
    /// whether LY >= 144 from the previous cycle. When high, the PPU
    /// reports VBlank mode. Async-reset by VID_RST (LCD off).
    pub vblank: bool,

    /// POPU holdover counter. Models the NYPE→POPU DFF propagation delay
    /// at the 153→0 frame boundary: POPU stays high for 1 extra dot after
    /// the internal vblank flag clears, extending the STAT-visible mode 1
    /// window. Does NOT affect memory lock gates or VBlank IF request.
    pub popu_holdover: bool,
}

impl VideoControl {
    /// VID_RST: reset all video timing and control fields to their
    /// power-on state. Used when the LCD is turned off (VID_RST asserted)
    /// and when LCD turns on (VID_RST released after initialization).
    pub fn vid_rst(&mut self) {
        self.ly = 0;
        self.dot_position = 0;
        self.dot_divider = false;
        self.mcycle_divider = false;
        self.vblank = false;
        self.popu_holdover = false;
        self.delayed_line_end = false;
        self.line_end_pending = false;
        self.line_end_active = false;
        self.line_end_detected = false;
        self.frame_end_reset = false;
        self.myta_suppress_new_match = false;
    }

    /// TALU signal: buffered VENA.qp (4-dot M-cycle clock).
    /// Rising edge clocks LX, ROPO, and NYPE.
    pub fn talu(&self) -> bool {
        self.mcycle_divider
    }

    /// XUPY = NOT(WUVU). True when WUVU.Q is low. Clocks OAM scan
    /// counter (via GAVA), BYBA/CATU pipeline DFFs.
    pub fn xupy(&self) -> bool {
        !self.dot_divider
    }

    /// Delayed line-end signal (NYPE output). High for one M-cycle
    /// (4 dots; one LX-counter-clock period) starting 2 dots after RUTU fires.
    pub fn delayed_line_end(&self) -> bool {
        self.delayed_line_end
    }

    /// CPU-visible LY value. On line 153, the frame-end reset (MYTA)
    /// drives LAMA low, making LY read as 0 while the internal counter
    /// is still 153. Cleared when the internal counter wraps at RUTU.
    pub fn ly(&self) -> u8 {
        if self.frame_end_reset { 0 } else { self.ly }
    }

    pub fn ly_eq_lyc(&self) -> bool {
        self.ly_comparison_latched
    }

    /// RUPO output: the STAT bit 2 visible value. Clears immediately
    /// when comparison goes false; onset follows ROPO latch cadence.
    pub fn ly_eq_lyc_stat(&self) -> bool {
        self.ly_comparison_stat_visible
    }

    /// Update the STAT-visible comparison (RUPO) from the pending value.
    pub fn latch_stat_visible(&mut self) {
        self.ly_comparison_stat_visible = self.ly_comparison_pending;
    }

    /// RUPO immediate clear: when PALY goes false, PAGO (always asserted)
    /// immediately drives RUPO to "no match" without waiting for ROPO.
    /// Match onset (false→true) still requires the ROPO DFF pipeline.
    pub fn clear_stat_visible_if_no_match(&mut self) {
        if !self.ly_comparison_pending {
            self.ly_comparison_stat_visible = false;
        }
    }

    pub fn write_ly(&mut self, value: u8) {
        self.ly = value;
    }

    /// ROPO DFF latch: unconditionally capture the pending PALY comparison
    /// result at LX counter clock rise. On hardware, RUPO (the NOR latch
    /// downstream of ROPO) is transparent because PAGO is permanently held
    /// high during normal operation. STAT bit 2 and the STAT interrupt both
    /// read this value.
    pub fn latch_ly_comparison(&mut self) {
        self.ly_comparison_latched = self.ly_comparison_pending;
    }

    /// PALY combinational comparator: recompute the pending LY==LYC
    /// result. Called at LX counter clock fall and on CPU writes to LYC.
    ///
    /// When `myta_suppress_new_match` is active, the comparison runs
    /// normally but only false-to-true transitions (new match onset)
    /// are suppressed. True-to-false transitions (match clearing) pass
    /// through, modeling the reg_old lag after MYTA async reset.
    pub fn update_ly_comparison(&mut self) {
        let result = self.ly() == self.lyc;

        if self.myta_suppress_new_match {
            self.myta_suppress_new_match = false;
            // Only apply clearing (true→false); suppress new match onset
            // (false→true). The old LY value is gone from reg_old so PALY
            // sees the mismatch, but the new LY=0 hasn't propagated yet.
            if !result {
                self.ly_comparison_pending = false;
            }
            return;
        }

        self.ly_comparison_pending = result;
    }

    /// Whether the line-end pulse is active (RUTU, 2-dot window at
    /// each scanline boundary).
    pub fn line_end_active(&self) -> bool {
        self.line_end_active
    }

    /// VBlank latch output (POPU). True during VBlank (lines 144-153),
    /// activated at NYPE rising edge rather than immediately at LY change.
    pub fn vblank(&self) -> bool {
        self.vblank
    }

    /// Whether POPU is effectively active for STAT mode and interrupt
    /// purposes. Includes the holdover period after the internal vblank
    /// flag clears at the 153→0 boundary, modeling the NYPE→POPU DFF
    /// propagation delay. NOT used for memory lock gates or VBlank IF.
    pub fn popu_active(&self) -> bool {
        self.vblank || self.popu_holdover
    }

    // ── Clock divider ticks ──────────────────────────────────

    /// XOTA rising edge: toggle the 2-dot divider (WUVU). Called every dot.
    pub fn tick_dot(&mut self) {
        self.dot_divider = !self.dot_divider;
        self.popu_holdover = false;
    }

    /// Whether the 2-dot divider (WUVU) just fell. Check after `tick_dot()`
    /// to know if the M-cycle divider should toggle.
    pub fn dot_divider_fell(&self) -> bool {
        !self.dot_divider
    }

    /// Toggle the M-cycle divider (VENA). Called when the 2-dot divider
    /// falls. Returns the previous TALU state so the caller can detect edges.
    pub fn tick_mcycle_divider(&mut self) -> bool {
        let talu_was = self.mcycle_divider;
        self.mcycle_divider = !self.mcycle_divider;
        talu_was
    }

    // ── LX counter clock rise (gate: TALU rising): NYPE, POPU, MYTA, LX, SANU ──

    /// LX counter clock rising edge (gate: TALU rising). Dispatcher —
    /// four hardware events fire in sequence at this edge:
    ///
    /// 1. NYPE DFF captures the pending line-end state
    /// 2. NYPE rising edge clocks POPU (VBlank) and MYTA (frame-end)
    /// 3. LX advances (suppressed during RUTU line-end pulse)
    /// 4. SANU detects LX=113 (line-end for this scanline)
    pub fn on_lx_counter_clock_rise(&mut self) {
        // 1. NYPE DFF: latch pending line-end on TALU rising.
        let nype_was = self.delayed_line_end;
        self.delayed_line_end = self.line_end_pending;
        self.line_end_pending = false;

        // 2. NYPE rising edge → POPU (VBlank latch) and MYTA (frame-end).
        if !nype_was && self.delayed_line_end {
            // POPU DFF: latch whether we're in VBlank (LY >= 144).
            self.vblank = self.ly >= 144;

            // MYTA DFF: latch frame-end (LY == 153). Makes ly() return 0.
            if self.ly == 153 {
                self.frame_end_reset = true;
                self.myta_suppress_new_match = true;
            }
        }

        // 3. Advance dot position (LX). Suppressed during RUTU line-end
        //    pulse — MUDE async-resets LX at the same TALU falling as RUTU.
        if !self.line_end_active {
            self.dot_position += 1;
        }
        self.line_end_active = false;

        // 4. SANU: combinational detect of LX reaching 113.
        self.line_end_detected = self.dot_position == 113;
    }

    // ── LX counter clock fall (gate: TALU falling): RUTU ──────

    /// LX counter clock falling edge (gate: TALU falling). Fires RUTU
    /// if line-end was detected; returns true at scanline boundary.
    pub fn on_lx_counter_clock_fall(&mut self) -> bool {
        if self.line_end_detected {
            self.line_end_detected = false;
            if self.ly >= 153 {
                self.ly = 0;
                self.frame_end_reset = false;
                self.popu_holdover = true;
                self.vblank = false;
            } else {
                self.ly += 1;
            }
            self.dot_position = 0;
            self.line_end_active = true;
            self.line_end_pending = true;
            return true;
        }

        false
    }
}
