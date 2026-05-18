//! NYPE pipeline — LINE_END redistribution DFF.
//!
//! NYPE captures RUTU on TALU rising. Its Q output and Q_n complement
//! drive distinct downstream consumers:
//! - **NYPE rising** (Q): POPU captures (VBlank flag from XYVO).
//! - **NYPE falling** (Q_n rising / nype_n): MYTA and MEDA capture
//!   (FRAME_END from NOKO, LY=0 from NERU) — one TALU period later.
//!
//! MEDA drives s_pad (LCD VSYNC) via the `mure` inverter. Its first
//! 0→1 transition after VID_RST deassertion is the LCD's first VSYNC
//! pulse — the LCD only latches frames after it has fired.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ppu) enum NypeEdge {
    /// NYPE rising — POPU fires.
    Rising,
    /// NYPE falling (nype_n rising) — MYTA and MEDA fire.
    Falling,
    /// No edge this TALU rise.
    None,
}

pub struct LineEndPipeline {
    /// NYPE DFF output (Q).
    pub(in crate::ppu) delayed_line_end: bool,
    /// NYPE D input pending feed (set when RUTU fires; consumed on the
    /// next TALU rising capture).
    pub(in crate::ppu) line_end_pending: bool,
    /// MEDA DFF — captures NERU on NYPE-falling. Drives s_pad VSYNC
    /// via the `mure` inverter. Held at 0 by VID_RST during LCD-off.
    pub(in crate::ppu) meda: bool,
    /// Latched: MEDA has gone 0→1 at least once since the most recent
    /// VID_RST deassertion. The first 0→1 transition is the LCD's
    /// first VSYNC pulse since LCD-on.
    pub(in crate::ppu) vsync_committed: bool,
}

impl LineEndPipeline {
    /// Signal LINE_END to NYPE's D input. Called when RUTU fires (from
    /// LineCounterX's fire_rutu_and_reset). NYPE captures this on the
    /// next TALU rising edge.
    pub(in crate::ppu) fn signal_line_end(&mut self) {
        self.line_end_pending = true;
    }

    /// Capture NYPE's D input on TALU rising. Returns the edge type
    /// based on the Q transition:
    /// - Rising: Q transitioned 0 → 1 (POPU fires this edge)
    /// - Falling: Q transitioned 1 → 0 (MYTA fires on nype_n rising)
    /// - None: no Q transition
    pub(in crate::ppu) fn capture(&mut self) -> NypeEdge {
        let prev = self.delayed_line_end;
        self.delayed_line_end = self.line_end_pending;
        self.line_end_pending = false;
        match (prev, self.delayed_line_end) {
            (false, true) => NypeEdge::Rising,
            (true, false) => NypeEdge::Falling,
            _ => NypeEdge::None,
        }
    }

    /// Capture NERU into MEDA on NYPE-falling. Latches `vsync_committed`
    /// on the first 0→1 transition since VID_RST.
    pub(in crate::ppu) fn capture_meda(&mut self, neru: bool) {
        if !self.meda && neru {
            self.vsync_committed = true;
        }
        self.meda = neru;
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.delayed_line_end = false;
        self.line_end_pending = false;
        self.meda = false;
        self.vsync_committed = false;
    }
}
