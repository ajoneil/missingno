//! The CGB VRAM DMA ($FF51-55): the byte engine, the per-HBlank block
//! arbitration, and the bus tenure it takes from the CPU and the OAM DMA.

use missingno_gb::ppu::rendering::Mode;
use missingno_gb::{Chassis, ConsoleShadow, VramDmaClaim};

use crate::Cgb;
use crate::bus;

/// How the active VRAM DMA is paced. GDMA holds the CPU and flows continuously;
/// HDMA copies one 16-byte block per HBlank, releasing the CPU between blocks.
#[derive(Default, PartialEq)]
pub(crate) enum TransferMode {
    #[default]
    Idle,
    General,
    HBlank,
}

/// A one-shot down-counter: armed to a tick count, stepped toward zero, then
/// inert. Models the VRAM-DMA's scattered `if n > 0 { n -= 1 }` timers as one
/// shape. Counters that step *up* (`halted_falls`, `seize_falls`, `pend_age`)
/// or act as a running semaphore (`granted_ahead`) keep their own arithmetic —
/// they are not this idiom.
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) struct Countdown(u8);

impl Countdown {
    pub(crate) fn arm(&mut self, ticks: u8) {
        self.0 = ticks;
    }
    pub(crate) fn clear(&mut self) {
        self.0 = 0;
    }
    pub(crate) fn active(&self) -> bool {
        self.0 > 0
    }
    pub(crate) fn remaining(&self) -> u8 {
        self.0
    }
    /// Step one tick while active; returns whether a tick was consumed (it was
    /// active before this step).
    pub(crate) fn tick(&mut self) -> bool {
        let active = self.0 > 0;
        self.0 = self.0.saturating_sub(1);
        active
    }
    /// Step one tick; returns whether this step drained it to zero.
    pub(crate) fn expired(&mut self) -> bool {
        self.0 > 0 && {
            self.0 -= 1;
            self.0 == 0
        }
    }
}

/// CGB VRAM DMA ($FF51-55) controller. The source and destination pointers run
/// as bytes are copied and persist after a transfer, so a follow-on transfer
/// continues where the last left off. The step loop ticks it each M-cycle: a
/// transfer flows `quota` bytes per M-cycle while it holds the CPU.
/// The HBlank-block pend/grant arbiter: when a mode-0 entry (or an FF55 arm)
/// requests a block, whether it commits now or defers across a HALT/wake, and
/// how the FF55 status count tracks blocks granted ahead of their drain.
///
/// `pend_from_arm` and `pend_granted` are independent modifiers on the pending
/// block, not mutually-exclusive origins: the halt-latch path grants a pend
/// (`pend_granted`) whose block was launched by an arm strobe (`pend_from_arm`),
/// so both hold at once. `pend_granted` also outlives `pend` (the grant survives
/// a fall where the pend recomputes to false under an IRQ-latched halt). An
/// origin enum can't represent either, so the flags stay independent.
#[derive(Default)]
pub(crate) struct HaltArbiter {
    /// Trigger pend stage: the previous fall's view showed armed ∧ mode 0;
    /// commits to a cancel-immune block one fall later.
    pub(crate) pend: bool,
    /// The pend formed on the fall of the FF55 arm commit itself — the arm
    /// strobe pre-loads the engine's working pointers, so no setup cell.
    pub(crate) pend_from_arm: bool,
    /// FF55 armed on this fall (set by the write path, consumed by the tick).
    pub(crate) armed_this_fall: bool,
    /// Falls since the halt gate rose: the taken-clear path runs one
    /// boundary-clocked synchronizer stage behind the gate, so a clear in
    /// flight at the halt latch (within its M-cycle, 4 falls) still lands;
    /// later clears wait for the resume.
    pub(crate) halted_falls: u8,
    /// The previous fall's mode view showed mode 0 — entry-edge detection for
    /// the IF-rise-to-resume window (only an entry pends there).
    pub(crate) prev_view_hblank: bool,
    /// Falls since the registered request entered the trigger's two-stage
    /// pipe: a token still inside (≤2 falls) at the IF-rise thaw commits
    /// there; an older token relaunches through the pipe — the one-fall
    /// penalty that decides the grant-vs-dispatch tie.
    pub(crate) pend_age: u8,
    /// The in-halt grant latched this pend: it survives the engine thaw and
    /// commits there (the wake drains it), regardless of the live mode.
    pub(crate) pend_granted: bool,
    /// Running ticks left of the wake drain's bus tenure — an HBlank entry
    /// inside it passes unserviced.
    pub(crate) wake_tenure: Countdown,
    /// This block committed onto a running CPU: its bus grant waits for the
    /// in-flight instruction to retire. A halted-CPU commit (including the
    /// same-fall wake flip) grants at the next M-boundary.
    pub(crate) park_waits_for_fetch: bool,
    /// A wake-tenure-consumed entry's standing claim: the engine holds the
    /// VRAM select without moving bytes until the owed block really services.
    /// CPU VRAM reads during the hold capture the undriven bus (0x00).
    pub(crate) idle_claim: bool,
    /// Running ticks left of the halt-wake entry blind: a mode-0 entry edge
    /// on the wake fall or just after it passes unserviced (no retry). Unlike
    /// the STOP-armed blind there is no first-tick exemption.
    pub(crate) halt_wake_blind: Countdown,
    /// `cpu_halted` at the previous running tick — the wake-flip detector.
    pub(crate) prev_cpu_halted: bool,
    /// The commit already counted this pending block's grant (the in-halt
    /// grant path ran); the commit-time count skips it.
    pub(crate) grant_counted: bool,
    /// Post-switch re-engage window (running vram_dma ticks): a fresh mode-0
    /// entry edge inside it does not pend — the HBlank passes unserviced. The
    /// first tick is exempt (a blackout-carried mode-0 level, not an entry).
    pub(crate) wake_pend_blind: Countdown,
    /// HBlank blocks granted in-halt but not yet drained. A halted CPU does not
    /// contend the bus, so an in-halt mode-0 grants a block (status complete)
    /// while its bus-seizure transfer stays on the post-resume path; this offsets
    /// the FF55 block count until the post-resume drain catches up.
    pub(crate) granted_ahead: u8,
}

/// The running byte engine: the source/dest pointers, the transfer mode, and
/// the whole-transfer/per-M-cycle byte counts. Pointers persist after a
/// transfer so a follow-on continues where the last left off.
#[derive(Default)]
pub(crate) struct TransferCursor {
    /// Running source pointer, 16-byte aligned (HDMA1/HDMA2).
    pub(crate) source: u16,
    /// Running destination, a raw 16-bit HDMA3/HDMA4 pointer. The write address
    /// folds to VRAM via `write_address`; the transfer ends when it carries past
    /// $FFFF rather than wrapping back into VRAM.
    pub(crate) dest: u16,
    pub(crate) mode: TransferMode,
    /// Bytes left in the whole transfer.
    pub(crate) remaining: u16,
    /// Bytes still movable this M-cycle (refilled per tick: 2 single, 1 double).
    pub(crate) quota: u8,
    /// A speed-switch cancel caught the engine mid-byte: that byte completes
    /// (pointers advance) without counting against the latched length.
    pub(crate) escape_byte: bool,
}

impl TransferCursor {
    /// VRAM address the next byte lands on; the dest register is 16-bit but VRAM
    /// decodes only the low 13 bits.
    fn write_address(&self) -> u16 {
        0x8000 | (self.dest & 0x1FFF)
    }
}

/// The per-HBlank 16-byte block sequencer and its bus tenure: how many bytes of
/// the current block remain, when it fired, its setup/readiness timing, and the
/// arm-strobe launch bookkeeping.
#[derive(Default)]
pub(crate) struct HblankBlock {
    /// Bytes left in the current HBlank block (HBlank mode). The CPU is held
    /// while this is >0.
    pub(crate) remaining: u8,
    /// Master edge at which the current HBlank block fired — its byte clock's
    /// phase origin, aligned against a concurrent OAM-DMA's bus.
    pub(crate) start_edge: u64,
    /// This HBlank's block has been latched — one block per mode-0 period.
    pub(crate) taken: bool,
    /// Leading no-data cells of the block: the engine loads its working
    /// pointers from the HDMA1-4 holding registers on a PPU-triggered block.
    pub(crate) setup_cells: Countdown,
    /// Dots until a committed block claims the bus (the transfer readies two
    /// dots after the commit).
    pub(crate) ready_in: Countdown,
    /// Falls the bus has been seized continuously. A CPU read of the block's
    /// destination sees the written byte only once the seize has settled one
    /// fall (the double-speed half-dot from bus seizure to byte-readable).
    pub(crate) seize_falls: u8,
    /// The active block launched from an FF55 arm strobe. Its readiness must
    /// complete inside mode 0 — a launch whose ready pipe crosses the mode-0
    /// exit reverts and waits for the next entry.
    pub(crate) from_arm: bool,
    /// The arm-strobe readiness latch awaits its mode-0 confirmation sample:
    /// the fall after expiry must still be mode 0, else the launch reverts.
    pub(crate) arm_ready_probation: bool,
}

/// The OAM-DMA ↔ VRAM-DMA byte-clock co-arbitration: the two engines share one
/// bus, so a coinciding byte is edge-detected and steered to OAM.
#[derive(Default)]
pub(crate) struct OamContention {
    /// Whether the OAM-DMA drove a byte last M-cycle — edge-detects its
    /// active→done boundary so the completion M-cycle still shares the bus with
    /// a concurrent VRAM-DMA.
    pub(crate) was_transferring: bool,
    /// The pre-OAM arbitration found an OAM-DMA↔VRAM-DMA byte contention this
    /// M-cycle: the coinciding VRAM-DMA byte lands at OAM. Carried from the
    /// pre-OAM arbitration to the post-OAM byte engine.
    pub(crate) contended: bool,
}

#[derive(Default)]
pub(crate) struct VramDma {
    /// The running byte engine (pointers, mode, counts).
    pub(crate) cursor: TransferCursor,
    /// The per-HBlank block sequencer and its bus tenure.
    pub(crate) block: HblankBlock,
    /// HBlank-block pend/grant arbitration across HALT/wake.
    pub(crate) arb: HaltArbiter,
    /// OAM-DMA ↔ VRAM-DMA byte-clock co-arbitration.
    pub(crate) oam: OamContention,
}

/// A read-only view of the VRAM-DMA engine for the debugger sidebar. GDMA holds
/// the CPU for the whole transfer; HDMA copies one 16-byte block per HBlank.
#[derive(Clone, Copy)]
pub enum VramDmaStatus {
    Idle,
    General {
        remaining: u16,
    },
    HBlank {
        remaining: u16,
        source: u16,
        dest: u16,
    },
}

impl VramDma {
    /// A snapshot of the running transfer for inspection. `remaining` is the
    /// whole-transfer byte count; `dest` folds to the VRAM write address.
    pub(crate) fn status(&self) -> VramDmaStatus {
        match self.cursor.mode {
            TransferMode::Idle => VramDmaStatus::Idle,
            TransferMode::General => VramDmaStatus::General {
                remaining: self.cursor.remaining,
            },
            TransferMode::HBlank => VramDmaStatus::HBlank {
                remaining: self.cursor.remaining,
                source: self.cursor.source,
                dest: self.cursor.write_address(),
            },
        }
    }

    pub(crate) const WAKE_PEND_BLIND_TICKS: u8 = 6;
    /// Halt-wake blind width — bracketed by the m0halt pair (`_2`'s entry on
    /// the flip fall blinded; `_1`'s at ≈flip+4 must run).
    pub(crate) const HALT_WAKE_BLIND_TICKS: u8 = 3;
    /// The wake drain's bus tenure in running ticks (per-fall): 72 master
    /// edges — the A-pair variants' entries at thaw+66/+74 bracket it.
    pub(crate) const WAKE_TENURE_TICKS: u8 = 36;

    /// Whether the engine has nothing to arbitrate: no transfer bytes, no block
    /// or its setup/readiness cells, no pend or grant, no halt view to unwind,
    /// and no timer cell counting. Every conditional in the fall edge is then
    /// unreachable — including the ones whose `Countdown` step is the condition —
    /// so the edge reduces to `settle_inert`. The caller adds the CPU's own
    /// gate, which the engine does not hold.
    pub(crate) fn inert(&self) -> bool {
        self.cursor.remaining == 0
            && self.block.remaining == 0
            && !self.block.setup_cells.active()
            && !self.block.ready_in.active()
            && !self.block.arm_ready_probation
            && !self.arb.pend
            && !self.arb.pend_granted
            && !self.arb.prev_cpu_halted
            && !self.arb.armed_this_fall
            && !self.arb.halt_wake_blind.active()
            && !self.arb.wake_pend_blind.active()
            && !self.arb.wake_tenure.active()
    }

    /// The whole of the fall edge under `inert`: the mode-0 view the entry
    /// detector compares against, the halt counter, the taken-clear outside
    /// mode 0, and the two zeroed budgets.
    pub(crate) fn settle_inert(&mut self, in_hblank: bool) {
        self.arb.prev_view_hblank = in_hblank;
        self.arb.halted_falls = 0;
        if !in_hblank {
            self.block.taken = false;
        }
        self.cursor.quota = 0;
        self.block.seize_falls = 0;
    }

    /// Whether a byte may move this M-cycle: a GDMA runs while bytes remain; a
    /// latched HBlank block runs to completion regardless of the live `mode`
    /// (the block sequencer, once started, does not consult the arming flag).
    pub(crate) fn moving(&self) -> bool {
        (self.block.remaining > 0 && self.cursor.remaining > 0 && !self.block.ready_in.active())
            || (self.cursor.mode == TransferMode::General && self.cursor.remaining > 0)
    }

    /// VRAM address the next byte lands on.
    pub(crate) fn write_address(&self) -> u16 {
        self.cursor.write_address()
    }

    /// Whether the VRAM DMA will move at least one byte this M-cycle (block
    /// active, quota available, no setup cell pending) — for the OAM-DMA
    /// bus-contention check.
    pub(crate) fn will_move(&self) -> bool {
        !self.block.setup_cells.active() && self.cursor.quota > 0 && self.moving()
    }

    /// The byte the VRAM DMA is about to move is a switch-cancel escape byte;
    /// its bus tenure stalls a concurrent OAM-DMA byte at double speed.
    pub(crate) fn escape_pending(&self) -> bool {
        self.cursor.escape_byte && self.will_move()
    }

    /// An entry-triggered block spends one leading no-data cell — the engine
    /// loading its working pointers from the HDMA1-4 holding registers (the FF55
    /// arm strobe performs that load itself). Consumed once per block.
    pub(crate) fn take_setup_cell(&mut self) -> bool {
        self.block.setup_cells.tick()
    }

    /// The next byte the VRAM DMA moves this M-cycle — `(source, destination)`
    /// resolved addresses — advancing its cursor. `None` once this M-cycle's
    /// quota is spent.
    pub(crate) fn next_byte(&mut self) -> Option<(u16, u16)> {
        if self.cursor.quota == 0 || !self.moving() {
            return None;
        }
        let pair = (self.cursor.source, self.write_address());
        // Pointers advance per byte and persist for any follow-on transfer. A
        // switch-cancel escape byte does not count against the latched length.
        self.cursor.source = self.cursor.source.wrapping_add(1);
        let (next_dest, carried) = self.cursor.dest.overflowing_add(1);
        self.cursor.dest = next_dest;
        if self.cursor.escape_byte {
            self.cursor.escape_byte = false;
        } else {
            self.cursor.remaining -= 1;
        }
        self.cursor.quota -= 1;
        if self.block.remaining > 0 {
            self.block.remaining -= 1;
            // A block granted ahead in-halt rejoins the FF55 count as its bytes
            // finally drain on the post-resume path.
            if self.block.remaining == 0 {
                if self.arb.granted_ahead > 0 {
                    self.arb.granted_ahead -= 1;
                }
                self.arb.park_waits_for_fetch = false;
                self.block.from_arm = false;
            }
        }
        if carried {
            // The 16-bit dest register carried out of $FFFF — the transfer ends
            // here rather than wrapping back into VRAM.
            self.cursor.remaining = 0;
        }
        if self.cursor.remaining == 0 {
            self.cursor.mode = TransferMode::Idle;
            self.arb.idle_claim = false;
        }
        Some(pair)
    }

    /// The switch-cancel escape byte, moved outside the latched length.
    pub(crate) fn drain_escape(&mut self) -> Option<(u16, u16)> {
        if self.cursor.escape_byte && self.moving() {
            self.cursor.quota = 1;
            self.next_byte()
        } else {
            None
        }
    }

    pub(crate) fn park_waits_for_fetch(&self) -> bool {
        self.arb.park_waits_for_fetch
    }

    pub(crate) fn instruction_retired(&mut self) {
        self.arb.park_waits_for_fetch = false;
    }

    pub(crate) fn request_standing(&self) -> bool {
        self.arb.pend || (self.block.remaining > 0 && self.cursor.remaining > 0)
    }

    pub(crate) fn holds_cpu(&self) -> bool {
        self.cursor.mode == TransferMode::General && self.cursor.remaining > 0
    }

    pub(crate) fn seizes_bus(&self) -> bool {
        !self.block.ready_in.active()
            && (self.block.setup_cells.active()
                || (self.block.remaining > 0 && self.cursor.remaining > 0))
    }
}

/// Open-bus value a VRAM-DMA source read returns, or None for a normal read.
/// A VRAM-DMA source must be ROM/cart-RAM; VRAM ($8000-$9FFF) is off that
/// source bus and floats to `$FF`.
fn source_open_bus(source: u16) -> Option<u8> {
    (0x8000..=0x9FFF).contains(&source).then_some(0xFF)
}

impl Cgb {
    pub(crate) fn vram_dma_fall_edge(&mut self, chassis: &mut Chassis<Self>, mode: Mode) {
        let in_hblank = mode == Mode::HorizontalBlank;
        let cpu_halted = chassis.cpu.is_halted();
        if self.vram_dma.inert() && !cpu_halted && !chassis.cpu.is_stopped() {
            self.vram_dma.settle_inert(in_hblank);
            return;
        }
        // The engine thaws at the IF rise, ahead of the CPU's halt-exit latency
        // (a wake-coincident block is decided before the first fetch and the
        // dispatch pick); the taken-clear waits for the CPU's own resume.
        let engine_gated = (cpu_halted && !chassis.cpu.irq_latched()) || chassis.cpu.is_stopped();
        let master_edge = chassis.clock.master_edge();
        let entry_edge = in_hblank && !self.vram_dma.arb.prev_view_hblank;
        self.vram_dma.arb.prev_view_hblank = in_hblank;
        if cpu_halted {
            self.vram_dma.arb.halted_falls = self.vram_dma.arb.halted_falls.saturating_add(1);
        } else {
            self.vram_dma.arb.halted_falls = 0;
        }
        // The taken-clear stays live through the halt-latch M-cycle, then
        // freezes until the CPU's own resume (halt only; STOP freezes it
        // outright via the engine gate). One M-cycle is 4 PPU dots single
        // speed, 2 in double speed.
        let taken_clear_window = if self.double_speed { 2 } else { 4 };
        if !in_hblank && (!cpu_halted || self.vram_dma.arb.halted_falls <= taken_clear_window) {
            self.vram_dma.block.taken = false;
        }
        // A halt wake blinds entry edges on the wake fall and just after it
        // (the halt analogue of the STOP re-engage window, without the
        // first-tick exemption — the wake fall's own entry is blinded too).
        // The halt line is sampled every tick, gated or not — a gated halt
        // (no latched IRQ) still arms the blind at its wake; the countdown
        // runs on engine ticks only.
        if self.vram_dma.arb.prev_cpu_halted && !cpu_halted {
            self.vram_dma
                .arb
                .halt_wake_blind
                .arm(VramDma::HALT_WAKE_BLIND_TICKS);
        }
        // An arm-strobe launch must ready with a fall of mode-0 margin: the
        // readiness latch's confirmation sample on the following fall reads
        // the mode-0 level, and a launch confirmed outside mode 0 reverts
        // (FF55 count restored) to wait for the next entry.
        if self.vram_dma.block.arm_ready_probation {
            self.vram_dma.block.arm_ready_probation = false;
            if !in_hblank && self.vram_dma.block.from_arm && self.vram_dma.block.remaining == 16 {
                self.vram_dma.block.remaining = 0;
                self.vram_dma.block.from_arm = false;
                if self.vram_dma.arb.granted_ahead > 0 {
                    self.vram_dma.arb.granted_ahead -= 1;
                }
            }
        }
        // A block whose readiness the HALT latch lands inside joins the halt:
        // its bytes defer to the wake as a standing granted claim (FF55
        // already counted it), where the halt-release handover applies. A
        // latch after readiness leaves the block to drain in-halt.
        if cpu_halted
            && !self.vram_dma.arb.prev_cpu_halted
            && self.vram_dma.block.remaining == 16
            && self.vram_dma.block.ready_in.active()
        {
            self.vram_dma.block.remaining = 0;
            self.vram_dma.block.ready_in.clear();
            self.vram_dma.block.setup_cells.clear();
            self.vram_dma.arb.pend = true;
            self.vram_dma.arb.pend_granted = true;
            self.vram_dma.arb.grant_counted = true;
            self.vram_dma.arb.pend_age = 0;
        }
        // The halted view entering this fall — a wake flipping on the commit
        // fall itself still counts as a halted-CPU commit for the grant mode.
        let halted_entering = cpu_halted || self.vram_dma.arb.prev_cpu_halted;
        self.vram_dma.arb.prev_cpu_halted = cpu_halted;
        // The engine gate freezes commit/grant; the mode-0 entry detector
        // keeps running — an entry registers a pend-request (consulting the
        // taken flag) that persists through the freeze and commits at the
        // thaw. A latched block keeps draining.
        if engine_gated {
            if self.vram_dma.arb.pend {
                self.vram_dma.arb.pend_age = self.vram_dma.arb.pend_age.saturating_add(1);
            }
            if cpu_halted
                && entry_edge
                && self.vram_dma.cursor.mode == TransferMode::HBlank
                && !self.vram_dma.block.taken
                && self.vram_dma.cursor.remaining > 0
            {
                self.vram_dma.arb.pend = true;
                self.vram_dma.arb.pend_from_arm = false;
                self.vram_dma.arb.pend_age = 0;
            }
            // An in-halt mode-0 within the halt-latch window grants the block's
            // FF55 status (the halted CPU does not contend); the seizure transfer
            // still waits for the resume.
            if cpu_halted
                && entry_edge
                && self.vram_dma.cursor.mode == TransferMode::HBlank
                && self.vram_dma.arb.halted_falls <= 2
                && self.vram_dma.cursor.remaining / 16 > self.vram_dma.arb.granted_ahead as u16
            {
                self.vram_dma.arb.granted_ahead += 1;
                self.vram_dma.arb.grant_counted = true;
                self.vram_dma.arb.pend_granted = true;
            }
            self.vram_dma.cursor.quota = if self.vram_dma.moving() {
                if self.double_speed { 1 } else { 2 }
            } else {
                0
            };
            return;
        }

        if self.vram_dma.arb.halt_wake_blind.tick() && entry_edge {
            self.vram_dma.block.taken = true;
        }

        // The post-switch re-engage window: a fresh mode-0 entry edge inside
        // it passes unserviced (no pend, no retry). The first running tick
        // after the blackout carries the frozen pre-switch view — a mode-0
        // level there is blackout-carried, not an entry, and stays eligible.
        let wake_pend_first_tick =
            self.vram_dma.arb.wake_pend_blind.remaining() == VramDma::WAKE_PEND_BLIND_TICKS;
        if self.vram_dma.arb.wake_pend_blind.tick() && entry_edge && !wake_pend_first_tick {
            self.vram_dma.block.taken = true;
        }

        // Two-stage trigger, evaluated each fall on the post-rise mode view
        // with this fall's FF55 commit visible: a pend commits to a
        // cancel-immune block one fall later; an FF55 write at either fall
        // kills the pend (armed is consulted at both stages).
        let armed = self.vram_dma.cursor.mode == TransferMode::HBlank;
        let committing = self.vram_dma.arb.pend
            && armed
            // An arm-strobe pend latched while in HBlank commits even if HBlank
            // ended in the one-fall pend->commit gap; an in-halt-granted pend
            // commits at the thaw whatever the live mode.
            && (in_hblank || self.vram_dma.arb.pend_from_arm || self.vram_dma.arb.pend_granted)
            // A granted pend commits at the engine thaw even while the CPU is
            // still halted (an interrupt-dispatch wake) — the grant is the halt's.
            && (!cpu_halted || self.vram_dma.arb.pend_age <= 2 || self.vram_dma.arb.pend_granted);
        if committing {
            self.vram_dma.block.remaining = 16;
            self.vram_dma.block.start_edge = master_edge;
            self.vram_dma.block.taken = true;
            self.vram_dma.arb.idle_claim = false;
            self.vram_dma.block.from_arm = self.vram_dma.arb.pend_from_arm;
            // A granted-DRIVEN commit (only reachable through the grant: the
            // thaw lies outside HBlank) pre-charged its setup during the halt;
            // a granted pend that commits inside a live HBlank is an ordinary
            // commit and charges setup normally.
            let granted = self.vram_dma.arb.pend_granted && !in_hblank;
            // A dispatch wake (the CPU still halted at the thaw) spends the
            // halt-exit window dispatching, so its setup runs after instead of
            // arriving pre-charged.
            let precharged = granted && !cpu_halted;
            self.vram_dma.arb.park_waits_for_fetch =
                !(self.vram_dma.arb.pend_from_arm || halted_entering);
            self.vram_dma
                .block
                .ready_in
                .arm(if precharged { 0 } else { 2 });
            self.vram_dma
                .block
                .setup_cells
                .arm(if self.vram_dma.arb.pend_from_arm || precharged {
                    0
                } else {
                    1
                });
            // FF55 counts the block out at commit, not at drain end.
            if !self.vram_dma.arb.grant_counted {
                self.vram_dma.arb.granted_ahead += 1;
            }
            self.vram_dma.arb.grant_counted = false;
            if granted {
                self.vram_dma
                    .arb
                    .wake_tenure
                    .arm(VramDma::WAKE_TENURE_TICKS);
            }
            self.vram_dma.arb.pend_granted = false;
        }
        // A granted pend whose thaw tick passes without committing reverts to
        // an ordinary pend (its next-HBlank commit charges setup normally).
        if !committing && !cpu_halted {
            self.vram_dma.arb.pend_granted = false;
        }
        // The wake drain's bus tenure consumes an HBlank entry landing inside
        // it — the entry neither pends nor retries, but its claim stands: the
        // engine holds the VRAM select, undriven, until the owed block is
        // really serviced.
        if self.vram_dma.arb.wake_tenure.tick() && entry_edge {
            self.vram_dma.block.taken = true;
            if self.vram_dma.cursor.remaining > 0 {
                self.vram_dma.arb.idle_claim = true;
            }
        }
        self.vram_dma.arb.pend = !committing
            && armed
            && in_hblank
            && !self.vram_dma.block.taken
            && self.vram_dma.cursor.remaining > 0
            && self.vram_dma.block.remaining == 0;
        if self.vram_dma.arb.pend {
            self.vram_dma.arb.pend_from_arm = self.vram_dma.arb.armed_this_fall;
            self.vram_dma.arb.pend_age = 0;
        }
        self.vram_dma.arb.armed_this_fall = false;
        // An arm-strobe launch must ready inside mode 0: readiness completing
        // after the mode-0 exit reverts the block (FF55 count restored) to wait
        // for the next entry. Single speed only: the double-speed
        // readiness-vs-exit margin is unmeasured, and the DS late_enable rows
        // expect the arm serviced.
        if self.vram_dma.block.ready_in.expired()
            && self.vram_dma.block.from_arm
            && self.vram_dma.block.remaining == 16
            && !self.double_speed
        {
            self.vram_dma.block.arm_ready_probation = true;
        }

        // Refill this M-cycle's byte budget while the transfer is moving bytes:
        // 2/M-cycle single speed, 1 in double speed.
        self.vram_dma.cursor.quota = if self.vram_dma.moving() {
            if self.double_speed { 1 } else { 2 }
        } else {
            0
        };
        if self.vram_dma.seizes_bus() {
            self.vram_dma.block.seize_falls = self.vram_dma.block.seize_falls.saturating_add(1);
        } else {
            self.vram_dma.block.seize_falls = 0;
        }
        let claim = VramDmaClaim {
            committed: committing,
            // A claim is standing once it has aged through one full M-cycle
            // of the freeze — the synchronizer stage that carries it into
            // the CPU's M-cycle clock domain; a younger claim hasn't crossed when
            // the halt-release fetch starts.
            standing: committing && self.vram_dma.arb.pend_age >= 4,
        };
        if claim.committed {
            // An active OAM DMA already owns a bus, blocking the handover that
            // would take the halt-release fetch's tail.
            let bus_free = chassis.dma.is_active_on_bus().is_none();
            self.console_state.set_vram_dma_claim(VramDmaClaim {
                committed: true,
                standing: claim.standing && bus_free,
            });
        }
    }

    pub(crate) fn vram_dma_on_lcd_disabled(&mut self) {
        // VID_RST re-anchors the dot unit: the mux displacement is void.
        self.switch_relock_debit = false;
        self.vram_dma.arb.idle_claim = false;
        if self.vram_dma.cursor.mode == TransferMode::HBlank
            && self.vram_dma.cursor.remaining > 0
            && !self.vram_dma.block.taken
            && self.vram_dma.block.remaining == 0
        {
            self.vram_dma.block.remaining = 16;
            self.vram_dma.arb.pend_from_arm = true;
            self.vram_dma.block.setup_cells.clear();
            self.vram_dma.block.ready_in.arm(2);
        }
    }

    pub(crate) fn vram_dma_write_conflict_source(&self, address: u16) -> Option<u16> {
        // Double speed only: at single speed the CPU read-latch lands after the
        // block on its own (the byte-identical single-speed reads pass already);
        // at 2× the read collides with the byte the block is writing — but only
        // once the bus seizure has settled one full prior fall (the half-dot from
        // seizure to byte-readable; the count includes this fall, so `>= 2`).
        let writing = self.double_speed
            && self.vram_dma.block.seize_falls >= 2
            && self.vram_dma.block.remaining > 0
            && self.vram_dma.cursor.remaining > 0;
        (writing && address == self.vram_dma.write_address()).then_some(self.vram_dma.cursor.source)
    }

    pub(crate) fn vram_dma_arbitrate_oam_bus(&mut self, chassis: &mut Chassis<Self>) -> bool {
        let oam = chassis.dma.peek_transfer();
        let hdma_active = self.console_state.dma_cpu_hold() || self.console_state.bus_suspended();
        // The OAM-DMA's final byte still shares the bus on the M-cycle it
        // completes; edge-detect its active→done boundary so that M-cycle still
        // contends with a concurrent VRAM-DMA.
        let oam_transferring = oam.is_some();
        let oam_just_completed = self.vram_dma.oam.was_transferring && !oam_transferring;
        self.vram_dma.oam.was_transferring = oam_transferring;
        // The two engines share one bus: when an OAM-DMA and a VRAM-DMA block
        // both move a byte this M-cycle, the OAM-DMA latches the VRAM-DMA byte
        // that coincides with its write rather than its own source.
        let contended = !self.double_speed
            && (oam_transferring || oam_just_completed)
            && hdma_active
            && self.vram_dma.will_move();
        // Double speed: a switch-cancel escape byte's bus tenure stalls the
        // concurrent OAM-DMA byte one M-cycle (the engine resumes it next M).
        let escape_stall =
            self.double_speed && oam_transferring && hdma_active && self.vram_dma.escape_pending();
        if escape_stall {
            chassis.dma.stall_advance();
        }
        self.vram_dma.oam.contended = contended;
        contended || escape_stall
    }

    pub(crate) fn vram_dma_commit_bytes(&mut self, chassis: &mut Chassis<Self>) {
        let hdma_active = self.console_state.dma_cpu_hold() || self.console_state.bus_suspended();
        if !hdma_active {
            return;
        }
        let contended = self.vram_dma.oam.contended;
        // Commit the bytes the VRAM DMA moves while it actually holds the bus —
        // the hold keeps the transfer from overlapping the arming instruction.
        // (The trigger/quota tick ran before this edge's write commit.)
        let mut hdma_bytes: [Option<(u16, u8)>; 2] = [None, None];
        if !self.vram_dma.take_setup_cell() {
            while let Some((src, dst)) = self.vram_dma.next_byte() {
                let byte = self.read_hdma_source(chassis, src);
                chassis.dma_commit(src, dst, byte);
                if contended {
                    if hdma_bytes[0].is_none() {
                        hdma_bytes[0] = Some((src, byte));
                    } else if hdma_bytes[1].is_none() {
                        hdma_bytes[1] = Some((src, byte));
                    }
                }
            }
        }

        // The OAM-DMA's deposit this M-cycle is the coinciding VRAM-DMA byte,
        // landing at OAM[source_low]. Which of the M-cycle's two VRAM-DMA bytes
        // coincides is the phase between the two byte clocks (the OAM-DMA's start
        // edge vs the block's), 2nd byte at residue {0,3}. The coinciding OAM
        // byte commits one T-cycle after start_edge marks dma_run engaging, so
        // align the phase to that commit before taking the residue.
        if contended {
            const OAM_BYTE_COMMIT_LAG_EDGES: u64 = 2;
            let phase = self
                .vram_dma
                .block
                .start_edge
                .wrapping_sub(chassis.dma.start_edge())
                .wrapping_sub(OAM_BYTE_COMMIT_LAG_EDGES)
                / 2
                % 4;
            let coinciding = if matches!(phase, 0 | 3) {
                hdma_bytes[1].or(hdma_bytes[0])
            } else {
                hdma_bytes[0]
            };
            if let Some((hsrc, hdata)) = coinciding {
                chassis.dma_commit(hsrc, 0xfe00 | (hsrc & 0xFF), hdata);
            }
        }
    }

    /// Read one VRAM-DMA source byte: the VRAM float, then the cart-bus float,
    /// the CGB register/banked-WRAM map, and finally chassis storage — the
    /// VRAM-DMA counterpart of `Console::read_dma_source`.
    fn read_hdma_source(&self, chassis: &Chassis<Self>, source: u16) -> u8 {
        if let Some(open) = source_open_bus(source) {
            return open;
        }
        if let Some(value) = bus::dma_source_open_bus(source) {
            return value;
        }
        if let Some(value) = self.map_read_byte(source, &chassis.ppu, &chassis.vram_bus.vram) {
            return value;
        }
        chassis.read_dma_storage(source)
    }
}
