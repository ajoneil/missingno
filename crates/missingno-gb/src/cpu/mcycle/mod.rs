//! CPU M-cycle state machine. The CPU is a persistent state machine
//! that yields one [`BusAction`] per T-cycle via [`Cpu::next_tcycle`].
//!
//! Submodules:
//! - [`types`] — bus / phase / helper enums and `TCycle`
//! - [`scheduler`] — T-cycle output and per-M-cycle dispatch
//! - [`fetch`] — opcode fetch, decode, fetch-overlap entry
//! - [`execute`] — operand reads + post-decode M-cycle stepping
//! - [`isr`] — interrupt dispatch
//! - [`apply`] / [`build`] — pure CPU mutations and phase builders

use super::Cpu;

mod apply;
mod build;
mod execute;
mod fetch;
mod isr;
mod scheduler;
mod types;

pub(super) use types::{AluOp, PopAction, ReadAction, RmwOp};
pub use types::{BusAction, TCycle};
pub(crate) use types::{CpuPhase, HaltPhase, MCycleAction, Phase};

impl Cpu {
    /// The T-cycle that produced the most recent `BusAction`.
    pub fn last_tcycle(&self) -> TCycle {
        self.last_tcycle
    }

    /// True at the last T-cycle of the current M-cycle, where boundary
    /// work (timers, DMA, serial, audio, PPU boundary) completes before
    /// the next M-cycle begins.
    pub fn at_mcycle_boundary(&self) -> bool {
        self.last_tcycle.as_u8() == 3
    }

    /// Check and consume the instruction boundary flag.
    pub fn take_instruction_boundary(&mut self) -> bool {
        if self.boundary_flag {
            self.boundary_flag = false;
            true
        } else {
            false
        }
    }

    /// Check the instruction boundary flag without consuming it.
    pub fn at_instruction_boundary(&self) -> bool {
        self.boundary_flag
    }

    /// IE push bug: consume the pending vector-resolution request.
    pub fn take_pending_vector_resolve(&mut self) -> bool {
        if self.irq.pending_vector_resolve {
            self.irq.pending_vector_resolve = false;
            true
        } else {
            false
        }
    }

    /// Update `irq_pending` from the priority-encoded `IF & IE`.
    /// Combinational, not IME-gated — the IME gate sits in
    /// `dispatch_trigger`; the vector resolves separately via
    /// `pending_vector_resolve` at the ISR's M3→M4 push.
    pub fn update_interrupt_state(
        &mut self,
        triggered: Option<super::super::interrupts::Interrupt>,
    ) {
        self.irq.irq_pending = triggered.is_some();
    }

    /// Capture the wake comparator state at the T2 rise — the sample
    /// the CGB's halt-release chain consumes at the next boundary.
    pub fn presample_halt_wake(&mut self) {
        self.irq.halt_wake_presample = !self.dispatch.latched().is_empty();
    }

    /// Clock `irq_latched` (yoii). D is the data-phase-gated priority
    /// chain output (`dispatch.latched()`), not raw `irq_pending`.
    /// Drives the HALT-release chain. With `samples_early` (CGB), a
    /// halted CPU's capture consumes the T2 presample instead, so an
    /// IF edge in the final two T-cycles waits one more M-cycle.
    pub fn tick_irq_latched(&mut self, samples_early: bool) {
        let d = if samples_early && self.halt_rs_latched() {
            self.irq.halt_wake_presample
        } else {
            !self.dispatch.latched().is_empty()
        };
        self.irq.irq_latched.write(d);
        self.irq.irq_latched.tick();
    }
}
