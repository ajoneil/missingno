use commit::Commit;
use dff::Dff;
use flags::{Flag, Flags};
use mcycle::{BusAction, CpuPhase, MCycleAction, Phase, TCycle};
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

/// CPU execution state w.r.t. the HALT instruction. Halt-release
/// fires combinationally via g43 → g49 once `irq_latched` (yoii)
/// captures `(IF & IE) != 0`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HaltState {
    /// Normal execution — CPU fetches and executes instructions.
    Running,
    /// HALT decoded — the next fetch runs as a dummy fetch (read
    /// [PC] without incrementing) before transitioning to `Halted`.
    Halting,
    /// HALT idle loop — ticking hardware, waiting for `(IF & IE) != 0`.
    Halted,
}

/// All halt-related state on the CPU: the high-level execution mode
/// plus the four hardware-level flags that together drive HALT-bug
/// behaviour, the data_phase_n gating during halt-spin, and the
/// PPU's post-HALT-wake timing offset.
pub struct HaltContext {
    pub state: HaltState,
    /// HALT-bug flag: set when HALT is decoded with IME=0 and an
    /// interrupt already pending. The next opcode fetch reads its
    /// byte twice (PC fails to increment).
    pub bug: bool,
    /// Window between HALT decode and the next M-cycle boundary, where
    /// the halt-bug-vs-halt-state decision fires (yoii/ysbt parallel
    /// capture at M_h start CLK9↑).
    pub bug_check_pending: bool,
    /// True when the halt RS-latch (`ynkw`) is set: false during the
    /// HALT body M-cycle, true during halt-state spin. While true,
    /// `data_phase` is held LOW so the per-bit irq_latch stays
    /// transparent.
    pub rs_latched: bool,
    /// True while executing a handler reached via IME=1 HALT-wake
    /// dispatch. Read by the BGP CUPA write path to defer the
    /// `dlatch_ee` effect — post-HALT-wake BGP writes land 4-5 LCD
    /// columns later than the running-CPU path. Behavioural; no
    /// gate-level anchor.
    pub wake_active: bool,
}

impl HaltContext {
    pub fn new() -> Self {
        Self {
            state: HaltState::Running,
            bug: false,
            bug_check_pending: false,
            rs_latched: false,
            wake_active: false,
        }
    }
}

impl Default for HaltContext {
    fn default() -> Self {
        Self::new()
    }
}

/// All CPU-side interrupt state apart from the dispatch chain itself.
/// The IF/IE register file lives on `interrupts::Registers` (bus-side);
/// this struct holds the latches inside the SM83 that gate it.
pub struct IrqContext {
    /// IME flip-flop. Promoted from `ime_delay` at every M-cycle
    /// boundary — that staging produces EI's one-instruction delay.
    /// DI clears both stages immediately; RETI sets both immediately.
    pub ime: Dff<InterruptMasterEnable>,
    /// One-stage shadow for IME. EI sets this; the next M-cycle
    /// boundary copies it into `ime`.
    pub ime_delay: bool,
    /// IE-push-bug flag — set during dispatch's M3 vector-resolve window.
    pub(super) pending_vector_resolve: bool,
    /// Combinational `(IF & IE) != 0`. Coarse signal kept for the
    /// gbtrace adapter; dispatch reads the data-phase-gated
    /// `dispatch.latched()` instead.
    pub(super) irq_pending: bool,
    /// `irq_latched` (yoii) DFF. CLK9-cadence capture of the data-
    /// phase-gated `dispatch.latched()`. Drives the HALT-release
    /// chain (yoii → ykua → ynkw).
    pub(super) irq_latched: Dff<bool>,
}

impl IrqContext {
    pub fn new() -> Self {
        Self {
            ime: Dff::new(InterruptMasterEnable::Disabled),
            ime_delay: false,
            pending_vector_resolve: false,
            irq_pending: false,
            irq_latched: Dff::new(false),
        }
    }
}

impl Default for IrqContext {
    fn default() -> Self {
        Self::new()
    }
}

/// The SM83 CPU. Owns register file, IME, halt state, and the
/// state-machine fields that sequence each instruction's M-cycles.
pub struct Cpu {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,

    pub stack_pointer: u16,
    /// IDU address counter. Drives the address bus during fetch and
    /// operand reads; advances by 1 per fetched byte. Functions as
    /// the working PC; the hardware-truthful split into `reg.pc` DFF
    /// vs IDU output isn't modelled (no observable consumer yet).
    pub bus_counter: u16,

    /// Bus data latch — the byte the SM83 captured off `cpu_port_d`
    /// near the end of T-cycle 3 of a read M-cycle. Holds until the
    /// next read latches. Read by the state machine each T-cycle.
    pub data_latch: u8,

    /// The `BusAction` produced by the most recent `next_tcycle`. The
    /// executor reads this between rise/fall edges of the same T-cycle
    /// to route memory reads/writes.
    pub last_bus_action: BusAction,

    pub flags: Flags,

    pub irq: IrqContext,
    pub halt: HaltContext,

    // ── Persistent state machine fields ──
    /// Current execution phase of the CPU state machine.
    pub(super) phase: CpuPhase,
    /// The decoded instruction, preserved for debugger display.
    pub(super) instruction: instructions::Instruction,
    /// T-cycle position within the current M-cycle (0–3).
    pub(super) tcycle: TCycle,
    /// Whether an M-cycle is in flight.
    pub(super) mcycle_active: bool,
    /// Whether the next rise() should fire the M-cycle-boundary block.
    /// Decoupled from `mcycle_active` so the skip-boot constructor can
    /// encode "M-cycle in flight, but the opening CLK9↑'s boundary work
    /// fired in the boot ROM's domain." Normally tracks `!mcycle_active`.
    pub(super) boundary_pending: bool,
    /// The `MCycleAction` for the current M-cycle.
    current_action: Option<mcycle::MCycleAction>,
    /// Step counter for Fetch / Halted phases (tracks M-cycle sub-steps).
    pub(super) exec_step: u8,
    /// Pending jump target. Set by CondJump's internal M-cycle,
    /// consumed by the next `enter_fetch()` to issue the fetch Read
    /// from the target instead of `bus_counter`.
    pub(super) pending_jump_target: Option<u16>,
    /// Scratch byte for multi-read phases (Pop, CondReturn).
    pub(super) scratch: u8,
    /// T-cycle that produced the last `BusAction` — the executor reads
    /// this to time per-T-cycle work after `next_tcycle()`.
    pub(super) last_tcycle: TCycle,
    /// Set when the CPU transitions to Fetch. The executor reads this
    /// to detect instruction boundaries.
    pub(super) boundary_flag: bool,
    /// Running-CPU dispatch chain: per-bit irq_latch_inst<i> →
    /// priority chain → int_take → zaij → zkog/zloz → zfex → zacw.
    /// Owns the `data_phase_n` latch and the EI/DI block.
    pub(super) dispatch: dispatch_chain::DispatchChain,
}

impl Cpu {
    /// Cold-start state: all registers zeroed, PC=0x0000 ready for
    /// the boot ROM to execute from address 0.
    pub fn new() -> Cpu {
        Self::boundary_state()
    }

    /// Post-boot-ROM state at PC=0x0100. The in-flight M-cycle is the
    /// cartridge m1 fetch overlapping the boot ROM's final
    /// `LDH (0xFF50),A` — `boundary_pending` is false because the
    /// opening CLK9↑'s boundary work fired in the boot ROM's domain
    /// before simulator t=0.
    pub fn post_boot(checksum: u8) -> Cpu {
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

            flags: if checksum == 0 {
                Flags::ZERO
            } else {
                Flags::ZERO | Flags::CARRY | Flags::HALF_CARRY
            },

            // ── In-flight M-cycle: opening m1 fetch of cartridge code ──
            phase: CpuPhase::Execute {
                phase: Phase::FetchOverlap {
                    commit: Commit::NoOperation,
                },
                step: 1,
            },
            tcycle: TCycle::ONE,
            mcycle_active: true,
            boundary_pending: false,
            current_action: Some(MCycleAction::Read { address: 0x0100 }),
            exec_step: 1,

            ..Self::boundary_state()
        }
    }

    /// Construct a CPU from a gbtrace snapshot at an instruction
    /// boundary. The state machine fields are reset to their boundary
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
            flags: Flags::from_bits_retain(snap.f),
            irq: IrqContext {
                ime: Dff::new(if snap.ime {
                    InterruptMasterEnable::Enabled
                } else {
                    InterruptMasterEnable::Disabled
                }),
                ime_delay: snap.ime,
                ..IrqContext::new()
            },
            halt: HaltContext {
                state: match snap.halt_state {
                    1 => HaltState::Halting,
                    2 => HaltState::Halted,
                    _ => HaltState::Running,
                },
                bug: snap.halt_bug,
                ..HaltContext::new()
            },
            ..Self::boundary_state()
        }
    }

    /// Boundary-aligned defaults: zeroed registers, Fetch phase, no
    /// pending actions, dispatch chain fresh. Used by `new`, and as
    /// the `..base` for `post_boot` and `from_snapshot`.
    fn boundary_state() -> Cpu {
        Cpu {
            a: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            stack_pointer: 0,
            bus_counter: 0,
            data_latch: 0,
            flags: Flags::empty(),
            irq: IrqContext::new(),
            halt: HaltContext::new(),
            phase: CpuPhase::Fetch,
            instruction: instructions::Instruction::NoOperation,
            tcycle: TCycle::ZERO,
            mcycle_active: false,
            boundary_pending: true,
            current_action: None,
            exec_step: 0,
            pending_jump_target: None,
            scratch: 0,
            last_tcycle: TCycle::ZERO,
            last_bus_action: BusAction::Idle,
            boundary_flag: true,
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
        let [high, low] = value.to_be_bytes();
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
        self.irq.ime.output() != InterruptMasterEnable::Disabled
    }

    /// Per-instruction (or dispatch) M-cycle index — 0 = fetch,
    /// 1 = first post-fetch M-cycle, etc. Interrupt dispatch's
    /// 5 M-cycles report 0..=4 (M0 overlaps the fetch). Halt: 0.
    pub fn op_state(&self) -> u8 {
        match &self.phase {
            mcycle::CpuPhase::Fetch => 0,
            mcycle::CpuPhase::Execute { step, .. } => *step as u8,
            mcycle::CpuPhase::InterruptDispatch { step, .. } => *step as u8,
            mcycle::CpuPhase::Halted(_) => 0,
        }
    }

    /// AFUR/ALEF/APUK/ADYK ring-counter state, packed
    /// `AFUR<<3 | ALEF<<2 | APUK<<1 | ADYK<<0` to match GateBoy's
    /// encoding at the post-fall sampling instant.
    pub fn mcycle_phase(&self) -> u8 {
        match self.last_tcycle.as_u8() {
            0 => 0x0E, // AFUR=1 ALEF=1 APUK=1 ADYK=0
            1 => 0x07, // AFUR=0 ALEF=1 APUK=1 ADYK=1
            2 => 0x01, // AFUR=0 ALEF=0 APUK=0 ADYK=1
            3 => 0x08, // AFUR=1 ALEF=0 APUK=0 ADYK=0
            _ => unreachable!(),
        }
    }

    /// Address on the CPU bus for the current M-cycle.
    pub fn bus_address(&self) -> u16 {
        match &self.current_action {
            Some(mcycle::MCycleAction::Read { address }) => *address,
            Some(mcycle::MCycleAction::Write { address, .. }) => *address,
            Some(mcycle::MCycleAction::InternalOamBug { address }) => *address,
            Some(mcycle::MCycleAction::Internal { address }) => *address,
            None => 0,
        }
    }

    /// Combinational `(IF & IE) != 0` — level-sensitive input to the
    /// dispatch chain and to the `irq_latched` (yoii) DFF.
    pub fn irq_pending(&self) -> bool {
        self.irq.irq_pending
    }

    /// Captured running-CPU dispatch decision (zacw). True while the
    /// 5-M-cycle dispatch sequence is in progress.
    pub fn dispatch_active(&self) -> bool {
        self.dispatch.dispatch_active()
    }

    /// CLK9-cadence captured `(IF & IE) != 0` (yoii). Drives the
    /// HALT-release chain.
    pub fn irq_latched(&self) -> bool {
        self.irq.irq_latched.output()
    }

    /// Return `boundary_pending` and clear it. Called once per rise().
    pub(super) fn consume_boundary_pending(&mut self) -> bool {
        let pending = self.boundary_pending;
        self.boundary_pending = false;
        pending
    }

    /// IE-push-bug flag (gbtrace extension). Set during dispatch's M3
    /// vector-resolve window.
    pub fn pending_vector_resolve_flag(&self) -> bool {
        self.irq.pending_vector_resolve
    }

    /// HALT-bug flag (gbtrace extension). See `HaltContext::bug`.
    pub fn halt_bug_flag(&self) -> bool {
        self.halt.bug
    }

    pub fn is_halted(&self) -> bool {
        self.halt.state == HaltState::Halted
    }

    pub fn halt_rs_latched(&self) -> bool {
        self.halt.rs_latched
    }

    pub fn is_halt_wake_active(&self) -> bool {
        self.halt.wake_active
    }

    /// Whether the CPU is in a fetch M-cycle (reading the next opcode).
    /// Drives `ctl_fetch` in the dispatch chain's xogs gate.
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

    /// Address + value the CPU is writing this M-cycle.
    pub fn pending_bus_write(&self) -> Option<(u16, u8)> {
        match &self.current_action {
            Some(mcycle::MCycleAction::Write { address, value }) => Some((*address, *value)),
            _ => None,
        }
    }

    /// Address the CPU is reading this M-cycle.
    pub fn pending_bus_read(&self) -> Option<u16> {
        match &self.current_action {
            Some(mcycle::MCycleAction::Read { address }) => Some(*address),
            _ => None,
        }
    }
}
