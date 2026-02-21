use super::{
    GameBoy,
    cpu::{
        InterruptMasterEnable,
        instructions::Instruction,
        mcycle::{Processor, TCycle},
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

/// Returns true if the address maps to a PPU register (FF40-FF4B,
/// excluding FF46 which is DMA). Used to determine whether a write
/// needs PPU dot deferral for write conflict timing.
fn is_ppu_register(address: u16) -> bool {
    matches!(address, 0xFF40..=0xFF4B if address != 0xFF46)
}

/// Returns the target address if this opcode's first post-decode
/// action is a write and the address can be determined from the opcode
/// and current register state. Returns `None` if the instruction
/// doesn't write memory, or if the address depends on operand bytes
/// not yet read (e.g. LD [a16], A).
fn opcode_write_address(opcode: u8, cpu: &super::cpu::Cpu) -> Option<u16> {
    use super::cpu::registers::{Register8, Register16};
    match opcode {
        // LD [C], A — target is 0xFF00 + C
        0xE2 => Some(0xFF00 + cpu.get_register8(Register8::C) as u16),
        // LD [HL], r — target is HL
        0x70..=0x75 | 0x77 => Some(cpu.get_register16(Register16::Hl)),
        // LD [HL], d8 — target is HL (has 1 operand, write is step 0)
        0x36 => Some(cpu.get_register16(Register16::Hl)),
        _ => None,
    }
}

/// Returns true if this opcode is a 2-operand instruction whose first
/// post-decode action is a memory write. The target address won't be
/// known until both operands are read.
fn opcode_is_deferred_address_write(opcode: u8) -> bool {
    match opcode {
        // LD [a16], A
        0xEA => true,
        // LD [a16], SP (Write16 — two consecutive writes)
        0x08 => true,
        _ => false,
    }
}

/// Returns the number of operand bytes following a given opcode (0, 1, or 2).
fn operand_count(opcode: u8) -> u8 {
    match opcode {
        // 1 operand byte: LD r,d8 / LD [HL],d8
        0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x36 | 0x3e => 1,
        // 1 operand byte: ALU A,d8
        0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => 1,
        // 1 operand byte: JR e8, JR cc,e8
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => 1,
        // 1 operand byte: LDH [a8],A / LDH A,[a8]
        0xe0 | 0xf0 => 1,
        // 1 operand byte: ADD SP,e8 / LD HL,SP+e8
        0xe8 | 0xf8 => 1,
        // 1 operand byte: CB prefix
        0xcb => 1,
        // 1 operand byte: STOP
        0x10 => 1,

        // 2 operand bytes: LD r16,d16
        0x01 | 0x11 | 0x21 | 0x31 => 2,
        // 2 operand bytes: LD [a16],SP
        0x08 => 2,
        // 2 operand bytes: LD [a16],A / LD A,[a16]
        0xea | 0xfa => 2,
        // 2 operand bytes: JP a16, JP cc,a16
        0xc3 | 0xc2 | 0xca | 0xd2 | 0xda => 2,
        // 2 operand bytes: CALL a16, CALL cc,a16
        0xcd | 0xc4 | 0xcc | 0xd4 | 0xdc => 2,

        // Everything else: 0 operand bytes
        _ => 0,
    }
}

impl GameBoy {
    pub fn step(&mut self) -> bool {
        let mut new_screen = false;
        self.cpu.ei_delay_consumed = false;

        let mut processor = if let Some(_interrupt) = self.check_for_interrupt() {
            Processor::interrupt(&mut self.cpu)
        } else if self.cpu.halted {
            Processor::halted_nop(self.cpu.program_counter)
        } else {
            // Fetch phase: each byte read takes one M-cycle (8 phases).
            // The bus read happens at phase 5 (after 3 dots at phases
            // 0, 2, 4), matching the current tick(3)→bus→tick(1) timing.

            // Read opcode byte
            //
            // Write conflict: tentatively start accumulating PPU dots
            // at T0 of the opcode fetch if the PPU is drawing. This
            // captures T0 in case the instruction turns out to be a
            // 0-operand PPU register write (like LD [HL],r or LD [C],A),
            // giving 4 fetch dots + 1 write T0 = 5 pending for conflict
            // splitting. Cancelled after reading the opcode if not needed.
            let tentative = self.ppu.ppu_is_drawing();
            if tentative {
                self.ppu.start_accumulating();
            }
            // Phases 0–4: tick 3 dots (at phases 0, 2, 4)
            for _ in 0..5 {
                new_screen |= self.tick_hardware_phase();
            }
            // Phase 5: bus read (after 3 dots)
            let opcode = self.cpu_read(self.cpu.program_counter);
            if self.cpu.halt_bug {
                self.cpu.halt_bug = false;
            } else {
                self.cpu.program_counter += 1;
            }
            let op_count = operand_count(opcode);

            // Determine whether this instruction's first post-decode
            // action writes to a PPU register.
            let known_ppu_write =
                opcode_write_address(opcode, &self.cpu).is_some_and(|addr| is_ppu_register(addr));
            let deferred_addr_write = opcode_is_deferred_address_write(opcode);
            let defer_for_write = known_ppu_write || deferred_addr_write;

            if tentative && !(op_count == 0 && known_ppu_write) {
                // Not a 0-operand PPU write — cancel tentative
                // accumulation and flush the captured T0 dot.
                self.ppu.stop_accumulating_and_flush(&self.vram_bus.vram);
            }
            // Phases 5–7: tick 1 more dot (at phase 6, with M-cycle boundary)
            for _ in 5..8 {
                new_screen |= self.tick_hardware_phase();
            }

            // Read operand bytes
            let mut bytes = [opcode, 0, 0];
            for i in 0..op_count {
                if i == op_count - 1 && defer_for_write {
                    // Last operand M-cycle: start accumulating from
                    // T0 so the full M-cycle (4 dots) is deferred.
                    self.ppu.start_accumulating();
                }
                // Phases 0–4: tick 3 dots
                for _ in 0..5 {
                    new_screen |= self.tick_hardware_phase();
                }
                // Phase 5: bus read (after 3 dots)
                bytes[1 + i as usize] = self.cpu_read(self.cpu.program_counter);
                self.cpu.program_counter += 1;
                // Phases 5–7: tick 1 more dot (with M-cycle boundary)
                for _ in 5..8 {
                    new_screen |= self.tick_hardware_phase();
                }
            }

            // For deferred-address writes, check the actual target now
            // that operands have been read. Cancel accumulation if the
            // target isn't a PPU register.
            if deferred_addr_write {
                let target = (bytes[2] as u16) << 8 | bytes[1] as u16;
                if !is_ppu_register(target) {
                    self.ppu.stop_accumulating_and_flush(&self.vram_bus.vram);
                }
            }

            // Decode from buffered bytes
            let mut iter = bytes[..1 + op_count as usize].iter().copied();
            let instruction = Instruction::decode(&mut iter).unwrap();
            let processor = Processor::new(instruction, &mut self.cpu);

            // HALT bug: if HALT was just executed with IME=0 and an
            // interrupt is already pending, the CPU doesn't truly halt.
            // It resumes immediately but fails to increment PC on the
            // next opcode fetch.
            if self.cpu.halted && self.interrupts.triggered().is_some() {
                if self.cpu.interrupt_master_enable == InterruptMasterEnable::Disabled {
                    self.cpu.halted = false;
                    self.cpu.halt_bug = true;
                } else if self.cpu.ei_delay_consumed {
                    // EI immediately before HALT: on real hardware IME
                    // was still 0 when HALT checked it, so the halt bug
                    // triggers. The interrupt will be dispatched (IME is
                    // now Enabled), but the return address must point to
                    // HALT so the CPU re-enters halt after the handler.
                    // Rewind PC (incremented during fetch) instead of
                    // setting halt_bug, which would bleed into the
                    // interrupt handler's first fetch.
                    self.cpu.program_counter -= 1;
                    self.cpu.halted = false;
                }
            }

            processor
        };

        // Run post-decode M-cycles. Each M-cycle is 8 clock phases.
        // The processor yields one T-cycle per dot phase (phases 0, 2,
        // 4, 6 — 4 yields per M-cycle). Bus ops execute at phase 4
        // (after 2 dots), OAM bug fires at phase 5.
        let mut read_value: u8 = 0;
        let mut vector_resolved = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;

        loop {
            // Collect one M-cycle (8 phases), deferring bus action.
            let mut deferred_bus_action: Option<TCycle> = None;

            for phase_in_mcycle in 0u8..8 {
                // Yield a T-cycle from the processor on dot phases (even).
                if phase_in_mcycle & 1 == 0 {
                    let tcycle = match processor.next_tcycle(read_value, &mut self.cpu) {
                        Some(t) => t,
                        None => {
                            // Instruction complete at M-cycle boundary.
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

                    // Record bus action for deferred execution; detect OAM
                    // bug address at the point the action is yielded.
                    match &tcycle {
                        TCycle::Read { address } => {
                            if (0xFE00..=0xFEFF).contains(address) {
                                pending_oam_bug = Some(OamBugKind::Read);
                            }
                            deferred_bus_action = Some(tcycle);
                        }
                        TCycle::Write { address, .. } => {
                            if (0xFE00..=0xFEFF).contains(address) {
                                pending_oam_bug = Some(OamBugKind::Write);
                            }
                            deferred_bus_action = Some(tcycle);
                        }
                        TCycle::Hardware => {}
                    }
                }

                // Phase 0: IDU OAM bug check + write conflict lookahead.
                if phase_in_mcycle == 0 {
                    // IDU OAM bug: record the pending corruption kind.
                    if let Some(addr) = processor.oam_bug_address() {
                        if (0xFE00..=0xFEFF).contains(&addr) {
                            match pending_oam_bug {
                                Some(OamBugKind::Read) => {}
                                _ => {
                                    pending_oam_bug = Some(OamBugKind::Write);
                                }
                            }
                        }
                    }

                    // Write conflict: check if the NEXT M-cycle will write
                    // a PPU register. Start accumulating dots so this
                    // M-cycle's 4 dots are deferred.
                    if let Some(addr) = processor.peek_next_write_address() {
                        if is_ppu_register(addr) && self.ppu.ppu_is_drawing() {
                            self.ppu.start_accumulating();
                        }
                    }
                }

                // Phase 5: OAM bug fires (after 3 dots at phases 0, 2, 4).
                // Preserves current timing where OAM bug applies at T3
                // after T0+T1+T2 ticks.
                if phase_in_mcycle == 5 {
                    if let Some(kind) = pending_oam_bug.take() {
                        match kind {
                            OamBugKind::Read => self.ppu.oam_bug_read(),
                            OamBugKind::Write => self.ppu.oam_bug_write(),
                        }
                    }
                }

                // Phase 4: bus execution (after 2 dots at phases 0, 2).
                // Hardware: writes latch at G→H, reads capture at H→A —
                // both after 2 dots in the M-cycle.
                if phase_in_mcycle == 4 {
                    match deferred_bus_action.take() {
                        Some(TCycle::Write { address, value }) => {
                            self.write_byte(address, value);
                        }
                        Some(TCycle::Read { address }) => {
                            read_value = self.cpu_read(address);
                        }
                        _ => {}
                    }
                }

                new_screen |= self.tick_hardware_phase();
            }
        }
    }

    /// Advance hardware by one clock phase.
    ///
    /// Each M-cycle has 8 phases (0–7). PPU and timer tick on even
    /// phases (0, 2, 4, 6 — one dot per T-cycle, 4 per M-cycle).
    /// M-cycle-rate subsystems (serial, DMA, audio) tick once when the
    /// M-cycle boundary fires.
    ///
    /// Phase assignments:
    ///   Phases 0, 2:     dot ticks (T1, T2)
    ///   Phase 4:         bus action window (writes then reads, after 2 dots)
    ///   Phases 4, 6:     dot ticks (T3, T4; phase 6 also fires M-cycle boundary)
    ///   Phase 5:         OAM bug
    ///   Phases 1, 3, 7:  non-dot phases
    fn tick_hardware_phase(&mut self) -> bool {
        let phase = self.phase_counter;
        self.phase_counter = (self.phase_counter + 1) & 7;

        let is_dot = phase & 1 == 0; // phases 0, 2, 4, 6
        let is_mcycle_boundary = phase == 6; // 4th dot, same as old counter wrapping to 0

        let mut new_screen = false;

        if is_dot {
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
