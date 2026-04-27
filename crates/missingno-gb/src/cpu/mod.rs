use dff::Dff;
use flags::{Flag, Flags};
use mcycle::{BusAction, BusDot, CpuPhase, Phase};
use registers::{Register8, Register16};

pub mod commit;
pub mod dff;
pub mod flags;
pub mod instructions;
pub mod mcycle;
pub mod registers;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InterruptMasterEnable {
    Disabled,
    Enabled,
}

/// CPU execution state w.r.t. the HALT instruction. Halt-release fires
/// combinationally via g43 → g49 once g42 captures `(IF & IE) != 0`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HaltState {
    /// Normal execution — CPU fetches and executes instructions.
    Running,
    /// HALT instruction decoded — the next fetch runs as a dummy
    /// fetch (read [PC] without incrementing), then transitions to
    /// Halted. Models the hardware's 1 M-cycle transition from HALT
    /// execution to idle mode.
    Halting,
    /// HALT idle loop — ticking hardware, waiting for `(IF & IE) != 0`.
    Halted,
}

/// Selected-interrupt latch consumed at the dispatch check.
pub struct Cpu {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,

    pub stack_pointer: u16,
    /// The IDU address counter. Drives the address bus during fetch and
    /// operand reads. Advances by 1 each time a byte is fetched. Also
    /// used for relative jump resolution. On hardware this is the IDU
    /// output, distinct from the PC register DFF (reg.pc).
    pub bus_counter: u16,
    /// The PC register DFF (hardware reg.pc). On hardware this latches
    /// when the CPU issues a bus_read (reg.pc = bus_addr + 1), but does
    /// NOT latch after the last operand byte of JP/JR (which use
    /// bus_pass instead of bus_read). Currently mirrors bus_counter —
    /// the divergence points will be added incrementally.
    pub pc: u16,
    /// The PC at the start of the current instruction. Latched at each
    /// instruction boundary — stays constant during operand fetches.
    pub instruction_pc: u16,

    pub flags: Flags,

    /// IME flip-flop (zivv). EI/DI mutate this synchronously inside
    /// their commit; the underlying SR-latch chain (zjje, zrsy) is
    /// combinational, so by the retire edge the IME DFF has already
    /// captured the new value. The "1-instruction delay" sits in the
    /// int_entry chain (zaij/zkog gated against EI/DI data_phase),
    /// not here.
    pub ime: Dff<InterruptMasterEnable>,
    pub halt_state: HaltState,
    /// HALT bug: when HALT is executed with IME=0 and an interrupt is
    /// already pending, the CPU doesn't truly halt — it resumes
    /// immediately but fails to increment PC on the next opcode fetch,
    /// causing that byte to be read twice.
    pub halt_bug: bool,

    // ── Persistent state machine fields ──
    /// Current execution phase of the CPU state machine.
    pub(super) phase: CpuPhase,
    /// The decoded instruction, preserved for debugger display.
    pub(super) instruction: instructions::Instruction,
    /// Dot position within the current M-cycle (0–3).
    pub(super) dot: BusDot,
    /// Whether we have a pending M-cycle.
    pub(super) mcycle_active: bool,
    /// The BusAction for the current M-cycle.
    current_action: Option<mcycle::BusAction>,
    /// Step counter for Fetch/Halted phases (tracks M-cycle sub-steps).
    pub(super) exec_step: u8,
    /// Continuous M-cycle counter within the current instruction.
    /// 0 = fetch M-cycle, 1 = first execute M-cycle, etc.
    /// Incremented by next_mcycle(), reset by enter_fetch().
    /// Matches the hardware op_state sequencer.
    pub(super) op_state: u8,
    /// Pending jump target address. Set by CondJump's internal M-cycle,
    /// consumed by the next enter_fetch() to issue the fetch Read from
    /// the target instead of bus_counter. On hardware, the PC
    /// register stays at the post-operand address during the internal
    /// M-cycle; it only advances to target+1 when the fetch processes.
    pub(super) pending_jump_target: Option<u16>,
    /// Scratch byte for multi-read phases (Pop, CondReturn).
    pub(super) scratch: u8,
    /// The dot position that produced the last DotAction (for the executor
    /// to check timing signals like boga, bowa, mopa).
    pub(super) last_dot: BusDot,
    /// IE push bug flag.
    pub(super) pending_vector_resolve: bool,
    /// Set when the CPU transitions to the Fetch phase. The executor
    /// reads this to detect instruction boundaries for EI delay and
    /// step_instruction().
    pub(super) boundary_flag: bool,
    /// Whether an interrupt is currently pending (IF & IE != 0).
    /// Combinational input to the g42 DFF; also consumed directly by the
    /// HALT bug check.
    pub(super) interrupt_pending: bool,
    /// g42 (yoii) flip-flop. CLK9-cadence; captures `interrupt_pending`
    /// once per M-cycle on the master-clock rising edge. Drives the HALT
    /// release chain (g42 → g43 → g49) and produces the per-source
    /// HALT-wake timing differential (timer vs PPU IFs).
    pub(super) g42: Dff<bool>,
    /// int_entry (zacw) flip-flop for the running-CPU dispatch-ready
    /// chain. Captures `interrupt_pending` once per M-cycle on the
    /// master-clock rising edge — single DFF stage between IF assertion
    /// and ISR M1 entry. Hardware cell: dmg-sim `zacw_inst.d = zfex`.
    pub(super) int_entry: Dff<bool>,
    /// Carries the retire-rise int_entry-chain gate decision (set by
    /// `commit()` when the retiring Commit is EnableInterrupts /
    /// DisableInterrupts) across to the same-dot fall, where the zacw
    /// DFF actually captures.
    pub(super) pending_int_entry_gate: bool,
    /// Carries the retire-rise dispatch-readiness state. Set by
    /// `commit()` when a retire ran; the executor then calls
    /// `tick_int_entry()` at the matching fall to perform the zacw
    /// capture using the IF settled by that same dot's master-clock
    /// fall. Cleared by `tick_int_entry()`.
    pub(super) pending_int_entry_capture: bool,
    /// Carries the HALT-wake-rise dispatch-readiness decision across to
    /// the same dot's master-clock fall, where the post-fall int_take
    /// view determines ISR-vs-Fetch. Mirrors `pending_int_entry_capture`
    /// but for the HALT-wake exit path: g42.q↑ at the boundary rise
    /// drops halt combinationally; the int_entry (zacw) capture that
    /// chooses ISR vs running-CPU continuation reads int_take post-fall.
    pub(super) pending_halt_wake_dispatch: bool,
}

impl Cpu {
    pub fn new(checksum: u8) -> Cpu {
        Cpu {
            a: 0x01,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,

            stack_pointer: 0xfffe,
            bus_counter: 0x0100,
            pc: 0x0100,
            instruction_pc: 0x0100,

            flags: if checksum == 0 {
                Flags::ZERO
            } else {
                Flags::ZERO | Flags::CARRY | Flags::HALF_CARRY
            },

            ime: Dff::new(InterruptMasterEnable::Disabled),
            halt_state: HaltState::Running,
            halt_bug: false,

            // The skip-boot CPU enters mid-WriteOp: the boot ROM's
            // terminating LDH (0xFF50), A is still draining its WriteOp
            // M-cycle when the IDU has already advanced bus_counter to
            // 0x0100. Pre-load the persistent state machine fields with
            // the in-flight WriteOp residual so next_dot drains it
            // through its remaining dots (Idle, Idle, Write{0xFF50,0x01})
            // before chaining into the first cartridge fetch.
            phase: CpuPhase::Execute {
                phase: Phase::WriteOp {
                    address: 0xFF50,
                    value: 0x01,
                    hl_post: 0,
                },
                step: 1,
            },
            instruction: instructions::Instruction::NoOperation,
            dot: BusDot::ONE,
            mcycle_active: true,
            current_action: Some(BusAction::Write {
                address: 0xFF50,
                value: 0x01,
            }),
            exec_step: 0,
            op_state: 2,
            pending_jump_target: None,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            boundary_flag: true, // Start at an instruction boundary

            interrupt_pending: false,
            g42: Dff::new(false),
            int_entry: Dff::new(false),
            pending_int_entry_gate: false,
            pending_int_entry_capture: false,
            pending_halt_wake_dispatch: false,
        }
    }

    /// Power-on state: all registers zeroed, PC=0x0000 for boot ROM entry.
    pub fn power_on() -> Cpu {
        Cpu {
            a: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            stack_pointer: 0x0000,
            bus_counter: 0x0000,
            pc: 0x0000,
            instruction_pc: 0x0000,
            flags: Flags::empty(),
            ime: Dff::new(InterruptMasterEnable::Disabled),
            halt_state: HaltState::Running,
            halt_bug: false,
            phase: CpuPhase::Fetch,
            instruction: instructions::Instruction::NoOperation,
            dot: BusDot::ZERO,
            mcycle_active: false,
            current_action: None,
            exec_step: 0,
            // Initialized to MAX so the first wrapping_add(1) in
            // next_dot() wraps to 0 for the first fetch M-cycle.
            // enter_fetch() resets to 0 at each instruction boundary.
            op_state: u8::MAX,
            pending_jump_target: None,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            boundary_flag: true,

            interrupt_pending: false,
            g42: Dff::new(false),
            int_entry: Dff::new(false),
            pending_int_entry_gate: false,
            pending_int_entry_capture: false,
            pending_halt_wake_dispatch: false,
        }
    }

    /// Construct a CPU from a gbtrace snapshot at an instruction boundary.
    ///
    /// Execution state machine fields are set to their instruction-boundary
    /// defaults (Fetch phase, step 0, no pending actions).
    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::CpuSnapshot) -> Cpu {
        Cpu {
            a: snap.a,
            b: snap.b,
            c: snap.c,
            d: snap.d,
            e: snap.e,
            h: snap.h,
            l: snap.l,
            stack_pointer: snap.sp,
            bus_counter: snap.pc,
            pc: snap.pc,
            instruction_pc: snap.pc,
            flags: Flags::from_bits_retain(snap.f),
            ime: Dff::new(if snap.ime {
                InterruptMasterEnable::Enabled
            } else {
                InterruptMasterEnable::Disabled
            }),
            // int_entry_gate is transient (set only during EI/DI's
            halt_state: match snap.halt_state {
                1 => HaltState::Halting,
                2 => HaltState::Halted,
                _ => HaltState::Running,
            },
            halt_bug: snap.halt_bug,
            phase: CpuPhase::Fetch,
            instruction: instructions::Instruction::NoOperation,
            dot: BusDot::ZERO,
            mcycle_active: false,
            current_action: None,
            exec_step: 0,
            // Initialized to MAX so the first wrapping_add(1) in
            // next_dot() wraps to 0 for the first fetch M-cycle.
            // enter_fetch() resets to 0 at each instruction boundary.
            op_state: u8::MAX,
            pending_jump_target: None,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            boundary_flag: true,

            interrupt_pending: false,
            g42: Dff::new(false),
            int_entry: Dff::new(false),
            pending_int_entry_gate: false,
            pending_int_entry_capture: false,
            pending_halt_wake_dispatch: false,
        }
    }

    pub fn get_register8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.a,
            Register8::B => self.b,
            Register8::C => self.c,
            Register8::D => self.d,
            Register8::E => self.e,
            Register8::H => self.h,
            Register8::L => self.l,
        }
    }

    pub(crate) fn set_register8(&mut self, register: Register8, value: u8) {
        match register {
            Register8::A => self.a = value,
            Register8::B => self.b = value,
            Register8::C => self.c = value,
            Register8::D => self.d = value,
            Register8::E => self.e = value,
            Register8::H => self.h = value,
            Register8::L => self.l = value,
        }
    }

    pub fn get_register16(&self, register: Register16) -> u16 {
        match register {
            Register16::Bc => u16::from_be_bytes([self.b, self.c]),
            Register16::De => u16::from_be_bytes([self.d, self.e]),
            Register16::Hl => u16::from_be_bytes([self.h, self.l]),
            Register16::StackPointer => self.stack_pointer,
            Register16::Af => u16::from_be_bytes([self.a, self.flags.bits()]),
        }
    }

    pub(crate) fn set_register16(&mut self, register: Register16, value: u16) {
        let high = (value / 0x100) as u8;
        let low = (value % 0x100) as u8;

        match register {
            Register16::Bc => {
                self.b = high;
                self.c = low;
            }
            Register16::De => {
                self.d = high;
                self.e = low;
            }
            Register16::Hl => {
                self.h = high;
                self.l = low;
            }
            Register16::StackPointer => self.stack_pointer = value,
            Register16::Af => {
                self.a = high;
                self.flags = Flags::from_bits_retain(low & 0xF0);
            }
        }
    }

    pub fn interrupts_enabled(&self) -> bool {
        self.ime.output() != InterruptMasterEnable::Disabled
    }

    /// Continuous instruction M-cycle counter. 0 = fetch M-cycle,
    /// 1 = first execute M-cycle, 2 = second, etc. Matches GateBoy's
    /// op_state hardware sequencer.
    pub fn op_state(&self) -> u8 {
        self.op_state
    }

    /// Ring counter state (AFUR<<3|ALEF<<2|APUK<<1|ADYK<<0), matching
    /// GateBoy's encoding. The trace captures after both sub-phases of
    /// each dot, so we report the state at the even sub-phase (B,D,F,H).
    pub fn mcycle_phase(&self) -> u8 {
        match self.last_dot.as_u8() {
            0 => 0x0C, // sub-phase B: AFUR=1 ALEF=1 APUK=0 ADYK=0
            1 => 0x0F, // sub-phase D: AFUR=1 ALEF=1 APUK=1 ADYK=1
            2 => 0x03, // sub-phase F: AFUR=0 ALEF=0 APUK=1 ADYK=1
            3 => 0x00, // sub-phase H: AFUR=0 ALEF=0 APUK=0 ADYK=0
            _ => unreachable!(),
        }
    }

    /// The address on the CPU bus for the current M-cycle.
    pub fn bus_address(&self) -> u16 {
        match &self.current_action {
            Some(mcycle::BusAction::Read { address }) => *address,
            Some(mcycle::BusAction::Write { address, .. }) => *address,
            Some(mcycle::BusAction::InternalOamBug { address }) => *address,
            Some(mcycle::BusAction::Internal { address }) => *address,
            None => 0,
        }
    }

    /// Whether the CPU is currently halted.
    pub fn is_halted(&self) -> bool {
        self.halt_state == HaltState::Halted
    }

    /// The pending bus write for the current M-cycle, if any.
    /// On hardware, the CPU places the address on the bus at phase A
    /// and drives write data from phase E.
    pub fn pending_bus_write(&self) -> Option<(u16, u8)> {
        match &self.current_action {
            Some(mcycle::BusAction::Write { address, value }) => Some((*address, *value)),
            _ => None,
        }
    }
}
