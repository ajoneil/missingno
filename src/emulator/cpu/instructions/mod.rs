use core::fmt;

mod arithmetic;
mod bit_flag;
mod bit_shift;
mod bitwise;
mod carry_flag;
mod interrupt;
pub mod jump;
mod load;
mod parameters;
mod stack;

pub use parameters::*;

pub use arithmetic::{Arithmetic, Arithmetic8};
pub use bit_flag::BitFlag;
pub use bit_shift::BitShift;
pub use bitwise::Bitwise;
pub use carry_flag::CarryFlag;
pub use interrupt::Interrupt;
pub use jump::Jump;
pub use load::Load;
pub use stack::Stack;

pub enum Instruction {
    Load(Load),
    Arithmetic(Arithmetic),
    Bitwise(Bitwise),
    BitFlag(BitFlag),
    BitShift(BitShift),
    Jump(Jump),
    CarryFlag(CarryFlag),
    Stack(Stack),
    Interrupt(Interrupt),
    DecimalAdjustAccumulator,
    NoOperation,
    Stop,
    Invalid(u8),
}

impl Instruction {
    pub fn decode(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        let op = ops.next()?;

        Some(match op {
            0x00 => Self::NoOperation,
            0x10 => Self::Stop,
            0x07 => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Left,
                bit_shift::Carry::SetOnly,
            )),
            0x17 => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Left,
                bit_shift::Carry::Through,
            )),
            0x27 => Self::DecimalAdjustAccumulator,
            0x37 => Self::CarryFlag(CarryFlag::Set),
            0x0f => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Right,
                bit_shift::Carry::SetOnly,
            )),
            0x1f => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Right,
                bit_shift::Carry::Through,
            )),
            0x3f => Self::CarryFlag(CarryFlag::Complement),
            0x76 => Self::Interrupt(Interrupt::Await),
            0xf3 => Self::Interrupt(Interrupt::Disable),
            0xfb => Self::Interrupt(Interrupt::Enable),
            0xcb => {
                let op = ops.next()?;
                match op {
                    0x00..=0x3f => Self::BitFlag(BitFlag::decode(op)),
                    _ => Self::BitShift(BitShift::decode(op)),
                }
            }
            _ => {
                if let Some(load) = Load::decode(op, ops) {
                    Self::Load(load)
                } else if let Some(arithmetic) = Arithmetic::decode(op, ops) {
                    Self::Arithmetic(arithmetic)
                } else if let Some(bitwise) = Bitwise::decode(op, ops) {
                    Self::Bitwise(bitwise)
                } else if let Some(jump) = Jump::decode(op, ops) {
                    Self::Jump(jump)
                } else if let Some(stack) = Stack::decode(op, ops) {
                    Self::Stack(stack)
                } else {
                    Self::Invalid(op)
                }
            }
        })
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(load) => load.fmt(f),
            Self::Arithmetic(arithmetic) => arithmetic.fmt(f),
            Self::Bitwise(bitwise) => bitwise.fmt(f),
            Self::BitFlag(bit_flag) => bit_flag.fmt(f),
            Self::BitShift(bit_shift) => bit_shift.fmt(f),
            Self::Jump(jump) => jump.fmt(f),
            Self::CarryFlag(carry_flag) => carry_flag.fmt(f),
            Self::Stack(stack) => stack.fmt(f),
            Self::Interrupt(interrupt) => interrupt.fmt(f),
            Self::DecimalAdjustAccumulator => write!(f, "daa"),
            Self::NoOperation => write!(f, "nop"),
            Self::Stop => write!(f, "stop"),
            Self::Invalid(op) => write!(f, "Invalid op {:02x}", op),
        }
    }
}
