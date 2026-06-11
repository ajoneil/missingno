//! Execute phase: operand reads + post-decode M-cycle stepping.

use super::super::Cpu;
use super::super::commit::Commit;
use super::super::registers::Register16;
use super::fetch::operand_count;
use super::types::{CpuPhase, MCycleAction, Phase, PopAction};

impl Cpu {
    /// Execute phase: operand reading and post-decode M-cycles.
    /// Returns `None` when the instruction completes (CPU has
    /// transitioned to Fetch).
    pub(super) fn mcycle_execute(&mut self) -> Option<MCycleAction> {
        let taken = std::mem::replace(&mut self.phase, CpuPhase::Fetch);
        let (mut phase, mut step) = match taken {
            CpuPhase::Execute { phase, step } => (phase, step),
            _ => unreachable!("mcycle_execute called outside Execute phase"),
        };

        let current_step = step;
        step += 1;

        let (action, put_back) = self.execute_phase_step(&mut phase, current_step);

        if put_back {
            self.phase = CpuPhase::Execute { phase, step };
        }

        action
    }

    /// Route a fetched opcode to its decoded phase: dispatch check,
    /// PC advance past the fetch address, then decode — 1-M
    /// instructions retire through the next fetch overlap.
    fn route_fetched_opcode(
        &mut self,
        opcode: u8,
        fetch_addr: u16,
    ) -> (Option<MCycleAction>, bool) {
        // zacw captures dispatch_active.q HIGH at this M-cycle's
        // closing edge — dispatch saves PC = fetch_addr so RETI
        // resumes at the prefetched-then-discarded instruction.
        if self.dispatch.dispatch_active() {
            let pc = fetch_addr;
            self.phase = CpuPhase::InterruptDispatch {
                sp: self.stack_pointer,
                pc_hi: (pc >> 8) as u8,
                pc_lo: (pc & 0xff) as u8,
                step: 0,
            };
            self.exec_step = 0;
            self.irq.pending_vector_resolve = false;
            self.boundary_flag = true;
            self.pc = pc;
            return (self.next_mcycle(), false);
        }

        if self.halt.bug {
            self.halt.bug = false;
        } else {
            self.pc = fetch_addr.wrapping_add(1);
        }

        let needed = operand_count(opcode);
        if needed == 0 {
            let bytes = [opcode, 0, 0];
            let (instruction, next_phase, next_commit) = self.decode_retire(bytes, 1);
            self.instruction = instruction;
            if matches!(next_phase, Phase::Empty) {
                (Some(self.enter_fetch_overlap(next_commit)), false)
            } else {
                self.phase = CpuPhase::Execute {
                    phase: next_phase,
                    step: 0,
                };
                self.exec_step = 0;
                (self.next_mcycle(), false)
            }
        } else {
            self.phase = CpuPhase::Execute {
                phase: Phase::Operands {
                    pc: self.pc,
                    bytes: [opcode, 0, 0],
                    bytes_read: 1,
                    bytes_needed: 1 + needed,
                },
                step: 0,
            };
            self.exec_step = 0;
            (self.next_mcycle(), false)
        }
    }

    /// One step of the active `Phase`. Returns `(action, put_back)` —
    /// `put_back = true` means the phase is still in flight and should
    /// be restored to `self.phase`.
    fn execute_phase_step(
        &mut self,
        phase: &mut Phase,
        current_step: u8,
    ) -> (Option<MCycleAction>, bool) {
        match phase {
            Phase::Operands {
                pc,
                bytes,
                bytes_read,
                bytes_needed,
            } => {
                if current_step == 0 && *bytes_read < *bytes_needed {
                    return (Some(MCycleAction::Read { address: *pc }), true);
                }

                // STOP's operand fetch discards its byte, so it is the one
                // bus transaction that yields to a DMA bus claim committed
                // during its tenure: the discard is what gets dropped — the
                // byte stays in IR through the stop spin and executes as
                // the next opcode at resume (no re-fetch).
                let yields_to_claim = bytes[0] == 0x10 && self.dma_bus_claim;
                if yields_to_claim {
                    self.stop_retained = Some(self.data_latch);
                }
                if !yields_to_claim {
                    bytes[*bytes_read as usize] = self.data_latch;
                    *bytes_read += 1;
                    *pc = pc.wrapping_add(1);
                    self.pc = *pc;
                }

                if yields_to_claim || *bytes_read >= *bytes_needed {
                    let b = *bytes;
                    let n = *bytes_read;
                    let (instruction, phase, commit) = self.decode_retire(b, n);
                    self.instruction = instruction;
                    if matches!(phase, Phase::Empty) {
                        return (Some(self.enter_fetch_overlap(commit)), false);
                    }
                    self.phase = CpuPhase::Execute { phase, step: 0 };
                    self.exec_step = 0;
                    return (self.next_mcycle(), false);
                }

                (Some(MCycleAction::Read { address: *pc }), true)
            }

            Phase::Empty => {
                unreachable!(
                    "Phase::Empty routes through enter_fetch_overlap; never enters Execute"
                )
            }

            Phase::FetchOverlap { commit } => {
                debug_assert!(
                    current_step == 1,
                    "FetchOverlap step 0 is performed inline by enter_fetch_overlap"
                );

                let carried = std::mem::replace(commit, Commit::NoOperation);
                Self::apply_commit(self, carried);

                let opcode = self.data_latch;
                let fetch_addr = match &self.current_action {
                    Some(MCycleAction::Read { address }) => *address,
                    _ => self.pc,
                };
                self.route_fetched_opcode(opcode, fetch_addr)
            }

            // IR retained the yielded STOP operand through the stop spin;
            // it routes here as a just-fetched opcode — no re-fetch.
            Phase::RetainedOpcode { opcode } => {
                let opcode = *opcode;
                let fetch_addr = self.pc;
                self.route_fetched_opcode(opcode, fetch_addr)
            }

            Phase::ReadOp { address, action } => match current_step {
                0 => (Some(MCycleAction::Read { address: *address }), true),
                _ => {
                    Self::apply_read_action(self, action, self.data_latch);
                    (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                }
            },

            Phase::ReadModifyWrite { address, op } => {
                let address = *address;
                match current_step {
                    0 => (Some(MCycleAction::Read { address }), true),
                    1 => {
                        let result = Self::apply_rmw(self, op, self.data_latch);
                        (
                            Some(MCycleAction::Write {
                                address,
                                value: result,
                            }),
                            true,
                        )
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::WriteOp {
                address,
                value,
                hl_post,
            } => match current_step {
                0 => {
                    if *hl_post != 0 {
                        let hl = self.get_register16(Register16::Hl);
                        self.set_register16(Register16::Hl, hl.wrapping_add(*hl_post as u16));
                    }
                    (
                        Some(MCycleAction::Write {
                            address: *address,
                            value: *value,
                        }),
                        true,
                    )
                }
                _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
            },

            Phase::Write16 { address, lo, hi } => {
                let address = *address;
                match current_step {
                    0 => (
                        Some(MCycleAction::Write {
                            address,
                            value: *lo,
                        }),
                        true,
                    ),
                    1 => (
                        Some(MCycleAction::Write {
                            address: address.wrapping_add(1),
                            value: *hi,
                        }),
                        true,
                    ),
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::InternalOp { count } => {
                if current_step < *count {
                    (Some(MCycleAction::Internal { address: 0x0000 }), true)
                } else {
                    (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                }
            }

            Phase::InternalOamBug { address } => match current_step {
                0 => (
                    Some(MCycleAction::InternalOamBug { address: *address }),
                    true,
                ),
                _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
            },

            Phase::Pop { sp, action } => {
                let sp = *sp;
                match current_step {
                    0 => (Some(MCycleAction::Read { address: sp }), true),
                    1 => {
                        self.scratch = self.data_latch;
                        (
                            Some(MCycleAction::Read {
                                address: sp.wrapping_add(1),
                            }),
                            true,
                        )
                    }
                    2 => {
                        Self::apply_pop(self, action, self.scratch, self.data_latch, sp);
                        let has_trailing =
                            matches!(action, PopAction::SetPc | PopAction::SetPcEnableInterrupts);
                        if has_trailing {
                            (Some(MCycleAction::Internal { address: 0x0000 }), true)
                        } else {
                            (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                        }
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::Push { sp, hi, lo } => {
                let sp = *sp;
                match current_step {
                    0 => (Some(MCycleAction::InternalOamBug { address: sp }), true),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *hi,
                            }),
                            true,
                        )
                    }
                    2 => {
                        let addr = sp.wrapping_sub(2);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *lo,
                            }),
                            true,
                        )
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::CondJump { taken, target } => {
                if current_step == 0 && *taken {
                    // PC stays at the post-operand address this M-cycle; the
                    // target waits in wz until the retiring PC ← WZ copy.
                    self.wz = *target;
                    self.wz_to_pc = true;
                    (Some(MCycleAction::Internal { address: 0x0000 }), true)
                } else {
                    (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                }
            }

            Phase::CondCall { taken, sp, hi, lo } => {
                if !*taken {
                    return (Some(self.enter_fetch_overlap(Commit::NoOperation)), false);
                }
                let sp = *sp;
                match current_step {
                    0 => (Some(MCycleAction::InternalOamBug { address: sp }), true),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *hi,
                            }),
                            true,
                        )
                    }
                    2 => {
                        let addr = sp.wrapping_sub(2);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *lo,
                            }),
                            true,
                        )
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::CondReturn { taken, sp, action } => {
                let sp = *sp;
                let taken = *taken;
                match current_step {
                    0 => (Some(MCycleAction::Internal { address: 0x0000 }), true),
                    1 if !taken => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                    1 => (Some(MCycleAction::Read { address: sp }), true),
                    2 => {
                        self.scratch = self.data_latch;
                        (
                            Some(MCycleAction::Read {
                                address: sp.wrapping_add(1),
                            }),
                            true,
                        )
                    }
                    3 => {
                        Self::apply_pop(self, action, self.scratch, self.data_latch, sp);
                        (Some(MCycleAction::Internal { address: 0x0000 }), true)
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }
        }
    }
}
