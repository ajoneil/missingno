//! NYPE LINE_END redistribution DFF (TALU-rising capture of RUTU).
//! NYPE rising → POPU; NYPE falling (nype_n rising) → MYTA + MEDA, one TALU later.

use crate::ppu::DffBit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ppu) enum LineEndEdge {
    Rising,
    Falling,
    None,
}

pub struct LineEndPipeline {
    /// NYPE DFF. D signalled when RUTU fires; Q (LINE_END, delayed) captured on
    /// TALU rising, feeds POPU/MYTA/MEDA.
    pub(in crate::ppu) line_end_pending: DffBit,
    /// MEDA captures NERU on NYPE-falling; drives s_pad VSYNC via the `mure` inverter.
    pub(in crate::ppu) vsync_active: bool,
    /// MEDA has gone 0→1 since the most recent VID_RST deassertion.
    pub(in crate::ppu) vsync_committed: bool,
}

impl LineEndPipeline {
    /// Signal LINE_END to NYPE's D input (RUTU fired).
    pub(in crate::ppu) fn signal_line_end(&mut self) {
        self.line_end_pending.write(true);
    }

    /// Capture NYPE on TALU rising; returns the Q transition.
    pub(in crate::ppu) fn capture(&mut self) -> LineEndEdge {
        let prev = self.line_end_pending.output();
        let now = self.line_end_pending.tick();
        self.line_end_pending.write(false);
        match (prev, now) {
            (false, true) => LineEndEdge::Rising,
            (true, false) => LineEndEdge::Falling,
            _ => LineEndEdge::None,
        }
    }

    /// Capture NERU into MEDA on NYPE-falling; latch vsync_committed on first 0→1.
    pub(in crate::ppu) fn capture_vsync(&mut self, neru: bool) {
        if !self.vsync_active && neru {
            self.vsync_committed = true;
        }
        self.vsync_active = neru;
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.line_end_pending = DffBit::new(false, false);
        self.vsync_active = false;
        self.vsync_committed = false;
    }
}
