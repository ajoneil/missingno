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
//!
//! zloz (hold-chain SR latch, S = AND3(xogs, zkdu, zojz), holds
//! dispatch_active.q HIGH through dispatch M2-M5 after zkog resets at
//! ctl_int_entry_m6) is NOT modelled. The emulator only reads
//! dispatch_active at instruction boundaries (enter_fetch_overlap +
//! HaltPhase::WakeIntake), so the in-dispatch hold has no observable
//! effect — the InterruptDispatch CpuPhase ticks through steps 0..4
//! independently of dispatch_active.q.

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
    /// `ctl_op_di_or_ei` — combinational, HIGH during the M-cycle that
    /// decoded EI/DI. Drives the zzom block on zaij: zzom =
    /// NAND(opcode3_n_buf3, ctl_op_di_or_ei). Set by `mark_ei_di_decoded`
    /// when FetchOverlap step 1 applies an EI/DI commit; cleared by
    /// `enter_mcycle` at the next M-cycle boundary.
    ctl_op_di_or_ei: bool,
    /// zkog SR-latch — set by zaij rising during the in-flight
    /// instruction's eval phase, reset by ctl_int_entry_m6 / sys_reset.
    /// Once set, holds through to zacw's capture edge.
    zkog: bool,
    /// zacw DFF on master clock (CLK9). Captures zfex = zkog (zloz hold
    /// not modelled — see file header).
    /// q rising starts the 5-M-cycle dispatch sequence.
    zacw: Dff<bool>,
}

impl DispatchChain {
    pub(crate) fn new() -> Self {
        Self {
            irq_latch: InterruptFlags::empty(),
            data_phase_n: true,
            ctl_op_di_or_ei: false,
            zkog: false,
            zacw: Dff::new(false),
        }
    }

    /// Called at the start of each M-cycle (entry to next_mcycle).
    /// Clears the M-cycle-scoped `ctl_op_di_or_ei` so the zzom block
    /// only applies to the M-cycle that decoded EI/DI.
    pub(crate) fn enter_mcycle(&mut self) {
        self.ctl_op_di_or_ei = false;
    }

    /// Called from FetchOverlap step 1 when applying an EI or DI commit.
    /// Asserts `ctl_op_di_or_ei` for the rest of the current M-cycle.
    pub(crate) fn mark_ei_di_decoded(&mut self) {
        self.ctl_op_di_or_ei = true;
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

    /// IRQ-pending priority output (= NOT(irq_prio_nand_a_y) when the
    /// priority chain has evaluated). The wired-NAND bus is precharged
    /// HIGH while write_phase=0, so int_take is gated false outside
    /// the eval phase.
    pub(crate) fn int_take(&self, write_phase: bool) -> bool {
        write_phase && !self.irq_latch.is_empty()
    }

    /// Post-latch IF & IE — read at HaltPhase::WakeIntake to decide
    /// dispatch-vs-spurious-wake without going through zaij/zkog (which
    /// can't fire during HALT because data_phase is held LOW).
    pub(crate) fn latched(&self) -> InterruptFlags {
        self.irq_latch
    }

    /// Update the zkog SR-latch set chain each dot.
    ///   zkog: S = zaij = ime ∧ data_phase ∧ int_take ∧ xogs ∧ ¬(EI/DI
    ///         in flight). Reset path is `clear_dispatch()` (driven by
    ///         ctl_int_entry_m6 at the vector-resolve point).
    ///
    /// The HALT-wake dispatch path is handled at the sequencer level
    /// (HaltPhase::WakeIntake reads ime + latched IRQ directly), not
    /// through zkog — during HALT, data_phase is held LOW, so zaij's
    /// data_phase requirement blocks zkog from setting until the CPU
    /// phase ring restarts after halt drops.
    pub(crate) fn step_zkog(
        &mut self,
        ime_enabled: bool,
        data_phase: bool,
        write_phase: bool,
        xogs: bool,
    ) {
        let int_take = self.int_take(write_phase);
        let zaij =
            ime_enabled && data_phase && int_take && xogs && !self.ctl_op_di_or_ei;
        if zaij {
            self.zkog = true;
        }
    }

    /// Clock zacw on CLK9↑ (M-cycle boundary rise). Captures zfex = zkog
    /// (zloz hold-chain not modelled — see file header).
    pub(crate) fn tick_zacw(&mut self) {
        self.zacw.write(self.zkog);
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

    /// Reset zkog at ctl_int_entry_m6 — fires when the ISR commits to
    /// dispatch. Per netlist: zkog R_n = NOR(ctl_int_entry_m6, sys_reset).
    pub(crate) fn clear_dispatch(&mut self) {
        self.zkog = false;
    }
}
