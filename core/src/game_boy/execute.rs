use super::{
    BusAccess, BusAccessKind, GameBoy,
    cpu::mcycle::{BusDot, DotAction},
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

        // If step_dot() left us mid-instruction, drain to the next
        // boundary first, then run one full instruction.
        let mut new_screen = false;
        if !self.cpu.at_instruction_boundary() {
            new_screen |= self.step_instruction();
        }
        new_screen |= self.step_instruction();

        let trace = self.bus_trace.take().unwrap_or_default();
        (new_screen, trace)
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Runs dots until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> bool {
        let mut new_screen = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;
        let mut read_value: u8 = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.cpu.take_instruction_boundary();

        const DOT_BUDGET: u32 = 200;
        let mut dots_remaining = DOT_BUDGET;

        loop {
            assert!(
                dots_remaining > 0,
                "step() exceeded {DOT_BUDGET} dot budget — possible infinite loop in CPU"
            );
            dots_remaining -= 1;

            let dot_action = self.cpu.next_dot(read_value);

            // IE push bug: check after each M-cycle transition.
            if self.cpu.take_pending_vector_resolve() {
                if let Some(interrupt) = self.interrupts.triggered() {
                    self.interrupts.clear(interrupt);
                    self.cpu.program_counter = interrupt.vector();
                } else {
                    self.cpu.program_counter = 0x0000;
                }
            }

            let (new_screen_dot, new_read_value) = self.execute_dot(
                &dot_action,
                self.cpu.dot_for_execute(),
                &mut pending_oam_bug,
            );
            new_screen |= new_screen_dot;
            if let Some(v) = new_read_value {
                read_value = v;
            }

            // Check for the next instruction boundary.
            // HALT bug check and EI delay advance are handled internally
            // by the CPU state machine at the exact transition point.
            if self.cpu.at_instruction_boundary() {
                break;
            }
        }
        new_screen
    }

    /// Advance exactly one dot (T-cycle). Returns true if a new
    /// frame was produced.
    pub fn step_dot(&mut self) -> bool {
        let mut pending_oam_bug: Option<OamBugKind> = None;
        let read_value = self.last_read_value;

        let dot_action = self.cpu.next_dot(read_value);

        // IE push bug
        if self.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.interrupts.triggered() {
                self.interrupts.clear(interrupt);
                self.cpu.program_counter = interrupt.vector();
            } else {
                self.cpu.program_counter = 0x0000;
            }
        }

        let dot = self.cpu.dot_for_execute();
        let (new_screen, new_read_value) = self.execute_dot(&dot_action, dot, &mut pending_oam_bug);
        if let Some(v) = new_read_value {
            self.last_read_value = v;
        }

        // Consume instruction boundary flag (used by step_traced to detect
        // mid-instruction state). HALT bug and EI delay are handled
        // internally by the CPU state machine.
        self.cpu.take_instruction_boundary();

        new_screen
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
        let triggered = self.interrupts.triggered();
        self.cpu.update_interrupt_state(triggered);

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
            // Serial ticks once per M-cycle.
            let counter = self.timers.internal_counter();
            if let Some(interrupt) = self.serial.mcycle(counter) {
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

        new_screen
    }
}
