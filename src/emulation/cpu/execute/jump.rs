use crate::emulation::{
    Cpu,
    cpu::{
        Register16,
        cycles::Cycles,
        instructions::{Address, Jump, jump},
    },
};

impl Cpu {
    pub fn execute_jump(&mut self, jump: Jump) -> Cycles {
        match jump {
            Jump::Jump(condition, location) => {
                if let Some(condition) = condition {
                    todo!()
                }

                match location {
                    jump::Location::Address(address) => match address {
                        Address::Fixed(address) => {
                            self.program_counter = address;
                            Cycles(4)
                        }

                        Address::Relative(offset) => {
                            self.program_counter = match offset {
                                0.. => self.program_counter + offset.abs() as u16,
                                ..0 => self.program_counter - offset.abs() as u16,
                            };
                            Cycles(3)
                        }

                        _ => unreachable!(),
                    },

                    jump::Location::RegisterHl => {
                        self.program_counter = self.get_register16(Register16::Hl);
                        Cycles(1)
                    }
                }
            }
            Jump::Call(condition, location) => todo!(),
            Jump::Return(condition) => todo!(),
            Jump::ReturnAndEnableInterrupts => todo!(),
            Jump::Restart(_) => todo!(),
        }
    }
}
