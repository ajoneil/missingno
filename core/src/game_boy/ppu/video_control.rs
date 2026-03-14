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
    /// at LX=113, counting 0–153 and wrapping. On line 153, MYTA
    /// (frame-end DFF, clocked by NYPE one half-cycle after RUTU) drives
    /// LAMA low, resetting all LY bits to 0. The CPU sees LY=153 only
    /// during the first M-cycle (LX=0); from LX=1 onward, `ly()`
    /// returns 0.
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
}

impl VideoControl {
    /// TALU signal: buffered VENA.qp. High during phases C,D,E,F.
    /// TALU rising edge clocks LX, ROPO, and NYPE.
    pub fn talu(&self) -> bool {
        self.vena
    }

    /// XUPY signal: buffered WUVU.qp. High during phases A,B,E,F.
    /// Clocks OAM scan counter (via GAVA), BYBA/CATU pipeline DFFs.
    pub fn xupy(&self) -> bool {
        self.wuvu
    }

    /// CPU-visible LY value. On line 153, MYTA (frame-end DFF clocked
    /// by NYPE) drives LAMA low, resetting all LY bits to 0 after
    /// the first M-cycle. The CPU sees LY=153 only during LX=0;
    /// from LX=1 onward, `ly()` returns 0. The internal counter
    /// remains at 153 until RUTU at LX=113 naturally wraps it 153→0.
    pub fn ly(&self) -> u8 {
        if self.ly == 153 && self.lx >= 1 && self.lx < 113 {
            0
        } else {
            self.ly
        }
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
        self.ly_match_pending = self.ly == self.lyc;
    }

    /// One XOTA rising edge: toggles WUVU, cascades to VENA, cascades
    /// to LX on TALU rising. Returns true at scanline boundary (LX
    /// wraps 113→0). Called once per dot by the executor, independent
    /// of the rising/falling half-phase split.
    pub fn tick_xota(&mut self) -> bool {
        let wuvu_was = self.wuvu;

        // WUVU DFF17: clocked by XOTA (every dot), self-toggles.
        self.wuvu = !self.wuvu;

        // VENA DFF17: clocked by WUVU.qn rising = WUVU.qp falling.
        if wuvu_was && !self.wuvu {
            let talu_was = self.vena;
            self.vena = !self.vena;

            // TALU = VENA.qp. LX clocked on TALU rising edge.
            if !talu_was && self.vena {
                self.lx += 1;

                // SANU detects LX=113 combinationally (no action here;
                // RUTU latches on the NEXT TALU edge).

                // RUTU fires when LX reaches 114: resets LX to 0 (via MUDE)
                // and clocks the LY ripple counter. Both are driven by the
                // same RUTU DFF output.
                if self.lx >= 114 {
                    if self.ly >= 153 {
                        self.ly = 0;
                    } else {
                        self.ly += 1;
                    }
                    self.lx = 0;
                    return true;
                }
            }
        }

        false
    }
}
