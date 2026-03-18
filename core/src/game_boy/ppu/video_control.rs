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

/// Video control (schematic page 21): the LY counter, LYC comparator,
/// ROPO comparison latch, and STAT interrupt enable flags. Bidirectional —
/// the pipeline writes LY, the CPU writes LYC and STAT flags, and the
/// interrupt logic reads the latched comparison result. These signals
/// sit together on the die's video control section.
pub struct VideoControl {
    /// LX counter (SAXO-TYRY ripple counter, page 21).
    /// Counts 0-113 per scanline. Clocked by TALU (every 4 dots).
    /// SANU fires at LX=113, clocking RUTU (line-end).
    pub(super) lx: u8,

    /// WUVU divide-by-2 toggle DFF (GateBoyClocks.cpp).
    /// Clocked by XOTA (master clock rising edge, once per dot).
    /// Self-toggles: data = WUVU.qn_old. Period = 2 dots.
    /// High during phases A,B,E,F (`WUVU_ABxxEFxx`).
    pub(super) wuvu: bool,

    /// VENA divide-by-2 toggle DFF (GateBoyClocks.cpp).
    /// Clocked by WUVU.qn rising edge (= WUVU falling edge).
    /// Self-toggles: data = VENA.qn_old. Period = 4 dots = 1 M-cycle.
    /// High during phases C,D,E,F (`VENA_xxCDEFxx`).
    /// TALU = VENA.qp (buffered). TALU rising edge clocks LX.
    pub(super) vena: bool,

    /// LY counter (MUWY-LAFO ripple counter, page 21). Clocked by RUTU
    /// at LX=113, counting 0-153 and wrapping. On line 153, MYTA fires
    /// when NYPE rises (TALU falling of LX=0), driving LAMA low and
    /// resetting the CPU-visible LY to 0 via `ly()`. The internal counter
    /// remains at 153 until RUTU at LX=113 naturally wraps it.
    pub(super) ly: u8,

    /// LYC register (FF45). CPU-writable comparison value.
    pub(super) lyc: u8,

    /// Raw LY==LYC comparison from the current M-cycle (PALY_LY_MATCHa).
    /// On the next M-cycle, this is promoted to `ly_eq_lyc` (the ROPO
    /// output). Models the `_old` input to ROPO DFF17 — ROPO always
    /// samples the previous cycle's comparison result.
    pub(super) ly_match_pending: bool,

    /// Latched LY==LYC comparison result (ROPO_LY_MATCH_SYNCp, page 21).
    pub(super) ly_eq_lyc: bool,

    /// STAT interrupt enable flags (FF41 bits 3-6).
    pub(super) stat_flags: InterruptFlags,

    /// Previous STAT line state for rising-edge detection.
    pub(super) stat_line_was_high: bool,

    /// NYPE DFF17 (delayed line-end, GateBoyLCD.cpp line 125).
    /// Clocked by TALU falling edge. Data: RUTU_old (previous tick's
    /// line-end pulse). Goes high at phase_lx=4, 2 dots after RUTU.
    /// Active window: phase_lx [4, 11] within the scanline.
    pub(super) nype: bool,

    /// Previous RUTU state, serving as RUTU_old input to the NYPE DFF.
    /// Set true when RUTU fires (TALU falling of the LX=113 M-cycle),
    /// consumed by NYPE on the next TALU falling edge.
    pub(super) rutu_old: bool,

    /// SANU_x113p: combinational AND4 detecting LX=113. Set on the
    /// TALU rising edge where LX increments to 113. Consumed by RUTU
    /// on the next TALU falling edge (2 dots later, same M-cycle).
    pub(super) sanu: bool,

    /// Deferred LX reset flag. Set at TALU falling when RUTU fires
    /// (LX=113 detected). Consumed at the next TALU rising edge, which
    /// resets LX to 0 instead of incrementing. Models the hardware
    /// behavior where MUDE's async reset of the LX DFFs is invisible
    /// to all readers until the next tick (all comparators use _old values).
    pub(super) rutu_pending: bool,

    /// MYTA frame-end flag (GateBoyLCD.cpp line 127). Set when NYPE rises
    /// while LY==153 (NOKO detected). Drives LAMA low, which async-resets
    /// all LY DFFs to 0. Cleared when the internal counter wraps 153->0
    /// at RUTU. While set, `ly()` returns 0.
    pub(super) myta: bool,

    /// POPU VBlank latch (GateBoyLCD.cpp line 126). DFF17 clocked by NYPE
    /// rising edge, latching XYVO_old (LY >= 144 from the previous cycle).
    /// When high, the PPU reports VBlank mode. Async-reset by VID_RST.
    pub(super) popu: bool,
}

impl VideoControl {
    /// TALU signal: buffered VENA.qp. High during phases C,D,E,F.
    /// TALU rising edge clocks LX and ROPO. NYPE is clocked on TALU falling.
    pub fn talu(&self) -> bool {
        self.vena
    }

    /// XUPY signal: buffered WUVU.qp. High during phases A,B,E,F.
    /// Clocks OAM scan counter (via GAVA), BYBA/CATU pipeline DFFs.
    pub fn xupy(&self) -> bool {
        self.wuvu
    }

    /// NYPE DFF17 output. High for one TALU period (4 dots) starting
    /// at phase_lx=4 of each scanline. Used for VBlank IF (POPU) and
    /// MYTA (frame-end) clocking.
    pub fn nype(&self) -> bool {
        self.nype
    }

    /// CPU-visible LY value. On line 153, MYTA (frame-end DFF clocked
    /// by NYPE) drives LAMA low, resetting all LY bits to 0. The `myta`
    /// flag models this: set when NYPE rises while LY==153, cleared when
    /// the internal counter wraps at RUTU.
    pub fn ly(&self) -> u8 {
        if self.myta { 0 } else { self.ly }
    }

    pub fn ly_eq_lyc(&self) -> bool {
        self.ly_eq_lyc
    }

    pub fn write_ly(&mut self, value: u8) {
        self.ly = value;
    }

    pub fn latch_ly_comparison(&mut self) {
        // ROPO DFF17 latches PALY_LY_MATCHa_old on TALU rising edge.
        // Promote the previous cycle's raw comparison to the STAT-visible
        // latch, then compute the fresh comparison for next cycle.
        self.ly_eq_lyc = self.ly_match_pending;
        self.ly_match_pending = self.ly() == self.lyc;
    }

    /// POPU VBlank latch output. True during VBlank (lines 144-153),
    /// activated at NYPE rising edge rather than immediately at LY increment.
    pub fn popu(&self) -> bool {
        self.popu
    }

    /// XOTA rising edge: toggles WUVU. Called every dot.
    pub fn tick_xota(&mut self) {
        self.wuvu = !self.wuvu;
    }

    /// WUVU falling edge: toggles VENA. Returns true if WUVU just fell.
    /// Caller should check this after tick_xota() to know if VENA changed.
    pub fn wuvu_fell(&self) -> bool {
        // WUVU just toggled in tick_xota(). It fell if it's now false
        // (was true before toggle). Since tick_xota sets wuvu = !wuvu,
        // wuvu=false means it was true before = falling edge.
        !self.wuvu
    }

    /// Toggle VENA. Called when WUVU falls. Returns the previous TALU
    /// (VENA) state so the caller can detect TALU edges.
    pub fn tick_vena(&mut self) -> bool {
        let talu_was = self.vena;
        self.vena = !self.vena;
        talu_was
    }

    /// TALU rising edge: increment LX, detect SANU (LX=113).
    /// If RUTU fired on the previous TALU falling, reset LX to 0
    /// instead of incrementing (deferred MUDE reset).
    pub fn tick_talu_rise(&mut self) {
        if self.rutu_pending {
            self.lx = 0;
            self.rutu_pending = false;
        } else {
            self.lx += 1;
        }
        self.sanu = self.lx == 113;
    }

    /// TALU falling edge: NYPE latch, RUTU fire. Returns true at
    /// scanline boundary (RUTU fires when SANU detected LX=113).
    pub fn tick_talu_fall(&mut self) -> bool {
        // NYPE DFF17: clocked by TALU falling edge.
        // Latches rutu_old from the PREVIOUS TALU falling.
        // Must execute BEFORE RUTU so it sees pre-fire rutu_old.
        let nype_was = self.nype;
        self.nype = self.rutu_old;
        self.rutu_old = false;

        let nype_rose = !nype_was && self.nype;
        if nype_rose {
            // POPU DFF17: latches XYVO_old (LY >= 144) at NYPE rising edge.
            self.popu = self.ly >= 144;

            // MYTA DFF17: latches NOKO_old (LY == 153) at NYPE rising edge.
            if self.ly == 153 {
                self.myta = true;
            }
        }

        // RUTU DFF17: clocked by SONO (TALU falling). Latches SANU.
        if self.sanu {
            self.sanu = false;
            if self.ly >= 153 {
                self.ly = 0;
                self.myta = false; // Frame boundary complete; clear MYTA.
            } else {
                self.ly += 1;
            }
            self.rutu_pending = true;
            self.rutu_old = true;
            return true;
        }

        false
    }
}
