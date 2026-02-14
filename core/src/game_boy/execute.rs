use super::{
    EXTERNAL_BUS_DECAY_MCYCLES, GameBoy,
    cpu::{
        InterruptMasterEnable,
        instructions::Instruction,
        mcycle::{Processor, TCycle},
    },
    interrupts::Interrupt,
    memory::Bus,
    video,
};

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
            // Fetch phase: each byte read is 4 T-cycles with read at T2
            //   T1: tick hardware
            //   T2: read byte + tick hardware
            //   T3: tick hardware
            //   T4: tick hardware

            // Read opcode byte
            new_screen |= self.tick_hardware_tcycle();
            let opcode = self.mapped.cpu_read(self.cpu.program_counter);
            if self.cpu.halt_bug {
                // HALT bug: PC fails to increment on the fetch after HALT
                // exits with IME=0 and a pending interrupt, causing this
                // byte to be read twice (once now, once as the next opcode).
                self.cpu.halt_bug = false;
            } else {
                self.cpu.program_counter += 1;
            }
            new_screen |= self.tick_hardware_tcycle();
            new_screen |= self.tick_hardware_tcycle();
            new_screen |= self.tick_hardware_tcycle();

            // Read operand bytes
            let op_count = operand_count(opcode);
            let mut bytes = [opcode, 0, 0];
            for i in 0..op_count {
                new_screen |= self.tick_hardware_tcycle();
                bytes[1 + i as usize] = self.mapped.cpu_read(self.cpu.program_counter);
                self.cpu.program_counter += 1;
                new_screen |= self.tick_hardware_tcycle();
                new_screen |= self.tick_hardware_tcycle();
                new_screen |= self.tick_hardware_tcycle();
            }

            // Decode from buffered bytes
            let mut iter = bytes[..1 + op_count as usize].iter().copied();
            let instruction = Instruction::decode(&mut iter).unwrap();
            let processor = Processor::new(instruction, &mut self.cpu);

            // HALT bug: if HALT was just executed with IME=0 and an
            // interrupt is already pending, the CPU doesn't truly halt.
            // It resumes immediately but fails to increment PC on the
            // next opcode fetch.
            if self.cpu.halted && self.mapped.interrupts.triggered().is_some() {
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

        // Run post-decode T-cycles
        let mut read_value: u8 = 0;
        let mut vector_resolved = false;
        let mut tcycle_in_mcycle: u8 = 0;
        while let Some(tcycle) = processor.next_tcycle(read_value, &mut self.cpu) {
            // IE push bug: resolve the interrupt vector after the high byte
            // push completes but before the low byte push. The high byte
            // write may have modified IE (at 0xFFFF), changing which
            // interrupt is pending.
            if processor.needs_vector_resolve && !vector_resolved {
                vector_resolved = true;
                if let Some(interrupt) = self.mapped.interrupts.triggered() {
                    self.mapped.interrupts.clear(interrupt);
                    self.cpu.program_counter = interrupt.vector();
                } else {
                    self.cpu.program_counter = 0x0000;
                }
            }

            match tcycle {
                TCycle::Read { address } => {
                    self.mapped.oam_bug_read(address);
                    read_value = self.mapped.cpu_read(address);
                }
                TCycle::Write { address, value } => {
                    self.mapped.oam_bug_write(address);
                    self.mapped.write_byte(address, value);
                }
                TCycle::Hardware => {}
            }
            new_screen |= self.tick_hardware_tcycle();

            // OAM bug: IDU placing address on bus during internal cycle.
            // Check after the first tick of the M-cycle so the PPU state
            // reflects the current dot (matching fetch-phase tick ordering).
            if tcycle_in_mcycle == 0 {
                if let Some(addr) = processor.oam_bug_address() {
                    self.mapped.oam_bug_write(addr);
                }
            }
            tcycle_in_mcycle += 1;
            if tcycle_in_mcycle == 4 {
                tcycle_in_mcycle = 0;
            }
        }

        new_screen
    }

    /// Advance hardware by one T-cycle.
    ///
    /// Timers tick every T-cycle (DIV increments by 1 each time) so that
    /// reads/writes landing at different T-cycle offsets observe different
    /// DIV values. Overflow/reload processing only happens at M-cycle
    /// boundaries (every 4th T-cycle).
    ///
    /// Other subsystems (video, audio, serial, DMA) still tick once per
    /// M-cycle on the 4th T-cycle.
    fn tick_hardware_tcycle(&mut self) -> bool {
        self.mcycle_counter = self.mcycle_counter.wrapping_add(1) & 3;
        let is_mcycle_boundary = self.mcycle_counter == 0;

        // Timer ticks every T-cycle for DIV resolution
        if let Some(interrupt) = self.mapped.timers.tcycle(is_mcycle_boundary) {
            self.mapped.interrupts.request(interrupt);
        }

        // PPU ticks every T-cycle (1 dot per T-cycle); interrupt edge
        // detection and LYC comparison only run on M-cycle boundaries.
        let video_result = self.mapped.video.tcycle(is_mcycle_boundary);
        if video_result.request_vblank {
            self.mapped
                .interrupts
                .request(Interrupt::VideoBetweenFrames);
        }
        if video_result.request_stat {
            self.mapped.interrupts.request(Interrupt::VideoStatus);
        }

        let mut new_screen = false;
        if let Some(screen) = video_result.screen {
            if let Some(sgb) = &mut self.mapped.sgb {
                sgb.update_screen(&screen);
            }
            self.screen = screen;
            new_screen = true;
        }

        if !is_mcycle_boundary {
            return new_screen;
        }

        // Serial ticks once per M-cycle, using falling edges of the
        // internal counter's bit 7 to drive the serial shift clock.
        let counter = self.mapped.timers.internal_counter();
        if let Some(interrupt) = self.mapped.serial.mcycle(counter) {
            self.mapped.interrupts.request(interrupt);
        }

        // OAM DMA: transfer one byte per M-cycle. The DMA controller
        // drives the source bus with the byte it reads, updating the
        // bus latch so that CPU reads from the same bus see this value.
        if let Some((src_addr, dst_offset)) = self.mapped.dma.mcycle() {
            let byte = self.mapped.read_dma_source(src_addr);
            let dst = video::memory::MappedAddress::map(0xfe00 + dst_offset as u16);
            self.mapped.video.write_memory(dst, byte);
            match Bus::of(src_addr) {
                Some(Bus::External) => {
                    self.mapped.external_bus = byte;
                    self.mapped.external_bus_decay = EXTERNAL_BUS_DECAY_MCYCLES;
                }
                Some(Bus::Vram) => {
                    self.mapped.vram_bus = byte;
                }
                None => {}
            }
        }

        // External bus decay: with no device driving the bus, the
        // retained value trends toward 0xFF as parasitic capacitance
        // discharges.
        if self.mapped.external_bus_decay > 0 {
            self.mapped.external_bus_decay -= 1;
            if self.mapped.external_bus_decay == 0 {
                self.mapped.external_bus = 0xFF;
            }
        }

        self.mapped
            .audio
            .mcycle(self.mapped.timers.internal_counter());
        new_screen
    }

    fn check_for_interrupt(&mut self) -> Option<Interrupt> {
        match self.cpu.interrupt_master_enable {
            InterruptMasterEnable::EnableAfterNextInstruction => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                self.cpu.ei_delay_consumed = true;
                None
            }
            InterruptMasterEnable::Enabled => self.mapped.interrupts.triggered(),
            InterruptMasterEnable::Disabled => {
                if self.cpu.halted && self.mapped.interrupts.triggered().is_some() {
                    self.cpu.halted = false;
                }
                None
            }
        }
    }
}
