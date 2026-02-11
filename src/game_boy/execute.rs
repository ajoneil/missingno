use super::{
    GameBoy,
    cpu::{
        InterruptMasterEnable,
        instructions::Instruction,
        mcycle::{Processor, TCycle},
    },
    interrupts::Interrupt,
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

        let mut processor = if let Some(interrupt) = self.check_for_interrupt() {
            Processor::interrupt(&mut self.cpu, interrupt, &mut self.mapped)
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
            let opcode = self.mapped.read(self.cpu.program_counter);
            self.cpu.program_counter += 1;
            new_screen |= self.tick_hardware_tcycle();
            new_screen |= self.tick_hardware_tcycle();
            new_screen |= self.tick_hardware_tcycle();

            // Read operand bytes
            let op_count = operand_count(opcode);
            let mut bytes = [opcode, 0, 0];
            for i in 0..op_count {
                new_screen |= self.tick_hardware_tcycle();
                bytes[1 + i as usize] = self.mapped.read(self.cpu.program_counter);
                self.cpu.program_counter += 1;
                new_screen |= self.tick_hardware_tcycle();
                new_screen |= self.tick_hardware_tcycle();
                new_screen |= self.tick_hardware_tcycle();
            }

            // Decode from buffered bytes
            let mut iter = bytes[..1 + op_count as usize].iter().copied();
            let instruction = Instruction::decode(&mut iter).unwrap();
            Processor::new(instruction, &mut self.cpu)
        };

        // Run post-decode T-cycles
        let mut read_value: u8 = 0;
        while let Some(tcycle) = processor.next_tcycle(read_value, &mut self.cpu) {
            match tcycle {
                TCycle::Read { address } => {
                    read_value = self.mapped.read(address);
                }
                TCycle::Write { address, value } => {
                    self.mapped.write_byte(address, value);
                }
                TCycle::Hardware => {}
            }
            new_screen |= self.tick_hardware_tcycle();
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

        if !is_mcycle_boundary {
            return false;
        }

        // Everything else ticks once per M-cycle
        if let Some(dma_transfer_cycles) = &mut self.mapped.dma_transfer_cycles {
            dma_transfer_cycles.0 -= 1;
            if dma_transfer_cycles.0 == 0 {
                self.mapped.dma_transfer_cycles = None;
            }
        }

        if let Some(interrupt) = self.mapped.serial.mcycle() {
            self.mapped.interrupts.request(interrupt);
        }

        let video_result = self.mapped.video.mcycle();
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

        self.mapped.audio.mcycle();
        new_screen
    }

    fn check_for_interrupt(&mut self) -> Option<Interrupt> {
        match self.cpu.interrupt_master_enable {
            InterruptMasterEnable::EnableAfterNextInstruction => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
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
