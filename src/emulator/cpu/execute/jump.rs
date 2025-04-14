use super::OpResult;
use crate::emulator::{
    Cpu,
    cpu::{
        Register16,
        cycles::Cycles,
        instructions::{Address, Jump, Source16, Stack, jump},
    },
};

impl Cpu {
    pub fn execute_jump(&mut self, instruction: Jump) -> OpResult {
        match instruction {
            Jump::Jump(condition, location) => {
                let (address, address_cycles) = self.fetch_jump_address(location);

                if self.check_condition(condition) {
                    self.program_counter = address;
                    OpResult::cycles(1).add_cycles(address_cycles)
                } else {
                    OpResult::cycles(0).add_cycles(address_cycles)
                }
            }
            Jump::Call(condition, location) => {
                let (address, address_cycles) = self.fetch_jump_address(location);

                if self.check_condition(condition) {
                    self.execute_stack(Stack::Push(Source16::pc()));
                    self.program_counter = address;
                    OpResult::cycles(3).add_cycles(address_cycles)
                } else {
                    OpResult::cycles(0).add_cycles(address_cycles)
                }
            }
            Jump::Return(_) => todo!(),
            Jump::ReturnAndEnableInterrupts => todo!(),
            Jump::Restart(_) => todo!(),
        }
    }

    fn fetch_jump_address(&self, location: jump::Location) -> (u16, Cycles) {
        match location {
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
        }
    }

    fn check_condition(&self, condition: Option<jump::Condition>) -> bool {
        if let Some(jump::Condition(flag, value)) = condition {
            self.flags.contains(flag.into()) == value
        } else {
            true
        }
    }
}
