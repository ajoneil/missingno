//! Running-CPU dispatch chain.
//!
//! Per-bit irq_latch_inst<i> (data_phase_n-gated D-latch) → irq_prio_bit<i>
//! distributed-NOR priority chain → int_take buffer → zaij AND4 → zkog
//! SR-latch → zfex OR2 → zacw master-clock DFF.
//!
//! data_phase_n window:
//!   HIGH (transparent) — dots 0-1 of running M-cycles, AND throughout
//!     HALT (CPU phase ring frozen, data_phase held LOW).
//!   LOW  (held)         — dots 2-3 of running M-cycles only.
//!
//! xogs (instruction-boundary): (data_phase ∧ ctl_fetch ∧ ¬cb_prefix) ∨ halt.
//! Asserted across the data-phase of any instruction-fetch M-cycle, plus
//! continuously during halt.

use crate::cpu::dff::Dff;
use crate::interrupts::{Interrupt, InterruptFlags};

pub(crate) struct DispatchChain {
    /// irq_latch_inst<i> outputs: per-bit post-latch IF.
    /// Bit i holds the (IE ∧ IF) bit i value sampled through the
    /// data_phase_n-gated D-latch.
    irq_latch: InterruptFlags,
    /// data_phase_n state — drives the per-bit latch enable.
    /// True = transparent (irq_latch tracks IE & IF live);
    /// false = held (irq_latch frozen at the value at last close).
    data_phase_n: bool,
    /// zkog SR-latch — set by zaij rising during the in-flight
    /// instruction's eval phase, reset by ctl_int_entry_m6 / sys_reset.
    /// Once set, holds through to zacw's capture edge.
    zkog: bool,
    /// zloz SR-latch — NMI dispatch path. Always false on DMG (no NMI).
    zloz: bool,
    /// zacw DFF on master clock (CLK9). Captures zfex = OR(zkog, zloz).
    /// q rising starts the 5-M-cycle dispatch sequence (§13.3).
    zacw: Dff<bool>,
}

impl DispatchChain {
    pub(crate) fn new() -> Self {
        Self {
            irq_latch: InterruptFlags::empty(),
            data_phase_n: true,
            zkog: false,
            zloz: false,
            zacw: Dff::new(false),
        }
    }

    /// Boot-ROM-handoff state: latch transparent, dispatch idle.
    pub(crate) fn from_snapshot() -> Self {
        Self::new()
    }

    /// Drive data_phase_n from the CPU phase ring. Called every dot.
    /// When transparent (true), irq_latch tracks live IE & IF; when held
    /// (false), irq_latch stays frozen.
    pub(crate) fn set_data_phase_n(&mut self, transparent: bool) {
        self.data_phase_n = transparent;
    }

    /// Recompute irq_latch from (IE ∧ IF) when transparent. Held values
    /// stay frozen — caller's IE/IF writes during the held window are not
    /// reflected until the next set_data_phase_n(true).
    pub(crate) fn update_latch(&mut self, ie: InterruptFlags, requested: InterruptFlags) {
        if self.data_phase_n {
            self.irq_latch = ie & requested;
        }
    }

    /// Combinational int_take = OR of irq_latch bits. Per netlist:
    /// int_take = NOT(irq_prio_nand_a_y) where irq_prio_nand_a_y is the
    /// distributed wired-NAND output of the priority chain.
    pub(crate) fn int_take(&self) -> bool {
        !self.irq_latch.is_empty()
    }

    /// Set zfex.D inputs each dot. Drives:
    ///   zkog: SR-latch set by zaij = ime ∧ data_phase ∧ int_take ∧
    ///         xogs ∧ ¬(EI/DI in flight); reset by ctl_int_entry_m6 ∨
    ///         sys_reset.
    ///   zloz: SR-latch set by AND3(xogs, zkdu, zojz). The exact zkdu /
    ///         zojz semantics aren't in the spec, but the AND3 fires
    ///         during HALT (xogs ∨ halt term) when ime ∧ int_take —
    ///         providing the HALT-wake dispatch path that bypasses
    ///         zaij's data_phase requirement.
    pub(crate) fn step_zkog(
        &mut self,
        ime_enabled: bool,
        data_phase: bool,
        xogs: bool,
        halt: bool,
        ei_di_in_flight: bool,
        ctl_int_entry_m6: bool,
        sys_reset: bool,
    ) {
        let zaij =
            ime_enabled && data_phase && self.int_take() && xogs && !ei_di_in_flight;
        if zaij {
            self.zkog = true;
        }
        // HALT-wake path (zloz set chain): xogs is HIGH during HALT,
        // and IF rises propagate combinationally through the
        // transparent latch. zloz lets dispatch arm at the next CLK9↑.
        let zloz_set = halt && ime_enabled && self.int_take();
        if zloz_set {
            self.zloz = true;
        }
        if ctl_int_entry_m6 || sys_reset {
            self.zkog = false;
            self.zloz = false;
        }
    }

    /// Clock zacw on CLK9↑ (M-cycle boundary rise). Captures zfex.
    pub(crate) fn tick_zacw(&mut self) {
        let zfex = self.zkog || self.zloz;
        self.zacw.write(zfex);
        self.zacw.tick();
    }

    /// dispatch_active output (zacw.q). Drives the running-CPU
    /// fetch-vs-dispatch sequencer decision.
    pub(crate) fn dispatch_active(&self) -> bool {
        self.zacw.output()
    }

    /// Priority-encode the latched IF for ISR vector resolution.
    /// Reads post-latch state — what zacw captured. Used at the ISR's
    /// vector-resolve point (M3→M4 boundary, IE push bug window).
    pub(crate) fn vector(&self) -> Option<Interrupt> {
        for interrupt in Interrupt::priority_order() {
            if self.irq_latch.contains((*interrupt).into()) {
                return Some(*interrupt);
            }
        }
        None
    }

    /// Reset both zkog and zloz SR-latches at ctl_int_entry_m6 — fires
    /// when the ISR commits to dispatch. Per netlist:
    ///   zkog R_n = NOR(ctl_int_entry_m6, sys_reset)
    ///   zloz R_n = AOI21(nmi_entry, ctl_int_entry_m6, sys_reset)
    pub(crate) fn clear_dispatch(&mut self) {
        self.zkog = false;
        self.zloz = false;
    }
}
