//! The two nested machines that walk an instruction one T-state at a time:
//! a machine-cycle runner carrying each cycle's fixed per-T schedule, and an
//! instruction plan above it, consulted only at cycle boundaries.

use crate::decode::{AluOp, Fields, INT_MODE, Index, Reg, RotOp, real_reg, reg_at};
use crate::{Bus, Cpu, InterruptMode, Pins};

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

/// Where the instruction is: M1 in flight, a prefix's second M1 in flight, or
/// a chosen body part-way through its machine cycles.
#[derive(Clone, Copy)]
enum Plan {
    OpcodeFetch,
    PrefixFetch { prefix: Prefix },
    Body { body: Body, step: u8 },
}

/// The prefix whose second opcode fetch is in flight — each is its own
/// instruction table.
#[derive(Clone, Copy)]
enum Prefix {
    /// CB: rotates, shifts, BIT/RES/SET.
    Bit,
    /// ED: the extended table, including the block instructions.
    Extended,
    /// DD/FD: the main table again, with IX or IY substituted for HL.
    Indexed { index: Index },
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
        index: Index,
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
    LoadPairAbsolute {
        pair: u8,
        index: Index,
    },
    StoreAccumulatorAbsolute,
    StorePairAbsolute {
        value: u16,
    },
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
    ExchangeStackTop {
        value: u16,
        index: Index,
    },
    /// A CB operation on (HL): read, one internal T-state, then test or
    /// write back.
    BitOperation {
        address: u16,
        effect: BitEffect,
    },
    /// IN r,(C) / OUT (C),r — the port is BC, and the F/0 forms name no
    /// register.
    PortIndirect {
        write: bool,
        register: Option<Reg>,
    },
    /// RRD/RLD: the nibble shuffle between A and (HL).
    RotateDigits {
        address: u16,
        right: bool,
    },
    Block {
        kind: Block,
        increment: bool,
        repeat: bool,
    },
    /// An operand at (IX/IY + d): the displacement read, its padding, and the
    /// operation the effective address feeds.
    Indexed {
        base: u16,
        op: IndexedOp,
    },
    /// DDCB/FDCB: displacement and sub-opcode arrive as plain reads, then the
    /// operation writes its result back to memory (and, undocumented, to a
    /// named register).
    IndexBit {
        base: u16,
    },
    /// A halted CPU's re-fetch period, which leaves PC where it stands.
    HaltRefetch {
        pc: u16,
    },
    AcceptNmi,
    AcceptIrq {
        mode: InterruptMode,
    },
}

#[derive(Clone, Copy)]
enum PopDest {
    Pair { pair: u8, index: Index },
    ProgramCounter,
}

/// What an (IX/IY + d) effective address feeds once its displacement byte has
/// been read.
#[derive(Clone, Copy)]
enum IndexedOp {
    Load {
        dest: Reg,
    },
    Store {
        data: u8,
    },
    Alu {
        op: AluOp,
    },
    ReadModifyWrite {
        increment: bool,
    },
    /// LD (IX+d),n — the immediate follows the displacement, and its padding
    /// falls between the two reads and the write.
    StoreImmediate,
}

/// What a CB-prefixed opcode does to its operand byte.
#[derive(Clone, Copy)]
enum BitEffect {
    Rotate(RotOp),
    Test(u8),
    Reset(u8),
    Set(u8),
}

impl BitEffect {
    fn from_fields(f: Fields) -> Self {
        match f.x {
            0 => BitEffect::Rotate(RotOp::from_index(f.y)),
            1 => BitEffect::Test(f.y),
            2 => BitEffect::Reset(f.y),
            _ => BitEffect::Set(f.y),
        }
    }
}

/// The four block-instruction shapes, each stepping its pointers by ±1 and
/// optionally re-running until its terminating condition.
#[derive(Clone, Copy)]
enum Block {
    /// LDI/LDD/LDIR/LDDR.
    Transfer,
    /// CPI/CPD/CPIR/CPDR.
    Compare,
    /// INI/IND/INIR/INDR.
    Input,
    /// OUTI/OUTD/OTIR/OTDR.
    Output,
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

    /// A body no opcode fetch introduced — an interrupt entry or HALT's
    /// re-fetch. Its caller names the opening cycle and applies whatever the
    /// entry settles up front, so the body's own steps start at 1.
    fn entering(cycle: Cycle, body: Body) -> Self {
        Sequencer {
            cycle,
            t: 0,
            plan: Plan::Body { body, step: 1 },
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

    /// Advance the sequenced instruction by one T-state. Returns false once it
    /// has retired.
    pub(super) fn tick_sequencer(&mut self, bus: &mut impl Bus, seq: &mut Sequencer) -> bool {
        self.tick_cycle(bus, seq.cycle, seq.t, &mut seq.latched);
        seq.t += 1;
        if seq.t < seq.cycle.length() {
            return true;
        }
        match self.next_cycle(seq) {
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
    fn next_cycle(&mut self, seq: &mut Sequencer) -> Option<Cycle> {
        match seq.plan {
            Plan::OpcodeFetch => {
                let opcode = seq.latched;
                match opcode {
                    // A prefix spends its own M1 fetching the opcode that
                    // names the instruction.
                    0xCB => self.fetch_prefixed(seq, Prefix::Bit),
                    0xED => self.fetch_prefixed(seq, Prefix::Extended),
                    0xDD => self.fetch_indexed(seq, Index::Ix),
                    0xFD => self.fetch_indexed(seq, Index::Iy),
                    _ => {
                        seq.plan = Plan::Body {
                            body: self.plan_instruction(opcode, Index::Hl),
                            step: 0,
                        };
                        self.next_cycle(seq)
                    }
                }
            }
            Plan::PrefixFetch { prefix } => {
                let opcode = seq.latched;
                let body = match prefix {
                    Prefix::Bit => self.plan_bit(opcode),
                    Prefix::Extended => self.plan_extended(opcode),
                    Prefix::Indexed { index } => match opcode {
                        // The last index prefix of a run names the register.
                        0xDD => return self.fetch_indexed(seq, Index::Ix),
                        0xFD => return self.fetch_indexed(seq, Index::Iy),
                        0xED => return self.fetch_prefixed(seq, Prefix::Extended),
                        0xCB => Body::IndexBit {
                            base: index.base(self),
                        },
                        _ => self.plan_instruction(opcode, index),
                    },
                };
                seq.plan = Plan::Body { body, step: 0 };
                self.next_cycle(seq)
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

    fn fetch_prefixed(&mut self, seq: &mut Sequencer, prefix: Prefix) -> Option<Cycle> {
        seq.plan = Plan::PrefixFetch { prefix };
        Some(Cycle::OpcodeFetch)
    }

    fn fetch_indexed(&mut self, seq: &mut Sequencer, index: Index) -> Option<Cycle> {
        // The prefix byte completes as a non-flag-modifying step, clearing the
        // Q shadow that SCF/CCF fold into their X/Y flags.
        self.q = 0;
        self.fetch_prefixed(seq, Prefix::Indexed { index })
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
            Body::LoadPairImmediate { pair, index } => match step {
                0 => Some(self.read_at_pc()),
                1 => {
                    seq.take_low();
                    Some(self.read_at_pc())
                }
                _ => {
                    seq.take_high();
                    self.set_pair(pair, index, seq.operand);
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
            Body::LoadPairAbsolute { pair, index } => match step {
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
                    self.set_pair(pair, index, u16::from_le_bytes([seq.held, seq.latched]));
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
            Body::StorePairAbsolute { value } => match step {
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
                        data: value as u8,
                    })
                }
                3 => Some(Cycle::MemWrite {
                    address: self.wz,
                    data: (value >> 8) as u8,
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
                        PopDest::Pair { pair, index } => self.set_stack_pair(pair, index, value),
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
            Body::ExchangeStackTop { value, index } => match step {
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
                    data: (value >> 8) as u8,
                }),
                4 => Some(Cycle::MemWrite {
                    address: self.sp,
                    data: value as u8,
                }),
                5 => {
                    self.set_pair(2, index, seq.operand);
                    self.wz = seq.operand;
                    Some(Cycle::Internal { length: 2 })
                }
                _ => None,
            },
            Body::BitOperation { address, effect } => match step {
                0 => Some(Cycle::MemRead { address }),
                1 => Some(Cycle::Internal { length: 1 }),
                2 => {
                    // BIT's undocumented X/Y come from WZ's high byte.
                    let xy = (self.wz >> 8) as u8;
                    self.bit_effect(effect, seq.latched, xy)
                        .map(|data| Cycle::MemWrite { address, data })
                }
                _ => None,
            },
            Body::PortIndirect { write, register } => match step {
                0 => {
                    let port = self.bc();
                    self.wz = port.wrapping_add(1);
                    if write {
                        let data = register.map_or(0, |r| self.reg(r));
                        Some(Cycle::IoWrite { port, data })
                    } else {
                        Some(Cycle::IoRead { port })
                    }
                }
                _ => {
                    if !write {
                        self.set_input_flags(seq.latched);
                        if let Some(r) = register {
                            self.set_reg(r, seq.latched);
                        }
                    }
                    None
                }
            },
            Body::RotateDigits { address, right } => match step {
                0 => Some(Cycle::MemRead { address }),
                1 => Some(Cycle::Internal { length: 4 }),
                2 => {
                    let m = seq.latched;
                    let (written, a) = if right {
                        ((m >> 4) | (self.a << 4), (self.a & 0xF0) | (m & 0x0F))
                    } else {
                        ((m << 4) | (self.a & 0x0F), (self.a & 0xF0) | (m >> 4))
                    };
                    self.a = a;
                    self.set_input_flags(a);
                    self.wz = address.wrapping_add(1);
                    Some(Cycle::MemWrite {
                        address,
                        data: written,
                    })
                }
                _ => None,
            },
            Body::Block {
                kind,
                increment,
                repeat,
            } => self.run_block(seq, kind, increment, repeat, step),
            Body::Indexed { base, op } => self.run_indexed(seq, base, op, step),
            Body::IndexBit { base } => self.run_index_bit(seq, base, step),
            Body::HaltRefetch { pc } => {
                self.pc = pc;
                None
            }
            Body::AcceptNmi => match step {
                1 => {
                    self.inc_r();
                    Some(Cycle::Internal { length: 1 })
                }
                2 => Some(self.push_byte((self.pc >> 8) as u8)),
                3 => Some(self.push_byte(self.pc as u8)),
                _ => {
                    self.pc = 0x0066;
                    self.wz = 0x0066;
                    None
                }
            },
            Body::AcceptIrq { mode } => match step {
                1 => Some(self.push_byte((self.pc >> 8) as u8)),
                2 => Some(self.push_byte(self.pc as u8)),
                3 => match mode {
                    InterruptMode::Mode2 => {
                        // The vector's low byte comes from the device; the
                        // oracle-silent entry drives $FF.
                        seq.operand = u16::from_be_bytes([self.i, 0xFF]);
                        Some(Cycle::MemRead {
                            address: seq.operand,
                        })
                    }
                    _ => {
                        self.pc = 0x0038;
                        self.wz = 0x0038;
                        None
                    }
                },
                4 => {
                    seq.held = seq.latched;
                    Some(Cycle::MemRead {
                        address: seq.operand.wrapping_add(1),
                    })
                }
                _ => {
                    let target = u16::from_le_bytes([seq.held, seq.latched]);
                    self.pc = target;
                    self.wz = target;
                    None
                }
            },
        }
    }

    /// Schedule the machine cycle at `step` of an (IX/IY + d) operand: the
    /// displacement read forms the address, then the operation runs against
    /// it.
    fn run_indexed(
        &mut self,
        seq: &mut Sequencer,
        base: u16,
        op: IndexedOp,
        step: u8,
    ) -> Option<Cycle> {
        match step {
            0 => Some(self.read_at_pc()),
            1 => {
                seq.operand = base.wrapping_add(seq.latched as i8 as u16);
                self.wz = seq.operand;
                match op {
                    IndexedOp::StoreImmediate => Some(self.read_at_pc()),
                    _ => Some(Cycle::Internal { length: 5 }),
                }
            }
            _ => {
                let address = seq.operand;
                match op {
                    IndexedOp::Load { dest } => match step {
                        2 => Some(Cycle::MemRead { address }),
                        _ => {
                            self.set_reg(dest, seq.latched);
                            None
                        }
                    },
                    IndexedOp::Store { data } => match step {
                        2 => Some(Cycle::MemWrite { address, data }),
                        _ => None,
                    },
                    IndexedOp::Alu { op } => match step {
                        2 => Some(Cycle::MemRead { address }),
                        _ => {
                            self.alu(op, seq.latched);
                            None
                        }
                    },
                    IndexedOp::ReadModifyWrite { increment } => match step {
                        2 => Some(Cycle::MemRead { address }),
                        3 => Some(Cycle::Internal { length: 1 }),
                        4 => {
                            let data = if increment {
                                self.inc8(seq.latched)
                            } else {
                                self.dec8(seq.latched)
                            };
                            Some(Cycle::MemWrite { address, data })
                        }
                        _ => None,
                    },
                    IndexedOp::StoreImmediate => match step {
                        2 => {
                            seq.held = seq.latched;
                            Some(Cycle::Internal { length: 2 })
                        }
                        3 => Some(Cycle::MemWrite {
                            address,
                            data: seq.held,
                        }),
                        _ => None,
                    },
                }
            }
        }
    }

    /// Schedule the machine cycle at `step` of a DDCB/FDCB operation. Its
    /// displacement and sub-opcode arrive as ordinary reads — neither is an
    /// M1, so neither refreshes.
    fn run_index_bit(&mut self, seq: &mut Sequencer, base: u16, step: u8) -> Option<Cycle> {
        match step {
            0 => Some(self.read_at_pc()),
            1 => {
                seq.operand = base.wrapping_add(seq.latched as i8 as u16);
                self.wz = seq.operand;
                Some(self.read_at_pc())
            }
            2 => {
                seq.held = seq.latched;
                Some(Cycle::Internal { length: 2 })
            }
            3 => Some(Cycle::MemRead {
                address: seq.operand,
            }),
            4 => Some(Cycle::Internal { length: 1 }),
            5 => {
                let f = Fields::new(seq.held);
                let address = seq.operand;
                let effect = BitEffect::from_fields(f);
                // BIT's undocumented X/Y come from the effective address.
                let result = self.bit_effect(effect, seq.latched, (address >> 8) as u8)?;
                // The undocumented forms also drop the result into the
                // register the opcode names.
                if f.z != 6 {
                    self.set_reg(real_reg(f.z), result);
                }
                Some(Cycle::MemWrite {
                    address,
                    data: result,
                })
            }
            _ => None,
        }
    }

    /// Schedule the machine cycle at `step` of a block instruction. A
    /// repeating one ends on an extra internal cycle and rewinds PC over its
    /// two opcode bytes, so the next fetch runs the same plan again.
    fn run_block(
        &mut self,
        seq: &mut Sequencer,
        kind: Block,
        increment: bool,
        repeat: bool,
        step: u8,
    ) -> Option<Cycle> {
        let delta = if increment { 1u16 } else { 1u16.wrapping_neg() };
        match kind {
            Block::Transfer => match step {
                0 => Some(Cycle::MemRead { address: self.hl() }),
                1 => Some(Cycle::MemWrite {
                    address: self.de(),
                    data: seq.latched,
                }),
                2 => Some(Cycle::Internal { length: 2 }),
                3 => {
                    self.set_hl(self.hl().wrapping_add(delta));
                    self.set_de(self.de().wrapping_add(delta));
                    self.set_bc(self.bc().wrapping_sub(1));
                    let remaining = self.bc() != 0;
                    self.block_transfer_flags(seq.latched, remaining);
                    (repeat && remaining).then_some(Cycle::Internal { length: 5 })
                }
                _ => self.repeat_iteration(),
            },
            Block::Compare => match step {
                0 => Some(Cycle::MemRead { address: self.hl() }),
                1 => Some(Cycle::Internal { length: 5 }),
                2 => {
                    self.set_hl(self.hl().wrapping_add(delta));
                    self.set_bc(self.bc().wrapping_sub(1));
                    let remaining = self.bc() != 0;
                    let found = self.a == seq.latched;
                    self.block_compare_flags(seq.latched, remaining);
                    self.wz = self.wz.wrapping_add(delta);
                    (repeat && remaining && !found).then_some(Cycle::Internal { length: 5 })
                }
                _ => self.repeat_iteration(),
            },
            Block::Input => match step {
                0 => Some(Cycle::Internal { length: 1 }),
                1 => {
                    seq.operand = self.bc();
                    Some(Cycle::IoRead { port: seq.operand })
                }
                2 => {
                    self.wz = seq.operand.wrapping_add(delta);
                    Some(Cycle::MemWrite {
                        address: self.hl(),
                        data: seq.latched,
                    })
                }
                3 => {
                    self.b = self.b.wrapping_sub(1);
                    self.set_hl(self.hl().wrapping_add(delta));
                    let port_term = self.c.wrapping_add(delta as u8);
                    let repeating = repeat && self.b != 0;
                    self.block_io_flags(seq.latched, self.b, port_term, repeating);
                    repeating.then_some(Cycle::Internal { length: 5 })
                }
                _ => self.repeat_iteration(),
            },
            Block::Output => match step {
                0 => Some(Cycle::Internal { length: 1 }),
                1 => Some(Cycle::MemRead { address: self.hl() }),
                2 => {
                    self.b = self.b.wrapping_sub(1);
                    seq.operand = self.bc();
                    Some(Cycle::IoWrite {
                        port: seq.operand,
                        data: seq.latched,
                    })
                }
                3 => {
                    self.set_hl(self.hl().wrapping_add(delta));
                    self.wz = seq.operand.wrapping_add(delta);
                    let repeating = repeat && self.b != 0;
                    self.block_io_flags(seq.latched, self.b, self.l, repeating);
                    repeating.then_some(Cycle::Internal { length: 5 })
                }
                _ => self.repeat_iteration(),
            },
        }
    }

    fn repeat_iteration(&mut self) -> Option<Cycle> {
        self.pc = self.pc.wrapping_sub(2);
        self.wz = self.pc.wrapping_add(1);
        self.repeat_flag_xy();
        None
    }

    /// Apply a CB operation to `value`, returning the byte to write back —
    /// `None` for BIT, which only tests. `xy` sources its undocumented flags.
    fn bit_effect(&mut self, effect: BitEffect, value: u8, xy: u8) -> Option<u8> {
        match effect {
            BitEffect::Rotate(op) => Some(self.rotate(op, value)),
            BitEffect::Test(index) => {
                self.bit(index, value, xy);
                None
            }
            BitEffect::Reset(index) => Some(value & !(1 << index)),
            BitEffect::Set(index) => Some(value | (1 << index)),
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

/// The entry sequences no opcode introduces: interrupt acceptance and the
/// re-fetch a halted CPU repeats until one arrives.
impl Cpu {
    pub(super) fn accept_nmi(&mut self) -> Sequencer {
        self.halted = false;
        self.iff1 = false;
        // Acknowledge holds PC on the address bus.
        self.last_address = self.pc;
        Sequencer::entering(Cycle::Internal { length: 2 }, Body::AcceptNmi)
    }

    pub(super) fn accept_irq(&mut self) -> Sequencer {
        self.halted = false;
        self.iff1 = false;
        self.iff2 = false;
        self.inc_r();
        let mode = self.im;
        Sequencer::entering(Cycle::Internal { length: 2 }, Body::AcceptIrq { mode })
    }

    /// A halted CPU re-fetches its successor byte each period without
    /// advancing PC; execution resumes when an interrupt lands.
    pub(super) fn halt_refetch(&mut self) -> Sequencer {
        Sequencer::entering(Cycle::OpcodeFetch, Body::HaltRefetch { pc: self.pc })
    }
}

/// Instruction planning: the main table's octal-field groups, choosing each
/// opcode's execution shape and applying whatever it settles at dispatch.
/// `index` is what an index prefix substituted for HL, `Index::Hl` unprefixed.
impl Cpu {
    fn plan_instruction(&mut self, opcode: u8, index: Index) -> Body {
        let f = Fields::new(opcode);
        match f.x {
            0 => self.plan_x0(f, index),
            1 => self.plan_x1(f, index),
            2 => self.plan_x2(f, index),
            _ => self.plan_x3(f, index),
        }
    }

    fn plan_x0(&mut self, f: Fields, index: Index) -> Body {
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
                    Body::LoadPairImmediate { pair: f.p, index }
                } else {
                    let base = index.base(self);
                    self.wz = base.wrapping_add(1);
                    let sum = self.add16(base, self.pair(f.p, index));
                    self.set_pair(2, index, sum);
                    Body::InternalPad { length: 7 }
                }
            }
            2 => self.plan_load_group(f, index),
            3 => {
                let value = self.pair(f.p, index);
                let delta = if f.q == 0 { 1u16 } else { 0u16.wrapping_sub(1) };
                self.set_pair(f.p, index, value.wrapping_add(delta));
                Body::InternalPad { length: 2 }
            }
            4 => self.plan_inc_dec(f.y, index, true),
            5 => self.plan_inc_dec(f.y, index, false),
            6 => {
                if f.y == 6 {
                    match index {
                        Index::Hl => Body::StoreImmediate { address: self.hl() },
                        _ => Body::Indexed {
                            base: index.base(self),
                            op: IndexedOp::StoreImmediate,
                        },
                    }
                } else {
                    Body::LoadImmediate {
                        dest: reg_at(f.y, index),
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

    fn plan_load_group(&mut self, f: Fields, index: Index) -> Body {
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
                2 => Body::StorePairAbsolute {
                    value: index.base(self),
                },
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
                2 => Body::LoadPairAbsolute { pair: 2, index },
                _ => Body::LoadAccumulatorAbsolute,
            }
        }
    }

    fn plan_inc_dec(&mut self, y: u8, index: Index, increment: bool) -> Body {
        if y == 6 {
            return match index {
                Index::Hl => Body::ReadModifyWrite {
                    address: self.hl(),
                    increment,
                },
                _ => Body::Indexed {
                    base: index.base(self),
                    op: IndexedOp::ReadModifyWrite { increment },
                },
            };
        }
        let reg = reg_at(y, index);
        let value = self.reg(reg);
        let result = if increment {
            self.inc8(value)
        } else {
            self.dec8(value)
        };
        self.set_reg(reg, result);
        Body::Complete
    }

    fn plan_x1(&mut self, f: Fields, index: Index) -> Body {
        if f.y == 6 && f.z == 6 {
            self.halted = true;
            return Body::Complete;
        }
        // A memory operand suppresses IX/IY half-register substitution on
        // the paired register.
        if f.z == 6 {
            let dest = real_reg(f.y);
            match index {
                Index::Hl => Body::Load {
                    address: self.hl(),
                    dest,
                },
                _ => Body::Indexed {
                    base: index.base(self),
                    op: IndexedOp::Load { dest },
                },
            }
        } else if f.y == 6 {
            let data = self.reg(real_reg(f.z));
            match index {
                Index::Hl => Body::Store {
                    address: self.hl(),
                    data,
                },
                _ => Body::Indexed {
                    base: index.base(self),
                    op: IndexedOp::Store { data },
                },
            }
        } else {
            let value = self.reg(reg_at(f.z, index));
            self.set_reg(reg_at(f.y, index), value);
            Body::Complete
        }
    }

    fn plan_x2(&mut self, f: Fields, index: Index) -> Body {
        let op = AluOp::from_index(f.y);
        if f.z == 6 {
            match index {
                Index::Hl => Body::AluIndirect {
                    address: self.hl(),
                    op,
                },
                _ => Body::Indexed {
                    base: index.base(self),
                    op: IndexedOp::Alu { op },
                },
            }
        } else {
            let value = self.reg(reg_at(f.z, index));
            self.alu(op, value);
            Body::Complete
        }
    }

    fn plan_x3(&mut self, f: Fields, index: Index) -> Body {
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
                        dest: PopDest::Pair { pair: f.p, index },
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
                            self.pc = index.base(self);
                            Body::Complete
                        }
                        _ => {
                            self.sp = index.base(self);
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
                4 => Body::ExchangeStackTop {
                    value: index.base(self),
                    index,
                },
                // EX DE,HL names HL even under an index prefix.
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
                        value: self.stack_pair(f.p, index),
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

/// The prefixed tables, planned once their own opcode fetch has latched: CB's
/// bit operations and ED's extended group. Neither substitutes an index — an
/// index prefix ahead of ED only spends its own M1.
impl Cpu {
    fn plan_bit(&mut self, opcode: u8) -> Body {
        let f = Fields::new(opcode);
        let effect = BitEffect::from_fields(f);
        if f.z == 6 {
            return Body::BitOperation {
                address: self.hl(),
                effect,
            };
        }
        let reg = real_reg(f.z);
        let value = self.reg(reg);
        if let Some(result) = self.bit_effect(effect, value, value) {
            self.set_reg(reg, result);
        }
        Body::Complete
    }

    fn plan_extended(&mut self, opcode: u8) -> Body {
        let f = Fields::new(opcode);
        match (f.x, f.z) {
            (1, 0) => Body::PortIndirect {
                write: false,
                register: (f.y != 6).then(|| real_reg(f.y)),
            },
            (1, 1) => Body::PortIndirect {
                write: true,
                register: (f.y != 6).then(|| real_reg(f.y)),
            },
            (1, 2) => {
                let hl = self.hl();
                self.wz = hl.wrapping_add(1);
                let rp = self.pair(f.p, Index::Hl);
                let result = if f.q == 0 {
                    self.sbc16(hl, rp)
                } else {
                    self.adc16(hl, rp)
                };
                self.set_hl(result);
                Body::InternalPad { length: 7 }
            }
            (1, 3) => {
                if f.q == 0 {
                    Body::StorePairAbsolute {
                        value: self.pair(f.p, Index::Hl),
                    }
                } else {
                    Body::LoadPairAbsolute {
                        pair: f.p,
                        index: Index::Hl,
                    }
                }
            }
            (1, 4) => {
                let a = self.a;
                self.a = 0;
                self.sub8(a, 0, true);
                Body::Complete
            }
            (1, 5) => {
                self.iff1 = self.iff2;
                Body::Pop {
                    dest: PopDest::ProgramCounter,
                    lead: 0,
                }
            }
            (1, 6) => {
                self.im = INT_MODE[(f.y & 3) as usize];
                Body::Complete
            }
            (1, _) => self.plan_extended_group7(f.y),
            (2, _) if f.y >= 4 && f.z <= 3 => Body::Block {
                kind: match f.z {
                    0 => Block::Transfer,
                    1 => Block::Compare,
                    2 => Block::Input,
                    _ => Block::Output,
                },
                increment: f.y & 1 == 0,
                repeat: f.y >= 6,
            },
            _ => Body::Complete,
        }
    }

    /// ED z=7: the I/R transfers and the digit rotates.
    fn plan_extended_group7(&mut self, y: u8) -> Body {
        match y {
            0 => {
                self.i = self.a;
                Body::InternalPad { length: 1 }
            }
            1 => {
                self.r = self.a;
                Body::InternalPad { length: 1 }
            }
            2 | 3 => {
                self.a = if y == 2 { self.i } else { self.r };
                self.set_ld_a_ir_flags(self.a, self.iff2);
                self.p = true;
                Body::InternalPad { length: 1 }
            }
            4 | 5 => Body::RotateDigits {
                address: self.hl(),
                right: y == 4,
            },
            _ => Body::Complete,
        }
    }
}
