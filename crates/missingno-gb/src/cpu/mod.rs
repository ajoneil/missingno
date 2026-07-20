use commit::Commit;
use dff::Dff;
use flags::{Flag, Flags};
use mcycle::{BusAction, CpuPhase, HaltPhase, MCycleAction, Phase, TCycle};
use registers::{Register8, Register16};

use crate::interrupts::Interrupt;

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

/// CPU execution state w.r.t. the HALT instruction and illegal-opcode
/// lockup. Halt-release fires combinationally via g43 → g49 once
/// `irq_latched` (yoii) captures `(IF & IE) != 0`; lockup has no
/// release path.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HaltState {
    /// Normal execution — CPU fetches and executes instructions.
    Running,
    /// HALT decoded — the next fetch runs as a dummy fetch (read
    /// [PC] without incrementing) before transitioning to `Halted`.
    Halting,
    /// HALT idle loop — ticking hardware, waiting for `(IF & IE) != 0`.
    Halted,
    /// STOP idle — ticking hardware, no interrupt-wake. Distinct from HALT:
    /// the CPU resumes only when the system re-engages it (CGB speed switch
    /// completing, or a joypad wake). Driven externally via `resume_from_stop`.
    Stopped,
    /// Illegal-opcode hard-lock — ticking hardware, never wakes.
    /// Reached only via `Commit::Invalid` (opcodes $D3, $DB, $DD,
    /// $E3, $E4, $EB, $EC, $ED, $F4, $FC, $FD).
    Locked,
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
    /// `cpu_irq_ack1` HIGH pulse — LALU.r_n driven LOW via lety/movu AND-tree.
    /// Rises with apply_vector_resolve at tcycle 3 of the dispatching M-cycle;
    /// falls at the next M-cycle boundary entry. While HIGH, same-M-cycle
    /// SUKO rising edges are absorbed (LALU.q forced to 0).
    pub(super) cpu_irq_ack1_pulse: bool,
    /// The serviced IF bit held in reset while `cpu_irq_ack1` is HIGH (the
    /// LALU.r_n target). The reset is held across the dispatch window, so a
    /// PC-push that writes FF0F cannot re-set this bit.
    pub(super) irq_ack_held: Option<Interrupt>,
    /// Combinational `(IF & IE) != 0`. Coarse signal kept for the
    /// morepork adapter; dispatch reads the data-phase-gated
    /// `dispatch.latched()` instead.
    pub(super) irq_pending: bool,
    /// `irq_latched` (yoii) DFF. CLK9-cadence capture of the data-
    /// phase-gated `dispatch.latched()`. Drives the HALT-release
    /// chain (yoii → ykua → ynkw).
    pub(super) irq_latched: Dff<bool>,
    /// CGB halt-release presample: the wake comparator state captured
    /// at the T2 rise, consumed by yoii's boundary capture while halted
    /// (the CGB samples IF&IE two T-cycles earlier than the DMG).
    pub(super) halt_wake_presample: bool,
}

impl IrqContext {
    pub fn new() -> Self {
        Self {
            ime: Dff::new(InterruptMasterEnable::Disabled),
            ime_delay: false,
            pending_vector_resolve: false,
            cpu_irq_ack1_pulse: false,
            irq_ack_held: None,
            irq_pending: false,
            irq_latched: Dff::new(false),
            halt_wake_presample: false,
        }
    }
}

impl Default for IrqContext {
    fn default() -> Self {
        Self::new()
    }
}

/// The dispatch's one-shot M1 arbitration against a standing VRAM-DMA request.
/// A request standing at the pick makes this dispatch yield its entire tenure
/// to the block (chained blocks included); otherwise the dispatch holds the bus
/// through its M-cycles against all later seizures. The pick's result is
/// meaningless until it is taken, so it exists only inside `Sampled` — a
/// "parks, but no pick was taken" state is unrepresentable.
pub(crate) enum DispatchPick {
    /// No pick has been taken for the dispatch in flight.
    Unsampled,
    /// The pick has been taken; `parks_behind_dma` is its result.
    Sampled { parks_behind_dma: bool },
}

/// The DMA↔dispatch bus-pick synchronizer: the CPU-side latches that decide,
/// each M-boundary, whether the in-flight instruction holds the shared bus or
/// yields it to a VRAM-DMA block. Read and advanced as a unit by the step
/// loop's boundary work; the ALU/sequencer never touch it.
pub(crate) struct BusArbitration {
    /// The dispatch's one-shot M1 pick.
    pick: DispatchPick,
    /// Two-boundary synchronizer on the VRAM-DMA request line: `[0]` is the
    /// request captured at the previous M-boundary, `[1]` the boundary before
    /// that. A commit landing within the pick's own M-cycle hasn't reached
    /// `[1]` (the dispatch wins); one standing a full boundary earlier has (the
    /// block wins). Entry detection runs one boundary after the pick, so the
    /// pick reads `[1]`.
    request_sync: [bool; 2],
    /// The M-cycle picked at the next boundary starts a fresh instruction — the
    /// retirement point a running-CPU block grant engages at. Handed over by
    /// `take_instruction_boundary`, read and cleared by the arbiter each
    /// M-boundary.
    at_boundary: bool,
    /// Bus access selected while a DMA owns its bus: it waits at the pick — no
    /// edge has run — and starts as the next M-cycle when the bus releases.
    pub(crate) parked_action: Option<MCycleAction>,
}

impl BusArbitration {
    fn new() -> Self {
        Self {
            pick: DispatchPick::Unsampled,
            request_sync: [false; 2],
            at_boundary: false,
            parked_action: None,
        }
    }

    /// Latch the dispatch's M1 pick on the first M-boundary of a dispatch (a
    /// request standing a full boundary before M1 makes it park); idempotent
    /// across the dispatch's later boundaries.
    pub(crate) fn sample_pick_if_entering(&mut self) {
        if matches!(self.pick, DispatchPick::Unsampled) {
            self.pick = DispatchPick::Sampled {
                parks_behind_dma: self.request_sync[1],
            };
        }
    }

    /// Drop the pick when no dispatch is in flight.
    pub(crate) fn clear_pick(&mut self) {
        self.pick = DispatchPick::Unsampled;
    }

    /// Whether the dispatch in flight yielded the bus to a standing block.
    pub(crate) fn parks_behind_dma(&self) -> bool {
        matches!(
            self.pick,
            DispatchPick::Sampled {
                parks_behind_dma: true
            }
        )
    }

    /// Shift the request-line synchronizer one boundary and load the current
    /// standing state into the first stage.
    pub(crate) fn shift_request(&mut self, standing: bool) {
        self.request_sync[1] = self.request_sync[0];
        self.request_sync[0] = standing;
    }

    /// Flag that the next M-boundary pick starts a fresh instruction.
    pub(crate) fn mark_at_boundary(&mut self) {
        self.at_boundary = true;
    }

    /// Consume the fresh-instruction flag.
    pub(crate) fn take_at_boundary(&mut self) -> bool {
        std::mem::take(&mut self.at_boundary)
    }
}

/// A next-opcode overlap prefetch's retention across a bus-master hold. When a
/// CGB GDMA seizes the bus mid-prefetch, the prefetch's read has already
/// latched the pre-transfer byte (the transfer suppresses the fetch, it does
/// not re-drive the read); retaining that byte lets the post-hold re-fetch
/// decode it rather than the transfer-clobbered open-bus re-read. Arm and
/// latched byte are one machine — a byte can't be both awaited and held.
enum OverlapPrefetch {
    /// No prefetch byte is being retained.
    Idle,
    /// A hold engaged during a fetch overlap; the prefetch's read latch will
    /// capture its byte into `Held` when this M-cycle's read commits.
    ArmedOverHold,
    /// The retained pre-transfer opcode, consumed by the post-hold fetch.
    Held(u8),
}

/// The SM83's bus interface: the M-cycle action currently driven, the
/// `BusAction` it resolves to, the byte captured off `cpu_port_d`, and the
/// overlap-prefetch retention held across a bus-master hold.
pub(crate) struct BusInterface {
    /// The `MCycleAction` for the current M-cycle (read/write/internal +
    /// address). `None` between M-cycles.
    current_action: Option<MCycleAction>,
    /// The `BusAction` produced by the most recent `next_tcycle`. The executor
    /// reads this between rise/fall edges of the same T-cycle to route memory.
    pub(crate) last_bus_action: BusAction,
    /// Bus data latch — the byte the SM83 captured off `cpu_port_d` near the
    /// end of T-cycle 3 of a read M-cycle. Holds until the next read latches.
    pub(crate) data_latch: u8,
    /// A next-opcode overlap prefetch retained across a bus-master hold; see
    /// [`OverlapPrefetch`]. `Held` only across such a hold; the fetch consumes it.
    overlap_prefetch: OverlapPrefetch,
}

impl BusInterface {
    fn new() -> Self {
        Self {
            current_action: None,
            last_bus_action: BusAction::Idle,
            data_latch: 0,
            overlap_prefetch: OverlapPrefetch::Idle,
        }
    }
}

/// The SM83 instruction sequencer: the microcode/timing state machine that
/// walks each instruction (or dispatch) through its M-cycles, plus the
/// internal working registers it fills along the way. Not programmer-visible —
/// the architectural register file sits directly on `Cpu`.
pub(crate) struct Sequencer {
    /// Current execution phase of the CPU state machine.
    pub(crate) phase: CpuPhase,
    /// The decoded instruction, preserved for debugger display.
    pub(crate) instruction: instructions::Instruction,
    /// T-cycle position within the current M-cycle (0–3).
    pub(crate) tcycle: TCycle,
    /// T-cycle that produced the last `BusAction` — the executor reads
    /// this to time per-T-cycle work after `next_tcycle()`.
    pub(crate) last_tcycle: TCycle,
    /// Step counter for Fetch / Halted phases (tracks M-cycle sub-steps).
    pub(crate) exec_step: u8,
    /// Whether an M-cycle is in flight.
    pub(crate) mcycle_active: bool,
    /// Whether the next rise() should fire the M-cycle-boundary block.
    /// Decoupled from `mcycle_active` so the skip-boot constructor can
    /// encode "M-cycle in flight, but the opening CLK9↑'s boundary work
    /// fired in the boot ROM's domain." Normally tracks `!mcycle_active`.
    pub(crate) boundary_pending: bool,
    /// Set when the CPU transitions to Fetch. The executor reads this
    /// to detect instruction boundaries.
    pub(crate) boundary_flag: bool,
    /// WZ temp register. Holds an assembled new-PC value (jump/call
    /// target, restart vector, or address popped from the stack) until
    /// the `PC ← WZ` copy at the instruction's retiring cycle. Every
    /// control-flow op routes its new PC through here so `pc` never holds
    /// the target before the install.
    pub(crate) wz: u16,
    /// The `PC ← WZ` misc-op: set when a control-flow op has assembled a
    /// new PC in `wz`, consumed (and cleared) by the copy at the retiring
    /// M-cycle. Distinct from `wz` itself, which always holds a value.
    pub(crate) wz_to_pc: bool,
    /// Scratch byte for multi-read phases (Pop, CondReturn).
    pub(crate) scratch: u8,
    /// The operand byte a yielded STOP discard-fetch latched: IR retains it
    /// through the stop spin; resume routes it as a just-fetched opcode.
    pub(crate) stop_retained: Option<u8>,
    /// The M-cycle in flight is the first fetch after a halt exit (the
    /// halt-release path drives it); cleared when it routes.
    pub(crate) post_halt_fetch: bool,
}

impl Sequencer {
    fn new() -> Self {
        Self {
            phase: CpuPhase::Fetch,
            instruction: instructions::Instruction::NoOperation,
            tcycle: TCycle::ZERO,
            last_tcycle: TCycle::ZERO,
            exec_step: 0,
            mcycle_active: false,
            boundary_pending: true,
            boundary_flag: true,
            wz: 0,
            wz_to_pc: false,
            scratch: 0,
            stop_retained: None,
            post_halt_fetch: false,
        }
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
    /// Program counter. Drives the address bus during fetch and operand
    /// reads and advances by 1 per fetched byte. New values for jumps,
    /// calls and returns are assembled in `wz` first, then copied here.
    pub pc: u16,

    /// Address the opcode currently in IR was fetched from — the
    /// instruction address. Captured on the opcode fetch and held for
    /// the whole instruction. `pc` advances through operand reads; this
    /// stays pinned, so it is the value disassembly, breakpoints, and
    /// the debugger key on.
    pub ir_address: u16,

    /// Address/data bus latches and the overlap-prefetch retention.
    pub(crate) bus: BusInterface,

    pub flags: Flags,

    pub irq: IrqContext,
    pub halt: HaltContext,

    /// The instruction sequencer: phase machine, T-cycle/M-cycle counters,
    /// boundary flags, and the WZ/scratch working registers.
    pub(crate) seq: Sequencer,
    /// The DMA↔dispatch bus-pick synchronizer that arbitrates the shared bus
    /// between an in-flight instruction and a pending VRAM-DMA block.
    pub(crate) bus_arbitration: BusArbitration,
    /// Running-CPU dispatch chain: per-bit irq_latch_inst<i> →
    /// priority chain → int_take → zaij → zkog/zloz → zfex → zacw.
    /// Owns the `data_phase_n` latch and the EI/DI block.
    pub dispatch: dispatch_chain::DispatchChain,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
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
        let flags = if checksum == 0 {
            Flags::ZERO
        } else {
            Flags::ZERO | Flags::CARRY | Flags::HALF_CARRY
        };
        Self::post_boot_with(0x01, 0x00, 0x13, 0x00, 0xd8, 0x01, 0x4d, flags)
    }

    /// Post-boot register file with an explicit register set — the handoff
    /// state a boot ROM leaves, parameterised so each console supplies its own
    /// values (the CGB installs its fixed CPU-CGB-C file this way).
    #[allow(clippy::too_many_arguments)]
    pub fn post_boot_with(a: u8, b: u8, c: u8, d: u8, e: u8, h: u8, l: u8, flags: Flags) -> Cpu {
        Cpu {
            a,
            b,
            c,
            d,
            e,
            h,
            l,

            stack_pointer: 0xfffe,
            pc: 0x0100,
            ir_address: 0x0100,

            flags,

            // ── In-flight M-cycle: opening m1 fetch of cartridge code ──
            seq: Sequencer {
                phase: CpuPhase::Execute {
                    phase: Phase::FetchOverlap {
                        commit: Commit::NoOperation,
                    },
                    step: 1,
                },
                tcycle: TCycle::ONE,
                mcycle_active: true,
                boundary_pending: false,
                exec_step: 1,
                ..Sequencer::new()
            },
            bus: BusInterface {
                current_action: Some(MCycleAction::Read { address: 0x0100 }),
                ..BusInterface::new()
            },

            ..Self::boundary_state()
        }
    }

    /// Construct a CPU from a save-state snapshot at an instruction
    /// boundary. The state machine fields are reset to their boundary
    /// defaults (Fetch phase, step 0, no pending actions).
    pub fn from_snapshot(snap: &crate::snapshot::CpuSnapshot) -> Cpu {
        Cpu {
            a: snap.a,
            b: snap.b,
            c: snap.c,
            d: snap.d,
            e: snap.e,
            h: snap.h,
            l: snap.l,
            stack_pointer: snap.sp,
            pc: snap.pc,
            ir_address: snap.pc,
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
            pc: 0,
            ir_address: 0,
            bus: BusInterface::new(),
            flags: Flags::empty(),
            irq: IrqContext::new(),
            halt: HaltContext::new(),
            seq: Sequencer::new(),
            bus_arbitration: BusArbitration::new(),
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
        match &self.seq.phase {
            mcycle::CpuPhase::Fetch => 0,
            mcycle::CpuPhase::Execute { step, .. } => *step,
            mcycle::CpuPhase::InterruptDispatch { step, .. } => *step,
            mcycle::CpuPhase::Halted(_) => 0,
            mcycle::CpuPhase::Locked => 0,
        }
    }

    /// AFUR/ALEF/APUK/ADYK ring-counter state, packed
    /// `AFUR<<3 | ALEF<<2 | APUK<<1 | ADYK<<0` to match GateBoy's
    /// encoding at the post-fall sampling instant.
    pub fn mcycle_phase(&self) -> u8 {
        match self.seq.last_tcycle.as_u8() {
            0 => 0x0E, // AFUR=1 ALEF=1 APUK=1 ADYK=0
            1 => 0x07, // AFUR=0 ALEF=1 APUK=1 ADYK=1
            2 => 0x01, // AFUR=0 ALEF=0 APUK=0 ADYK=1
            3 => 0x08, // AFUR=1 ALEF=0 APUK=0 ADYK=0
            _ => unreachable!(),
        }
    }

    /// Address on the CPU bus for the current M-cycle.
    pub fn bus_address(&self) -> u16 {
        match &self.bus.current_action {
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
    pub fn consume_boundary_pending(&mut self) -> bool {
        let pending = self.seq.boundary_pending;
        self.seq.boundary_pending = false;
        pending
    }

    /// IE-push-bug flag (morepork extension). Set during dispatch's M3
    /// vector-resolve window.
    pub fn pending_vector_resolve_flag(&self) -> bool {
        self.irq.pending_vector_resolve
    }

    /// HALT-bug flag (morepork extension). See `HaltContext::bug`.
    pub fn halt_bug_flag(&self) -> bool {
        self.halt.bug
    }

    pub fn is_halted(&self) -> bool {
        self.halt.state == HaltState::Halted
    }

    /// Whether the CPU is idling in STOP (awaiting an external re-engage).
    pub fn is_stopped(&self) -> bool {
        self.halt.state == HaltState::Stopped
    }

    /// The CPU is inside its interrupt-dispatch sequence — one indivisible
    /// bus tenure that a DMA grant waits behind.
    pub(crate) fn in_dispatch(&self) -> bool {
        matches!(self.seq.phase, mcycle::CpuPhase::InterruptDispatch { .. })
    }

    /// Re-engage the CPU after STOP: resume at the instruction following
    /// STOP. Called by the system when a CGB speed switch (or a joypad wake)
    /// completes. The blackout count expires on an arbitrary master edge, so
    /// start a fresh M-cycle here (`mcycle_active=false`) and arm its boundary
    /// work (`boundary_pending`); the fetch then runs on the next CPU rise,
    /// offset from the dot grid by however many master edges the blackout held.
    pub fn resume_from_stop(&mut self, dispatch_step: Option<u8>) {
        self.halt.state = HaltState::Running;
        self.halt.rs_latched = false;

        // An interrupt serviced at re-engage dispatches from the post-STOP
        // boundary, exactly like an ordinary HALT wake: the byte after STOP is
        // the pushed return target, not run before the handler. The 1-byte STOP
        // (pending at the STOP) resumes at M2 — its dispatch M1 was the operand
        // fetch spent before the DIV reset. The 2-byte STOP (a timer waking the
        // HALT) resumes at M1, after the HALT-wake's WakeIntake cycle.
        if let Some(step) = dispatch_step {
            let pc = self.pc;
            self.irq
                .ime
                .write_immediate(InterruptMasterEnable::Disabled);
            self.irq.ime_delay = false;
            self.seq.phase = CpuPhase::InterruptDispatch {
                sp: self.stack_pointer,
                pc_hi: (pc >> 8) as u8,
                pc_lo: (pc & 0xff) as u8,
                step,
            };
            self.irq.pending_vector_resolve = false;
        } else {
            self.seq.phase = match self.seq.stop_retained.take() {
                // A yielded discard-fetch left its byte in IR: route it as a
                // just-fetched opcode instead of re-fetching.
                Some(opcode) => CpuPhase::Execute {
                    phase: Phase::RetainedOpcode { opcode },
                    step: 0,
                },
                None => CpuPhase::Fetch,
            };
        }
        self.seq.exec_step = 0;
        self.seq.mcycle_active = false;
        self.seq.boundary_pending = true;
        self.seq.boundary_flag = true;
    }

    /// Mark an instruction boundary so the step driver returns. Used by the
    /// held speed-switch blackout, which advances the master clock without
    /// stepping the SM83's own M-cycle state machine.
    pub fn mark_instruction_boundary(&mut self) {
        self.seq.boundary_flag = true;
    }

    /// Hold every CPU cycle until `end_bus_hold`: a bus master (CGB GDMA)
    /// consumes the full bus bandwidth while the peripherals keep running.
    /// Call at an instruction boundary; halt state is untouched — the spin
    /// is the scheduler's.
    pub fn begin_bus_hold(&mut self) {
        // The next-opcode overlap prefetch is in flight; its read latches this
        // M-cycle (with the pre-transfer byte). Arm the retention so the
        // post-hold fetch decodes it rather than re-reading.
        if matches!(
            self.seq.phase,
            CpuPhase::Execute {
                phase: Phase::FetchOverlap { .. },
                ..
            }
        ) {
            self.bus.overlap_prefetch = OverlapPrefetch::ArmedOverHold;
        } else if !matches!(self.bus.overlap_prefetch, OverlapPrefetch::Held(_)) {
            self.bus.overlap_prefetch = OverlapPrefetch::Idle;
        }
    }

    /// Latch the pre-transfer byte into the retained overlap prefetch if a hold
    /// armed it; a no-op otherwise.
    pub(crate) fn latch_overlap_prefetch(&mut self, value: u8) {
        if matches!(self.bus.overlap_prefetch, OverlapPrefetch::ArmedOverHold) {
            self.bus.overlap_prefetch = OverlapPrefetch::Held(value);
        }
    }

    /// Take a retained overlap-prefetch byte if the post-hold fetch has one.
    fn take_overlap_prefetch(&mut self) -> Option<u8> {
        if let OverlapPrefetch::Held(byte) = self.bus.overlap_prefetch {
            self.bus.overlap_prefetch = OverlapPrefetch::Idle;
            Some(byte)
        } else {
            None
        }
    }

    /// Release the hold. The prefetch in flight when the hold engaged was
    /// cancelled by the bus master taking the cycle; it re-issues from PC.
    /// A stop spin held through the hold stays a stop spin.
    pub fn end_bus_hold(&mut self) {
        self.seq.phase = if self.halt.state == HaltState::Stopped {
            CpuPhase::Halted(HaltPhase::Spin)
        } else {
            CpuPhase::Fetch
        };
        self.seq.exec_step = 0;
        self.seq.boundary_flag = true;
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
            self.seq.phase,
            CpuPhase::Fetch
                | CpuPhase::Execute {
                    phase: Phase::FetchOverlap { .. },
                    ..
                }
        )
    }

    /// Address + value the CPU is writing this M-cycle.
    pub fn pending_bus_write(&self) -> Option<(u16, u8)> {
        match &self.bus.current_action {
            Some(mcycle::MCycleAction::Write { address, value }) => Some((*address, *value)),
            _ => None,
        }
    }

    /// Address the CPU is reading this M-cycle.
    pub fn pending_bus_read(&self) -> Option<u16> {
        match &self.bus.current_action {
            Some(mcycle::MCycleAction::Read { address }) => Some(*address),
            _ => None,
        }
    }
}
