//! Per-T bus-call placement: every `Bus` call must arrive on the tick whose
//! recorded `BusCycle` asserts the matching pins. The SingleStepTests sweep
//! drives whole instructions, so it cannot see where inside one an access
//! lands; this probe counts ticks and watches the calls arrive.

use missingno_zilog_z80::{Bus, Cpu, InterruptMode};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    MemRead,
    MemWrite,
    IoRead,
    IoWrite,
}

/// One bus call: the tick it arrived on, what it was, and the address.
type Call = (usize, Access, u16);

struct ProbeBus {
    memory: Vec<u8>,
    port_input: u8,
    tick: usize,
    calls: Vec<Call>,
}

impl ProbeBus {
    fn new(program: &[u8]) -> Self {
        let mut memory = vec![0; 0x10000];
        memory[..program.len()].copy_from_slice(program);
        ProbeBus {
            memory,
            port_input: 0x5A,
            tick: 0,
            calls: Vec::new(),
        }
    }
}

impl Bus for ProbeBus {
    fn read(&mut self, address: u16) -> u8 {
        self.calls.push((self.tick, Access::MemRead, address));
        self.memory[address as usize]
    }

    fn write(&mut self, address: u16, data: u8) {
        self.calls.push((self.tick, Access::MemWrite, address));
        self.memory[address as usize] = data;
    }

    fn input(&mut self, port: u16) -> u8 {
        self.calls.push((self.tick, Access::IoRead, port));
        self.port_input
    }

    fn output(&mut self, port: u16, _data: u8) {
        self.calls.push((self.tick, Access::IoWrite, port));
    }
}

/// Runs one instruction a tick at a time, returning the retired CPU, the bus
/// calls it made, the T-state count, and the tick each pin-asserted trace
/// entry sits on.
fn probe(program: &[u8], prepare: impl FnOnce(&mut Cpu)) -> (Cpu, Vec<Call>, usize, Vec<Call>) {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0000;
    prepare(&mut cpu);
    let mut bus = ProbeBus::new(program);

    let mut ticks = 0;
    loop {
        bus.tick = ticks;
        cpu.tick(&mut bus);
        ticks += 1;
        if cpu.at_instruction_boundary() {
            break;
        }
    }
    assert_eq!(ticks, cpu.bus_trace().len(), "ticks vs recorded T-states");

    let asserted = cpu
        .bus_trace()
        .iter()
        .enumerate()
        .filter_map(|(index, cycle)| {
            let access = match (cycle.pins.read, cycle.pins.write, cycle.pins.iorq) {
                (true, false, false) => Access::MemRead,
                (false, true, false) => Access::MemWrite,
                (true, false, true) => Access::IoRead,
                (false, true, true) => Access::IoWrite,
                _ => return None,
            };
            Some((index, access, cycle.address))
        })
        .collect();

    (cpu, bus.calls, ticks, asserted)
}

/// The per-T contract: call `n` arrives on the tick recording the `n`th
/// pin-asserted `BusCycle`.
fn sequenced(program: &[u8], prepare: impl FnOnce(&mut Cpu)) -> (Vec<Call>, usize) {
    let (_, calls, ticks) = sequenced_state(program, prepare);
    (calls, ticks)
}

/// As `sequenced`, also handing back the CPU the instruction retired into.
fn sequenced_state(program: &[u8], prepare: impl FnOnce(&mut Cpu)) -> (Cpu, Vec<Call>, usize) {
    let (cpu, calls, ticks, asserted) = probe(program, prepare);
    assert_eq!(calls, asserted, "bus calls vs pin-asserted T-states");
    (cpu, calls, ticks)
}

#[test]
fn load_a_from_hl() {
    let (calls, ticks) = sequenced(&[0x7E], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 7);
    assert_eq!(
        calls,
        [(1, Access::MemRead, 0x0000), (5, Access::MemRead, 0x1234),]
    );
}

#[test]
fn store_immediate_at_hl() {
    let (calls, ticks) = sequenced(&[0x36, 0xA5], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 10);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (8, Access::MemWrite, 0x1234),
        ]
    );
}

#[test]
fn push_bc() {
    let (calls, ticks) = sequenced(&[0xC5], |cpu| {
        cpu.sp = 0x2000;
        cpu.b = 0x11;
        cpu.c = 0x22;
    });
    assert_eq!(ticks, 11);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (6, Access::MemWrite, 0x1FFF),
            (9, Access::MemWrite, 0x1FFE),
        ]
    );
}

#[test]
fn increment_at_hl() {
    let (calls, ticks) = sequenced(&[0x34], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 11);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x1234),
            (9, Access::MemWrite, 0x1234),
        ]
    );
}

#[test]
fn output_to_immediate_port() {
    let (calls, ticks) = sequenced(&[0xD3, 0x10], |cpu| cpu.a = 0x42);
    assert_eq!(ticks, 11);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::IoWrite, 0x4210),
        ]
    );
}

#[test]
fn input_from_immediate_port() {
    let (calls, ticks) = sequenced(&[0xDB, 0x10], |cpu| cpu.a = 0x42);
    assert_eq!(ticks, 11);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::IoRead, 0x4210),
        ]
    );
}

#[test]
fn call_immediate() {
    let (calls, ticks) = sequenced(&[0xCD, 0x00, 0x40], |cpu| cpu.sp = 0x2000);
    assert_eq!(ticks, 17);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (8, Access::MemRead, 0x0002),
            (12, Access::MemWrite, 0x1FFF),
            (15, Access::MemWrite, 0x1FFE),
        ]
    );
}

#[test]
fn exchange_stack_top() {
    let (calls, ticks) = sequenced(&[0xE3], |cpu| {
        cpu.sp = 0x2000;
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 19);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x2000),
            (8, Access::MemRead, 0x2001),
            (12, Access::MemWrite, 0x2001),
            (15, Access::MemWrite, 0x2000),
        ]
    );
}

#[test]
fn test_bit_at_hl() {
    let (calls, ticks) = sequenced(&[0xCB, 0x46], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 12);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x1234),
        ]
    );
}

#[test]
fn set_bit_at_hl() {
    let (calls, ticks) = sequenced(&[0xCB, 0xCE], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 15);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x1234),
            (13, Access::MemWrite, 0x1234),
        ]
    );
}

#[test]
fn block_transfer() {
    let (calls, ticks) = sequenced(&[0xED, 0xA0], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
        cpu.d = 0x23;
        cpu.e = 0x45;
        cpu.b = 0x00;
        cpu.c = 0x02;
    });
    assert_eq!(ticks, 16);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x1234),
            (12, Access::MemWrite, 0x2345),
        ]
    );
}

/// A repeating iteration adds its five internal T-states and rewinds PC over
/// the two opcode bytes, so the next fetch runs the same instruction again.
#[test]
fn repeating_block_transfer_iteration() {
    let (cpu, calls, ticks) = sequenced_state(&[0xED, 0xB0], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
        cpu.d = 0x23;
        cpu.e = 0x45;
        cpu.b = 0x00;
        cpu.c = 0x02;
    });
    assert_eq!(ticks, 21);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x1234),
            (12, Access::MemWrite, 0x2345),
        ]
    );
    assert_eq!(cpu.pc, 0x0000);
}

#[test]
fn input_from_bc_port() {
    let (calls, ticks) = sequenced(&[0xED, 0x40], |cpu| {
        cpu.b = 0x12;
        cpu.c = 0x34;
    });
    assert_eq!(ticks, 12);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (10, Access::IoRead, 0x1234),
        ]
    );
}

#[test]
fn rotate_digits_right() {
    let (calls, ticks) = sequenced(&[0xED, 0x67], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 18);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x1234),
            (16, Access::MemWrite, 0x1234),
        ]
    );
}

/// An index prefix spends its own M1, and the displacement's five internal
/// T-states separate it from the operand read.
#[test]
fn load_a_from_index_displacement() {
    let (calls, ticks) = sequenced(&[0xDD, 0x7E, 0x05], |cpu| cpu.ix = 0x1230);
    assert_eq!(ticks, 19);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x0002),
            (17, Access::MemRead, 0x1235),
        ]
    );
}

/// DDCB reads its displacement and sub-opcode as plain memory cycles, then
/// pads before the read-modify-write on the effective address.
#[test]
fn set_bit_at_index_displacement() {
    let (cpu, calls, ticks) = sequenced_state(&[0xDD, 0xCB, 0x05, 0xC6], |cpu| cpu.ix = 0x1230);
    assert_eq!(ticks, 23);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x0002),
            (12, Access::MemRead, 0x0003),
            (17, Access::MemRead, 0x1235),
            (21, Access::MemWrite, 0x1235),
        ]
    );
    assert_eq!(cpu.wz, 0x1235);
}

/// A halted CPU spends each period on a 4-T re-fetch that leaves PC where it
/// stands, with the instruction boundary observable between them.
#[test]
fn halt_refetch_cadence() {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0000;
    let mut bus = ProbeBus::new(&[0x76]);

    let mut boundaries = Vec::new();
    for tick in 0..16 {
        bus.tick = tick;
        cpu.tick(&mut bus);
        if cpu.at_instruction_boundary() {
            boundaries.push(tick);
        }
    }

    assert!(cpu.halted);
    assert_eq!(cpu.pc, 0x0001);
    assert_eq!(boundaries, [3, 7, 11, 15]);
    assert_eq!(
        bus.calls,
        [
            (1, Access::MemRead, 0x0000),
            (5, Access::MemRead, 0x0001),
            (9, Access::MemRead, 0x0001),
            (13, Access::MemRead, 0x0001),
        ]
    );
}

/// IM 1 acceptance pushes PC over two write cycles before vectoring to $0038.
/// The entry is oracle-silent, so this pins the T-states the crate records,
/// not a measured hardware sequence.
#[test]
fn interrupt_mode1_acceptance() {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0000;
    cpu.sp = 0x2000;
    cpu.im = InterruptMode::Mode1;
    cpu.iff1 = true;
    cpu.set_irq(true);
    let mut bus = ProbeBus::new(&[0x00]);

    // The line is sampled at the rising edge of an instruction's final
    // T-state, so acceptance follows the NOP that carries the sample.
    for tick in 0..4 {
        bus.tick = tick;
        cpu.tick(&mut bus);
    }
    assert!(cpu.at_instruction_boundary());
    bus.calls.clear();

    let mut ticks = 0;
    loop {
        bus.tick = ticks;
        cpu.tick(&mut bus);
        ticks += 1;
        if cpu.at_instruction_boundary() {
            break;
        }
    }
    assert_eq!(ticks, 13);
    assert_eq!(
        bus.calls,
        [
            (8, Access::MemWrite, 0x1FFF),
            (11, Access::MemWrite, 0x1FFE),
        ]
    );
    assert_eq!(cpu.pc, 0x0038);
    assert!(!cpu.iff1);
}
