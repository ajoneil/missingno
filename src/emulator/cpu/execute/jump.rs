use super::OpResult;
use crate::emulator::{
    Cpu, MemoryMapped,
    cpu::{
        Register16,
        cycles::Cycles,
        instructions::{Address, Jump, Stack, jump},
    },
};

impl Cpu {
    pub fn execute_jump(&mut self, instruction: Jump, memory: &MemoryMapped) -> OpResult {
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
                let (address, _) = self.fetch_jump_address(location);

                if self.check_condition(condition) {
                    let result =
                        self.execute_stack(Stack::Push(Register16::ProgramCounter), memory);
                    self.program_counter = address;
                    OpResult(Cycles(6), result.1)
                } else {
                    OpResult::cycles(3)
                }
            }

            Jump::Return(condition) => match condition {
                Some(_) => todo!(),
                None => {
                    self.execute_stack(Stack::Pop(Register16::ProgramCounter), memory);
                    OpResult::cycles(4)
                }
            },

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
