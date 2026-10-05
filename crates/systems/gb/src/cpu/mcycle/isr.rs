//! ISR dispatch: M1..M5 after the detecting fetch.

use super::super::commit::Commit;
use super::super::{Cpu, InterruptMasterEnable};
use super::types::{CpuPhase, MCycleAction};
use crate::VramDmaClaim;

impl Cpu {
    /// ISR dispatch: 5 M-cycles (steps 0..=4), gb-ctr RST n p129.
    ///   step 0 → M1 InternalOamBug(PC, before the IDU's −1)
    ///   step 1 → M2 InternalOamBug(SP)
    ///   step 2 → M3 push pc_hi (Write {sp-1})
    ///   step 3 → M4 push pc_lo (Write {sp-2}); vector resolved here
    ///   step 4 → M5 vector fetch (via enter_fetch_overlap)
    /// IME clears at step 0 (ZACW on dispatching CLK9↑).
    pub(super) fn mcycle_isr(&mut self, claim: VramDmaClaim) -> Option<MCycleAction> {
        let (sp, pc_hi, pc_lo, step) = match &mut self.seq.phase {
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
            // M1: IDU PC-. Hardware undoes the discarded fetch's PC
            // increment; emulator skips both increment and decrement for the
            // same net effect. The bus carries PC as the IDU takes it, before
            // the decrement: the return address plus one (dmg-sim: cpu_port_a
            // holds the incremented PC through M1, with no read). Like any
            // IDU M-cycle, that address triggers the OAM bug in mode 2
            // (dmg-sim with its full OAM model corrupts the scanned row as
            // DEC rr does). Clear both IME stages so the boundary copy doesn't
            // restore IME on the next M-cycle.
            0 => {
                self.irq
                    .ime
                    .write_immediate(InterruptMasterEnable::Disabled);
                self.irq.ime_delay = false;
                Some(MCycleAction::InternalOamBug {
                    address: self.pc.wrapping_add(1),
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
            4 => Some(self.enter_fetch_overlap(claim, Commit::NoOperation)),
            _ => unreachable!(),
        }
    }
}
