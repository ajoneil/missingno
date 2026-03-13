use super::{
    BusAccess, BusAccessKind, ExecutionState, GameBoy, InterruptLatch,
    cpu::{
        EiDelay, HaltState, InterruptMasterEnable,
        mcycle::{BusDot, DotAction, Processor},
    },
    interrupts::Interrupt,
    memory::Bus,
    ppu,
};

/// Whether the OAM bug corruption uses the read or write formula.
/// Determined by the CPU operation type, not by the OAM control
/// signals at the moment of the spurious SRAM clock.
pub(super) enum OamBugKind {
    Read,
    Write,
}

impl GameBoy {
    pub fn step(&mut self) -> bool {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    /// Returns (new_screen, trace). Trace is empty when `trace` is false.
    pub fn step_traced(&mut self, trace: bool) -> (bool, Vec<BusAccess>) {
        if trace {
            self.bus_trace = Some(Vec::new());
        }

        // Drain any mid-instruction state left by step_dot().
        let mut new_screen = false;
        while !matches!(self.execution, ExecutionState::Ready) {
            new_screen |= self.step_dot();
        }

        // Now at an instruction boundary — run one full instruction.
        new_screen |= self.step_instruction();

        let trace = self.bus_trace.take().unwrap_or_default();
        (new_screen, trace)
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Running path: leading fetch (4 dots + opcode read) → dispatch
    /// decision → Processor → advance EI delay → return.
    ///
    /// Halted/Halting paths: preserved from previous model with
    /// minimal changes (no prefetched_opcode).
    fn step_instruction(&mut self) -> bool {
        let mut new_screen = false;

        match self.cpu.halt_state {
            HaltState::Running => {
                // ── Leading fetch ──
                // Run the opcode read M-cycle: 4 dots of hardware ticking
                // followed by a bus read from PC. This IS the generic fetch
                // on hardware — the first M-cycle of every instruction.
                let fetch_addr = self.cpu.program_counter;
                let mut fetch_dot = BusDot::ZERO;
                for _ in 0u8..4 {
                    let is_mcycle_boundary = fetch_dot.boga();
                    new_screen |= self.tick_dot(is_mcycle_boundary);
                    fetch_dot = if fetch_dot.boga() {
                        BusDot::ZERO
                    } else {
                        fetch_dot.advance()
                    };
                }
                let opcode = self.cpu_read(fetch_addr);

                // ── Dispatch decision ──
                // After the leading fetch's ticking, check for interrupt
                // dispatch. This matches hardware's A→B boundary decision.
                self.interrupt_latch.promote();
                let dispatch = self.interrupt_latch.take_ready().is_some();

                let mut processor = if dispatch {
                    // Leading fetch M-cycle = ISR M0 (suppressed fetch).
                    // Processor starts at ISR M1 (4 post-fetch M-cycles).
                    Processor::interrupt(&mut self.cpu)
                } else {
                    Processor::fetch_with_opcode(&mut self.cpu, opcode)
                };

                // ── Run Processor dots ──
                let mut read_value: u8 = 0;
                let mut pending_oam_bug: Option<OamBugKind> = None;
                let mut dot = BusDot::ZERO;

                const DOT_BUDGET: u32 = 52;
                let mut dots_remaining = DOT_BUDGET;

                loop {
                    assert!(
                        dots_remaining > 0,
                        "step() exceeded {DOT_BUDGET} dot budget — possible infinite loop in Processor"
                    );
                    dots_remaining -= 1;
                    let dot_action = match processor.next_dot(read_value, &mut self.cpu) {
                        Some(action) => action,
                        None => {
                            self.check_halt_bug();

                            if self.cpu.halt_state == HaltState::Halting {
                                // HALT's dummy fetch: read [PC] without incrementing.
                                // Run 4 dots of hardware ticking, then transition
                                // to Halted.
                                let halt_addr = self.cpu.program_counter;
                                let mut halt_dot = BusDot::ZERO;
                                for _ in 0u8..4 {
                                    let is_mcycle_boundary = halt_dot.boga();
                                    new_screen |= self.tick_dot(is_mcycle_boundary);
                                    halt_dot = if halt_dot.boga() {
                                        BusDot::ZERO
                                    } else {
                                        halt_dot.advance()
                                    };
                                }
                                let _ = self.cpu_read(halt_addr);
                                self.cpu.halt_state = HaltState::Halted;
                            }

                            self.advance_ei_delay();
                            return new_screen;
                        }
                    };

                    // IE push bug.
                    if processor.take_pending_vector_resolve() {
                        if let Some(interrupt) = self.interrupts.triggered() {
                            self.interrupts.clear(interrupt);
                            self.cpu.program_counter = interrupt.vector();
                        } else {
                            self.cpu.program_counter = 0x0000;
                        }
                    }

                    let (new_screen_dot, new_read_value) =
                        self.execute_dot(&dot_action, dot, &mut pending_oam_bug);
                    new_screen |= new_screen_dot;
                    if let Some(v) = new_read_value {
                        read_value = v;
                    }

                    dot = if dot.boga() {
                        BusDot::ZERO
                    } else {
                        dot.advance()
                    };
                }
            }

            HaltState::Halted => {
                // Halted path: preserved from previous model.
                // promote() at top, deferred dispatch, HaltedNop, etc.
                self.interrupt_latch.promote();
                let dispatch_interrupt = self.interrupt_latch.take_ready().is_some();

                let mut processor = if dispatch_interrupt {
                    Processor::interrupt(&mut self.cpu)
                } else {
                    Processor::begin(&mut self.cpu)
                };

                let mut was_halted = true;

                let mut read_value: u8 = 0;
                let mut pending_oam_bug: Option<OamBugKind> = None;
                let mut dot = BusDot::ZERO;

                const DOT_BUDGET: u32 = 52;
                let mut dots_remaining = DOT_BUDGET;

                loop {
                    assert!(
                        dots_remaining > 0,
                        "step() exceeded {DOT_BUDGET} dot budget — possible infinite loop in Processor"
                    );
                    dots_remaining -= 1;
                    let dot_action = match processor.next_dot(read_value, &mut self.cpu) {
                        Some(action) => action,
                        None => {
                            if self.cpu.halt_state == HaltState::Halted {
                                if self.interrupt_latch.take_ready().is_some() {
                                    // Interrupt was Ready — the HaltedNop's
                                    // M-cycle served as the wakeup NOP. ISR
                                    // dispatch begins immediately.
                                    was_halted = false;
                                    processor = Processor::interrupt(&mut self.cpu);
                                    continue;
                                }
                                // Fresh capture or still idle — stay halted.
                                self.advance_ei_delay();
                                return new_screen;
                            }

                            // IME=0 HALT wakeup: the HaltedNop's M-cycle
                            // already served as the leading fetch — it read
                            // the opcode at [PC]. Build a new Processor with
                            // that opcode and run it inline.
                            if was_halted && self.cpu.halt_state == HaltState::Running {
                                was_halted = false;
                                processor =
                                    Processor::fetch_with_opcode(&mut self.cpu, read_value);
                                continue;
                            }

                            // ISR or post-wakeup instruction complete.
                            self.check_halt_bug();

                            if self.cpu.halt_state == HaltState::Halting {
                                let halt_addr = self.cpu.program_counter;
                                let mut halt_dot = BusDot::ZERO;
                                for _ in 0u8..4 {
                                    let is_mcycle_boundary = halt_dot.boga();
                                    new_screen |= self.tick_dot(is_mcycle_boundary);
                                    halt_dot = if halt_dot.boga() {
                                        BusDot::ZERO
                                    } else {
                                        halt_dot.advance()
                                    };
                                }
                                let _ = self.cpu_read(halt_addr);
                                self.cpu.halt_state = HaltState::Halted;
                            }

                            self.advance_ei_delay();
                            return new_screen;
                        }
                    };

                    // IE push bug.
                    if processor.take_pending_vector_resolve() {
                        if let Some(interrupt) = self.interrupts.triggered() {
                            self.interrupts.clear(interrupt);
                            self.cpu.program_counter = interrupt.vector();
                        } else {
                            self.cpu.program_counter = 0x0000;
                        }
                    }

                    let (new_screen_dot, new_read_value) =
                        self.execute_dot(&dot_action, dot, &mut pending_oam_bug);
                    new_screen |= new_screen_dot;
                    if let Some(v) = new_read_value {
                        read_value = v;
                    }

                    dot = if dot.boga() {
                        BusDot::ZERO
                    } else {
                        dot.advance()
                    };
                }
            }

            HaltState::Halting => {
                // HALT entry: the HALT instruction was decoded in the
                // previous step. Run the HaltedNop (which does the
                // dummy fetch), then transition to Halted.
                let mut processor = Processor::begin(&mut self.cpu);

                let mut read_value: u8 = 0;
                let mut pending_oam_bug: Option<OamBugKind> = None;
                let mut dot = BusDot::ZERO;

                const DOT_BUDGET: u32 = 52;
                let mut dots_remaining = DOT_BUDGET;

                loop {
                    assert!(
                        dots_remaining > 0,
                        "step() exceeded {DOT_BUDGET} dot budget — possible infinite loop in Processor"
                    );
                    dots_remaining -= 1;
                    let dot_action = match processor.next_dot(read_value, &mut self.cpu) {
                        Some(action) => action,
                        None => {
                            self.cpu.halt_state = HaltState::Halted;
                            self.advance_ei_delay();
                            return new_screen;
                        }
                    };

                    let (new_screen_dot, new_read_value) =
                        self.execute_dot(&dot_action, dot, &mut pending_oam_bug);
                    new_screen |= new_screen_dot;
                    if let Some(v) = new_read_value {
                        read_value = v;
                    }

                    dot = if dot.boga() {
                        BusDot::ZERO
                    } else {
                        dot.advance()
                    };
                }
            }
        }
    }

    /// Advance exactly one dot (T-cycle). Returns true if a new
    /// frame was produced. The execution state machine tracks where
    /// we are in the instruction lifecycle across calls.
    pub fn step_dot(&mut self) -> bool {
        match std::mem::replace(&mut self.execution, ExecutionState::Ready) {
            ExecutionState::Ready => {
                match self.cpu.halt_state {
                    HaltState::Running => {
                        // Start the leading fetch.
                        self.execution = ExecutionState::LeadingFetch {
                            dot: BusDot::ZERO,
                            fetch_addr: self.cpu.program_counter,
                        };
                        self.step_dot()
                    }
                    HaltState::Halted => {
                        // Halted path: promote, then dispatch or HaltedNop.
                        self.interrupt_latch.promote();
                        let dispatch = self.interrupt_latch.take_ready().is_some();

                        let processor = if dispatch {
                            Processor::interrupt(&mut self.cpu)
                        } else {
                            Processor::begin(&mut self.cpu)
                        };

                        let was_halted = true;

                        self.execution = ExecutionState::Running {
                            processor,
                            read_value: 0,
                            dot: BusDot::ZERO,
                            pending_oam_bug: None,
                            was_halted,
                        };
                        self.step_dot()
                    }
                    HaltState::Halting => {
                        // HALT entry: run HaltedNop then transition.
                        let processor = Processor::begin(&mut self.cpu);

                        self.execution = ExecutionState::Running {
                            processor,
                            read_value: 0,
                            dot: BusDot::ZERO,
                            pending_oam_bug: None,
                            was_halted: false,
                        };
                        self.step_dot()
                    }
                }
            }
            ExecutionState::LeadingFetch { dot, fetch_addr } => {
                let is_mcycle_boundary = dot.boga();
                let new_screen = self.tick_dot(is_mcycle_boundary);

                if is_mcycle_boundary {
                    // Final dot of the leading fetch M-cycle.
                    let opcode = self.cpu_read(fetch_addr);

                    // Dispatch decision after the leading fetch's ticking.
                    self.interrupt_latch.promote();
                    let dispatch = self.interrupt_latch.take_ready().is_some();

                    let processor = if dispatch {
                        Processor::interrupt(&mut self.cpu)
                    } else {
                        Processor::fetch_with_opcode(&mut self.cpu, opcode)
                    };

                    self.execution = ExecutionState::Running {
                        processor,
                        read_value: 0,
                        dot: BusDot::ZERO,
                        pending_oam_bug: None,
                        was_halted: false,
                    };
                } else {
                    self.execution = ExecutionState::LeadingFetch {
                        dot: dot.advance(),
                        fetch_addr,
                    };
                }

                new_screen
            }
            ExecutionState::Running {
                mut processor,
                mut read_value,
                mut dot,
                mut pending_oam_bug,
                was_halted,
            } => {
                let dot_action = match processor.next_dot(read_value, &mut self.cpu) {
                    Some(action) => action,
                    None => {
                        // Instruction complete — handle post-instruction
                        // transitions and determine next state.
                        self.check_halt_bug();

                        if self.cpu.halt_state == HaltState::Halting {
                            self.execution = ExecutionState::HaltDummyFetch {
                                dot: BusDot::ZERO,
                                fetch_addr: self.cpu.program_counter,
                            };
                            return self.step_dot();
                        }

                        if self.cpu.halt_state == HaltState::Halted {
                            if self.interrupt_latch.take_ready().is_some() {
                                // Restart with interrupt dispatch.
                                self.execution = ExecutionState::Running {
                                    processor: Processor::interrupt(&mut self.cpu),
                                    read_value: 0,
                                    dot: BusDot::ZERO,
                                    pending_oam_bug: None,
                                    was_halted: false,
                                };
                                return self.step_dot();
                            }
                            // Stay halted.
                            self.advance_ei_delay();
                            self.execution = ExecutionState::Ready;
                            return false;
                        }

                        // IME=0 HALT wakeup: the HaltedNop's M-cycle
                        // already read the opcode at [PC]. Build a
                        // Processor with that opcode and continue inline.
                        if was_halted && self.cpu.halt_state == HaltState::Running {
                            self.execution = ExecutionState::Running {
                                processor: Processor::fetch_with_opcode(
                                    &mut self.cpu,
                                    read_value,
                                ),
                                read_value: 0,
                                dot: BusDot::ZERO,
                                pending_oam_bug: None,
                                was_halted: false,
                            };
                            return self.step_dot();
                        }

                        // Normal instruction complete.
                        self.advance_ei_delay();
                        self.execution = ExecutionState::Ready;
                        return false;
                    }
                };

                // IE push bug.
                if processor.take_pending_vector_resolve() {
                    if let Some(interrupt) = self.interrupts.triggered() {
                        self.interrupts.clear(interrupt);
                        self.cpu.program_counter = interrupt.vector();
                    } else {
                        self.cpu.program_counter = 0x0000;
                    }
                }

                let (new_screen, new_read_value) =
                    self.execute_dot(&dot_action, dot, &mut pending_oam_bug);
                if let Some(v) = new_read_value {
                    read_value = v;
                }

                // Advance dot counter.
                dot = if dot.boga() {
                    BusDot::ZERO
                } else {
                    dot.advance()
                };

                self.execution = ExecutionState::Running {
                    processor,
                    read_value,
                    dot,
                    pending_oam_bug,
                    was_halted,
                };

                new_screen
            }
            ExecutionState::HaltDummyFetch { dot, fetch_addr } => {
                let is_mcycle_boundary = dot.boga();
                let new_screen = self.tick_dot(is_mcycle_boundary);

                if is_mcycle_boundary {
                    // Final dot of the HALT dummy fetch.
                    let _ = self.cpu_read(fetch_addr);
                    self.cpu.halt_state = HaltState::Halted;
                    self.advance_ei_delay();
                    self.execution = ExecutionState::Ready;
                } else {
                    self.execution = ExecutionState::HaltDummyFetch {
                        dot: dot.advance(),
                        fetch_addr,
                    };
                }

                new_screen
            }
        }
    }

    /// Execute one dot of hardware: OAM bug recording, phase ticks
    /// (DriveBus-aware ordering), interrupt capture, and bus actions.
    /// Returns `(new_screen, read_value)` where `read_value` is `Some`
    /// only if the dot performed a bus Read.
    fn execute_dot(
        &mut self,
        dot_action: &DotAction,
        dot: BusDot,
        pending_oam_bug: &mut Option<OamBugKind>,
    ) -> (bool, Option<u8>) {
        // BOWA (dot 0): record OAM bug from address in the upcoming action.
        if dot.bowa()
            && let DotAction::InternalOamBug { address } = dot_action
            && (0xFE00..=0xFEFF).contains(address)
        {
            match pending_oam_bug {
                Some(OamBugKind::Read) => {}
                _ => {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
            }
        }

        let is_mcycle_boundary = dot.boga();
        let is_drivebus = matches!(dot_action, DotAction::DriveBus { .. });
        let mut new_screen;

        if is_drivebus {
            // DriveBus dot: hardware order is EVEN first, then ODD.
            new_screen = self.tick_dot_falling(is_mcycle_boundary);

            if let DotAction::DriveBus { address, value } = dot_action
                && self.drive_ppu_bus(*address, *value)
            {
                self.interrupts.request(Interrupt::VideoStatus);
            }

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

            new_screen |= self.tick_dot_rising(is_mcycle_boundary);
        } else {
            // All other dots: normal order, ODD first then EVEN.
            new_screen = self.tick_dot_rising(is_mcycle_boundary);

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

            new_screen |= self.tick_dot_falling(is_mcycle_boundary);
        }

        // Interrupt latch capture (every dot) — models g42 CLK9-edge sampling.
        // Must run after BOTH phases so it sees all interrupt sources.
        self.capture_interrupt_latch();
        self.halt_wakeup_check();

        // Bus actions after phase ticks.
        let read_value = match dot_action {
            DotAction::Idle | DotAction::InternalOamBug { .. } | DotAction::DriveBus { .. } => None,
            DotAction::Read { address } => {
                if (0xFE00..=0xFEFF).contains(address) {
                    *pending_oam_bug = Some(OamBugKind::Read);
                }
                Some(self.cpu_read(*address))
            }
            DotAction::Write { address, value } => {
                if (0xFE00..=0xFEFF).contains(address) {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
                self.write_byte(*address, *value);
                None
            }
        };

        (new_screen, read_value)
    }

    /// Rising phase (DELTA_ODD) of one dot: timer tick, DFF8 palette
    /// latch advance, PPU pixel output pipeline.
    fn tick_dot_rising(&mut self, is_mcycle_boundary: bool) -> bool {
        // Timer ticks every T-cycle for DIV resolution
        if let Some(interrupt) = self.timers.tcycle(is_mcycle_boundary) {
            self.interrupts.request(interrupt);
        }

        // PPU rising phase: DFF8 palette latches, LCD init, pixel output.
        self.ppu.tcycle_rising(&self.vram_bus.vram);

        // SUKO is combinational — check for STAT edge after every phase.
        // Mode 0 (WODU/TARU) fires on the rising phase, so this catches
        // it immediately rather than deferring to the next falling phase.
        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        false
    }

    /// Falling phase (DELTA_EVEN) of one dot: PPU fetcher pipeline,
    /// DFF9 resolve, dot advance, interrupt edge detection, and
    /// M-cycle subsystems.
    fn tick_dot_falling(&mut self, is_mcycle_boundary: bool) -> bool {
        let mut new_screen = false;

        // PPU falling phase: fetcher, DFF9, dot advance, interrupts.
        let video_result = self
            .ppu
            .tcycle_falling(is_mcycle_boundary, &self.vram_bus.vram);
        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }

        // SUKO is combinational — check for STAT edge after every phase.
        // Falling phase changes mode transitions, LYC comparison, etc.
        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        if let Some(screen) = video_result.screen {
            if let Some(sgb) = &mut self.sgb {
                sgb.update_screen(&screen);
            }
            self.screen = screen;
            new_screen = true;
        }

        if is_mcycle_boundary {
            // Serial ticks once per M-cycle, using falling edges of the
            // internal counter's bit 7 to drive the serial shift clock.
            let counter = self.timers.internal_counter();
            if let Some(interrupt) = self.serial.mcycle(counter) {
                self.interrupts.request(interrupt);
            }

            // OAM DMA: transfer one byte per M-cycle. The DMA controller
            // drives the source bus with the byte it reads, updating the
            // bus latch so that CPU reads from the same bus see this value.
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

            // External bus decay: with no device driving the bus, the
            // retained value trends toward 0xFF as parasitic capacitance
            // discharges.
            self.external.tick_decay();

            self.audio.mcycle(self.timers.internal_counter());
        }

        new_screen
    }

    /// Tick hardware for one dot (both phases).
    ///
    /// Convenience wrapper for callers that don't need to route bus
    /// actions between phases (e.g. the leading fetch loop).
    fn tick_dot(&mut self, is_mcycle_boundary: bool) -> bool {
        let s1 = self.tick_dot_rising(is_mcycle_boundary);
        let s2 = self.tick_dot_falling(is_mcycle_boundary);
        self.capture_interrupt_latch();
        self.halt_wakeup_check();
        s1 || s2
    }

    /// Capture the interrupt latch, modeling g42's CLK9-edge sampling.
    /// Called every dot (4x per M-cycle), matching hardware where g42
    /// latches SeqControl_1 on every CLK9 rising edge. New captures are
    /// Fresh (not yet dispatchable); Ready values from a prior step keep
    /// their propagation state so the wakeup NOP's own tick doesn't
    /// regress a Ready back to Fresh.
    fn capture_interrupt_latch(&mut self) {
        self.interrupt_latch = match self.cpu.interrupt_master_enable {
            InterruptMasterEnable::Enabled => match self.interrupts.triggered() {
                Some(interrupt) => match self.interrupt_latch {
                    InterruptLatch::Ready(_) => InterruptLatch::Ready(interrupt),
                    _ if self.cpu.halt_state == HaltState::Halted => {
                        InterruptLatch::Ready(interrupt)
                    }
                    _ => InterruptLatch::Fresh(interrupt),
                },
                None => InterruptLatch::Empty,
            },
            InterruptMasterEnable::Disabled => InterruptLatch::Empty,
        };
    }

    /// Check for HALT wakeup (IME=0 path). On hardware, the HALT latch
    /// (g49) is reset combinationally from g42's output on the same CLK9
    /// edge. Called every dot alongside capture_interrupt_latch().
    ///
    /// Even with IME=Disabled, a pending interrupt wakes the CPU from
    /// HALT (without dispatching). Setting Running here causes the
    /// HaltedNop completion to use its bus read value as the opcode
    /// for the next instruction — the wakeup NOP IS the leading fetch
    /// (1 M-cycle total, not 2).
    fn halt_wakeup_check(&mut self) {
        if self.cpu.halt_state == HaltState::Halted
            && self.cpu.interrupt_master_enable == InterruptMasterEnable::Disabled
            && self.interrupts.triggered().is_some()
        {
            self.cpu.halt_state = HaltState::Running;
        }
    }

    /// HALT bug: if HALT was just executed with IME=0 and an interrupt
    /// is already pending, the CPU doesn't truly halt. It resumes
    /// immediately but fails to increment PC on the next opcode fetch.
    fn check_halt_bug(&mut self) {
        if !matches!(self.cpu.halt_state, HaltState::Halted | HaltState::Halting)
            || self.interrupts.triggered().is_none()
        {
            return;
        }
        if self.cpu.ei_delay == Some(EiDelay::Fired) {
            // EI immediately before HALT: on real hardware HALT saw
            // IME=0 (the DFF pipeline hadn't propagated yet). The halt
            // bug triggers — PC is not incremented. But EI's IME
            // promotion still takes effect, so the interrupt dispatches.
            self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
            self.cpu.program_counter -= 1;
            self.cpu.halt_state = HaltState::Running;
            self.cpu.ei_delay = None;
        } else if self.cpu.interrupt_master_enable == InterruptMasterEnable::Disabled {
            self.cpu.halt_state = HaltState::Running;
            self.cpu.halt_bug = true;
        }
    }

    /// Advance the EI delay pipeline one stage per instruction
    /// completion, modeling the DFF cascade from EI's decode signal
    /// to the IME flip-flop.
    fn advance_ei_delay(&mut self) {
        self.cpu.ei_delay = match self.cpu.ei_delay {
            Some(EiDelay::Pending) => Some(EiDelay::Fired),
            Some(EiDelay::Fired) => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                None
            }
            None => None,
        };
    }
}
