use super::{
    GameBoy,
    cpu::{
        InterruptMasterEnable,
        mcycle::{DotAction, Processor},
    },
    interrupts::Interrupt,
    memory::Bus,
    ppu,
};

/// Whether the OAM bug corruption uses the read or write formula.
/// Determined by the CPU operation type, not by the OAM control
/// signals at the moment of the spurious SRAM clock.
enum OamBugKind {
    Read,
    Write,
}

impl GameBoy {
    pub fn step(&mut self) -> bool {
        let mut new_screen = false;
        self.cpu.ei_delay_consumed = false;

        let mut processor = if let Some(_interrupt) = self.check_for_interrupt() {
            Processor::interrupt(&mut self.cpu)
        } else {
            Processor::begin(&mut self.cpu)
        };

        // Run dots. Each M-cycle is 4 dots; the processor yields one
        // DotAction per dot with bus operations at dot 3 (end of M-cycle).
        let mut read_value: u8 = 0;
        let mut vector_resolved = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;
        let mut dot_in_mcycle: u8 = 0;

        // Safety budget: longest instruction is 6 M-cycles = 24 dots
        // (3 fetch + 3 execute). Interrupt dispatch is 5+1 = 24 dots.
        // Budget of 48 dots gives generous margin for debugging.
        const DOT_BUDGET: u32 = 48;
        let mut dots_remaining = DOT_BUDGET;

        loop {
            assert!(dots_remaining > 0, "step() exceeded {DOT_BUDGET} dot budget — possible infinite loop in Processor");
            dots_remaining -= 1;
            let dot_action = match processor.next_dot(read_value, &mut self.cpu) {
                Some(action) => action,
                None => {
                    // Instruction complete. Check for HALT bug: if the
                    // instruction was HALT and an interrupt is pending,
                    // the CPU resumes but skips PC increment on the next
                    // fetch (or rewinds PC for EI+HALT).
                    self.check_halt_bug();
                    return new_screen;
                }
            };

            // IE push bug: resolve the interrupt vector after the
            // high byte push completes but before the low byte push.
            if processor.needs_vector_resolve && !vector_resolved {
                vector_resolved = true;
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

            // Tick hardware (one dot).
            let is_mcycle_boundary = dot_in_mcycle == 3;
            new_screen |= self.tick_dot(is_mcycle_boundary);

            // After dot 2 tick (before dot 3): fire OAM bug.
            // This preserves the timing where OAM corruption fires
            // after 3 dot ticks within the M-cycle.
            if dot_in_mcycle == 2 {
                if let Some(kind) = pending_oam_bug.take() {
                    match kind {
                        OamBugKind::Read => self.ppu.oam_bug_read(),
                        OamBugKind::Write => self.ppu.oam_bug_write(),
                    }
                }
            }

            // Route bus action.
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

            // Advance dot counter, wrapping at M-cycle boundary.
            dot_in_mcycle = if is_mcycle_boundary { 0 } else { dot_in_mcycle + 1 };
        }
    }

    /// Tick hardware for one dot.
    ///
    /// Timer and PPU tick every dot. M-cycle-rate subsystems (serial,
    /// DMA, audio) tick once when `is_mcycle_boundary` is true (every
    /// 4th dot).
    fn tick_dot(&mut self, is_mcycle_boundary: bool) -> bool {
        let mut new_screen = false;

        // Timer ticks every T-cycle for DIV resolution
        if let Some(interrupt) = self.timers.tcycle(is_mcycle_boundary) {
            self.interrupts.request(interrupt);
        }

        // PPU ticks every T-cycle (1 dot per T-cycle); interrupt edge
        // detection and LYC comparison only run on M-cycle boundaries.
        let video_result = self.ppu.tcycle(is_mcycle_boundary, &self.vram_bus.vram);
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
                let oam_addr = match ppu::memory::MappedAddress::map(0xfe00 + dst_offset as u16) {
                    ppu::memory::MappedAddress::Oam(addr) => addr,
                    _ => unreachable!(),
                };
                self.ppu.write_oam(oam_addr, byte);
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

    /// HALT bug: if HALT was just executed with IME=0 and an interrupt
    /// is already pending, the CPU doesn't truly halt. It resumes
    /// immediately but fails to increment PC on the next opcode fetch.
    fn check_halt_bug(&mut self) {
        if !self.cpu.halted || self.interrupts.triggered().is_none() {
            return;
        }
        if self.cpu.interrupt_master_enable == InterruptMasterEnable::Disabled {
            self.cpu.halted = false;
            self.cpu.halt_bug = true;
        } else if self.cpu.ei_delay_consumed {
            // EI immediately before HALT: on real hardware IME was still
            // 0 when HALT checked it, so the halt bug triggers. The
            // interrupt will be dispatched (IME is now Enabled), but the
            // return address must point to HALT so the CPU re-enters
            // halt after the handler. Rewind PC instead of setting
            // halt_bug, which would bleed into the handler's first fetch.
            self.cpu.program_counter -= 1;
            self.cpu.halted = false;
        }
    }

    fn check_for_interrupt(&mut self) -> Option<Interrupt> {
        match self.cpu.interrupt_master_enable {
            InterruptMasterEnable::EnableAfterNextInstruction => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                self.cpu.ei_delay_consumed = true;
                None
            }
            InterruptMasterEnable::Enabled => self.interrupts.triggered(),
            InterruptMasterEnable::Disabled => {
                if self.cpu.halted && self.interrupts.triggered().is_some() {
                    self.cpu.halted = false;
                }
                None
            }
        }
    }
}
