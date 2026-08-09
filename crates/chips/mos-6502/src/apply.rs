//! Pure register/flag mutations — the operation half of each instruction.

use super::decode::{Flag, Op};
use super::{Cpu, flags};

/// A|X for ANE, A for LXA: the open-bus term in the unstable illegals.
/// Analog-dependent on real silicon; this constant matches the
/// SingleStepTests reference captures.
const UNSTABLE_MAGIC: u8 = 0xEE;

impl Cpu {
    pub(super) fn set_zn(&mut self, value: u8) {
        self.set_flag(flags::ZERO, value == 0);
        self.set_flag(flags::NEGATIVE, value & 0x80 != 0);
    }

    pub(super) fn flag(&self, mask: u8) -> bool {
        self.p & mask != 0
    }

    pub(super) fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.p |= mask;
        } else {
            self.p &= !mask;
        }
    }

    pub(super) fn branch_taken(&self, flag: Flag, expected: bool) -> bool {
        let set = match flag {
            Flag::Carry => self.flag(flags::CARRY),
            Flag::Zero => self.flag(flags::ZERO),
            Flag::Negative => self.flag(flags::NEGATIVE),
            Flag::Overflow => self.flag(flags::OVERFLOW),
        };
        set == expected
    }

    fn adc(&mut self, value: u8) {
        let carry_in = self.flag(flags::CARRY) as u16;
        let binary = self.a as u16 + value as u16 + carry_in;
        if self.flag(flags::DECIMAL) && self.decimal_enabled {
            // NMOS decimal: Z from the binary sum; N/V from the
            // pre-correction high nibble; C from the corrected result.
            let mut lo = (self.a & 0x0F) as u16 + (value & 0x0F) as u16 + carry_in;
            if lo > 9 {
                lo += 6;
            }
            let mut hi = (self.a >> 4) as u16 + (value >> 4) as u16 + if lo > 0x0F { 1 } else { 0 };
            self.set_flag(flags::ZERO, binary & 0xFF == 0);
            self.set_flag(flags::NEGATIVE, hi & 0x08 != 0);
            let pre = (hi << 4) as u8;
            self.set_flag(
                flags::OVERFLOW,
                (self.a ^ value) & 0x80 == 0 && (self.a ^ pre) & 0x80 != 0,
            );
            if hi > 9 {
                hi += 6;
            }
            self.set_flag(flags::CARRY, hi > 0x0F);
            self.a = ((hi << 4) as u8) | (lo & 0x0F) as u8;
        } else {
            let result = binary as u8;
            self.set_flag(flags::CARRY, binary > 0xFF);
            self.set_flag(
                flags::OVERFLOW,
                (self.a ^ result) & (value ^ result) & 0x80 != 0,
            );
            self.a = result;
            self.set_zn(result);
        }
    }

    fn sbc(&mut self, value: u8) {
        let borrow = !self.flag(flags::CARRY) as i16;
        let binary = self.a as i16 - value as i16 - borrow;
        let result = binary as u8;
        // NMOS decimal SBC: all flags from the binary difference.
        self.set_flag(flags::CARRY, binary >= 0);
        self.set_flag(
            flags::OVERFLOW,
            (self.a ^ value) & (self.a ^ result) & 0x80 != 0,
        );
        self.set_zn(result);
        if self.flag(flags::DECIMAL) && self.decimal_enabled {
            let mut lo = (self.a & 0x0F) as i16 - (value & 0x0F) as i16 - borrow;
            let mut hi = (self.a >> 4) as i16 - (value >> 4) as i16;
            if lo < 0 {
                lo -= 6;
                hi -= 1;
            }
            if hi < 0 {
                hi -= 6;
            }
            self.a = (((hi as u8) << 4) & 0xF0) | (lo as u8 & 0x0F);
        } else {
            self.a = result;
        }
    }

    fn compare(&mut self, register: u8, value: u8) {
        let diff = register.wrapping_sub(value);
        self.set_flag(flags::CARRY, register >= value);
        self.set_zn(diff);
    }

    /// Operations consuming a read operand (or an immediate).
    pub(super) fn apply_read(&mut self, op: Op, value: u8) {
        match op {
            Op::Lda => {
                self.a = value;
                self.set_zn(value);
            }
            Op::Ldx => {
                self.x = value;
                self.set_zn(value);
            }
            Op::Ldy => {
                self.y = value;
                self.set_zn(value);
            }
            Op::Lax => {
                self.a = value;
                self.x = value;
                self.set_zn(value);
            }
            Op::Adc => self.adc(value),
            Op::Sbc => self.sbc(value),
            Op::And => {
                self.a &= value;
                self.set_zn(self.a);
            }
            Op::Ora => {
                self.a |= value;
                self.set_zn(self.a);
            }
            Op::Eor => {
                self.a ^= value;
                self.set_zn(self.a);
            }
            Op::Cmp => self.compare(self.a, value),
            Op::Cpx => self.compare(self.x, value),
            Op::Cpy => self.compare(self.y, value),
            Op::Bit => {
                self.set_flag(flags::ZERO, self.a & value == 0);
                self.set_flag(flags::NEGATIVE, value & 0x80 != 0);
                self.set_flag(flags::OVERFLOW, value & 0x40 != 0);
            }
            Op::Anc => {
                self.a &= value;
                self.set_zn(self.a);
                self.set_flag(flags::CARRY, self.a & 0x80 != 0);
            }
            Op::Alr => {
                let and = self.a & value;
                self.set_flag(flags::CARRY, and & 0x01 != 0);
                self.a = and >> 1;
                self.set_zn(self.a);
            }
            Op::Arr => self.arr(value),
            Op::Ane => {
                self.a = (self.a | UNSTABLE_MAGIC) & self.x & value;
                self.set_zn(self.a);
            }
            Op::Lxa => {
                let result = (self.a | UNSTABLE_MAGIC) & value;
                self.a = result;
                self.x = result;
                self.set_zn(result);
            }
            Op::Sbx => {
                let and = self.a & self.x;
                self.set_flag(flags::CARRY, and >= value);
                self.x = and.wrapping_sub(value);
                self.set_zn(self.x);
            }
            Op::Las => {
                let result = value & self.s;
                self.a = result;
                self.x = result;
                self.s = result;
                self.set_zn(result);
            }
            Op::Nop => {}
            _ => unreachable!("not a read op: {op:?}"),
        }
    }

    fn arr(&mut self, value: u8) {
        let and = self.a & value;
        let carry_in = self.flag(flags::CARRY) as u8;
        let rotated = (and >> 1) | (carry_in << 7);
        self.set_zn(rotated);
        self.set_flag(flags::OVERFLOW, (and ^ rotated) & 0x40 != 0);
        if self.flag(flags::DECIMAL) && self.decimal_enabled {
            let mut result = rotated;
            if (and & 0x0F) + (and & 0x01) > 5 {
                result = (result & 0xF0) | (result.wrapping_add(6) & 0x0F);
            }
            let high_adjust = (and >> 4) + ((and >> 4) & 1) > 5;
            self.set_flag(flags::CARRY, high_adjust);
            if high_adjust {
                result = result.wrapping_add(0x60);
            }
            self.a = result;
        } else {
            self.set_flag(flags::CARRY, rotated & 0x40 != 0);
            self.a = rotated;
        }
    }

    /// Read-modify-write mutation: old memory value in, new value out.
    /// The combined illegals also fold their ALU half into the registers.
    pub(super) fn apply_rmw(&mut self, op: Op, old: u8) -> u8 {
        match op {
            Op::Asl => {
                self.set_flag(flags::CARRY, old & 0x80 != 0);
                let new = old << 1;
                self.set_zn(new);
                new
            }
            Op::Lsr => {
                self.set_flag(flags::CARRY, old & 0x01 != 0);
                let new = old >> 1;
                self.set_zn(new);
                new
            }
            Op::Rol => {
                let carry_in = self.flag(flags::CARRY) as u8;
                self.set_flag(flags::CARRY, old & 0x80 != 0);
                let new = (old << 1) | carry_in;
                self.set_zn(new);
                new
            }
            Op::Ror => {
                let carry_in = self.flag(flags::CARRY) as u8;
                self.set_flag(flags::CARRY, old & 0x01 != 0);
                let new = (old >> 1) | (carry_in << 7);
                self.set_zn(new);
                new
            }
            Op::Inc => {
                let new = old.wrapping_add(1);
                self.set_zn(new);
                new
            }
            Op::Dec => {
                let new = old.wrapping_sub(1);
                self.set_zn(new);
                new
            }
            Op::Dcp => {
                let new = old.wrapping_sub(1);
                self.compare(self.a, new);
                new
            }
            Op::Isc => {
                let new = old.wrapping_add(1);
                self.sbc(new);
                new
            }
            Op::Slo => {
                self.set_flag(flags::CARRY, old & 0x80 != 0);
                let new = old << 1;
                self.a |= new;
                self.set_zn(self.a);
                new
            }
            Op::Rla => {
                let carry_in = self.flag(flags::CARRY) as u8;
                self.set_flag(flags::CARRY, old & 0x80 != 0);
                let new = (old << 1) | carry_in;
                self.a &= new;
                self.set_zn(self.a);
                new
            }
            Op::Sre => {
                self.set_flag(flags::CARRY, old & 0x01 != 0);
                let new = old >> 1;
                self.a ^= new;
                self.set_zn(self.a);
                new
            }
            Op::Rra => {
                let carry_in = self.flag(flags::CARRY) as u8;
                self.set_flag(flags::CARRY, old & 0x01 != 0);
                let new = (old >> 1) | (carry_in << 7);
                self.adc(new);
                new
            }
            _ => unreachable!("not an rmw op: {op:?}"),
        }
    }

    /// Two-cycle implied/accumulator instructions.
    pub(super) fn apply_implied(&mut self, op: Op) {
        match op {
            Op::Tax => {
                self.x = self.a;
                self.set_zn(self.x);
            }
            Op::Tay => {
                self.y = self.a;
                self.set_zn(self.y);
            }
            Op::Txa => {
                self.a = self.x;
                self.set_zn(self.a);
            }
            Op::Tya => {
                self.a = self.y;
                self.set_zn(self.a);
            }
            Op::Tsx => {
                self.x = self.s;
                self.set_zn(self.x);
            }
            Op::Txs => self.s = self.x,
            Op::Inx => {
                self.x = self.x.wrapping_add(1);
                self.set_zn(self.x);
            }
            Op::Iny => {
                self.y = self.y.wrapping_add(1);
                self.set_zn(self.y);
            }
            Op::Dex => {
                self.x = self.x.wrapping_sub(1);
                self.set_zn(self.x);
            }
            Op::Dey => {
                self.y = self.y.wrapping_sub(1);
                self.set_zn(self.y);
            }
            Op::Clc => self.set_flag(flags::CARRY, false),
            Op::Sec => self.set_flag(flags::CARRY, true),
            Op::Cli => self.set_flag(flags::INTERRUPT_DISABLE, false),
            Op::Sei => self.set_flag(flags::INTERRUPT_DISABLE, true),
            Op::Clv => self.set_flag(flags::OVERFLOW, false),
            Op::Cld => self.set_flag(flags::DECIMAL, false),
            Op::Sed => self.set_flag(flags::DECIMAL, true),
            Op::Nop => {}
            _ => unreachable!("not an implied op: {op:?}"),
        }
    }
}
