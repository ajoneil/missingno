//! §1.2 clock dividers — WUVU and VENA toggle cascade.
//!
//! WUVU (`half_mcycle`): dffr toggling on XOTA rising; Q period = 2 dots
//! = half an M-cycle (= 2 MHz at the 4.194 MHz dot rate).
//!
//! VENA (`mcycle`): dffr toggling on WUVU.~Q rising (= WUVU.Q falling);
//! Q period = 4 dots = 1 M-cycle (= 1 MHz). The emulator stores VENA.Q
//! directly as `mcycle`. TALU is the inverter on VENA's Q output
//! (TALU = NOT(VENA.Q)) and is exposed via `talu()`. SONO equals VENA
//! phase (SONO rising = VENA rising = TALU falling) and is exposed via
//! `sono()`.
//!
//! Naming: both fields anchor to the M-cycle as the meaningful subsystem
//! unit. WUVU is "halfway to M-cycle" (half-period); VENA produces the
//! M-cycle cadence directly. Dot is the PPU's primary time unit per
//! alignment-log subsystem-vocabulary primacy.
//!
//! Hardware reference: netlist `wuvu_inst` / `vena_inst` / `talu_inst` /
//! `sono_inst` / `xupy_inst` in
//! `receipts/resources/dmg-sim/dmg_cpu_b/dmg_cpu_b.sv`.

pub struct Dividers {
    /// WUVU.Q — 2-dot period (half M-cycle).
    pub(in crate::ppu) half_mcycle: bool,
    /// VENA.Q — 4-dot period (1 M-cycle).
    pub(in crate::ppu) mcycle: bool,
}

impl Dividers {
    /// Advance on XOTA rising (once per dot). Toggles WUVU.
    pub(in crate::ppu) fn tick_dot(&mut self) {
        self.half_mcycle = !self.half_mcycle;
    }

    /// Whether WUVU.Q is currently low — equivalent to "WUVU just fell"
    /// when read immediately after `tick_dot` (since tick_dot toggled it
    /// to its current state). VENA captures on this edge.
    pub(in crate::ppu) fn half_mcycle_fell(&self) -> bool {
        !self.half_mcycle
    }

    /// Toggle mcycle (VENA). Returns the previous VENA.Q value so the
    /// caller can detect VENA edges (and derive TALU edges as their
    /// inverse). Caller gates this with `half_mcycle_fell()`.
    pub(in crate::ppu) fn tick_mcycle(&mut self) -> bool {
        let was = self.mcycle;
        self.mcycle = !self.mcycle;
        was
    }

    pub(in crate::ppu) fn mcycle(&self) -> bool {
        self.mcycle
    }

    /// TALU (not_x4) = NOT(VENA.Q) — 1 MHz LX counter clock.
    pub(in crate::ppu) fn talu(&self) -> bool {
        !self.mcycle
    }

    /// SONO = VENA phase — clocks RUTU's capture of SANU on its rising
    /// edge.
    pub(in crate::ppu) fn sono(&self) -> bool {
        self.mcycle
    }

    /// XUPY = complement of WUVU's stored state (`half_mcycle`). XUPY is
    /// the scan-counter / OAM-pipeline clock per §1.2/§1.3; consumers
    /// read it as the signal whose rising edge captures BYBA, CATU, etc.
    ///
    /// Polarity convention: the emulator's `half_mcycle` models WUVU's
    /// stored state in an internally-consistent convention; `xupy()`
    /// returns its complement. See commit `1cc599c` for the polarity
    /// fix that established the current convention. Gate name XUPY is
    /// preserved at external-reference sites.
    pub(in crate::ppu) fn xupy(&self) -> bool {
        !self.half_mcycle
    }

    /// VID_RST: dividers reset to 0 (Q=0 for both DFFs). With VENA.Q=0,
    /// TALU = NOT(VENA) is held at 1 until VENA's first rise.
    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.half_mcycle = false;
        self.mcycle = false;
    }
}
