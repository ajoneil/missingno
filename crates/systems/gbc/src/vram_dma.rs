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
}
