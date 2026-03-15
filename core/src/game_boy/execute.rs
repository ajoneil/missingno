use super::{
    BusAccess, BusAccessKind, ClockPhase, GameBoy, cpu::mcycle::DotAction, interrupts::Interrupt,
    memory::Bus, ppu,
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
    /// Runs phases until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> bool {
        let mut new_screen = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;
        self.last_read_value = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.cpu.take_instruction_boundary();

        const PHASE_BUDGET: u32 = 400;
        let mut phases_remaining = PHASE_BUDGET;

        loop {
            assert!(
                phases_remaining > 0,
                "step() exceeded {PHASE_BUDGET} phase budget — possible infinite loop in CPU"
            );
            phases_remaining -= 1;

            let ns = self.execute_phase(&mut pending_oam_bug);
            new_screen |= ns;

            // Check for instruction boundary after completing a dot
            // (clock_phase is now Rising = just finished Falling = dot complete)
            if self.clock_phase == ClockPhase::Rising && self.cpu.at_instruction_boundary() {
                break;
            }
        }
        new_screen
    }

    /// Advance exactly one dot (T-cycle). Returns true if a new
    /// frame was produced.
    pub fn step_dot(&mut self) -> bool {
        let mut new_screen = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;

        // Run phases until we complete a dot (return to Rising)
        loop {
            let ns = self.execute_phase(&mut pending_oam_bug);
            new_screen |= ns;
            if self.clock_phase == ClockPhase::Rising {
                break;
            }
        }

        // Consume instruction boundary flag (used by step_traced to detect
        // mid-instruction state). HALT bug and EI delay are handled
        // internally by the CPU state machine.
        self.cpu.take_instruction_boundary();

        new_screen
    }

    /// Execute one phase (half-dot) of hardware. The master clock
    /// alternates Rising → Falling uniformly. Rising starts a new dot;
    /// Falling completes it.
    fn execute_phase(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> bool {
        match self.clock_phase {
            ClockPhase::Rising => self.rise(pending_oam_bug),
            ClockPhase::Falling => self.fall(pending_oam_bug),
        }
    }

    /// Rising phase (first half of dot): advance CPU state machine,
    /// IE push bug, OAM bug recording, PPU rising tick, OAM bug fire.
    fn rise(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> bool {
        let dot_action = self.cpu.next_dot(self.last_read_value);
        self.current_dot_action = dot_action;

        // IE push bug: check after each M-cycle transition.
        if self.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.interrupts.triggered() {
                self.interrupts.clear(interrupt);
                self.cpu.program_counter = interrupt.vector();
            } else {
                self.cpu.program_counter = 0x0000;
            }
        }

        let dot = self.cpu.dot_for_execute();
        self.current_dot = dot;

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

        let is_mcycle_boundary = dot.boga();
        let new_screen = self.tick_dot_rising(is_mcycle_boundary);

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

        self.clock_phase = ClockPhase::Falling;
        new_screen
    }

    /// Falling phase (second half of dot): PPU falling tick,
    /// interrupt latch capture, bus actions.
    fn fall(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> bool {
        let dot = self.current_dot;
        let is_mcycle_boundary = dot.boga();

        let new_screen = self.tick_dot_falling(is_mcycle_boundary);

        // Interrupt latch capture (every dot) — models g42 CLK9-edge sampling.
        // Must run after BOTH phases so it sees all interrupt sources.
        let triggered = self.interrupts.triggered();
        self.cpu.update_interrupt_state(triggered);

        // Bus actions after phase ticks.
        match &self.current_dot_action {
            DotAction::Idle | DotAction::InternalOamBug { .. } => {}
            DotAction::Read { address } => {
                if (0xFE00..=0xFEFF).contains(address) {
                    *pending_oam_bug = Some(OamBugKind::Read);
                }
                self.last_read_value = self.cpu_read(*address);
            }
            DotAction::Write { address, value } => {
                let address = *address;
                let value = *value;
                if (0xFE00..=0xFEFF).contains(&address) {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
                // PPU register writes (DFF8/DFF9) latch at AFAS falling
                // (G→H boundary, end of dot 3). drive_ppu_bus handles
                // the PPU write; write_byte handles everything else.
                if self.drive_ppu_bus(address, value) {
                    self.interrupts.request(Interrupt::VideoStatus);
                }
                self.write_byte(address, value);
            }
        }

        self.clock_phase = ClockPhase::Rising;
        new_screen
    }

    /// Rising phase (DELTA_ODD) of one dot: timer tick, DFF8 palette
    /// latch advance, PPU pixel output pipeline, master clock divider
    /// chain (WUVU/VENA/TALU/LX).
    fn tick_dot_rising(&mut self, is_mcycle_boundary: bool) -> bool {
        let mut new_screen = false;

        // Timer ticks every T-cycle for DIV resolution
        if let Some(interrupt) = self.timers.tcycle(is_mcycle_boundary) {
            self.interrupts.request(interrupt);
        }

        // PPU rising phase: DFF8 palette latches, LCD init, pixel output.
        self.ppu.tcycle_rising(&self.vram_bus.vram);

        // XOTA rising edge (H→A boundary): toggles WUVU/VENA, increments
        // LX, detects scanline boundaries, VBlank IF, LYC comparison.
        let xota_result = self.ppu.tick_xota(is_mcycle_boundary);
        if xota_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }
        if let Some(screen) = xota_result.screen {
            if let Some(sgb) = &mut self.sgb {
                sgb.update_screen(&screen);
            }
            self.screen = screen;
            new_screen = true;
        }

        // SUKO is combinational — check for STAT edge after every phase.
        // Mode 0 (WODU/TARU) fires on the rising phase, so this catches
        // it immediately rather than deferring to the next falling phase.
        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        new_screen
    }

    /// Falling phase (DELTA_EVEN) of one dot: PPU fetcher pipeline,
    /// DFF9 resolve, LCD-off handling, and M-cycle subsystems.
    fn tick_dot_falling(&mut self, is_mcycle_boundary: bool) -> bool {
        let mut new_screen = false;

        // PPU falling phase: fetcher, DFF9, LCD-off.
        let video_result = self
            .ppu
            .tcycle_falling(is_mcycle_boundary, &self.vram_bus.vram);

        // SUKO is combinational — check for STAT edge after every phase.
        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        // LCD-off produces a blank screen.
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
