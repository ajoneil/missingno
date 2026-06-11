//! T-cycle and M-cycle scheduling: `next_tcycle` produces a
//! `BusAction` per T-cycle; `next_mcycle` picks each M-cycle's
//! `MCycleAction` by dispatching on the CPU's current phase.

use super::super::{Cpu, HaltState, InterruptMasterEnable};
use super::types::{BusAction, CpuPhase, HaltPhase, MCycleAction, TCycle};

impl Cpu {
    /// Advance one T-cycle. Returns a `BusAction` that the executor
    /// must handle. The CPU is a continuous state machine, so this
    /// always returns — when an instruction completes, the boundary
    /// flag is set and the first T-cycle of the next instruction is
    /// deferred to the next call.
    pub fn next_tcycle(&mut self) -> BusAction {
        if !self.mcycle_active {
            // Bus arbitration, M-boundary-quantized: the grant takes effect
            // between M-cycles, so a transaction in flight always completes;
            // one STARTING while the DMA owns the VRAM/external buses waits
            // for release. IO/HRAM/OAM and internal M-cycles proceed
            // concurrently; the ring keeps counting throughout.
            if self.bus_held {
                // GDMA owns the full bus bandwidth: passive spin cells, each
                // an instruction boundary, with no instruction-state advance.
                self.boundary_flag = true;
                self.current_action = Some(MCycleAction::Internal { address: self.pc });
                self.tcycle = TCycle::ZERO;
                self.mcycle_active = true;
                self.dma_bus_claim = false;
                self.handover_kill = false;
            } else {
            let mut action = if self.parked_action.is_some() {
                if self.bus_suspended {
                    MCycleAction::Internal { address: self.pc }
                } else {
                    self.parked_action.take().expect("checked is_some")
                }
            } else {
                self.next_mcycle()
                    .expect("next_mcycle must always return Some (CPU chains at boundaries)")
            };
            if self.bus_suspended && self.parked_action.is_none() {
                let targets_bus = match &action {
                    MCycleAction::Read { address } | MCycleAction::Write { address, .. } => {
                        crate::memory::Bus::of(*address).is_some()
                    }
                    _ => false,
                };
                // The dispatch sequence asserts bus ownership end-to-end,
                // so its tenure cannot begin while the DMA owns the bus —
                // the mirror of the grant deferring to an in-flight
                // dispatch.
                if targets_bus || self.in_dispatch() {
                    self.parked_action = Some(action);
                    action = MCycleAction::Internal { address: self.pc };
                }
            }
            self.current_action = Some(action);
            self.tcycle = TCycle::ZERO;
            self.mcycle_active = true;
            // Claims are per-M-cycle: the pick above consumed any claim
            // committed during the M-cycle that just ended.
            self.dma_bus_claim = false;
            self.handover_kill = false;
            }
        }

        let tcycle = self.tcycle;
        self.last_tcycle = tcycle;
        self.tcycle = if tcycle.as_u8() == 3 {
            TCycle::ZERO
        } else {
            tcycle.advance()
        };

        let result = match &self.current_action {
            Some(MCycleAction::Read { address }) => {
                // CPU latches read data at the end of the M-cycle.
                if tcycle.as_u8() == 3 {
                    BusAction::Read { address: *address }
                } else {
                    BusAction::Idle
                }
            }
            Some(MCycleAction::Write { address, value }) => {
                // CPU write commits at the end of the M-cycle.
                if tcycle.as_u8() == 3 {
                    BusAction::Write {
                        address: *address,
                        value: *value,
                    }
                } else {
                    BusAction::Idle
                }
            }
            Some(MCycleAction::InternalOamBug { address }) => {
                // IDU address is on the bus during the first T-cycle.
                if tcycle.as_u8() == 0 {
                    BusAction::InternalOamBug { address: *address }
                } else {
                    BusAction::Idle
                }
            }
            Some(MCycleAction::Internal { .. }) => BusAction::Idle,
            None => unreachable!(),
        };

        if tcycle.as_u8() == 3 {
            self.mcycle_active = false;
            self.boundary_pending = true;
        }

        self.last_bus_action = result;
        result
    }

    /// Pick the next M-cycle's bus action. Single combinational
    /// selector over post-edge state — `irq_latched.q`,
    /// `dispatch_active.q`, and `irq_pending` have all settled when
    /// this runs.
    pub(super) fn next_mcycle(&mut self) -> Option<MCycleAction> {
        // M_h start: halt-bug-vs-halt-state decision. yoii captured
        // the pre-update_latch dispatch.latched() at this boundary, so
        // IF rises held by the per-bit latch through HALT body's
        // data-phase see the pre-release value here.
        if self.halt.bug_check_pending {
            self.halt.bug_check_pending = false;
            if self.irq.irq_latched.output() {
                // Halt RS-latch can't set (ykua holds reset LOW).
                self.halt.state = HaltState::Running;
                let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                if ime_enabled {
                    // Collapse HALT-IDU+1 + dispatch's universal -1
                    // step: PC HALT+1 → HALT_addr.
                    self.pc = self.pc.wrapping_sub(1);
                    if self.dispatch.dispatch_active() {
                        let pc = self.pc;
                        self.phase = CpuPhase::InterruptDispatch {
                            sp: self.stack_pointer,
                            pc_hi: (pc >> 8) as u8,
                            pc_lo: (pc & 0xff) as u8,
                            step: 0,
                        };
                        self.exec_step = 0;
                        self.irq.pending_vector_resolve = false;
                        self.boundary_flag = true;
                        return self.mcycle_isr();
                    }
                } else {
                    // HALT-bug: PC++ suppression at the next opcode
                    // fetch makes the byte after HALT execute twice.
                    self.halt.bug = true;
                }
                // Phase is already Execute(FetchOverlap step 1) from
                // enter_fetch_overlap's halt-entry branch; this M-cycle
                // reads HALT+1 (or HALT_addr after pc--).
            } else {
                // No IF pending at M_h start: halt RS-latch sets.
                self.halt.rs_latched = true;
                self.phase = CpuPhase::Halted(HaltPhase::Spin);
                self.exec_step = 0;
            }
        }

        match &self.phase {
            CpuPhase::Fetch => self.mcycle_fetch(),
            CpuPhase::Execute { .. } => self.mcycle_execute(),
            CpuPhase::InterruptDispatch { .. } => self.mcycle_isr(),
            CpuPhase::Halted(HaltPhase::Spin) => {
                if self.halt.state == HaltState::Stopped {
                    // STOP idle: no interrupt-wake; resume is external.
                    return Some(self.mcycle_halted_entry(HaltPhase::Spin));
                }
                if self.irq.irq_latched.output() {
                    let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                    let dispatch_pending = ime_enabled && !self.dispatch.latched().is_empty();
                    if dispatch_pending {
                        Some(self.mcycle_halted_entry(HaltPhase::WakeIntake))
                    } else {
                        self.enter_post_halt_fetch()
                    }
                } else if self.irq.irq_pending {
                    Some(self.mcycle_halted_entry(HaltPhase::SetupMiss))
                } else {
                    Some(self.mcycle_halted_entry(HaltPhase::Spin))
                }
            }
            CpuPhase::Halted(HaltPhase::SetupMiss) => {
                // yoii captured on this M-cycle's opening CLK9↑; mcyc is still
                // parked at 0b111 from HALT, so ctl_fetch=1 drives the post-halt
                // opcode read on this same M-cycle's data_phase. Only the IME=1
                // dispatch path needs a separate WakeIntake M-cycle (to capture
                // dispatch_active.q before dispatch M1).
                let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                let dispatch_pending = ime_enabled && !self.dispatch.latched().is_empty();
                if dispatch_pending {
                    Some(self.mcycle_halted_entry(HaltPhase::WakeIntake))
                } else {
                    self.enter_post_halt_fetch()
                }
            }
            CpuPhase::Locked => {
                self.boundary_flag = true;
                Some(MCycleAction::Internal { address: self.pc })
            }
            CpuPhase::Halted(HaltPhase::WakeIntake) => {
                // IME=1 dispatch capture: zacw captures `dispatch_active.q = 1`
                // and routes the next M-cycle to dispatch M1. The IME=0
                // fall-through stays defensive for the SetupMiss path; the
                // primary IME=0 wake short-circuits at the Spin arm above.
                let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                let irq_pending_for_dispatch = ime_enabled && !self.dispatch.latched().is_empty();
                if irq_pending_for_dispatch {
                    self.halt.state = HaltState::Running;
                    self.halt.rs_latched = false;
                    self.halt.wake_active = true;
                    let pc = self.pc;
                    self.phase = CpuPhase::InterruptDispatch {
                        sp: self.stack_pointer,
                        pc_hi: (pc >> 8) as u8,
                        pc_lo: (pc & 0xff) as u8,
                        step: 0,
                    };
                    self.exec_step = 0;
                    self.irq.pending_vector_resolve = false;
                    self.boundary_flag = true;
                    self.mcycle_isr()
                } else {
                    self.enter_post_halt_fetch()
                }
            }
        }
    }

    /// Enter `CpuPhase::Halted(phase)`. No bus activity — the halted
    /// state holds the address bus passively (dmg-sim shows no
    /// `bus_read` fires in any of the three halt sub-phases).
    pub(super) fn mcycle_halted_entry(&mut self, phase: HaltPhase) -> MCycleAction {
        self.phase = CpuPhase::Halted(phase);
        self.exec_step = 0;
        self.boundary_flag = true;
        MCycleAction::Internal { address: self.pc }
    }
}
