use super::{
    BusAccess, BusAccessKind, ExecutionState, GameBoy, InterruptLatch,
    cpu::{
        EiDelay, HaltState, InterruptMasterEnable,
        mcycle::{DotAction, Processor},
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

    /// Run one complete instruction from start to finish (including
    /// trailing fetch). This is the original `step()` logic, factored
    /// out so `step()` can drain mid-instruction state first.
    fn step_instruction(&mut self) -> bool {
        let mut new_screen = false;

        // Advance the sequencer DFF pipeline: a Fresh interrupt from
        // the previous step's M-cycle boundary becomes Ready (dispatchable).
        // No separate propagation M-cycle is needed — for the Running path,
        // the trailing fetch at the end of the previous step() already
        // provided the DFF delay. For the Halted path, the HaltedNop's
        // M-cycle serves as the wakeup NOP. ISR dispatch begins immediately
        // when take_ready() succeeds.
        self.interrupt_latch.promote();
        let dispatch_interrupt = self.interrupt_latch.take_ready().is_some();

        let mut processor = match self.cpu.halt_state {
            HaltState::Running => {
                if dispatch_interrupt {
                    Processor::interrupt(&mut self.cpu)
                } else if let Some(opcode) = self.prefetched_opcode.take() {
                    Processor::fetch_with_opcode(&mut self.cpu, opcode)
                } else {
                    unreachable!("Running CPU must have a prefetched opcode")
                }
            }
            HaltState::Halted => {
                if dispatch_interrupt {
                    Processor::interrupt(&mut self.cpu)
                } else {
                    Processor::begin(&mut self.cpu)
                }
            }
            HaltState::Halting => Processor::begin(&mut self.cpu),
        };

        let was_halted = self.cpu.halt_state == HaltState::Halted;

        // Run dots. Each M-cycle is 4 dots; the processor yields one
        // DotAction per dot with bus operations at dot 3 (end of M-cycle).
        let mut read_value: u8 = 0;
        let mut pending_oam_bug: Option<OamBugKind> = None;
        let mut dot_in_mcycle: u8 = 0;

        // Safety budget: longest instruction is 6 M-cycles = 24 dots
        // (3 fetch + 3 execute). Interrupt dispatch is 20 dots (5 ISR
        // M-cycles). Budget of 52 gives margin.
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
                        // Run 4 dots of hardware ticking (same as any trailing
                        // fetch), then transition to Halted.
                        let fetch_addr = self.cpu.program_counter;
                        for dot in 0u8..4 {
                            let is_mcycle_boundary = dot == 3;
                            new_screen |= self.tick_dot(is_mcycle_boundary);
                        }
                        // Dummy fetch: read the bus but discard the result.
                        // PC is not incremented — the byte is thrown away.
                        let _ = self.cpu_read(fetch_addr);
                        self.cpu.halt_state = HaltState::Halted;
                        self.prefetched_opcode = None;
                        self.advance_ei_delay();
                        return new_screen;
                    }

                    if self.cpu.halt_state == HaltState::Halted {
                        if self.interrupt_latch.take_ready().is_some() {
                            // Interrupt was already Ready when this step began but
                            // the CPU was halted — the HaltedNop's M-cycle served
                            // as the wakeup NOP. ISR dispatch begins immediately.
                            processor = Processor::interrupt(&mut self.cpu);
                            continue;
                        }
                        // Fresh capture or still idle — stay halted. Fresh
                        // will promote to Ready at next step() entry via
                        // promote(), modeling the DFF cascade's 1 M-cycle
                        // propagation delay (CLK_ENA→PHI resumption).
                        self.prefetched_opcode = None;
                        self.advance_ei_delay();
                        return new_screen;
                    }

                    // IME=0 HALT wakeup: the HaltedNop's M-cycle already served
                    // as the trailing fetch — it read the opcode at [PC]. On
                    // hardware, the wakeup NOP IS the generic fetch. Capturing
                    // read_value as the prefetched opcode and returning avoids
                    // an extra M-cycle of hardware ticking.
                    if was_halted && self.cpu.halt_state == HaltState::Running {
                        self.prefetched_opcode = Some(read_value);
                        self.advance_ei_delay();
                        return new_screen;
                    }

                    // Run trailing fetch M-cycle: 4 dots of hardware ticks
                    // followed by an opcode bus read from PC.
                    //
                    // tick_dot() updates interrupt_latch at the M-cycle
                    // boundary, modeling the sequencer DFF pipeline.
                    let fetch_addr = self.cpu.program_counter;
                    for dot in 0u8..4 {
                        let is_mcycle_boundary = dot == 3;
                        new_screen |= self.tick_dot(is_mcycle_boundary);
                    }
                    self.prefetched_opcode = Some(self.cpu_read(fetch_addr));

                    // Advance the EI delay pipeline after the trailing fetch.
                    // IME promotion takes effect at instruction completion but
                    // the sequencer latch (updated during ticks above) doesn't
                    // see it until the NEXT trailing fetch's M-cycle boundary.
                    self.advance_ei_delay();

                    return new_screen;
                }
            };

            // IE push bug: the interrupt controller samples IF & IE
            // between the high-byte and low-byte push writes of interrupt
            // dispatch. The high-byte push may have landed on 0xFFFF (IE),
            // altering which interrupt (if any) is still triggered.
            if processor.take_pending_vector_resolve() {
                if let Some(interrupt) = self.interrupts.triggered() {
                    self.interrupts.clear(interrupt);
                    self.cpu.program_counter = interrupt.vector();
                } else {
                    self.cpu.program_counter = 0x0000;
                }
            }

            // Dot 0: record OAM bug from InternalOamBug actions and
            // from IDU address on bus.
            if dot_in_mcycle == 0 {
                if let DotAction::InternalOamBug { address } = &dot_action {
                    if (0xFE00..=0xFEFF).contains(address) {
                        match pending_oam_bug {
                            Some(OamBugKind::Read) => {}
                            _ => {
                                pending_oam_bug = Some(OamBugKind::Write);
                            }
                        }
                    }
                }
            }

            // Even phase (DELTA_EF): timer tick, DFF latch advance, PPU half_even.
            let is_mcycle_boundary = dot_in_mcycle == 3;
            new_screen |= self.tick_dot_even(is_mcycle_boundary);

            // After dot 2 even tick (before dot 3): fire OAM bug.
            // This preserves the timing where OAM corruption fires
            // after 3 dot ticks within the M-cycle.
            if dot_in_mcycle == 2
                && let Some(kind) = pending_oam_bug.take()
            {
                match kind {
                    OamBugKind::Read => self.ppu.oam_bug_read(),
                    OamBugKind::Write => self.ppu.oam_bug_write(),
                }
            }

            // Bus action (DELTA_GH): CPU read/write routes between phases.
            match dot_action {
                DotAction::Idle => {}
                DotAction::InternalOamBug { .. } => {
                    // Already handled above at dot 0.
                }
                DotAction::Read { address } => {
                    // Detect OAM bug from CPU reads to the OAM region.
                    if (0xFE00..=0xFEFF).contains(&address) {
                        pending_oam_bug = Some(OamBugKind::Read);
                    }
                    read_value = self.cpu_read(address);
                }
                DotAction::Write { address, value } => {
                    // Detect OAM bug from CPU writes to the OAM region.
                    if (0xFE00..=0xFEFF).contains(&address) {
                        pending_oam_bug = Some(OamBugKind::Write);
                    }
                    self.write_byte(address, value);
                }
            }

            // Odd phase (DELTA_HA): PPU half_odd, M-cycle subsystems.
            new_screen |= self.tick_dot_odd(is_mcycle_boundary);

            // Advance dot counter, wrapping at M-cycle boundary.
            dot_in_mcycle = if is_mcycle_boundary {
                0
            } else {
                dot_in_mcycle + 1
            };
        }
    }

    /// Advance exactly one dot (T-cycle). Returns true if a new
    /// frame was produced. The execution state machine tracks where
    /// we are in the instruction lifecycle across calls.
    pub fn step_dot(&mut self) -> bool {
        match std::mem::replace(&mut self.execution, ExecutionState::Ready) {
            ExecutionState::Ready => {
                // Start a new instruction.
                self.interrupt_latch.promote();
                let dispatch_interrupt = self.interrupt_latch.take_ready().is_some();

                let processor = match self.cpu.halt_state {
                    HaltState::Running => {
                        if dispatch_interrupt {
                            Processor::interrupt(&mut self.cpu)
                        } else if let Some(opcode) = self.prefetched_opcode.take() {
                            Processor::fetch_with_opcode(&mut self.cpu, opcode)
                        } else {
                            unreachable!("Running CPU must have a prefetched opcode")
                        }
                    }
                    HaltState::Halted => {
                        if dispatch_interrupt {
                            Processor::interrupt(&mut self.cpu)
                        } else {
                            Processor::begin(&mut self.cpu)
                        }
                    }
                    HaltState::Halting => Processor::begin(&mut self.cpu),
                };

                let was_halted = self.cpu.halt_state == HaltState::Halted;

                self.execution = ExecutionState::Running {
                    processor,
                    read_value: 0,
                    dot_in_mcycle: 0,
                    pending_oam_bug: None,
                    was_halted,
                };

                // Run the first dot of this new instruction.
                self.step_dot()
            }
            ExecutionState::Running {
                mut processor,
                mut read_value,
                mut dot_in_mcycle,
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
                            self.execution = ExecutionState::TrailingFetch {
                                dot: 0,
                                fetch_addr: self.cpu.program_counter,
                                halting: true,
                            };
                            return self.step_dot();
                        }

                        if self.cpu.halt_state == HaltState::Halted {
                            if self.interrupt_latch.take_ready().is_some() {
                                // Restart with interrupt dispatch.
                                self.execution = ExecutionState::Running {
                                    processor: Processor::interrupt(&mut self.cpu),
                                    read_value: 0,
                                    dot_in_mcycle: 0,
                                    pending_oam_bug: None,
                                    was_halted,
                                };
                                return self.step_dot();
                            }
                            // Stay halted, no trailing fetch needed.
                            self.prefetched_opcode = None;
                            self.advance_ei_delay();
                            self.execution = ExecutionState::Ready;
                            return false;
                        }

                        // IME=0 HALT wakeup: wakeup NOP IS the trailing
                        // fetch. read_value is the prefetched opcode.
                        if was_halted && self.cpu.halt_state == HaltState::Running {
                            self.prefetched_opcode = Some(read_value);
                            self.advance_ei_delay();
                            self.execution = ExecutionState::Ready;
                            return false;
                        }

                        // Normal: enter trailing fetch.
                        self.execution = ExecutionState::TrailingFetch {
                            dot: 0,
                            fetch_addr: self.cpu.program_counter,
                            halting: false,
                        };
                        return self.step_dot();
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

                // Dot 0: record OAM bug.
                if dot_in_mcycle == 0 {
                    if let DotAction::InternalOamBug { address } = &dot_action {
                        if (0xFE00..=0xFEFF).contains(address) {
                            match pending_oam_bug {
                                Some(OamBugKind::Read) => {}
                                _ => {
                                    pending_oam_bug = Some(OamBugKind::Write);
                                }
                            }
                        }
                    }
                }

                // Even phase.
                let is_mcycle_boundary = dot_in_mcycle == 3;
                let mut new_screen = self.tick_dot_even(is_mcycle_boundary);

                // After dot 2 even tick: fire OAM bug.
                if dot_in_mcycle == 2 {
                    if let Some(kind) = pending_oam_bug.take() {
                        match kind {
                            OamBugKind::Read => self.ppu.oam_bug_read(),
                            OamBugKind::Write => self.ppu.oam_bug_write(),
                        }
                    }
                }

                // Bus action.
                match dot_action {
                    DotAction::Idle => {}
                    DotAction::InternalOamBug { .. } => {}
                    DotAction::Read { address } => {
                        if (0xFE00..=0xFEFF).contains(&address) {
                            pending_oam_bug = Some(OamBugKind::Read);
                        }
                        read_value = self.cpu_read(address);
                    }
                    DotAction::Write { address, value } => {
                        if (0xFE00..=0xFEFF).contains(&address) {
                            pending_oam_bug = Some(OamBugKind::Write);
                        }
                        self.write_byte(address, value);
                    }
                }

                // Odd phase.
                new_screen |= self.tick_dot_odd(is_mcycle_boundary);

                // Advance dot counter.
                dot_in_mcycle = if is_mcycle_boundary {
                    0
                } else {
                    dot_in_mcycle + 1
                };

                self.execution = ExecutionState::Running {
                    processor,
                    read_value,
                    dot_in_mcycle,
                    pending_oam_bug,
                    was_halted,
                };

                new_screen
            }
            ExecutionState::TrailingFetch {
                dot,
                fetch_addr,
                halting,
            } => {
                let is_mcycle_boundary = dot == 3;
                let new_screen = self.tick_dot(is_mcycle_boundary);

                if is_mcycle_boundary {
                    // Final dot of the trailing fetch M-cycle.
                    let bus_value = self.cpu_read(fetch_addr);
                    if halting {
                        self.cpu.halt_state = HaltState::Halted;
                        self.prefetched_opcode = None;
                    } else {
                        self.prefetched_opcode = Some(bus_value);
                    }
                    self.advance_ei_delay();
                    self.execution = ExecutionState::Ready;
                } else {
                    self.execution = ExecutionState::TrailingFetch {
                        dot: dot + 1,
                        fetch_addr,
                        halting,
                    };
                }

                new_screen
            }
        }
    }

    /// Even phase (DELTA_EF) of one dot: timer tick, DFF latch
    /// advance, PPU half_even.
    fn tick_dot_even(&mut self, is_mcycle_boundary: bool) -> bool {
        // Timer ticks every T-cycle for DIV resolution
        if let Some(interrupt) = self.timers.tcycle(is_mcycle_boundary) {
            self.interrupts.request(interrupt);
        }

        // PPU even phase: DFF latch advance, fetcher control, mode transitions.
        self.ppu.tcycle_even(&self.vram_bus.vram);

        false
    }

    /// Odd phase (DELTA_HA) of one dot: PPU half_odd (pixel output),
    /// M-cycle-rate subsystems (serial, DMA, audio).
    fn tick_dot_odd(&mut self, is_mcycle_boundary: bool) -> bool {
        let mut new_screen = false;

        // PPU odd phase: pixel output, counter increment, interrupt
        // edge detection (M-cycle boundaries only).
        let video_result = self.ppu.tcycle_odd(is_mcycle_boundary, &self.vram_bus.vram);
        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }
        if video_result.request_stat {
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
                    trace.push(BusAccess { address: src_addr, value: byte, kind: BusAccessKind::DmaRead });
                    trace.push(BusAccess { address: dst_addr, value: byte, kind: BusAccessKind::DmaWrite });
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

            // Sequencer interrupt latch: captures IF & IE at each M-cycle
            // boundary, modeling DFF g42 (clocked by CLK9). New captures
            // are Fresh (not yet dispatchable); Ready values from a prior
            // step keep their propagation state so the wakeup NOP's own
            // boundary tick doesn't regress a Ready back to Fresh.
            self.interrupt_latch = match self.cpu.interrupt_master_enable {
                InterruptMasterEnable::Enabled => match self.interrupts.triggered() {
                    Some(interrupt) => match self.interrupt_latch {
                        InterruptLatch::Ready(_) => InterruptLatch::Ready(interrupt),
                        _ => InterruptLatch::Fresh(interrupt),
                    },
                    None => InterruptLatch::Empty,
                },
                InterruptMasterEnable::Disabled => InterruptLatch::Empty,
            };

            // HALT wakeup: even with IME=Disabled, a pending interrupt
            // wakes the CPU from HALT (without dispatching). Setting
            // Running here causes the HaltedNop completion to capture
            // its bus read as the prefetched opcode and return — the
            // wakeup NOP IS the trailing fetch (1 M-cycle total, not 2).
            // check_halt_bug() (which runs after the processor yields
            // None) sees Running and no-ops, avoiding a spurious halt
            // bug flag — the halt bug only fires at HALT entry, not
            // during idle wakeup.
            if self.cpu.halt_state == HaltState::Halted
                && self.cpu.interrupt_master_enable == InterruptMasterEnable::Disabled
                && self.interrupts.triggered().is_some()
            {
                self.cpu.halt_state = HaltState::Running;
            }
        }

        new_screen
    }

    /// Tick hardware for one dot (both phases).
    ///
    /// Convenience wrapper for callers that don't need to route bus
    /// actions between phases (e.g. the trailing fetch loop).
    fn tick_dot(&mut self, is_mcycle_boundary: bool) -> bool {
        let s1 = self.tick_dot_even(is_mcycle_boundary);
        let s2 = self.tick_dot_odd(is_mcycle_boundary);
        s1 || s2
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
            // EI immediately before HALT: IME was promoted by EI's
            // advance_ei_delay, but on real hardware HALT saw IME=0
            // (the DFF pipeline hadn't propagated yet). The halt bug
            // triggers — PC is not incremented. The interrupt will
            // dispatch (IME is Enabled), but returns into HALT.
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
            Some(EiDelay::Pending) => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                Some(EiDelay::Fired)
            }
            Some(EiDelay::Fired) => None,
            None => None,
        };
    }
}
