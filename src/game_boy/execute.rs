use super::{
    GameBoy,
    cpu::{
        InterruptMasterEnable,
        instructions::Instruction,
        mcycle::{BusAction, InstructionStepper},
    },
    interrupts::Interrupt,
};

impl Iterator for GameBoy {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.mapped.read(self.cpu.program_counter);
        self.cpu.program_counter += 1;
        Some(value)
    }
}

impl GameBoy {
    pub fn step(&mut self) -> bool {
        let mut stepper = if let Some(interrupt) = self.check_for_interrupt() {
            InstructionStepper::interrupt(&mut self.cpu, interrupt, &mut self.mapped)
        } else if self.cpu.halted {
            InstructionStepper::halted_nop(self.cpu.program_counter)
        } else {
            let instruction = Instruction::decode(self).unwrap();
            InstructionStepper::new(instruction, &mut self.cpu)
        };

        let mut new_screen = false;
        let mut read_value: u8 = 0;
        while let Some(action) = stepper.next(read_value, &mut self.cpu) {
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
