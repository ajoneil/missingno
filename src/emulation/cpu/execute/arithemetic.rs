use crate::emulation::{
    Cpu,
    cpu::{
        cycles::Cycles,
        instructions::{Arithmetic, Arithmetic8},
    },
};

impl Cpu {
    pub fn execute_arithmetic(&mut self, arithmetic: Arithmetic) -> Cycles {
        match arithmetic {
            Arithmetic::Arithmetic8(arithmetic8) => match arithmetic8 {
                Arithmetic8::Increment(target8) => todo!(),
                Arithmetic8::Decrement(target8) => todo!(),
                Arithmetic8::AddA(source8) => todo!(),
                Arithmetic8::SubtractA(source8) => todo!(),
                Arithmetic8::AddACarry(source8) => todo!(),
                Arithmetic8::SubtractACarry(source8) => todo!(),
                Arithmetic8::CompareA(source8) => todo!(),
            },
            Arithmetic::Arithmetic16(arithmetic16) => todo!(),
        }
    }
}
