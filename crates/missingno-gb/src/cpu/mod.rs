use commit::Commit;
use dff::Dff;
use flags::{Flag, Flags};
use mcycle::{BusAction, BusDot, CpuPhase, Phase};
use registers::{Register8, Register16};

pub mod commit;
pub mod dff;
pub mod dispatch_chain;
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
/// combinationally via g43 → g49 once irq_latched (yoii) captures
/// `(IF & IE) != 0`.
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
    /// dispatch_active chain (zaij/zkog gated against EI/DI data_phase),
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
    /// Whether the next rise() should fire the M-cycle-boundary block
    /// (timers.mcycle, tick_zacw, tick_irq_latched, boundary PPU rise,
    /// dispatch capture). Decoupled from mcycle_active to let the
    /// skip-boot constructor encode "M-cycle in flight, but the opening
    /// CLK9↑'s boundary work has already fired in the boot ROM's domain"
    /// — the post-CLK9↑ state hardware is in at PC=0x0100. In normal
    /// execution, this mirrors !mcycle_active (one boundary per
    /// M-cycle).
    pub(super) boundary_pending: bool,
    /// The BusAction for the current M-cycle.
    current_action: Option<mcycle::BusAction>,
    /// Step counter for Fetch/Halted phases (tracks M-cycle sub-steps).
    pub(super) exec_step: u8,
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
    /// Combinational input to the irq_latched (yoii) DFF; also consumed
    /// directly by the HALT bug check.
    pub(super) irq_pending: bool,
    /// irq_latched (yoii) flip-flop. CLK9-cadence; captures `irq_pending`
    /// once per M-cycle on the master-clock rising edge. Drives the HALT
    /// release chain (yoii → g43 → g49) and produces the per-source
    /// HALT-wake timing differential (timer vs PPU IFs).
    pub(super) irq_latched: Dff<bool>,
    /// Running-CPU dispatch chain: per-bit irq_latch_inst<i> →
    /// priority chain → int_take → zaij → zkog/zloz → zfex → zacw DFF.
    /// Owns the data_phase_n latch, the zzom EI/DI block, and the
    /// dispatch_active (zacw) capture; consumers read
    /// `dispatch.dispatch_active()`.
    pub(super) dispatch: dispatch_chain::DispatchChain,
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

            // Skip-boot anchor: simulator t=0 is the post-rise of
            // the M-cycle boundary CLK9↑ that opens LDH (0xFF50),A's
            // post-body fetch (= cartridge instruction m1 under SM83
            // fetch overlap). The ring counter captured the new
            // M-cycle's state on the edge; reg_pc holds 0x0100 and
            // cpu_port_a is driving 0x0100 combinationally; LDH's
            // FF50 write retired in the prior m2 cycle. The in-flight
            // M-cycle is the FetchOverlap carrying NoOperation; the
            // staged Read emits Idle for dots 0-2 and the cartridge
            // byte read at dot 3 = boga. boundary_pending is false:
            // the opening CLK9↑'s boundary work fired in the boot
            // ROM's domain, before simulator t=0 — the dispatch
            // chain DFFs, timers, and PPU init already reflect the
            // post-edge state.
            phase: CpuPhase::Execute {
                phase: Phase::FetchOverlap {
                    commit: Commit::NoOperation,
                },
                step: 1,
            },
            instruction: instructions::Instruction::NoOperation,
            dot: BusDot::ONE,
            mcycle_active: true,
            boundary_pending: false,
            current_action: Some(BusAction::Read { address: 0x0100 }),
            exec_step: 1,
            pending_jump_target: None,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            boundary_flag: true, // Start at an instruction boundary

            irq_pending: false,
            irq_latched: Dff::new(false),
            dispatch: dispatch_chain::DispatchChain::new(),
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
            boundary_pending: true,
            current_action: None,
            exec_step: 0,
            pending_jump_target: None,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            boundary_flag: true,

            irq_pending: false,
            irq_latched: Dff::new(false),
            dispatch: dispatch_chain::DispatchChain::new(),
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
            boundary_pending: true,
            current_action: None,
            exec_step: 0,
            pending_jump_target: None,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            boundary_flag: true,

            irq_pending: false,
            irq_latched: Dff::new(false),
            dispatch: dispatch_chain::DispatchChain::new(),
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

    /// Per-instruction (or dispatch) M-cycle index, matching GateBoy's
    /// hardware sequencer state. Resets to 0 at instruction boundaries.
    /// 0 = fetch M-cycle, 1 = first post-fetch M-cycle, ... For interrupt
    /// dispatch the 5 M-cycles report 0, 1, 2, 3, 4 (M0 = the fetch
    /// M-cycle the dispatch overlaps with). For halt the index is 0.
    pub fn op_state(&self) -> u8 {
        // Computed from the current phase + step. Both Execute and
        // InterruptDispatch's `step` fields are post-incremented inside
        // mcycle_execute / mcycle_isr, so by the after-fall sample point
        // they hold the M-cycle index of the *current* M-cycle.
        match &self.phase {
            mcycle::CpuPhase::Fetch => 0,
            mcycle::CpuPhase::Execute { step, .. } => *step as u8,
            mcycle::CpuPhase::InterruptDispatch { step, .. } => *step as u8,
            mcycle::CpuPhase::Halted(_) => 0,
        }
    }

    /// AFUR/ALEF/APUK/ADYK ring counter state (AFUR<<3|ALEF<<2|APUK<<1|ADYK<<0),
    /// matching GateBoy's encoding. Reports the post-fall settled DFF state at
    /// the after-fall sampling instant, so the value matches what GateBoy's
    /// adapter emits at the same physical edge.
    pub fn mcycle_phase(&self) -> u8 {
        match self.last_dot.as_u8() {
            0 => 0x0E, // AFUR=1 ALEF=1 APUK=1 ADYK=0
            1 => 0x07, // AFUR=0 ALEF=1 APUK=1 ADYK=1
            2 => 0x01, // AFUR=0 ALEF=0 APUK=0 ADYK=1
            3 => 0x08, // AFUR=1 ALEF=0 APUK=0 ADYK=0
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

    /// Combinational `(IF & IE) != 0` across the 5 active IRQ bits.
    /// Level-sensitive input to both the running-CPU dispatch chain and
    /// the `irq_latched` (yoii) DFF.
    pub fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    /// Captured running-CPU dispatch decision (zacw) DFF q. When true,
    /// the 5-M-cycle dispatch sequence is in progress.
    pub fn dispatch_active(&self) -> bool {
        self.dispatch.dispatch_active()
    }

    /// CLK9-cadence captured `(IF & IE) != 0` (yoii). Drives the
    /// HALT-release chain.
    pub fn irq_latched(&self) -> bool {
        self.irq_latched.output()
    }

    /// Consume `boundary_pending` — return its current value and clear
    /// it to false. Called by the executor at the start of each rise()
    /// to decide whether the M-cycle-boundary block fires this edge.
    pub(super) fn consume_boundary_pending(&mut self) -> bool {
        let pending = self.boundary_pending;
        self.boundary_pending = false;
        pending
    }

    /// IE push bug flag — set during dispatch M3 vector resolution
    /// (the spec's "vector resolved between M-cycles 3 and 4" window
    /// where a write to IE from the pushed PC's high byte can change
    /// which vector is dispatched). Exposed as a gbtrace extension
    /// field for emulator-internal debugging.
    pub fn pending_vector_resolve_flag(&self) -> bool {
        self.pending_vector_resolve
    }

    /// HALT bug flag — set when HALT decoded with IME=0 and an
    /// interrupt already pending. Causes the next opcode fetch to read
    /// the byte twice. Exposed as a gbtrace extension field.
    pub fn halt_bug_flag(&self) -> bool {
        self.halt_bug
    }

    /// Whether the CPU is currently halted.
    pub fn is_halted(&self) -> bool {
        self.halt_state == HaltState::Halted
    }

    /// Whether the CPU is in a fetch M-cycle (the bus is reading the
    /// next opcode). Drives ctl_fetch in the dispatch chain's xogs
    /// gate. True for CpuPhase::Fetch (M1 opcode fetch) and for
    /// Phase::FetchOverlap (the trailing fetch-overlap M-cycle that
    /// reads the next instruction's opcode while finishing the current).
    pub fn is_fetch_phase(&self) -> bool {
        matches!(
            self.phase,
            CpuPhase::Fetch
                | CpuPhase::Execute {
                    phase: Phase::FetchOverlap { .. },
                    ..
                }
        )
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

    /// The pending bus read address for the current M-cycle, if any.
    /// On hardware, the CPU places the address on the bus at phase A;
    /// the addressed peripheral's tri-state driver enables at dot 2
    /// (`tobe`/`wafu` rising) and drives the bus.
    pub fn pending_bus_read(&self) -> Option<u16> {
        match &self.current_action {
            Some(mcycle::BusAction::Read { address }) => Some(*address),
            _ => None,
        }
    }
}
