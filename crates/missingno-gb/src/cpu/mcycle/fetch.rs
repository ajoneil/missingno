//! Opcode fetch, decode, and fetch-overlap entry.

use super::super::commit::Commit;
use super::super::instructions::Instruction;
use super::super::instructions::Interrupt as InterruptInstruction;
use super::super::{Cpu, HaltState};
use super::types::{CpuPhase, HaltPhase, MCycleAction, Phase};

/// Number of operand bytes following a given opcode (0, 1, or 2).
pub(super) fn operand_count(opcode: u8) -> u8 {
    match opcode {
        // 1 operand byte: LD r,d8 / LD [HL],d8
        0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x36 | 0x3e => 1,
        // 1 operand byte: ALU A,d8
        0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => 1,
        // 1 operand byte: JR e8, JR cc,e8
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => 1,
        // 1 operand byte: LDH [a8],A / LDH A,[a8]
        0xe0 | 0xf0 => 1,
        // 1 operand byte: ADD SP,e8 / LD HL,SP+e8
        0xe8 | 0xf8 => 1,
        // 1 operand byte: CB prefix
        0xcb => 1,
        // 1 operand byte: STOP
        0x10 => 1,

        // 2 operand bytes: LD r16,d16
        0x01 | 0x11 | 0x21 | 0x31 => 2,
        // 2 operand bytes: LD [a16],SP
        0x08 => 2,
        // 2 operand bytes: LD [a16],A / LD A,[a16]
        0xea | 0xfa => 2,
        // 2 operand bytes: JP a16, JP cc,a16
        0xc3 | 0xc2 | 0xca | 0xd2 | 0xda => 2,
        // 2 operand bytes: CALL a16, CALL cc,a16
        0xcd | 0xc4 | 0xcc | 0xd4 | 0xdc => 2,

        _ => 0,
    }
}

impl Cpu {
    /// Fetch M-cycle: single read at [PC]. Returns `None` when the
    /// fetched instruction completes immediately (e.g. NOP).
    pub(super) fn mcycle_fetch(&mut self) -> Option<MCycleAction> {
        let step = self.exec_step;
        self.exec_step += 1;

        if step == 0 {
            self.ir_address = self.pc;
            return Some(MCycleAction::Read { address: self.pc });
        }

        // Step 1: opcode just arrived. PC advances (unless HALT-bug
        // suppresses) and we route to the decoded phase.
        let opcode = self.data_latch;
        let fetch_addr = match &self.current_action {
            Some(MCycleAction::Read { address }) => *address,
            _ => self.pc,
        };
        if self.halt.bug {
            self.halt.bug = false;
        } else {
            self.pc = fetch_addr.wrapping_add(1);
        }

        let needed = operand_count(opcode);
        if needed == 0 {
            let bytes = [opcode, 0, 0];
            let (instruction, phase, commit) = self.decode_retire(bytes, 1);
            self.instruction = instruction;
            if matches!(phase, Phase::Empty) {
                Some(self.enter_fetch_overlap(commit))
            } else {
                // Multi-Mcyc 0-operand op (LD (HL),A, POP rr, etc.):
                // run its execute phase before fetch overlap.
                self.phase = CpuPhase::Execute { phase, step: 0 };
                self.exec_step = 0;
                self.mcycle_execute()
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
            self.mcycle_execute()
        }
    }

    /// Drop halt and start the post-halt opcode fetch on the IME=0 wake
    /// path. With `mcyc = m7` parked through HALT, this M-cycle carries
    /// the m7-driven post-body fetch from PC.
    pub(super) fn enter_post_halt_fetch(&mut self) -> Option<MCycleAction> {
        self.halt.state = HaltState::Running;
        self.halt.rs_latched = false;
        self.phase = CpuPhase::Fetch;
        self.exec_step = 0;
        self.boundary_flag = true;
        self.mcycle_fetch()
    }

    /// Pure decode — returns the decoded `Instruction` with its `Phase`
    /// and retire-edge `Commit`. Does not mutate IME / dispatch state;
    /// `retire_edge` owns those.
    pub(super) fn decode_retire(
        &mut self,
        bytes: [u8; 3],
        bytes_read: u8,
    ) -> (Instruction, Phase, Commit) {
        let mut iter = bytes[..bytes_read as usize].iter().copied();
        let instruction = Instruction::decode(&mut iter).unwrap();

        let (phase, commit) = match &instruction {
            Instruction::Interrupt(InterruptInstruction::Await) => {
                (Phase::Empty, Commit::EnterHalt)
            }
            Instruction::Stop => (Phase::Empty, Commit::EnterStop),
            Instruction::Invalid(_) => (Phase::Empty, Commit::Invalid),
            Instruction::NoOperation => (Phase::Empty, Commit::NoOperation),
            Instruction::DecimalAdjustAccumulator => (Phase::Empty, Commit::Daa),
            Instruction::CarryFlag(cf) => (Phase::Empty, Commit::CarryFlag(cf.clone())),
            Instruction::Interrupt(InterruptInstruction::Enable) => {
                (Phase::Empty, Commit::EnableInterrupts)
            }
            Instruction::Interrupt(InterruptInstruction::Disable) => {
                (Phase::Empty, Commit::DisableInterrupts)
            }

            Instruction::Load(load) => Self::build_load(self, load),
            Instruction::Arithmetic(arith) => Self::build_arithmetic(self, arith),
            Instruction::Bitwise(bw) => Self::build_bitwise(self, bw),
            Instruction::BitShift(bs) => Self::build_bit_shift(self, bs),
            Instruction::BitFlag(bf) => Self::build_bit_flag(self, bf),
            Instruction::Jump(j) => Self::build_jump(self, j),
            Instruction::Stack(s) => Self::build_stack(self, s),
        };

        (instruction, phase, commit)
    }

    /// Enter the trailing fetch-overlap M-cycle at the opening edge.
    /// Captures `zacw` (dispatch_active) and routes early to dispatch
    /// or halt when needed. Commits apply inline so the new register
    /// values are visible at the start of the next M-cycle.
    pub(super) fn enter_fetch_overlap(&mut self, commit: Commit) -> MCycleAction {
        Self::apply_commit(self, commit);
        let deferred = Commit::NoOperation;

        if self.wz_to_pc {
            self.pc = self.wz;
            self.wz_to_pc = false;
        }
        self.boundary_flag = true;

        if self.dispatch.dispatch_active() {
            // zkog/zloz reset fires at ctl_int_entry_m6 (M3→M4 vector
            // resolve), driven by pending_vector_resolve in execute.rs.
            self.halt.state = HaltState::Running;
            self.halt.rs_latched = false;
            let pc = self.pc;
            self.phase = CpuPhase::InterruptDispatch {
                sp: self.stack_pointer,
                pc_hi: (pc >> 8) as u8,
                pc_lo: (pc & 0xff) as u8,
                step: 0,
            };
            self.exec_step = 0;
            self.irq.pending_vector_resolve = false;
            return self
                .next_mcycle()
                .expect("next_mcycle must return Some after dispatch arm");
        }

        if self.halt.state == HaltState::Locked {
            self.phase = CpuPhase::Locked;
            self.exec_step = 0;
            return MCycleAction::Internal { address: self.pc };
        }

        if self.halt.state == HaltState::Stopped {
            // STOP idles in the spin machinery with no halt-bug and no
            // interrupt-wake; the system re-engages via `resume_from_stop`.
            return self.mcycle_halted_entry(HaltPhase::Spin);
        }

        if self.halt.state == HaltState::Halting {
            // Defer the halt-bug-vs-halt-state decision to M_h start
            // (the boundary at the end of HALT's body M-cycle). The
            // body M-cycle reads HALT+1 like any overlap fetch;
            // `data_phase_n` pulses normally (rs_latched stays false)
            // so the per-bit irq_latch gates correctly.
            self.halt.state = HaltState::Halted;
            self.halt.rs_latched = false;
            self.halt.bug_check_pending = true;
        }

        self.phase = CpuPhase::Execute {
            phase: Phase::FetchOverlap { commit: deferred },
            step: 1,
        };
        self.exec_step = 1;
        self.ir_address = self.pc;
        MCycleAction::Read { address: self.pc }
    }
}
