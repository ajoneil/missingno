use crate::emulation::{
    Cpu,
    cpu::{cycles::Cycles, instructions::Interrupt},
};

impl Cpu {
    pub fn execute_interrupt(&mut self, instruction: Interrupt) -> Cycles {
        match instruction {
            Interrupt::Enable => {
                self.interrupt_master_enable = true;
                Cycles(1)
            }
            Interrupt::Disable => {
                self.interrupt_master_enable = false;
                Cycles(1)
            }
            Interrupt::Await => todo!(),
        }
    }
}
