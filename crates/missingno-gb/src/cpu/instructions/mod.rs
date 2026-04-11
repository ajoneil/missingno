use core::fmt;

pub use arithmetic::{Arithmetic, Arithmetic8, Arithmetic16};
pub use bit_flag::BitFlag;
pub use bit_shift::BitShift;
pub use bitwise::Bitwise;
pub use carry_flag::CarryFlag;
pub use interrupt::Interrupt;
pub use jump::Jump;
pub use load::Load;
pub use parameters::*;
pub use stack::Stack;

mod arithmetic;
mod bit_flag;
pub mod bit_shift;
mod bitwise;
mod carry_flag;
mod interrupt;
pub mod jump;
mod load;
mod parameters;
mod stack;

#[derive(Clone)]
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
            0x10 => {
                ops.next()?;
                Self::Stop
            }
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
                    0x00..=0x3f => Self::BitShift(BitShift::decode(op)),
                    _ => Self::BitFlag(BitFlag::decode(op)),
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

/// Returns the total byte length of the instruction at the given opcode byte.
/// This includes the opcode itself (1 byte) plus any operand bytes.
pub fn instruction_length(opcode: u8) -> u16 {
    let operands = match opcode {
        // 1 operand byte
        0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x36 | 0x3e => 1,
        0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => 1,
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => 1,
        0xe0 | 0xf0 => 1,
        0xe8 | 0xf8 => 1,
        0xcb => 1,
        0x10 => 1,
        // 2 operand bytes
        0x01 | 0x11 | 0x21 | 0x31 => 2,
        0x08 | 0xea | 0xfa => 2,
        0xc3 | 0xc2 | 0xca | 0xd2 | 0xda => 2,
        0xcd | 0xc4 | 0xcc | 0xd4 | 0xdc => 2,
        _ => 0,
    };
    1 + operands
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
            Self::Invalid(op) => write!(f, "Invalid op {:02X}", op),
        }
    }
}
