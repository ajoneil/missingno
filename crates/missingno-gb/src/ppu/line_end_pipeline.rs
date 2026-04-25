//! NYPE pipeline — LINE_END redistribution DFF.
//!
//! NYPE captures RUTU on TALU rising. Its Q output and Q_n complement
//! drive distinct downstream consumers:
//! - **NYPE rising** (Q): POPU captures (VBlank flag from XYVO).
//! - **NYPE falling** (Q_n rising / nype_n): MYTA and MEDA capture
//!   (FRAME_END from NOKO, LY=0 from NERU) — one TALU period later.
//!
//! MEDA's role is the LCD s_pad vertical-sync output only (not an
//! OAM-scan or mode-control consumer); not modelled at bit level in
//! this emulator — honest abstraction. Only POPU (rising) and MYTA
//! (falling) are dispatched from the emulator's NypeEdge distribution.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ppu) enum NypeEdge {
    /// NYPE rising — POPU fires.
    Rising,
    /// NYPE falling (nype_n rising) — MYTA fires. MEDA would also fire
    /// on this edge in hardware, but is not modelled (drives LCD s_pad
    /// only; honest abstraction).
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

    /// NYPE output accessor (Q high for one M-cycle after a RUTU pulse).
    pub(in crate::ppu) fn delayed_line_end(&self) -> bool {
        self.delayed_line_end
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.delayed_line_end = false;
        self.line_end_pending = false;
    }
}
