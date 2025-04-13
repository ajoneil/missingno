use super::OpResult;
use crate::emulator::{
    Cpu,
    cpu::{
        Register16,
        cycles::Cycles,
        instructions::{Address, Jump, jump},
    },
};

impl Cpu {
    pub fn execute_jump(&mut self, instruction: Jump) -> OpResult {
        match instruction {
            Jump::Jump(condition, location) => {
                let (address, address_cycles) = match location {
                    jump::Location::Address(address) => match address {
                        Address::Fixed(address) => (address, Cycles(3)),
                        Address::Relative(offset) => (
                            match offset {
                                0.. => self.program_counter + offset.abs() as u16,
                                ..0 => self.program_counter - offset.abs() as u16,
                            },
                            Cycles(2),
                        ),
                        _ => unreachable!(),
                    },

                    jump::Location::RegisterHl => (self.get_register16(Register16::Hl), Cycles(0)),
                };

                let jump = if let Some(jump::Condition(flag, value)) = condition {
                    self.flags.contains(flag.into()) == value
                } else {
                    true
                };

                if jump {
                    self.program_counter = address;
                    OpResult::cycles(1).add_cycles(address_cycles)
                } else {
                    OpResult::cycles(0).add_cycles(address_cycles)
                }
            }
            Jump::Call(_, _) => todo!(),
            Jump::Return(_) => todo!(),
            Jump::ReturnAndEnableInterrupts => todo!(),
            Jump::Restart(_) => todo!(),
        }
    }
}
