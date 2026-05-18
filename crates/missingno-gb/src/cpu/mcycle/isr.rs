//! ISR dispatch: M1..M5 after the detecting fetch.

use super::super::commit::Commit;
use super::super::{Cpu, InterruptMasterEnable};
use super::types::{CpuPhase, MCycleAction};

impl Cpu {
    /// ISR dispatch: 5 M-cycles (steps 0..=4), gb-ctr RST n p129.
    ///   step 0 → M1 internal (PC on bus)
    ///   step 1 → M2 InternalOamBug(SP)
    ///   step 2 → M3 push pc_hi (Write {sp-1})
    ///   step 3 → M4 push pc_lo (Write {sp-2}); vector resolved here
    ///   step 4 → M5 vector fetch (via enter_fetch_overlap)
    /// IME clears at step 0 (zacw on dispatching CLK9↑).
    pub(super) fn mcycle_isr(&mut self) -> Option<MCycleAction> {
        let (sp, pc_hi, pc_lo, step) = match &mut self.phase {
            CpuPhase::InterruptDispatch {
                sp,
                pc_hi,
                pc_lo,
                step,
            } => (*sp, *pc_hi, *pc_lo, step),
            _ => unreachable!("mcycle_isr called outside InterruptDispatch phase"),
        };

        let current_step = *step;
        *step += 1;

        match current_step {
            // M1: IDU PC-. Hardware undoes the wakeup NOP's PC increment;
            // emulator skips both increment and decrement for the same net
            // effect. Clear both stages so the boundary copy doesn't
            // restore IME on the next M-cycle.
            0 => {
                self.irq
                    .ime
                    .write_immediate(InterruptMasterEnable::Disabled);
                self.irq.ime_delay = false;
                Some(MCycleAction::Internal {
                    address: self.bus_counter,
                })
            }
            1 => Some(MCycleAction::InternalOamBug { address: sp }),
            2 => {
                let addr = sp.wrapping_sub(1);
                self.stack_pointer = addr;
                Some(MCycleAction::Write {
                    address: addr,
                    value: pc_hi,
                })
            }
            3 => {
                // IE push bug: vector resolves after step 2 (hi push) but
                // before this step's lo push.
                self.irq.pending_vector_resolve = true;
                let addr = sp.wrapping_sub(2);
                self.stack_pointer = addr;
                Some(MCycleAction::Write {
                    address: addr,
                    value: pc_lo,
                })
            }
            4 => Some(self.enter_fetch_overlap(Commit::NoOperation)),
            _ => unreachable!(),
        }
    }
}
