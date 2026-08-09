//! The two nested machines that walk an instruction one T-state at a time:
//! a machine-cycle runner carrying each cycle's fixed per-T schedule, and an
//! instruction plan above it, consulted only at cycle boundaries.

use crate::decode::{AluOp, Fields, Reg, RotOp};
use crate::execute::real_reg;
use crate::{Bus, Cpu, Pins};

/// One machine cycle. Each kind owns the per-T-state bus snapshots it records
/// and the T at which its pins assert.
#[derive(Clone, Copy)]
pub(crate) enum Cycle {
    OpcodeFetch,
    MemRead { address: u16 },
    MemWrite { address: u16, data: u8 },
    IoRead { port: u16 },
    IoWrite { port: u16, data: u8 },
    Internal { length: u8 },
}

impl Cycle {
    fn length(self) -> u8 {
        match self {
            Cycle::OpcodeFetch | Cycle::IoRead { .. } | Cycle::IoWrite { .. } => 4,
            Cycle::MemRead { .. } | Cycle::MemWrite { .. } => 3,
            Cycle::Internal { length } => length,
        }
    }
}

/// Where the instruction is: M1 in flight, or a chosen body part-way through
/// its machine cycles.
#[derive(Clone, Copy)]
enum Plan {
    OpcodeFetch,
    Body { body: Body, step: u8 },
}

/// The instruction's remainder after M1, grouped by execution shape rather
/// than by opcode.
#[derive(Clone, Copy)]
enum Body {
    /// The whole effect landed at dispatch — M1 is the instruction.
    Complete,
    /// Internal T-states trailing an effect applied at dispatch.
    InternalPad {
        length: u8,
    },
    LoadImmediate {
        dest: Reg,
    },
    AluImmediate {
        op: AluOp,
    },
    LoadPairImmediate {
        pair: u8,
    },
    StoreImmediate {
        address: u16,
    },
    Load {
        address: u16,
        dest: Reg,
    },
    Store {
        address: u16,
        data: u8,
    },
    AluIndirect {
        address: u16,
        op: AluOp,
    },
    ReadModifyWrite {
        address: u16,
        increment: bool,
    },
    LoadAccumulatorAbsolute,
    LoadPairAbsolute,
    StoreAccumulatorAbsolute,
    StorePairAbsolute,
    JumpRelative {
        taken: bool,
    },
    DecrementAndJump,
    Jump {
        taken: bool,
    },
    Call {
        taken: bool,
    },
    Push {
        value: u16,
        jump: Option<u16>,
    },
    Pop {
        dest: PopDest,
        lead: u8,
    },
    PortImmediate {
        write: bool,
    },
    ExchangeStackTop,
}

#[derive(Clone, Copy)]
enum PopDest {
    Pair(u8),
    ProgramCounter,
}

/// The in-flight instruction: one machine cycle, its T index, the plan above
/// it, and the scratch the plan carries between cycles.
#[derive(Clone, Copy)]
pub(crate) struct Sequencer {
    cycle: Cycle,
    t: u8,
    plan: Plan,
    /// The byte the in-flight read cycle latched.
    latched: u8,
    /// The low byte of a two-cycle read, held while the high byte arrives.
    held: u8,
    /// The instruction's immediate operand or effective address.
    operand: u16,
}

impl Sequencer {
    pub(crate) fn fetching() -> Self {
        Sequencer {
            cycle: Cycle::OpcodeFetch,
            t: 0,
            plan: Plan::OpcodeFetch,
            latched: 0,
            held: 0,
            operand: 0,
        }
    }

    fn take_low(&mut self) {
        self.operand = self.latched as u16;
    }

    fn take_high(&mut self) {
        self.operand |= (self.latched as u16) << 8;
    }
}

/// The opcodes the sequencer does not yet plan: the prefixes, whose remainders
/// run through the atomic executor, and HALT, whose re-fetch loop does.
fn unsequenced(opcode: u8) -> bool {
    matches!(opcode, 0xCB | 0xDD | 0xED | 0xFD | 0x76)
}

impl Cpu {
    /// Advance the in-flight machine cycle by one T-state: record its bus
    /// snapshot, and fire the bus call on the T whose pins assert.
    fn tick_cycle(&mut self, bus: &mut impl Bus, cycle: Cycle, t: u8, latch: &mut u8) {
        match cycle {
            Cycle::OpcodeFetch => match t {
                0 => self.record(self.pc, None, Pins::IDLE),
                1 => {
                    let pc = self.pc;
                    self.record(pc, None, Pins::MEM_READ);
                    *latch = bus.read(pc);
                    self.pc = pc.wrapping_add(1);
                }
                2 => self.record(self.refresh_address(), Some(*latch), Pins::IDLE),
                _ => {
                    self.record(self.refresh_address(), None, Pins::IDLE);
                    self.inc_r();
                }
            },
            Cycle::MemRead { address } => match t {
                0 => self.record(address, None, Pins::IDLE),
                1 => {
                    self.record(address, None, Pins::MEM_READ);
                    *latch = bus.read(address);
                }
                _ => self.record(address, Some(*latch), Pins::IDLE),
            },
            Cycle::MemWrite { address, data } => match t {
                0 => self.record(address, None, Pins::IDLE),
                1 => {
                    self.record(address, Some(data), Pins::MEM_WRITE);
                    bus.write(address, data);
                }
                _ => self.record(address, None, Pins::IDLE),
            },
            Cycle::IoRead { port } => match t {
                0 | 1 => self.record(port, None, Pins::IDLE),
                2 => {
                    self.record(port, None, Pins::IO_READ);
                    *latch = bus.input(port);
                }
                _ => self.record(port, Some(*latch), Pins::IDLE),
            },
            Cycle::IoWrite { port, data } => match t {
                0 | 1 => self.record(port, None, Pins::IDLE),
                2 => {
                    self.record(port, Some(data), Pins::IO_WRITE);
                    bus.output(port, data);
                }
                _ => self.record(port, None, Pins::IDLE),
            },
            Cycle::Internal { .. } => self.internal_tick(),
        }
    }

    /// Run every T-state of one machine cycle back to back, returning the byte
    /// a read cycle latched. The atomic executor's machine cycles and the
    /// sequencer's therefore share one schedule.
    pub(super) fn run_cycle(&mut self, bus: &mut impl Bus, cycle: Cycle) -> u8 {
        let mut latch = 0;
        for t in 0..cycle.length() {
            self.tick_cycle(bus, cycle, t, &mut latch);
        }
        latch
    }

    /// Advance the sequenced instruction by one T-state. Returns false once it
    /// has retired.
    pub(super) fn tick_sequencer(&mut self, bus: &mut impl Bus, seq: &mut Sequencer) -> bool {
        self.tick_cycle(bus, seq.cycle, seq.t, &mut seq.latched);
        seq.t += 1;
        if seq.t < seq.cycle.length() {
            return true;
        }
        match self.next_cycle(bus, seq) {
            Some(cycle) => {
                seq.cycle = cycle;
                seq.t = 0;
                true
            }
            None => false,
        }
    }

    /// At a machine-cycle boundary: apply what the completed cycle enables and
    /// choose the next one, or `None` to retire the instruction.
    fn next_cycle(&mut self, bus: &mut impl Bus, seq: &mut Sequencer) -> Option<Cycle> {
        match seq.plan {
            Plan::OpcodeFetch => {
                let opcode = seq.latched;
                if unsequenced(opcode) {
                    self.execute(bus, opcode);
                    return None;
                }
                seq.plan = Plan::Body {
                    body: self.plan_instruction(opcode),
                    step: 0,
                };
                self.next_cycle(bus, seq)
            }
            Plan::Body { body, step } => {
                seq.plan = Plan::Body {
                    body,
                    step: step + 1,
                };
                self.run_body(seq, body, step)
            }
        }
    }

    /// Schedule the machine cycle at `step` of `body`, first applying the
    /// effects the preceding cycle enabled.
    fn run_body(&mut self, seq: &mut Sequencer, body: Body, step: u8) -> Option<Cycle> {
        match body {
            Body::Complete => None,
            Body::InternalPad { length } => match step {
                0 => Some(Cycle::Internal { length }),
                _ => None,
            },
            Body::LoadImmediate { dest } => match step {
                0 => Some(self.read_at_pc()),
                _ => {
                    self.set_reg(dest, seq.latched);
                    None
                }
            },
            Body::AluImmediate { op } => match step {
                0 => Some(self.read_at_pc()),
                _ => {
                    self.alu(op, seq.latched);
                    None
                }
            },
            Body::LoadPairImmediate { pair } => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                _ => {
                    seq.take_high();
                    self.set_pair(pair, seq.operand);
                    None
                }
            },
            Body::StoreImmediate { address } => match step {
                0 => Some(self.read_at_pc()),
                1 => Some(Cycle::MemWrite {
                    address,
                    data: seq.latched,
                }),
                _ => None,
            },
            Body::Load { address, dest } => match step {
                0 => Some(Cycle::MemRead { address }),
                _ => {
                    self.set_reg(dest, seq.latched);
                    None
                }
            },
            Body::Store { address, data } => match step {
                0 => Some(Cycle::MemWrite { address, data }),
                _ => None,
            },
            Body::AluIndirect { address, op } => match step {
                0 => Some(Cycle::MemRead { address }),
                _ => {
                    self.alu(op, seq.latched);
                    None
                }
            },
            Body::ReadModifyWrite { address, increment } => match step {
                0 => Some(Cycle::MemRead { address }),
                1 => Some(Cycle::Internal { length: 1 }),
                2 => {
                    let data = if increment {
                        self.inc8(seq.latched)
                    } else {
                        self.dec8(seq.latched)
                    };
                    Some(Cycle::MemWrite { address, data })
                }
                _ => None,
            },
            Body::LoadAccumulatorAbsolute => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                2 => {
                    seq.take_high();
                    self.wz = seq.operand.wrapping_add(1);
                    Some(Cycle::MemRead {
                        address: seq.operand,
                    })
                }
                _ => {
                    self.a = seq.latched;
                    None
                }
            },
            Body::LoadPairAbsolute => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                2 => {
                    seq.take_high();
                    self.wz = seq.operand.wrapping_add(1);
                    Some(Cycle::MemRead {
                        address: seq.operand,
                    })
                }
                3 => {
                    seq.held = seq.latched;
                    Some(Cycle::MemRead { address: self.wz })
                }
                _ => {
                    self.set_pair(2, u16::from_le_bytes([seq.held, seq.latched]));
                    None
                }
            },
            Body::StoreAccumulatorAbsolute => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                2 => {
                    seq.take_high();
                    self.wz = ((self.a as u16) << 8) | (seq.operand.wrapping_add(1) & 0x00FF);
                    Some(Cycle::MemWrite {
                        address: seq.operand,
                        data: self.a,
                    })
                }
                _ => None,
            },
            Body::StorePairAbsolute => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                2 => {
                    seq.take_high();
                    self.wz = seq.operand.wrapping_add(1);
                    Some(Cycle::MemWrite {
                        address: seq.operand,
                        data: self.l,
                    })
                }
                3 => Some(Cycle::MemWrite {
                    address: self.wz,
                    data: self.h,
                }),
                _ => None,
            },
            Body::JumpRelative { taken } => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    if !taken {
                        return None;
                    }
                    seq.operand = seq.latched as i8 as u16;
                    Some(Cycle::Internal { length: 5 })
                }
                _ => {
                    self.pc = self.pc.wrapping_add(seq.operand);
                    self.wz = self.pc;
                    None
                }
            },
            Body::DecrementAndJump => match step {
                0 => Some(Cycle::Internal { length: 1 }),
                1 => Some(self.read_at_pc()),
                2 => {
                    seq.operand = seq.latched as i8 as u16;
                    self.b = self.b.wrapping_sub(1);
                    if self.b == 0 {
                        return None;
                    }
                    Some(Cycle::Internal { length: 5 })
                }
                _ => {
                    self.pc = self.pc.wrapping_add(seq.operand);
                    self.wz = self.pc;
                    None
                }
            },
            Body::Jump { taken } => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                _ => {
                    seq.take_high();
                    self.wz = seq.operand;
                    if taken {
                        self.pc = seq.operand;
                    }
                    None
                }
            },
            Body::Call { taken } => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                2 => {
                    seq.take_high();
                    self.wz = seq.operand;
                    if !taken {
                        return None;
                    }
                    Some(Cycle::Internal { length: 1 })
                }
                3 => Some(self.push_byte((self.pc >> 8) as u8)),
                4 => Some(self.push_byte(self.pc as u8)),
                _ => {
                    self.pc = seq.operand;
                    None
                }
            },
            Body::Push { value, jump } => match step {
                0 => Some(Cycle::Internal { length: 1 }),
                1 => Some(self.push_byte((value >> 8) as u8)),
                2 => Some(self.push_byte(value as u8)),
                _ => {
                    if let Some(target) = jump {
                        self.pc = target;
                    }
                    None
                }
            },
            Body::Pop { dest, lead } => match if lead == 0 { step + 1 } else { step } {
                0 => Some(Cycle::Internal { length: lead }),
                1 => Some(self.pop_byte()),
                2 => {
                    seq.held = seq.latched;
                    Some(self.pop_byte())
                }
                _ => {
                    let value = u16::from_le_bytes([seq.held, seq.latched]);
                    match dest {
                        PopDest::Pair(p) => self.set_pair2(p, value),
                        PopDest::ProgramCounter => {
                            self.wz = value;
                            self.pc = value;
                        }
                    }
                    None
                }
            },
            Body::PortImmediate { write } => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    let low = seq.latched;
                    let port = ((self.a as u16) << 8) | low as u16;
                    if write {
                        self.wz = ((self.a as u16) << 8) | (low.wrapping_add(1) as u16);
                        Some(Cycle::IoWrite { port, data: self.a })
                    } else {
                        self.wz = port.wrapping_add(1);
                        Some(Cycle::IoRead { port })
                    }
                }
                _ => {
                    if !write {
                        self.a = seq.latched;
                    }
                    None
                }
            },
            Body::ExchangeStackTop => match step {
                0 => Some(Cycle::MemRead { address: self.sp }),
                1 => {
                    seq.held = seq.latched;
                    Some(Cycle::MemRead {
                        address: self.sp.wrapping_add(1),
                    })
                }
                2 => {
                    seq.operand = u16::from_le_bytes([seq.held, seq.latched]);
                    Some(Cycle::Internal { length: 1 })
                }
                3 => Some(Cycle::MemWrite {
                    address: self.sp.wrapping_add(1),
                    data: self.h,
                }),
                4 => Some(Cycle::MemWrite {
                    address: self.sp,
                    data: self.l,
                }),
                5 => {
                    self.set_pair(2, seq.operand);
                    self.wz = seq.operand;
                    Some(Cycle::Internal { length: 2 })
                }
                _ => None,
            },
        }
    }

    fn read_at_pc(&mut self) -> Cycle {
        let address = self.pc;
        self.pc = self.pc.wrapping_add(1);
        Cycle::MemRead { address }
    }

    fn push_byte(&mut self, data: u8) -> Cycle {
        self.sp = self.sp.wrapping_sub(1);
        Cycle::MemWrite {
            address: self.sp,
            data,
        }
    }

    fn pop_byte(&mut self) -> Cycle {
        let address = self.sp;
        self.sp = self.sp.wrapping_add(1);
        Cycle::MemRead { address }
    }
}

/// Instruction planning: the unprefixed table's octal-field groups, choosing
/// each opcode's execution shape and applying whatever it settles at dispatch.
impl Cpu {
    fn plan_instruction(&mut self, opcode: u8) -> Body {
        let f = Fields::new(opcode);
        match f.x {
            0 => self.plan_x0(f),
            1 => self.plan_x1(f),
            2 => self.plan_x2(f),
            _ => self.plan_x3(f),
        }
    }

    fn plan_x0(&mut self, f: Fields) -> Body {
        match f.z {
            0 => match f.y {
                0 => Body::Complete,
                1 => {
                    std::mem::swap(&mut self.a, &mut self.a_);
                    std::mem::swap(&mut self.f, &mut self.f_);
                    Body::Complete
                }
                2 => Body::DecrementAndJump,
                3 => Body::JumpRelative { taken: true },
                _ => Body::JumpRelative {
                    taken: self.condition(f.y - 4),
                },
            },
            1 => {
                if f.q == 0 {
                    Body::LoadPairImmediate { pair: f.p }
                } else {
                    let base = self.hl();
                    self.wz = base.wrapping_add(1);
                    let sum = self.add16(base, self.pair(f.p));
                    self.set_pair(2, sum);
                    Body::InternalPad { length: 7 }
                }
            }
            2 => self.plan_load_group(f),
            3 => {
                let value = self.pair(f.p);
                let delta = if f.q == 0 { 1u16 } else { 0u16.wrapping_sub(1) };
                self.set_pair(f.p, value.wrapping_add(delta));
                Body::InternalPad { length: 2 }
            }
            4 => self.plan_inc_dec(f.y, true),
            5 => self.plan_inc_dec(f.y, false),
            6 => {
                if f.y == 6 {
                    Body::StoreImmediate { address: self.hl() }
                } else {
                    Body::LoadImmediate {
                        dest: real_reg(f.y),
                    }
                }
            }
            _ => {
                match f.y {
                    0 => self.rotate_a(RotOp::Rlc),
                    1 => self.rotate_a(RotOp::Rrc),
                    2 => self.rotate_a(RotOp::Rl),
                    3 => self.rotate_a(RotOp::Rr),
                    4 => self.daa(),
                    5 => self.cpl(),
                    6 => self.scf(self.q),
                    _ => self.ccf(self.q),
                }
                Body::Complete
            }
        }
    }

    fn plan_load_group(&mut self, f: Fields) -> Body {
        if f.q == 0 {
            match f.p {
                0 => {
                    self.wz = ((self.a as u16) << 8) | (self.c.wrapping_add(1) as u16);
                    Body::Store {
                        address: self.bc(),
                        data: self.a,
                    }
                }
                1 => {
                    self.wz = ((self.a as u16) << 8) | (self.e.wrapping_add(1) as u16);
                    Body::Store {
                        address: self.de(),
                        data: self.a,
                    }
                }
                2 => Body::StorePairAbsolute,
                _ => Body::StoreAccumulatorAbsolute,
            }
        } else {
            match f.p {
                0 => {
                    let address = self.bc();
                    self.wz = address.wrapping_add(1);
                    Body::Load {
                        address,
                        dest: Reg::A,
                    }
                }
                1 => {
                    let address = self.de();
                    self.wz = address.wrapping_add(1);
                    Body::Load {
                        address,
                        dest: Reg::A,
                    }
                }
                2 => Body::LoadPairAbsolute,
                _ => Body::LoadAccumulatorAbsolute,
            }
        }
    }

    fn plan_inc_dec(&mut self, y: u8, increment: bool) -> Body {
        if y == 6 {
            return Body::ReadModifyWrite {
                address: self.hl(),
                increment,
            };
        }
        let reg = real_reg(y);
        let value = self.reg(reg);
        let result = if increment {
            self.inc8(value)
        } else {
            self.dec8(value)
        };
        self.set_reg(reg, result);
        Body::Complete
    }

    fn plan_x1(&mut self, f: Fields) -> Body {
        if f.z == 6 {
            Body::Load {
                address: self.hl(),
                dest: real_reg(f.y),
            }
        } else if f.y == 6 {
            Body::Store {
                address: self.hl(),
                data: self.reg(real_reg(f.z)),
            }
        } else {
            let value = self.reg(real_reg(f.z));
            self.set_reg(real_reg(f.y), value);
            Body::Complete
        }
    }

    fn plan_x2(&mut self, f: Fields) -> Body {
        let op = AluOp::from_index(f.y);
        if f.z == 6 {
            Body::AluIndirect {
                address: self.hl(),
                op,
            }
        } else {
            let value = self.reg(real_reg(f.z));
            self.alu(op, value);
            Body::Complete
        }
    }

    fn plan_x3(&mut self, f: Fields) -> Body {
        match f.z {
            0 => {
                if self.condition(f.y) {
                    Body::Pop {
                        dest: PopDest::ProgramCounter,
                        lead: 1,
                    }
                } else {
                    Body::InternalPad { length: 1 }
                }
            }
            1 => {
                if f.q == 0 {
                    Body::Pop {
                        dest: PopDest::Pair(f.p),
                        lead: 0,
                    }
                } else {
                    match f.p {
                        0 => Body::Pop {
                            dest: PopDest::ProgramCounter,
                            lead: 0,
                        },
                        1 => {
                            std::mem::swap(&mut self.b, &mut self.b_);
                            std::mem::swap(&mut self.c, &mut self.c_);
                            std::mem::swap(&mut self.d, &mut self.d_);
                            std::mem::swap(&mut self.e, &mut self.e_);
                            std::mem::swap(&mut self.h, &mut self.h_);
                            std::mem::swap(&mut self.l, &mut self.l_);
                            Body::Complete
                        }
                        2 => {
                            self.pc = self.hl();
                            Body::Complete
                        }
                        _ => {
                            self.sp = self.hl();
                            Body::InternalPad { length: 2 }
                        }
                    }
                }
            }
            2 => Body::Jump {
                taken: self.condition(f.y),
            },
            3 => match f.y {
                0 => Body::Jump { taken: true },
                // y == 1 is the CB prefix, dispatched before planning.
                1 => unreachable!(),
                2 => Body::PortImmediate { write: true },
                3 => Body::PortImmediate { write: false },
                4 => Body::ExchangeStackTop,
                5 => {
                    std::mem::swap(&mut self.d, &mut self.h);
                    std::mem::swap(&mut self.e, &mut self.l);
                    Body::Complete
                }
                6 => {
                    self.iff1 = false;
                    self.iff2 = false;
                    Body::Complete
                }
                _ => {
                    self.iff1 = true;
                    self.iff2 = true;
                    self.ei_pending = true;
                    Body::Complete
                }
            },
            4 => Body::Call {
                taken: self.condition(f.y),
            },
            5 => {
                if f.q == 0 {
                    Body::Push {
                        value: self.pair2(f.p),
                        jump: None,
                    }
                } else if f.p == 0 {
                    Body::Call { taken: true }
                } else {
                    // p 1..3 are the DD/ED/FD prefixes, dispatched before planning.
                    unreachable!()
                }
            }
            6 => Body::AluImmediate {
                op: AluOp::from_index(f.y),
            },
            _ => {
                let target = (f.y as u16) * 8;
                self.wz = target;
                Body::Push {
                    value: self.pc,
                    jump: Some(target),
                }
            }
        }
    }
}
