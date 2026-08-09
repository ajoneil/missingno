//! Instruction dispatch: algorithmic decode over the octal opcode fields,
//! shared across the base table and the CB/ED/DD/FD/DDCB/FDCB prefixes.

use crate::decode::{AluOp, Fields, INT_MODE, Reg, RotOp};
use crate::{Bus, Cpu};

/// Which register pair the H/L slot and (HL) memory operand resolve to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Index {
    Hl,
    Ix,
    Iy,
}

impl Index {
    fn base(self, cpu: &Cpu) -> u16 {
        match self {
            Index::Hl => cpu.hl(),
            Index::Ix => cpu.ix,
            Index::Iy => cpu.iy,
        }
    }
}

fn reg_at(idx: u8, index: Index) -> Reg {
    match idx {
        0 => Reg::B,
        1 => Reg::C,
        2 => Reg::D,
        3 => Reg::E,
        4 => match index {
            Index::Hl => Reg::H,
            Index::Ix => Reg::IxH,
            Index::Iy => Reg::IyH,
        },
        5 => match index {
            Index::Hl => Reg::L,
            Index::Ix => Reg::IxL,
            Index::Iy => Reg::IyL,
        },
        7 => Reg::A,
        _ => unreachable!(),
    }
}

/// The plain (unsubstituted) register for an operand index.
pub(super) fn real_reg(idx: u8) -> Reg {
    reg_at(idx, Index::Hl)
}

impl Cpu {
    pub(super) fn execute(&mut self, bus: &mut impl Bus, opcode: u8) {
        match opcode {
            0xCB => {
                let sub = self.opcode_fetch(bus);
                self.execute_cb(bus, sub, Index::Hl);
            }
            0xED => {
                let sub = self.opcode_fetch(bus);
                self.execute_ed(bus, sub);
            }
            0xDD => self.execute_prefixed(bus, Index::Ix),
            0xFD => self.execute_prefixed(bus, Index::Iy),
            _ => self.execute_main(bus, opcode, Index::Hl),
        }
    }

    fn execute_prefixed(&mut self, bus: &mut impl Bus, index: Index) {
        // The prefix byte completes as a non-flag-modifying step, clearing
        // the Q shadow that SCF/CCF fold into their X/Y flags.
        self.q = 0;
        let opcode = self.opcode_fetch(bus);
        match opcode {
            0xDD => self.execute_prefixed(bus, Index::Ix),
            0xFD => self.execute_prefixed(bus, Index::Iy),
            0xED => {
                let sub = self.opcode_fetch(bus);
                self.execute_ed(bus, sub);
            }
            0xCB => self.execute_index_cb(bus, index),
            _ => self.execute_main(bus, opcode, index),
        }
    }

    fn get_rp(&self, p: u8, index: Index) -> u16 {
        match p {
            0 => self.bc(),
            1 => self.de(),
            2 => index.base(self),
            _ => self.sp,
        }
    }

    fn set_rp(&mut self, p: u8, index: Index, value: u16) {
        match p {
            0 => self.set_bc(value),
            1 => self.set_de(value),
            2 => match index {
                Index::Hl => self.set_hl(value),
                Index::Ix => self.ix = value,
                Index::Iy => self.iy = value,
            },
            _ => self.sp = value,
        }
    }

    /// The unprefixed table's register pairs — no IX/IY substitution, so the
    /// sequencer never carries an index.
    pub(super) fn pair(&self, p: u8) -> u16 {
        self.get_rp(p, Index::Hl)
    }

    pub(super) fn set_pair(&mut self, p: u8, value: u16) {
        self.set_rp(p, Index::Hl, value);
    }

    pub(super) fn pair2(&self, p: u8) -> u16 {
        self.get_rp2(p, Index::Hl)
    }

    pub(super) fn set_pair2(&mut self, p: u8, value: u16) {
        self.set_rp2(p, Index::Hl, value);
    }

    fn get_rp2(&self, p: u8, index: Index) -> u16 {
        match p {
            3 => self.af(),
            _ => self.get_rp(p, index),
        }
    }

    fn set_rp2(&mut self, p: u8, index: Index, value: u16) {
        match p {
            3 => [self.a, self.f] = value.to_be_bytes(),
            _ => self.set_rp(p, index, value),
        }
    }

    /// Read the signed displacement byte and form (IX/IY + d), updating WZ.
    fn displacement(&mut self, bus: &mut impl Bus, index: Index) -> u16 {
        let d = self.mem_read(bus, self.pc) as i8;
        self.pc = self.pc.wrapping_add(1);
        let address = index.base(self).wrapping_add(d as u16);
        self.wz = address;
        address
    }

    fn imm8(&mut self, bus: &mut impl Bus) -> u8 {
        let value = self.mem_read(bus, self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn imm16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.imm8(bus);
        let hi = self.imm8(bus);
        u16::from_le_bytes([lo, hi])
    }

    fn execute_main(&mut self, bus: &mut impl Bus, opcode: u8, index: Index) {
        let f = Fields::new(opcode);
        match f.x {
            0 => self.exec_x0(bus, f, index),
            1 => self.exec_x1(bus, f, index),
            2 => self.exec_x2(bus, f, index),
            _ => self.exec_x3(bus, f, index),
        }
    }

    fn exec_x0(&mut self, bus: &mut impl Bus, f: Fields, index: Index) {
        match f.z {
            0 => match f.y {
                0 => {}
                1 => {
                    std::mem::swap(&mut self.a, &mut self.a_);
                    std::mem::swap(&mut self.f, &mut self.f_);
                }
                2 => {
                    self.internal(1);
                    let d = self.imm8(bus) as i8;
                    self.b = self.b.wrapping_sub(1);
                    if self.b != 0 {
                        self.internal(5);
                        self.pc = self.pc.wrapping_add(d as u16);
                        self.wz = self.pc;
                    }
                }
                3 => {
                    let d = self.imm8(bus) as i8;
                    self.internal(5);
                    self.pc = self.pc.wrapping_add(d as u16);
                    self.wz = self.pc;
                }
                _ => {
                    let d = self.imm8(bus) as i8;
                    if self.condition(f.y - 4) {
                        self.internal(5);
                        self.pc = self.pc.wrapping_add(d as u16);
                        self.wz = self.pc;
                    }
                }
            },
            1 => {
                if f.q == 0 {
                    let value = self.imm16(bus);
                    self.set_rp(f.p, index, value);
                } else {
                    let base = index.base(self);
                    self.wz = base.wrapping_add(1);
                    let sum = self.add16(base, self.get_rp(f.p, index));
                    self.internal(7);
                    self.set_rp(2, index, sum);
                }
            }
            2 => self.exec_load_group(bus, f, index),
            3 => {
                let value = self.get_rp(f.p, index);
                let delta = if f.q == 0 { 1u16 } else { 0u16.wrapping_sub(1) };
                self.set_rp(f.p, index, value.wrapping_add(delta));
                self.internal(2);
            }
            4 => self.exec_inc_dec(bus, f.y, index, true),
            5 => self.exec_inc_dec(bus, f.y, index, false),
            6 => {
                if f.y == 6 {
                    // LD (IX+d),n reads the displacement before the immediate.
                    let address = match index {
                        Index::Hl => self.hl(),
                        _ => {
                            let d = self.mem_read(bus, self.pc) as i8;
                            self.pc = self.pc.wrapping_add(1);
                            index.base(self).wrapping_add(d as u16)
                        }
                    };
                    let value = self.imm8(bus);
                    if index != Index::Hl {
                        self.wz = address;
                        self.internal(2);
                    }
                    self.mem_write(bus, address, value);
                } else {
                    let value = self.imm8(bus);
                    self.set_reg(reg_at(f.y, index), value);
                }
            }
            _ => match f.y {
                0 => self.rotate_a(RotOp::Rlc),
                1 => self.rotate_a(RotOp::Rrc),
                2 => self.rotate_a(RotOp::Rl),
                3 => self.rotate_a(RotOp::Rr),
                4 => self.daa(),
                5 => self.cpl(),
                6 => self.scf(self.q),
                _ => self.ccf(self.q),
            },
        }
    }

    fn exec_load_group(&mut self, bus: &mut impl Bus, f: Fields, index: Index) {
        if f.q == 0 {
            match f.p {
                0 => {
                    let address = self.bc();
                    self.wz = ((self.a as u16) << 8) | (self.c.wrapping_add(1) as u16);
                    self.mem_write(bus, address, self.a);
                }
                1 => {
                    let address = self.de();
                    self.wz = ((self.a as u16) << 8) | (self.e.wrapping_add(1) as u16);
                    self.mem_write(bus, address, self.a);
                }
                2 => {
                    let address = self.imm16(bus);
                    self.wz = address.wrapping_add(1);
                    let value = index.base(self);
                    let [lo, hi] = value.to_le_bytes();
                    self.mem_write(bus, address, lo);
                    self.mem_write(bus, self.wz, hi);
                }
                _ => {
                    let address = self.imm16(bus);
                    self.wz = ((self.a as u16) << 8) | (address.wrapping_add(1) & 0x00FF);
                    self.mem_write(bus, address, self.a);
                }
            }
        } else {
            match f.p {
                0 => {
                    let address = self.bc();
                    self.wz = address.wrapping_add(1);
                    self.a = self.mem_read(bus, address);
                }
                1 => {
                    let address = self.de();
                    self.wz = address.wrapping_add(1);
                    self.a = self.mem_read(bus, address);
                }
                2 => {
                    let address = self.imm16(bus);
                    self.wz = address.wrapping_add(1);
                    let value = self.read16(bus, address);
                    self.set_rp(2, index, value);
                }
                _ => {
                    let address = self.imm16(bus);
                    self.wz = address.wrapping_add(1);
                    self.a = self.mem_read(bus, address);
                }
            }
        }
    }

    fn exec_x1(&mut self, bus: &mut impl Bus, f: Fields, index: Index) {
        if f.y == 6 && f.z == 6 {
            self.halted = true;
            return;
        }
        // A memory operand suppresses IX/IY half-register substitution on
        // the paired register.
        if f.z == 6 {
            let address = self.mem_hl(bus, index);
            let value = self.mem_read(bus, address);
            self.set_reg(real_reg(f.y), value);
        } else if f.y == 6 {
            let address = self.mem_hl(bus, index);
            let value = self.reg(real_reg(f.z));
            self.mem_write(bus, address, value);
        } else {
            let value = self.reg(reg_at(f.z, index));
            self.set_reg(reg_at(f.y, index), value);
        }
    }

    fn exec_x2(&mut self, bus: &mut impl Bus, f: Fields, index: Index) {
        let value = self.operand_value(bus, f.z, index);
        self.alu(AluOp::from_index(f.y), value);
    }

    fn exec_x3(&mut self, bus: &mut impl Bus, f: Fields, index: Index) {
        match f.z {
            0 => {
                self.internal(1);
                if self.condition(f.y) {
                    let address = self.pop16(bus);
                    self.wz = address;
                    self.pc = address;
                }
            }
            1 => {
                if f.q == 0 {
                    let value = self.pop16(bus);
                    self.set_rp2(f.p, index, value);
                } else {
                    match f.p {
                        0 => {
                            let address = self.pop16(bus);
                            self.wz = address;
                            self.pc = address;
                        }
                        1 => {
                            std::mem::swap(&mut self.b, &mut self.b_);
                            std::mem::swap(&mut self.c, &mut self.c_);
                            std::mem::swap(&mut self.d, &mut self.d_);
                            std::mem::swap(&mut self.e, &mut self.e_);
                            std::mem::swap(&mut self.h, &mut self.h_);
                            std::mem::swap(&mut self.l, &mut self.l_);
                        }
                        2 => self.pc = index.base(self),
                        _ => {
                            self.sp = index.base(self);
                            self.internal(2);
                        }
                    }
                }
            }
            2 => {
                let address = self.imm16(bus);
                self.wz = address;
                if self.condition(f.y) {
                    self.pc = address;
                }
            }
            3 => match f.y {
                0 => {
                    let address = self.imm16(bus);
                    self.wz = address;
                    self.pc = address;
                }
                1 => {
                    let sub = self.opcode_fetch(bus);
                    self.execute_cb(bus, sub, Index::Hl);
                }
                2 => {
                    let port_lo = self.imm8(bus);
                    let port = ((self.a as u16) << 8) | port_lo as u16;
                    self.wz = ((self.a as u16) << 8) | (port_lo.wrapping_add(1) as u16);
                    self.io_write(bus, port, self.a);
                }
                3 => {
                    let port_lo = self.imm8(bus);
                    let port = ((self.a as u16) << 8) | port_lo as u16;
                    self.wz = port.wrapping_add(1);
                    self.a = self.io_read(bus, port);
                }
                4 => {
                    let lo = self.mem_read(bus, self.sp);
                    let hi = self.mem_read(bus, self.sp.wrapping_add(1));
                    self.internal(1);
                    let value = index.base(self);
                    let [vlo, vhi] = value.to_le_bytes();
                    self.mem_write(bus, self.sp.wrapping_add(1), vhi);
                    self.mem_write(bus, self.sp, vlo);
                    let swapped = u16::from_le_bytes([lo, hi]);
                    self.set_rp(2, index, swapped);
                    self.wz = swapped;
                    self.internal(2);
                }
                5 => {
                    std::mem::swap(&mut self.d, &mut self.h);
                    std::mem::swap(&mut self.e, &mut self.l);
                }
                6 => {
                    self.iff1 = false;
                    self.iff2 = false;
                }
                _ => {
                    self.iff1 = true;
                    self.iff2 = true;
                    self.ei_pending = true;
                }
            },
            4 => {
                let address = self.imm16(bus);
                self.wz = address;
                if self.condition(f.y) {
                    self.internal(1);
                    self.push16(bus, self.pc);
                    self.pc = address;
                }
            }
            5 => {
                if f.q == 0 {
                    self.internal(1);
                    let value = self.get_rp2(f.p, index);
                    self.push16(bus, value);
                } else {
                    match f.p {
                        0 => {
                            let address = self.imm16(bus);
                            self.wz = address;
                            self.internal(1);
                            self.push16(bus, self.pc);
                            self.pc = address;
                        }
                        1 => self.execute_prefixed(bus, Index::Ix),
                        2 => {
                            let sub = self.opcode_fetch(bus);
                            self.execute_ed(bus, sub);
                        }
                        _ => self.execute_prefixed(bus, Index::Iy),
                    }
                }
            }
            6 => {
                let value = self.imm8(bus);
                self.alu(AluOp::from_index(f.y), value);
            }
            _ => {
                self.internal(1);
                self.push16(bus, self.pc);
                let target = (f.y as u16) * 8;
                self.pc = target;
                self.wz = target;
            }
        }
    }

    /// The (HL)/(IX+d) address for x=1 loads (displacement adds 5 idle
    /// T-states on the indexed forms).
    fn mem_hl(&mut self, bus: &mut impl Bus, index: Index) -> u16 {
        match index {
            Index::Hl => self.hl(),
            _ => {
                let address = self.displacement(bus, index);
                self.internal(5);
                address
            }
        }
    }

    /// An 8-bit source operand by index (register or memory).
    fn operand_value(&mut self, bus: &mut impl Bus, idx: u8, index: Index) -> u8 {
        if idx == 6 {
            let address = self.mem_hl(bus, index);
            self.mem_read(bus, address)
        } else {
            self.reg(reg_at(idx, index))
        }
    }

    fn exec_inc_dec(&mut self, bus: &mut impl Bus, idx: u8, index: Index, increment: bool) {
        if idx == 6 {
            let address = self.mem_hl(bus, index);
            let value = self.mem_read(bus, address);
            self.internal(1);
            let result = if increment {
                self.inc8(value)
            } else {
                self.dec8(value)
            };
            self.mem_write(bus, address, result);
        } else {
            let reg = reg_at(idx, index);
            let value = self.reg(reg);
            let result = if increment {
                self.inc8(value)
            } else {
                self.dec8(value)
            };
            self.set_reg(reg, result);
        }
    }

    fn execute_cb(&mut self, bus: &mut impl Bus, opcode: u8, _index: Index) {
        let f = Fields::new(opcode);
        if f.z == 6 {
            let address = self.hl();
            let value = self.mem_read(bus, address);
            self.internal(1);
            match f.x {
                0 => {
                    let result = self.rotate(RotOp::from_index(f.y), value);
                    self.mem_write(bus, address, result);
                }
                1 => self.bit(f.y, value, (self.wz >> 8) as u8),
                2 => self.mem_write(bus, address, value & !(1 << f.y)),
                _ => self.mem_write(bus, address, value | (1 << f.y)),
            }
        } else {
            let reg = real_reg(f.z);
            let value = self.reg(reg);
            match f.x {
                0 => {
                    let result = self.rotate(RotOp::from_index(f.y), value);
                    self.set_reg(reg, result);
                }
                1 => self.bit(f.y, value, value),
                2 => self.set_reg(reg, value & !(1 << f.y)),
                _ => self.set_reg(reg, value | (1 << f.y)),
            }
        }
    }

    fn execute_index_cb(&mut self, bus: &mut impl Bus, index: Index) {
        let d = self.mem_read(bus, self.pc) as i8;
        self.pc = self.pc.wrapping_add(1);
        let opcode = self.mem_read(bus, self.pc);
        self.pc = self.pc.wrapping_add(1);
        let address = index.base(self).wrapping_add(d as u16);
        self.wz = address;
        self.internal(2);
        let f = Fields::new(opcode);
        let value = self.mem_read(bus, address);
        if f.x == 1 {
            self.internal(1);
            self.bit(f.y, value, (address >> 8) as u8);
            return;
        }
        self.internal(1);
        let result = match f.x {
            0 => self.rotate(RotOp::from_index(f.y), value),
            2 => value & !(1 << f.y),
            _ => value | (1 << f.y),
        };
        self.mem_write(bus, address, result);
        // The undocumented forms also drop the result into the named register.
        if f.z != 6 {
            self.set_reg(real_reg(f.z), result);
        }
    }

    fn execute_ed(&mut self, bus: &mut impl Bus, opcode: u8) {
        let f = Fields::new(opcode);
        match (f.x, f.z) {
            (1, 0) => {
                let port = self.bc();
                self.wz = port.wrapping_add(1);
                let value = self.io_read(bus, port);
                self.set_input_flags(value);
                if f.y != 6 {
                    self.set_reg(real_reg(f.y), value);
                }
            }
            (1, 1) => {
                let port = self.bc();
                self.wz = port.wrapping_add(1);
                let value = if f.y == 6 { 0 } else { self.reg(real_reg(f.y)) };
                self.io_write(bus, port, value);
            }
            (1, 2) => {
                let hl = self.hl();
                self.wz = hl.wrapping_add(1);
                let rp = self.get_rp(f.p, Index::Hl);
                let result = if f.q == 0 {
                    self.sbc16(hl, rp)
                } else {
                    self.adc16(hl, rp)
                };
                self.internal(7);
                self.set_hl(result);
            }
            (1, 3) => {
                let address = self.imm16(bus);
                self.wz = address.wrapping_add(1);
                if f.q == 0 {
                    let value = self.get_rp(f.p, Index::Hl);
                    let [lo, hi] = value.to_le_bytes();
                    self.mem_write(bus, address, lo);
                    self.mem_write(bus, self.wz, hi);
                } else {
                    let value = self.read16(bus, address);
                    self.set_rp(f.p, Index::Hl, value);
                }
            }
            (1, 4) => {
                let a = self.a;
                self.a = 0;
                self.sub8(a, 0, true);
            }
            (1, 5) => {
                let address = self.pop16(bus);
                self.wz = address;
                self.pc = address;
                self.iff1 = self.iff2;
            }
            (1, 6) => self.im = INT_MODE[(f.y & 3) as usize],
            (1, _) => self.exec_ed_group7(bus, f.y),
            (2, _) if f.y >= 4 && f.z <= 3 => self.exec_block(bus, f),
            _ => {}
        }
    }

    fn exec_ed_group7(&mut self, bus: &mut impl Bus, y: u8) {
        match y {
            0 => {
                self.internal(1);
                self.i = self.a;
            }
            1 => {
                self.internal(1);
                self.r = self.a;
            }
            2 => {
                self.internal(1);
                self.a = self.i;
                self.set_ld_a_ir_flags(self.a, self.iff2);
                self.p = true;
            }
            3 => {
                self.internal(1);
                self.a = self.r;
                self.set_ld_a_ir_flags(self.a, self.iff2);
                self.p = true;
            }
            4 => {
                let hl = self.hl();
                let m = self.mem_read(bus, hl);
                self.internal(4);
                let new_m = (m >> 4) | (self.a << 4);
                self.a = (self.a & 0xF0) | (m & 0x0F);
                self.mem_write(bus, hl, new_m);
                self.set_input_flags(self.a);
                self.wz = hl.wrapping_add(1);
            }
            5 => {
                let hl = self.hl();
                let m = self.mem_read(bus, hl);
                self.internal(4);
                let new_m = (m << 4) | (self.a & 0x0F);
                self.a = (self.a & 0xF0) | (m >> 4);
                self.mem_write(bus, hl, new_m);
                self.set_input_flags(self.a);
                self.wz = hl.wrapping_add(1);
            }
            _ => {}
        }
    }

    fn exec_block(&mut self, bus: &mut impl Bus, f: Fields) {
        let increment = f.y & 1 == 0;
        let step = if increment {
            1u16
        } else {
            0u16.wrapping_sub(1)
        };
        let repeat = f.y >= 6;
        match f.z {
            0 => {
                let value = self.mem_read(bus, self.hl());
                self.mem_write(bus, self.de(), value);
                self.internal(2);
                self.set_hl(self.hl().wrapping_add(step));
                self.set_de(self.de().wrapping_add(step));
                self.set_bc(self.bc().wrapping_sub(1));
                let bc_nonzero = self.bc() != 0;
                self.block_transfer_flags(value, bc_nonzero);
                if repeat && bc_nonzero {
                    self.internal(5);
                    self.pc = self.pc.wrapping_sub(2);
                    self.wz = self.pc.wrapping_add(1);
                    self.repeat_flag_xy();
                }
            }
            1 => {
                let hl = self.hl();
                let value = self.mem_read(bus, hl);
                self.internal(5);
                self.set_hl(hl.wrapping_add(step));
                self.set_bc(self.bc().wrapping_sub(1));
                let bc_nonzero = self.bc() != 0;
                let equal = self.a == value;
                self.block_compare_flags(value, bc_nonzero);
                self.wz = self.wz.wrapping_add(step);
                if repeat && bc_nonzero && !equal {
                    self.internal(5);
                    self.pc = self.pc.wrapping_sub(2);
                    self.wz = self.pc.wrapping_add(1);
                    self.repeat_flag_xy();
                }
            }
            2 => {
                self.internal(1);
                let port = self.bc();
                let value = self.io_read(bus, port);
                self.wz = port.wrapping_add(step);
                self.mem_write(bus, self.hl(), value);
                self.b = self.b.wrapping_sub(1);
                self.set_hl(self.hl().wrapping_add(step));
                let port_term = self.c.wrapping_add(step as u8);
                let repeating = repeat && self.b != 0;
                self.block_io_flags(value, self.b, port_term, repeating);
                if repeating {
                    self.internal(5);
                    self.pc = self.pc.wrapping_sub(2);
                    self.wz = self.pc.wrapping_add(1);
                    self.repeat_flag_xy();
                }
            }
            _ => {
                self.internal(1);
                let value = self.mem_read(bus, self.hl());
                self.b = self.b.wrapping_sub(1);
                let port = self.bc();
                self.io_write(bus, port, value);
                self.set_hl(self.hl().wrapping_add(step));
                self.wz = port.wrapping_add(step);
                let port_term = self.l;
                let repeating = repeat && self.b != 0;
                self.block_io_flags(value, self.b, port_term, repeating);
                if repeating {
                    self.internal(5);
                    self.pc = self.pc.wrapping_sub(2);
                    self.wz = self.pc.wrapping_add(1);
                    self.repeat_flag_xy();
                }
            }
        }
    }
}
