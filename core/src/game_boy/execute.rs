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
            // (clock is Low = just finished fall() = dot complete)
            if self.clock_phase == ClockPhase::Low && self.cpu.at_instruction_boundary() {
                break;
            }
        }
        new_screen
    }

    /// Advance exactly one half-phase — execute rise() or fall()
    /// depending on current clock level. Returns true if a new frame
    /// was produced.
    pub fn step_phase(&mut self) -> bool {
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
            let ns = self.execute_phase(&mut pending_oam_bug);
            new_screen |= ns;
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
    fn execute_phase(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> bool {
        match self.clock_phase {
            ClockPhase::Low => self.rise(pending_oam_bug),
            ClockPhase::High => self.fall(pending_oam_bug),
        }
    }

    /// Rising edge: advance CPU state machine, capture bus reads,
    /// tick timer and PPU, fire OAM bugs.
    ///
    /// At M-cycle boundaries, the PPU rising phase and interrupt capture
    /// run BEFORE the CPU's M-cycle transition (`next_dot`), so that the
    /// CPU's dispatch check (`take_ready()` inside `mcycle_fetch` /
    /// `mcycle_halted`) sees interrupts fired at this boundary. This
    /// models the hardware's combinational path where the g42 DFF
    /// samples `IF & IE` at the same clock edge that the PPU fires IF.
    fn rise(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> bool {
        let mut new_screen = false;
        let is_mcycle_boundary = !self.cpu.mcycle_active;

        // ── M-cycle boundary: PPU + interrupt capture BEFORE CPU transition ──
        if is_mcycle_boundary {
            // PPU rising phase at the M-cycle boundary (dot 0).
            let ppu_result = self.ppu.rise(&self.vram_bus.vram);
            if ppu_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }
            if let Some(screen) = ppu_result.screen {
                if let Some(sgb) = &mut self.sgb {
                    sgb.update_screen(&screen);
                }
                self.screen = screen;
                new_screen = true;
            }

            // SUKO is combinational — check for STAT edge after PPU rise.
            if self.ppu.check_stat_edge() {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            // Capture interrupt state so the CPU's dispatch check sees it.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);

            // g42 DFF: sample IF & IE at the M-cycle boundary for HALT wakeup.
            // On hardware, g42 latches at the BOGA edge. Only matters for the
            // halted path — running-mode dispatch checks IF directly.
            // Save the previous g42 state before updating — if g42 was already
            // pending, the pipeline has propagated during the idle M-cycle.
            self.cpu.g42_was_pending = self.cpu.g42_interrupt_pending;
            self.cpu.g42_interrupt_pending = self.cpu.interrupt_pending;
        }

        // ── CPU dot advance ──
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

        // ── Non-boundary dots: PPU rise + interrupt capture AFTER CPU dot advance ──
        if !is_mcycle_boundary {
            // PPU rising phase for non-boundary dots.
            let ppu_result = self.ppu.rise(&self.vram_bus.vram);
            if ppu_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }
            if let Some(screen) = ppu_result.screen {
                if let Some(sgb) = &mut self.sgb {
                    sgb.update_screen(&screen);
                }
                self.screen = screen;
                new_screen = true;
            }

            // SUKO is combinational — check for STAT edge after PPU rise.
            if self.ppu.check_stat_edge() {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            // Capture interrupt state for non-boundary dots.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
        }

        // CPU data latch: capture bus value on the rising edge, after
        // PPU rise so the read sees the current PPU mode (for OAM/VRAM
        // blocking). The timer tick is on the falling edge, so reads
        // naturally see the pre-increment counter value.
        if let DotAction::Read { address } = &self.current_dot_action {
            if (0xFE00..=0xFEFF).contains(address) {
                *pending_oam_bug = Some(OamBugKind::Read);
            }
            self.last_read_value = self.cpu_read(*address);
        }

        // g151: CLK9-clocked DFF delays timer overflow → IF by 1 dot.
        // Drain at every rising edge so that overflow detected at fall()
        // is visible to update_interrupt_state in the next fall().
        if let Some(interrupt) = self.timers.take_pending_interrupt() {
            self.interrupts.request(interrupt);
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

        self.clock_phase = ClockPhase::High;
        new_screen
    }

    /// Falling edge: PPU falling phase, interrupt latch capture,
    /// bus writes, M-cycle subsystems (serial, DMA, audio).
    fn fall(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> bool {
        let mut new_screen = false;
        let dot = self.current_dot;
        let is_mcycle_boundary = dot.boga();

        // PPU falling phase: fetcher, DFF8/DFF9, LCD-off.
        let video_result = self.ppu.fall(is_mcycle_boundary, &self.vram_bus.vram);

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

        // Bus writes on the falling edge.
        match &self.current_dot_action {
            DotAction::Idle | DotAction::InternalOamBug { .. } | DotAction::Read { .. } => {}
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

        if is_mcycle_boundary {
            // Timer ticks once per M-cycle (BOGA). On the falling edge
            // so that bus writes (e.g. DIV reset) take effect before
            // the counter increments. Overflow sets g151_pending; the
            // interrupt is drained on the next CLK9 rising edge.
            self.timers.mcycle();

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

        self.clock_phase = ClockPhase::Low;
        new_screen
    }
}
