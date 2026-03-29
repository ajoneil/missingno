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
    /// Computed each M-cycle, promoted to `ly_comparison_latched` on the
    /// next TALU rising edge by the ROPO DFF. Models the `_old` input
    /// to ROPO — ROPO always samples the previous cycle's comparison.
    pub ly_comparison_pending: bool,

    /// Latched LY==LYC result (ROPO DFF output). This is what STAT
    /// bit 2 reads and what drives the LYC STAT interrupt source.
    pub ly_comparison_latched: bool,

    /// STAT interrupt enable flags (FF41 bits 3-6).
    pub stat_flags: InterruptFlags,

    /// Previous STAT line state for rising-edge detection.
    pub stat_line_was_high: bool,

    /// Delayed line-end signal (NYPE DFF). Clocked by TALU rising edge,
    /// captures the previous RUTU state. Goes high 2 dots after RUTU,
    /// providing the delayed clock for POPU (VBlank) and MYTA (frame-end).
    pub delayed_line_end: bool,

    /// Pending line-end state for NYPE's D input. Set true when RUTU
    /// fires (TALU falling of the LX=113 M-cycle), consumed by NYPE
    /// on the next TALU rising edge.
    pub line_end_pending: bool,

    /// Line-end detected flag (SANU combinational gate). True when
    /// dot_position == 113. Set on TALU rising, consumed by RUTU on
    /// the next TALU falling (2 dots later, same M-cycle).
    pub line_end_detected: bool,

    /// Line-end pulse active (RUTU). Set at TALU falling when the
    /// scanline boundary fires, cleared at the next TALU rising.
    /// Duration: 2 dots. Drives TAPA (Mode 2 interrupt) and the
    /// line-144 VBlank STAT condition.
    pub line_end_active: bool,

    /// Frame-end reset flag (MYTA DFF). Set when NYPE rises while
    /// LY==153, causing `ly()` to return 0 (LAMA async-resets all LY
    /// DFFs). Cleared when the internal counter wraps 153→0 at RUTU.
    pub frame_end_reset: bool,

    /// VBlank latch (POPU DFF). Clocked by NYPE rising edge, captures
    /// whether LY >= 144 from the previous cycle. When high, the PPU
    /// reports VBlank mode. Async-reset by VID_RST (LCD off).
    pub vblank: bool,
}

impl VideoControl {
    /// TALU signal: buffered VENA.qp (4-dot M-cycle clock).
    /// Rising edge clocks LX, ROPO, and NYPE.
    pub fn talu(&self) -> bool {
        self.mcycle_divider
    }

    /// XUPY signal: buffered WUVU.qp (2-dot clock).
    /// Clocks OAM scan counter (via GAVA), BYBA/CATU pipeline DFFs.
    pub fn xupy(&self) -> bool {
        self.dot_divider
    }

    /// Delayed line-end signal (NYPE output). High for one TALU period
    /// (4 dots) starting 2 dots after RUTU fires.
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

    pub fn write_ly(&mut self, value: u8) {
        self.ly = value;
    }

    /// ROPO DFF latch: promote the pending comparison to the STAT-visible
    /// output, then recompute the pending value for next cycle.
    pub fn latch_ly_comparison(&mut self) {
        self.ly_comparison_latched = self.ly_comparison_pending;
        self.ly_comparison_pending = self.ly() == self.lyc;
    }

    /// PALY combinational comparator: recompute the pending LY==LYC
    /// result. Call after any LY modification (RUTU increment, MYTA
    /// reset) so the next ROPO latch sees the updated value.
    pub fn update_ly_comparison(&mut self) {
        self.ly_comparison_pending = self.ly() == self.lyc;
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

    // ── Clock divider ticks ──────────────────────────────────

    /// XOTA rising edge: toggle the 2-dot divider (WUVU). Called every dot.
    pub fn tick_dot(&mut self) {
        self.dot_divider = !self.dot_divider;
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

    // ── TALU rising edge: NYPE, POPU, MYTA, LX, SANU ────────

    /// TALU rising edge processing. Four hardware events fire in sequence:
    ///
    /// 1. NYPE DFF captures the pending line-end state
    /// 2. NYPE rising edge clocks POPU (VBlank) and MYTA (frame-end)
    /// 3. LX advances (suppressed during RUTU line-end pulse)
    /// 4. SANU detects LX=113 (line-end for this scanline)
    pub fn tick_talu_rise(&mut self) {
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

    // ── TALU falling edge: RUTU ──────────────────────────────

    /// TALU falling edge: fire RUTU if line-end was detected.
    /// Returns true at scanline boundary (RUTU fires).
    pub fn tick_talu_fall(&mut self) -> bool {
        if self.line_end_detected {
            self.line_end_detected = false;
            if self.ly >= 153 {
                self.ly = 0;
                self.frame_end_reset = false;
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
