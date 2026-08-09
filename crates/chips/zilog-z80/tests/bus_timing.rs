//! Per-T bus-call placement: every `Bus` call must arrive on the tick whose
//! recorded `BusCycle` asserts the matching pins. The SingleStepTests sweep
//! drives whole instructions, so it cannot see where inside one an access
//! lands; this probe counts ticks and watches the calls arrive.

use missingno_zilog_z80::{Bus, Cpu};

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

/// Runs one instruction a tick at a time, returning the bus calls it made,
/// the T-state count, and the tick each pin-asserted trace entry sits on.
fn probe(program: &[u8], prepare: impl FnOnce(&mut Cpu)) -> (Vec<Call>, usize, Vec<Call>) {
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

    (bus.calls, ticks, asserted)
}

/// The per-T contract: call `n` arrives on the tick recording the `n`th
/// pin-asserted `BusCycle`.
fn sequenced(program: &[u8], prepare: impl FnOnce(&mut Cpu)) -> (Vec<Call>, usize) {
    let (calls, ticks, asserted) = probe(program, prepare);
    assert_eq!(calls, asserted, "bus calls vs pin-asserted T-states");
    (calls, ticks)
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

/// The prefixes still execute atomically at the T that latches their opcode —
/// the stated limit the main table no longer has.
#[test]
fn prefixed_accesses_still_bunch_at_the_opcode_tick() {
    let (calls, ticks, _) = probe(&[0xCB, 0x46], |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(ticks, 12);
    assert_eq!(
        calls,
        [
            (1, Access::MemRead, 0x0000),
            (3, Access::MemRead, 0x0001),
            (3, Access::MemRead, 0x1234),
        ]
    );
}
