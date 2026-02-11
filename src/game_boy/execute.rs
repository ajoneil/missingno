use super::{
    GameBoy,
    cpu::{
        InterruptMasterEnable,
        instructions::Instruction,
        mcycle::{BusAction, Processor},
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
            // Read opcode byte and tick hardware
            let opcode = self.mapped.read(self.cpu.program_counter);
            self.cpu.program_counter += 1;
            new_screen |= self.tick_hardware_once();

            // Read operand bytes, ticking hardware after each
            let op_count = operand_count(opcode);
            let mut bytes = [opcode, 0, 0];
            for i in 0..op_count {
                bytes[1 + i as usize] = self.mapped.read(self.cpu.program_counter);
                self.cpu.program_counter += 1;
                new_screen |= self.tick_hardware_once();
            }

            // Decode from buffered bytes
            let mut iter = bytes[..1 + op_count as usize].iter().copied();
            let instruction = Instruction::decode(&mut iter).unwrap();
            Processor::new(instruction, &mut self.cpu)
        };

        // Run post-decode M-cycles
        let mut read_value: u8 = 0;
        while let Some(action) = processor.next(read_value, &mut self.cpu) {
            match action {
                BusAction::Read { address } => {
                    read_value = self.mapped.read(address);
                }
                BusAction::Write { address, value } => {
                    self.mapped.write_byte(address, value);
                    read_value = 0;
                }
                BusAction::Internal => {
                    read_value = 0;
                }
            }
            new_screen |= self.tick_hardware_once();
        }
        new_screen
    }

    fn tick_hardware_once(&mut self) -> bool {
        if let Some(dma_transfer_cycles) = &mut self.mapped.dma_transfer_cycles {
            dma_transfer_cycles.0 -= 1;
            if dma_transfer_cycles.0 == 0 {
                self.mapped.dma_transfer_cycles = None;
            }
        }

        if let Some(interrupt) = self.mapped.timers.tick() {
            self.mapped.interrupts.request(interrupt);
        }

        if let Some(interrupt) = self.mapped.serial.tick() {
            self.mapped.interrupts.request(interrupt);
        }

        let video_result = self.mapped.video.tick();
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

        self.mapped.audio.tick();
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
