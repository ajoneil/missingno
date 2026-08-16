//! Running-CPU dispatch chain.
//!
//! Per-bit irq_latch_inst<i> (data_phase_n-gated D-latch) → irq_prio_bit<i>
//! distributed-NOR priority chain → int_take buffer → ZAIJ AND → ZKOG
//! SR-latch → zfex OR → ZACW master-clock DFF.
//!
//! The EI/DI 1-instruction delay is NOT in this chain — it lives in the
//! `Cpu.ime` ↔ `Cpu.ime_delay` two-stage promotion (see `cpu/mod.rs`).
//! Dispatch reads `ime` directly via step_dispatch_set's `ime_enabled` input.
//!
//! data_phase_n window:
//!   HIGH (transparent) — dots 0-1 of running M-cycles, AND throughout
//!     HALT (CPU phase ring frozen, data_phase held LOW).
//!   LOW  (held)         — dots 2-3 of running M-cycles only.
//!
//! XOGS: (data_phase ∧ ctl_fetch) ∨ halt. Asserted across the data-phase
//! of any instruction-fetch M-cycle, plus continuously during halt.
//!
//! zloz (hold-chain SR latch holding dispatch_active.q HIGH through
//! dispatch M2-M5 after ZKOG resets at ctl_int_entry_m6) is NOT
//! modelled — dispatch_active is only read at instruction boundaries.

use crate::cpu::dff::Dff;
use crate::interrupts::{Interrupt, InterruptFlags};

pub struct DispatchChain {
    /// irq_latch_inst<i> outputs: per-bit post-latch IF.
    /// Bit i holds the (IE ∧ IF) bit i value sampled through the
    /// data_phase_n-gated D-latch.
    irq_latch: InterruptFlags,
    /// data_phase_n state — drives the per-bit latch enable.
    /// True = transparent (irq_latch tracks IE & IF live);
    /// false = held (irq_latch frozen at the value at last close).
    data_phase_n: bool,
    /// ZKOG SR-latch — set by ZAIJ rising during the in-flight
    /// instruction's eval phase, reset by ctl_int_entry_m6 / sys_reset.
    /// Once set, holds through to ZACW's capture edge.
    dispatch_set: bool,
    /// ZACW DFF on master clock (CLK9). Captures zfex = ZKOG (zloz hold
    /// not modelled — see file header).
    /// q rising starts the 5-M-cycle dispatch sequence.
    dispatch_capture: Dff<bool>,
}

impl Default for DispatchChain {
    fn default() -> Self {
        Self::new()
    }
}

impl DispatchChain {
    pub fn new() -> Self {
        Self {
            irq_latch: InterruptFlags::empty(),
            data_phase_n: true,
            dispatch_set: false,
            dispatch_capture: Dff::new(false),
        }
    }

    /// Drive data_phase_n from the CPU phase ring. Called every dot.
    /// When transparent (true), irq_latch tracks live IE & IF; when held
    /// (false), irq_latch stays frozen.
    pub fn set_data_phase_n(&mut self, transparent: bool) {
        self.data_phase_n = transparent;
    }

    /// Recompute irq_latch from (IE ∧ IF) when transparent. Held values
    /// stay frozen — caller's IE/IF writes during the held window are not
    /// reflected until the next set_data_phase_n(true).
    ///
    /// Masked to valid IRQ bits (0-4); hardware has per-bit
    /// `irq_latch_inst<i>` cells only for V/S/T/Serial/Joypad, with
    /// no connection for bits 5-7. Writes to FF0F bits 5-7 don't
    /// produce any pending state.
    pub fn update_latch(&mut self, ie: InterruptFlags, requested: InterruptFlags) {
        if self.data_phase_n {
            self.irq_latch = (ie & requested)
                & (InterruptFlags::VIDEO_BETWEEN_FRAMES
                    | InterruptFlags::VIDEO_STATUS
                    | InterruptFlags::TIMER
                    | InterruptFlags::SERIAL
                    | InterruptFlags::JOYPAD);
        }
    }

    /// IRQ-pending priority output (= NOT(irq_prio_nand_a_y) when the
    /// priority chain has evaluated). The wired-NAND bus is precharged
    /// HIGH while write_phase=0, so int_take is gated false outside
    /// the eval phase.
    pub fn int_take(&self, write_phase: bool) -> bool {
        write_phase && !self.irq_latch.is_empty()
    }

    /// Post-latch IF & IE — read at HaltPhase::WakeIntake to decide
    /// dispatch-vs-spurious-wake without going through ZAIJ/ZKOG (which
    /// can't fire during HALT because data_phase is held LOW).
    pub fn latched(&self) -> InterruptFlags {
        self.irq_latch
    }

    /// Update the ZKOG SR-latch set chain each dot.
    ///   ZKOG: S = ZAIJ = ime ∧ data_phase ∧ int_take ∧ XOGS.
    ///   Reset path is `clear_dispatch()` (driven by ctl_int_entry_m6 at
    ///   the vector-resolve point).
    ///
    /// The EI/DI 1-instruction delay rides on the `Cpu.ime` ↔
    /// `Cpu.ime_delay` two-stage (the caller passes the post-promotion
    /// `ime` value into `ime_enabled`); no separate gate is needed here.
    ///
    /// The HALT-wake dispatch path is handled at the sequencer level
    /// (HaltPhase::WakeIntake reads ime + latched IRQ directly), not
    /// through ZKOG — during HALT, data_phase is held LOW, so ZAIJ's
    /// data_phase requirement blocks ZKOG from setting until the CPU
    /// phase ring restarts after halt drops.
    pub fn step_dispatch_set(
        &mut self,
        ime_enabled: bool,
        data_phase: bool,
        write_phase: bool,
        instruction_boundary: bool,
    ) {
        let int_take = self.int_take(write_phase);
        let dispatch_set_condition = ime_enabled && data_phase && int_take && instruction_boundary;
        if dispatch_set_condition {
            self.dispatch_set = true;
        }
    }

    /// Clock ZACW on CLK9↑ (M-cycle boundary rise). Captures zfex = ZKOG
    /// (zloz hold-chain not modelled — see file header).
    pub fn tick_dispatch_capture(&mut self) {
        self.dispatch_capture.write(self.dispatch_set);
        self.dispatch_capture.tick();
    }

    /// dispatch_active output (ZACW.q). Drives the running-CPU
    /// fetch-vs-dispatch sequencer decision.
    pub fn dispatch_active(&self) -> bool {
        self.dispatch_capture.output()
    }

    /// Priority-encode the latched IF for ISR vector resolution.
    /// Reads post-latch state — what ZACW captured. Used at the ISR's
    /// vector-resolve point (M3→M4 boundary, IE push bug window).
    pub fn vector(&self) -> Option<Interrupt> {
        Interrupt::from_pending_bits(self.irq_latch.bits())
    }

    /// Reset ZKOG at ctl_int_entry_m6 — fires when the ISR commits to
    /// dispatch. Per netlist: ZKOG R_n = NOR(ctl_int_entry_m6, sys_reset).
    pub fn clear_dispatch(&mut self) {
        self.dispatch_set = false;
    }
}
