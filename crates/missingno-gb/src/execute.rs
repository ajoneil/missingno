use super::{
    BusAccess, BusAccessKind, ClockPhase, GameBoy, StagedPpuWrite,
    cpu::mcycle::DotAction,
    interrupts::Interrupt,
    is_ppu_register,
    memory::Bus,
    ppu::{self, PpuTickResult, types::palette::PaletteIndex},
};

/// Whether the OAM bug corruption uses the read or write formula.
/// Determined by the CPU operation type, not by the OAM control
/// signals at the moment of the spurious SRAM clock.
pub(super) enum OamBugKind {
    Read,
    Write,
}

/// Result of executing one instruction.
pub struct StepResult {
    /// Whether a new video frame was produced during this instruction.
    pub new_screen: bool,
    /// Whether battery-backed SRAM was written during this instruction.
    pub sram_dirty: bool,
    /// Number of T-cycles (dots) consumed by this instruction.
    pub dots: u32,
}

/// Result of executing one half-phase (rise or fall).
pub struct PhaseResult {
    /// Whether a new video frame was produced.
    pub new_screen: bool,
    /// Pixel pushed to the LCD during this phase, if any.
    pub pixel: Option<ppu::PixelOutput>,
}

impl GameBoy {
    pub fn step(&mut self) -> StepResult {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    /// Returns (result, trace). Trace is empty when `trace` is false.
    pub fn step_traced(&mut self, trace: bool) -> (StepResult, Vec<BusAccess>) {
        if trace {
            self.bus_trace = Some(Vec::new());
        }

        // If step_dot() left us mid-instruction, drain to the next
        // boundary first, then run one full instruction.
        let mut new_screen = false;
        let mut dots = 0u32;
        if !self.cpu.at_instruction_boundary() {
            let r = self.step_instruction();
            new_screen |= r.new_screen;
            dots += r.dots;
        }
        let r = self.step_instruction();
        new_screen |= r.new_screen;
        dots += r.dots;

        let sram_dirty = self.external.cartridge.take_sram_dirty();
        let trace = self.bus_trace.take().unwrap_or_default();
        (
            StepResult {
                new_screen,
                sram_dirty,
                dots,
            },
            trace,
        )
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Runs phases until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> StepResult {
        let mut new_screen = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;
        self.last_read_value = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.cpu.take_instruction_boundary();

        const PHASE_BUDGET: u32 = 400;
        let mut phases_remaining = PHASE_BUDGET;
        let mut dots = 0u32;

        loop {
            assert!(
                phases_remaining > 0,
                "step() exceeded {PHASE_BUDGET} phase budget — possible infinite loop in CPU"
            );
            phases_remaining -= 1;

            let result = self.execute_phase(&mut pending_oam_bug);
            new_screen |= result.new_screen;

            // Check for instruction boundary after completing a dot
            // (clock is Low = just finished fall() = dot complete)
            if self.clock_phase == ClockPhase::Low {
                dots += 1;
                if self.cpu.at_instruction_boundary() {
                    break;
                }
            }
        }
        // Don't drain sram_dirty here — let the caller (step_traced) do it
        // so the flag accumulates across multiple step_instruction calls.
        let sram_dirty = self.external.cartridge.sram_dirty;
        StepResult {
            new_screen,
            sram_dirty,
            dots,
        }
    }

    /// Advance exactly one half-phase — execute rise() or fall()
    /// depending on current clock level.
    pub fn step_phase(&mut self) -> PhaseResult {
        let mut pending_oam_bug: Option<OamBugKind> = None;
        self.execute_phase(&mut pending_oam_bug)
    }

    /// Advance to the next dot (T-cycle) boundary — the next Low
    /// state. Executes 1 phase if clock is High, 2 if Low.
    /// Returns true if a new frame was produced.
    pub fn step_dot(&mut self) -> bool {
        let mut new_screen = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;

        // Run phases until clock returns to Low (dot complete)
        loop {
            let result = self.execute_phase(&mut pending_oam_bug);
            new_screen |= result.new_screen;
            if self.clock_phase == ClockPhase::Low {
                break;
            }
        }

        // Consume instruction boundary flag (used by step_traced to detect
        // mid-instruction state). HALT bug and EI delay are handled
        // internally by the CPU state machine.
        self.cpu.take_instruction_boundary();

        new_screen
    }

    /// Execute one phase (half-dot) of hardware. When the clock is
    /// Low, execute rise() (Low→High edge). When High, execute
    /// fall() (High→Low edge).
    fn execute_phase(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> PhaseResult {
        match self.clock_phase {
            ClockPhase::Low => self.rise(pending_oam_bug),
            ClockPhase::High => self.fall(pending_oam_bug),
        }
    }

    /// Rising edge: advance CPU state machine, capture bus reads,
    /// tick timer and PPU, fire OAM bugs.
    ///
    /// At each M-cycle boundary, `irq_latched` (yoii) captures
    /// `irq_pending`, driving the HALT-release chain (yoii → ykua →
    /// halt↓). The dispatching CLK9↑ — where zacw and the write-back
    /// DFFs share the same capture edge — happens inside `retire_edge()`,
    /// called via `next_mcycle` → `mcycle_*`; the sequencer combinationally
    /// selects fetch vs dispatch from `dispatch_active.q` on that same
    /// edge. On the HALT-wake path, `tick_dispatch_active` captures
    /// dispatch_active on the closing CLK9↑ of `Halted(WakeIntake)` so
    /// the next M-cycle's selector resolves dispatch vs fetch from the
    /// freshly-captured `dispatch_active.q`.
    fn rise(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> PhaseResult {
        let mut new_screen = false;
        let mut pixel = None;
        let is_mcycle_boundary = !self.cpu.mcycle_active;

        // ── M-cycle boundary: irq_latched capture, then PPU + interrupt updates ──
        if is_mcycle_boundary {
            // data_phase_n↑ just before the new M-cycle's CLK9↑: the
            // per-bit irq_latch_inst<i> enabled D-latch reopens and
            // re-snapshots IF. Must run before tick_irq_latched so yoii
            // captures the freshly settled post-latch IF.
            self.cpu
                .dispatch
                .set_data_phase_n(true);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);

            // zacw captures zfex on this CLK9↑.
            self.cpu.dispatch.tick_zacw();

            self.cpu.tick_irq_latched(); // samples pre-edge irq_pending

            // Clear any staged PPU write from the previous M-cycle.
            self.staged_ppu_write = None;

            // Timer ticks once per M-cycle (BOGA).
            self.timers.mcycle();

            // Drain g151 → g154: the CLK9-clocked g151 DFF latches the
            // reload-cycle pulse during timers.mcycle(); g154 captures
            // immediately after this CLK9↑, raising IF.bit2 before the
            // CPU's dispatch read at this M-cycle.
            if let Some(interrupt) = self.timers.take_pending_interrupt() {
                self.interrupts.request(interrupt);
            }

            // PPU master-clock rising edge at the M-cycle boundary (dot 0).
            let ppu_result = self.ppu.on_master_clock_rise(&self.vram_bus.vram);
            if ppu_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }
            let (ns, pix) = self.apply_ppu_result(&ppu_result);
            new_screen |= ns;
            if pixel.is_none() {
                pixel = pix;
            }

            // SUKO is combinational — check for STAT edge after PPU rise.
            if self.ppu.check_stat_edge() {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            // Capture interrupt state so the CPU's dispatch check sees it.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
        }

        // ── CPU dot advance ──
        let dot_action = self.cpu.next_dot(self.last_read_value);
        self.current_dot_action = dot_action;

        // IE push bug + ctl_int_entry_m6: vector-resolve fires between
        // ISR M3 and M4. Clears zkog/zloz (per netlist R_n =
        // NOR(ctl_int_entry_m6, sys_reset)) and the dispatched IF bit
        // (per netlist inta[i] = AND(ctl_int_entry_m6, irq_latch_gated_q_n[i])).
        if self.cpu.take_pending_vector_resolve() {
            // Read the post-latch priority chain output (matches the
            // hardware vector resolve which reads irq_latch_gated, not
            // raw IF).
            if let Some(interrupt) = self.cpu.dispatch.vector() {
                self.interrupts.clear(interrupt);
                self.cpu.bus_counter = interrupt.vector();
            } else {
                self.cpu.bus_counter = 0x0000;
            }
            self.cpu.dispatch.clear_dispatch();
        }

        let dot = self.cpu.dot_for_execute();
        self.current_dot = dot;

        // data_phase_n↓ at the dot 1→2 boundary closes the per-bit
        // irq_latch_inst<i> enabled D-latch. Subsequent IF requests this
        // M-cycle still set `requested` but do not propagate to the
        // latch until the next data_phase_n↑ at the M-cycle boundary.
        //
        // Skipped during HALT: the CPU phase ring (baly/buty) is frozen,
        // data_phase is held LOW, and the latch stays transparent.
        if dot.index() == 2 && !self.cpu.is_halted() {
            self.cpu.dispatch.set_data_phase_n(false);
        }

        // step_zkog drives zaij combinational + zkog SR-latch update.
        // zaij = ime ∧ data_phase ∧ int_take ∧ xogs ∧ ¬(EI/DI in flight).
        // xogs = (data_phase ∧ ctl_fetch) ∨ halt — only fires during
        // fetch M-cycles' data-phase, blocking dispatch during memory
        // ops (Read/Write/Operands).
        let halt = self.cpu.is_halted();
        let data_phase = !halt && (dot.index() == 2 || dot.index() == 3);
        let write_phase = !halt && dot.index() == 3;
        let ctl_fetch = self.cpu.is_fetch_phase();
        let xogs = (data_phase && ctl_fetch) || halt;
        let ime_enabled =
            self.cpu.ime.output() == crate::cpu::InterruptMasterEnable::Enabled;
        let ei_di = self.cpu.ei_di_in_flight();
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu
            .dispatch
            .step_zkog(ime_enabled, data_phase, write_phase, xogs, halt, ei_di);

        // Stage PPU register writes at dot 0. On hardware, the CPU
        // places the address on the bus at phase A and the address
        // decode chain begins propagating. The write is applied at
        // dot 2 rise (CUPA rises at phase E per §4.3, spanning 1.498
        // dots through phase H of dot 3).
        if is_mcycle_boundary && let Some((address, value)) = self.cpu.pending_bus_write() {
            if is_ppu_register(address) {
                self.staged_ppu_write = Some(StagedPpuWrite {
                    address,
                    value,
                    applied: false,
                });
            }
        }

        // BOWA (dot 0): record OAM bug from address in the upcoming action.
        if dot.bowa()
            && let DotAction::InternalOamBug { address } = &self.current_dot_action
            && (0xFE00..=0xFEFF).contains(address)
        {
            match pending_oam_bug {
                Some(OamBugKind::Read) => {}
                _ => {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
            }
        }

        // ── Non-boundary dots: PPU rise + interrupt capture AFTER CPU dot advance ──
        if !is_mcycle_boundary {
            // Apply staged PPU write at BusDot 2 rise (phase E). On
            // hardware, CUPA rises at dot 2 of the M-cycle coincident
            // with an alet rising edge, gating the PPU register latches
            // transparent. The latch stays transparent across dots 2-3
            // (CUPA pulse ~1.498 dots wide) — combinational PPU logic
            // sees the new value from phase E onward. Alet-clocked DFFs
            // (which capture on ALET rising = our fall()) see the new
            // value in THIS dot's fall().
            if dot.as_u8() == 2 {
                if let Some(staged) = self.staged_ppu_write.as_ref() {
                    if !staged.applied {
                        let (addr, val) = (staged.address, staged.value);
                        if self.drive_ppu_bus(addr, val) {
                            self.interrupts.request(Interrupt::VideoStatus);
                        }
                        self.staged_ppu_write.as_mut().unwrap().applied = true;
                    }
                }
            }

            // PPU master-clock rising edge for non-boundary dots.
            let ppu_result = self.ppu.on_master_clock_rise(&self.vram_bus.vram);
            if ppu_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }

            let (ns, pix) = self.apply_ppu_result(&ppu_result);
            new_screen |= ns;
            if pixel.is_none() {
                pixel = pix;
            }

            // SUKO is combinational — check for STAT edge after PPU rise.
            let stat_edge = self.ppu.check_stat_edge();
            if stat_edge {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            // Capture interrupt state for non-boundary dots. irq_latched
            // ticks in the matching fall(), capturing this dot's irq_pending
            // for the cascade-settling counter.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        // VOGA capture (HBlank) happens on the master-clock rising edge via
        // HblankPipeline::capture_voga(): VOGA is ALET-clocked, and per spec
        // §1.1/§1.2 ALET rises in-phase with ck1_ck2 (master-clock rising).
        // The mode() function uses `xymu && !wodu` to predict HBlank state
        // for CPU STAT reads, so settle_alet is not needed. G4.2 confirmed
        // WODU doesn't depend on XYMU, making the prediction reliable.

        // MOPA rising edge (dot 2): fire OAM bug.
        if dot.mopa()
            && !dot.boga()
            && let Some(kind) = pending_oam_bug.take()
        {
            match kind {
                OamBugKind::Read => self.ppu.oam_bug_read(),
                OamBugKind::Write => self.ppu.oam_bug_write(),
            }
        }

        self.clock_phase = ClockPhase::High;
        PhaseResult { new_screen, pixel }
    }

    /// Falling edge: PPU falling phase, interrupt latch capture,
    /// bus writes, M-cycle subsystems (serial, DMA, audio).
    fn fall(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> PhaseResult {
        let mut new_screen = false;
        let dot = self.current_dot;
        let is_mcycle_boundary = dot.boga();

        // PPU master-clock falling edge: divider chain (WUVU/VENA/TALU),
        // CATU, scanline boundaries, fetcher, DFF8/DFF9, LCD-off.
        let video_result = self.ppu.on_master_clock_fall(is_mcycle_boundary);

        // CPU data latch: capture bus value after PPU's master-clock
        // fall updates land. Hardware reads are combinational (spec
        // §4.6): the CPU samples the current DFF state via SM83-
        // internal data_phase. PPU DFF transitions on the same master-
        // clock cycle's TALU-rising edge (MYTA fire, ROPO capture) are
        // visible to the CPU read because they settle before the CPU's
        // data-phase latches. Placing the read after on_master_clock_fall
        // matches that ordering.
        if let DotAction::Read { address } = &self.current_dot_action {
            if (0xFE00..=0xFEFF).contains(address) {
                *pending_oam_bug = Some(OamBugKind::Read);
            }
            self.last_read_value = self.cpu_read(*address);
        }

        // VBlank IF: the divider chain now runs in fall(), so POPU
        // (VBlank) transitions happen here, not in rise().
        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }

        // SUKO is combinational — check for STAT edge after every phase.
        let stat_edge = self.ppu.check_stat_edge();
        if stat_edge {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        let (ns, pixel) = self.apply_ppu_result(&video_result);
        new_screen |= ns;

        // Bus writes on the falling edge.
        match &self.current_dot_action {
            DotAction::Idle | DotAction::InternalOamBug { .. } | DotAction::Read { .. } => {}
            DotAction::Write { address, value } => {
                let address = *address;
                let value = *value;
                if (0xFE00..=0xFEFF).contains(&address) {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
                // Skip drive_ppu_bus if the staged write mechanism already
                // applied this write at the correct visibility dot.
                let already_applied = self.staged_ppu_write.as_ref().is_some_and(|s| s.applied);
                if !already_applied && self.drive_ppu_bus(address, value) {
                    self.interrupts.request(Interrupt::VideoStatus);
                }
                self.write_byte(address, value);
            }
        }

        if is_mcycle_boundary {
            // Serial ticks once per M-cycle. Run before update_interrupt_state
            // so a serial-complete IF is visible to the same-fall capture and
            // the next rise's irq_latched sees it — keeping serial in the
            // mid-M-cycle source class alongside VBlank/STAT.
            let counter = self.timers.internal_counter();
            if let Some(interrupt) = self.serial.mcycle(counter, &mut *self.link) {
                self.interrupts.request(interrupt);
            }

            // OAM DMA: transfer one byte per M-cycle.
            if let Some((src_addr, dst_offset)) = self.dma.mcycle() {
                let byte = self.read_dma_source(src_addr);
                let dst_addr = 0xfe00 + dst_offset as u16;
                let oam_addr = match ppu::memory::MappedAddress::map(dst_addr) {
                    ppu::memory::MappedAddress::Oam(addr) => addr,
                    _ => unreachable!(),
                };
                self.ppu.write_oam(oam_addr, byte);
                if let Some(trace) = &mut self.bus_trace {
                    trace.push(BusAccess {
                        address: src_addr,
                        value: byte,
                        kind: BusAccessKind::DmaRead,
                    });
                    trace.push(BusAccess {
                        address: dst_addr,
                        value: byte,
                        kind: BusAccessKind::DmaWrite,
                    });
                }
                match Bus::of(src_addr) {
                    Some(Bus::External) => {
                        self.external.drive(byte);
                    }
                    Some(Bus::Vram) => {
                        self.vram_bus.drive(byte);
                    }
                    None => {}
                }
            }

            // External bus decay.
            self.external.tick_decay();

            self.audio.mcycle(self.timers.internal_counter());
        }

        // Capture interrupt state AFTER bus writes and M-cycle subsystems so
        // IF updates from a CPU write to 0xFF0F, STAT edges from PPU register
        // writes, and serial completion are all visible on the same fall.
        // irq_latched ticks on the next rise to sample irq_pending into the DFF.
        {
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        self.clock_phase = ClockPhase::Low;
        PhaseResult { new_screen, pixel }
    }

    /// Process a PPU tick result: write pixel to back buffer, present
    /// on frame boundary. Returns `(new_frame, pixel)`.
    fn apply_ppu_result(&mut self, result: &PpuTickResult) -> (bool, Option<ppu::PixelOutput>) {
        if let Some(pixel) = result.pixel {
            if pixel.x < ppu::screen::PIXELS_PER_LINE && pixel.y < ppu::screen::NUM_SCANLINES {
                self.screen
                    .draw_pixel(pixel.x, pixel.y, PaletteIndex(pixel.shade));
            }
        }
        if result.new_frame {
            self.screen.present();
            if let Some(sgb) = &mut self.sgb {
                sgb.update_screen(&self.screen);
            }
            return (true, result.pixel);
        }
        (false, result.pixel)
    }
}
