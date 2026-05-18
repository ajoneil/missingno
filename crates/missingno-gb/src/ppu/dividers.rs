//! WUVU/VENA divider cascade clocked off XOTA.

pub struct Dividers {
    /// WUVU.Q — 2-dot period (half M-cycle).
    pub(in crate::ppu) half_mcycle: bool,
    /// VENA.Q — 4-dot period (1 M-cycle).
    pub(in crate::ppu) mcycle: bool,
}

impl Dividers {
    /// Toggle WUVU on XOTA rising. Returns previous WUVU.Q (XUPY = WUVU.Q).
    pub(in crate::ppu) fn tick_dot(&mut self) -> bool {
        let was = self.half_mcycle;
        self.half_mcycle = !self.half_mcycle;
        was
    }

    /// True when WUVU.Q is low after a tick_dot — VENA captures on this edge.
    pub(in crate::ppu) fn half_mcycle_fell(&self) -> bool {
        !self.half_mcycle
    }

    /// Toggle VENA. Returns previous VENA.Q. Caller gates on `half_mcycle_fell()`.
    pub(in crate::ppu) fn tick_mcycle(&mut self) -> bool {
        let was = self.mcycle;
        self.mcycle = !self.mcycle;
        was
    }

    pub(in crate::ppu) fn mcycle(&self) -> bool {
        self.mcycle
    }

    /// XUPY = WUVU.Q — scan-counter / OAM-pipeline clock.
    pub(in crate::ppu) fn xupy(&self) -> bool {
        self.half_mcycle
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.half_mcycle = false;
        self.mcycle = false;
    }
}
