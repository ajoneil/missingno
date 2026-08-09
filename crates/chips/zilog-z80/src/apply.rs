//! Pure register/flag mutations — the operation half of each instruction.

use crate::decode::{AluOp, RotOp};
use crate::{Cpu, flags};

fn parity(value: u8) -> u8 {
    if value.count_ones().is_multiple_of(2) {
        flags::PARITY
    } else {
        0
    }
}

impl Cpu {
    pub(super) fn set_f(&mut self, value: u8) {
        self.f = value;
        self.flags_touched = true;
    }

    fn carry_in(&self) -> u8 {
        self.f & flags::CARRY
    }

    pub(super) fn alu(&mut self, op: AluOp, value: u8) {
        match op {
            AluOp::Add => self.add8(value, 0),
            AluOp::Adc => self.add8(value, self.carry_in()),
            AluOp::Sub => self.sub8(value, 0, true),
            AluOp::Sbc => self.sub8(value, self.carry_in(), true),
            AluOp::And => self.and8(value),
            AluOp::Xor => self.xor8(value),
            AluOp::Or => self.or8(value),
            AluOp::Cp => self.sub8(value, 0, false),
        }
    }

    fn add8(&mut self, value: u8, carry: u8) {
        let a = self.a;
        let wide = a as u16 + value as u16 + carry as u16;
        let result = wide as u8;
        let mut f = result & (flags::SIGN | flags::XY);
        if result == 0 {
            f |= flags::ZERO;
        }
        if (a & 0x0F) + (value & 0x0F) + carry > 0x0F {
            f |= flags::HALF;
        }
        if (!(a ^ value) & (a ^ result) & 0x80) != 0 {
            f |= flags::PARITY;
        }
        if wide > 0xFF {
            f |= flags::CARRY;
        }
        self.a = result;
        self.set_f(f);
    }

    /// Subtract; `store` distinguishes SUB/SBC from CP (CP discards the
    /// result and takes X/Y from the operand instead).
    pub(super) fn sub8(&mut self, value: u8, carry: u8, store: bool) {
        let a = self.a;
        let wide = (a as i16) - (value as i16) - (carry as i16);
        let result = wide as u8;
        let mut f = flags::SUBTRACT | (result & flags::SIGN);
        if result == 0 {
            f |= flags::ZERO;
        }
        if ((a & 0x0F) as i16 - (value & 0x0F) as i16 - carry as i16) & 0x10 != 0 {
            f |= flags::HALF;
        }
        if ((a ^ value) & (a ^ result) & 0x80) != 0 {
            f |= flags::PARITY;
        }
        if wide < 0 {
            f |= flags::CARRY;
        }
        f |= (if store { result } else { value }) & flags::XY;
        if store {
            self.a = result;
        }
        self.set_f(f);
    }

    fn and8(&mut self, value: u8) {
        self.a &= value;
        let a = self.a;
        self.set_f((a & (flags::SIGN | flags::XY)) | flags::HALF | parity(a) | zero(a));
    }

    fn or8(&mut self, value: u8) {
        self.a |= value;
        let a = self.a;
        self.set_f((a & (flags::SIGN | flags::XY)) | parity(a) | zero(a));
    }

    fn xor8(&mut self, value: u8) {
        self.a ^= value;
        let a = self.a;
        self.set_f((a & (flags::SIGN | flags::XY)) | parity(a) | zero(a));
    }

    pub(super) fn inc8(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        let mut f = self.f & flags::CARRY;
        f |= result & (flags::SIGN | flags::XY);
        f |= zero(result);
        if value & 0x0F == 0x0F {
            f |= flags::HALF;
        }
        if value == 0x7F {
            f |= flags::PARITY;
        }
        self.set_f(f);
        result
    }

    pub(super) fn dec8(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        let mut f = (self.f & flags::CARRY) | flags::SUBTRACT;
        f |= result & (flags::SIGN | flags::XY);
        f |= zero(result);
        if value & 0x0F == 0 {
            f |= flags::HALF;
        }
        if value == 0x80 {
            f |= flags::PARITY;
        }
        self.set_f(f);
        result
    }

    pub(super) fn add16(&mut self, lhs: u16, rhs: u16) -> u16 {
        let result = lhs.wrapping_add(rhs);
        let mut f = self.f & (flags::SIGN | flags::ZERO | flags::PARITY);
        f |= ((result >> 8) as u8) & flags::XY;
        if (lhs & 0x0FFF) + (rhs & 0x0FFF) > 0x0FFF {
            f |= flags::HALF;
        }
        if lhs as u32 + rhs as u32 > 0xFFFF {
            f |= flags::CARRY;
        }
        self.set_f(f);
        result
    }

    pub(super) fn adc16(&mut self, lhs: u16, rhs: u16) -> u16 {
        let carry = self.carry_in() as u32;
        let wide = lhs as u32 + rhs as u32 + carry;
        let result = wide as u16;
        let mut f = ((result >> 8) as u8) & (flags::SIGN | flags::XY);
        if result == 0 {
            f |= flags::ZERO;
        }
        if (lhs & 0x0FFF) + (rhs & 0x0FFF) + carry as u16 > 0x0FFF {
            f |= flags::HALF;
        }
        if (!(lhs ^ rhs) & (lhs ^ result) & 0x8000) != 0 {
            f |= flags::PARITY;
        }
        if wide > 0xFFFF {
            f |= flags::CARRY;
        }
        self.set_f(f);
        result
    }

    pub(super) fn sbc16(&mut self, lhs: u16, rhs: u16) -> u16 {
        let carry = self.carry_in() as i32;
        let wide = lhs as i32 - rhs as i32 - carry;
        let result = wide as u16;
        let mut f = flags::SUBTRACT | (((result >> 8) as u8) & (flags::SIGN | flags::XY));
        if result == 0 {
            f |= flags::ZERO;
        }
        if ((lhs & 0x0FFF) as i32 - (rhs & 0x0FFF) as i32 - carry) & 0x1000 != 0 {
            f |= flags::HALF;
        }
        if ((lhs ^ rhs) & (lhs ^ result) & 0x8000) != 0 {
            f |= flags::PARITY;
        }
        if wide < 0 {
            f |= flags::CARRY;
        }
        self.set_f(f);
        result
    }

    /// The CB-prefixed shifts/rotates: full flag set from the result.
    pub(super) fn rotate(&mut self, op: RotOp, value: u8) -> u8 {
        let carry = self.carry_in();
        let (result, carry_out) = match op {
            RotOp::Rlc => (value.rotate_left(1), value >> 7),
            RotOp::Rrc => (value.rotate_right(1), value & 1),
            RotOp::Rl => ((value << 1) | carry, value >> 7),
            RotOp::Rr => ((value >> 1) | (carry << 7), value & 1),
            RotOp::Sla => (value << 1, value >> 7),
            RotOp::Sra => ((value >> 1) | (value & 0x80), value & 1),
            RotOp::Sll => ((value << 1) | 1, value >> 7),
            RotOp::Srl => (value >> 1, value & 1),
        };
        let f = (result & (flags::SIGN | flags::XY)) | zero(result) | parity(result) | carry_out;
        self.set_f(f);
        result
    }

    /// The accumulator rotates (RLCA/RRCA/RLA/RRA): only H, N, C and X/Y
    /// change; S, Z, P/V are preserved.
    pub(super) fn rotate_a(&mut self, op: RotOp) {
        let carry = self.carry_in();
        let value = self.a;
        let (result, carry_out) = match op {
            RotOp::Rlc => (value.rotate_left(1), value >> 7),
            RotOp::Rrc => (value.rotate_right(1), value & 1),
            RotOp::Rl => ((value << 1) | carry, value >> 7),
            RotOp::Rr => ((value >> 1) | (carry << 7), value & 1),
            _ => unreachable!(),
        };
        self.a = result;
        let f = (self.f & (flags::SIGN | flags::ZERO | flags::PARITY))
            | (result & flags::XY)
            | carry_out;
        self.set_f(f);
    }

    pub(super) fn bit(&mut self, index: u8, value: u8, xy_source: u8) {
        let set = value & (1 << index);
        let mut f = (self.f & flags::CARRY) | flags::HALF;
        if set == 0 {
            f |= flags::ZERO | flags::PARITY;
        }
        f |= set & flags::SIGN;
        f |= xy_source & flags::XY;
        self.set_f(f);
    }

    pub(super) fn daa(&mut self) {
        let a = self.a;
        let subtract = self.f & flags::SUBTRACT != 0;
        let mut correction = 0u8;
        let mut carry = false;
        if self.f & flags::HALF != 0 || a & 0x0F > 9 {
            correction |= 0x06;
        }
        if self.f & flags::CARRY != 0 || a > 0x99 {
            correction |= 0x60;
            carry = true;
        }
        let half;
        let result = if subtract {
            half = self.f & flags::HALF != 0 && a & 0x0F < 6;
            a.wrapping_sub(correction)
        } else {
            half = a & 0x0F > 9;
            a.wrapping_add(correction)
        };
        self.a = result;
        let mut f = (self.f & flags::SUBTRACT) | (result & (flags::SIGN | flags::XY));
        f |= zero(result) | parity(result);
        if half {
            f |= flags::HALF;
        }
        if carry {
            f |= flags::CARRY;
        }
        self.set_f(f);
    }

    pub(super) fn cpl(&mut self) {
        self.a = !self.a;
        let f = (self.f & !(flags::XY)) | flags::HALF | flags::SUBTRACT | (self.a & flags::XY);
        self.set_f(f);
    }

    pub(super) fn scf(&mut self, old_q: u8) {
        let xy = ((old_q ^ self.f) | self.a) & flags::XY;
        let f = (self.f & (flags::SIGN | flags::ZERO | flags::PARITY)) | flags::CARRY | xy;
        self.set_f(f);
    }

    pub(super) fn ccf(&mut self, old_q: u8) {
        let carry = self.f & flags::CARRY;
        let xy = ((old_q ^ self.f) | self.a) & flags::XY;
        let mut f = (self.f & (flags::SIGN | flags::ZERO | flags::PARITY)) | xy;
        if carry != 0 {
            f |= flags::HALF;
        } else {
            f |= flags::CARRY;
        }
        self.set_f(f);
    }

    /// The A-relative flag update shared by LDI/LDD, given the byte moved
    /// and whether BC remains non-zero.
    pub(super) fn block_transfer_flags(&mut self, value: u8, bc_nonzero: bool) {
        let n = self.a.wrapping_add(value);
        let mut f = self.f & (flags::CARRY | flags::ZERO | flags::SIGN);
        if bc_nonzero {
            f |= flags::PARITY;
        }
        f |= n & flags::X;
        f |= (n << 4) & flags::Y;
        self.set_f(f);
    }

    /// The flag update shared by CPI/CPD, given the byte compared and
    /// whether BC remains non-zero.
    pub(super) fn block_compare_flags(&mut self, value: u8, bc_nonzero: bool) {
        let result = self.a.wrapping_sub(value);
        let half = (self.a & 0x0F) < (value & 0x0F);
        let n = result.wrapping_sub(half as u8);
        let mut f = (self.f & flags::CARRY) | flags::SUBTRACT;
        f |= result & flags::SIGN;
        f |= zero(result);
        if half {
            f |= flags::HALF;
        }
        if bc_nonzero {
            f |= flags::PARITY;
        }
        f |= n & flags::X;
        f |= (n << 4) & flags::Y;
        self.set_f(f);
    }

    /// The flag update shared by the block-I/O instructions. `port_term`
    /// is (C±1) for IN, or L for OUT, matching each variant's carry math.
    /// `repeat` selects the INIR/OTIR-family H/P corrections (Rak).
    pub(super) fn block_io_flags(&mut self, value: u8, b: u8, port_term: u8, repeat: bool) {
        let t = value as u16 + port_term as u16;
        let carry = t > 0xFF;
        let subtract = value & 0x80 != 0;
        let base_parity = ((t as u8 & 0x07) ^ b).count_ones().is_multiple_of(2);

        let mut f = b & (flags::SIGN | flags::XY);
        f |= zero(b);
        if subtract {
            f |= flags::SUBTRACT;
        }
        if carry {
            f |= flags::CARRY;
        }
        let (half, parity_bit) = if repeat {
            let half = carry
                && if subtract {
                    b & 0x0F == 0
                } else {
                    b & 0x0F == 0x0F
                };
            let delta = if carry {
                if subtract {
                    b.wrapping_sub(1) & 0x07
                } else {
                    b.wrapping_add(1) & 0x07
                }
            } else {
                b & 0x07
            };
            let parity_bit = delta.count_ones().is_multiple_of(2) ^ base_parity ^ true;
            (half, parity_bit)
        } else {
            (carry, base_parity)
        };
        if half {
            f |= flags::HALF;
        }
        if parity_bit {
            f |= flags::PARITY;
        }
        self.set_f(f);
    }

    /// On a repeating LDIR/CPIR-family iteration the X/Y flags are taken
    /// from the high byte of the (re-decremented) PC, not the moved byte.
    pub(super) fn repeat_flag_xy(&mut self) {
        let xy_source = (self.pc >> 8) as u8;
        let f = (self.f & !flags::XY) | (xy_source & flags::XY);
        self.set_f(f);
    }

    /// Flags for IN r,(C) and the RLD/RRD digit rotates: S/Z/X/Y/P from the
    /// result, H and N cleared, carry preserved.
    pub(super) fn set_input_flags(&mut self, value: u8) {
        let f = (self.f & flags::CARRY)
            | (value & (flags::SIGN | flags::XY))
            | zero(value)
            | parity(value);
        self.set_f(f);
    }

    /// Flags for LD A,I and LD A,R: like a load, but P/V reflects IFF2.
    pub(super) fn set_ld_a_ir_flags(&mut self, value: u8, iff2: bool) {
        let mut f = (self.f & flags::CARRY) | (value & (flags::SIGN | flags::XY)) | zero(value);
        if iff2 {
            f |= flags::PARITY;
        }
        self.set_f(f);
    }
}

fn zero(value: u8) -> u8 {
    if value == 0 { flags::ZERO } else { 0 }
}
