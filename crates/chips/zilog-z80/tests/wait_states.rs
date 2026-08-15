//! The /WAIT channel: a board holding the line low at a transfer cycle's
//! sample point stretches that cycle. The board answers for the pin through
//! `Bus::wait_requested`, whose default is a released line — the
//! SingleStepTests sweep rides that default, so this probe schedules
//! assertions against the tick the access lands on and counts the T-states
//! that follow.

use missingno_zilog_z80::{Bus, BusCycle, Cpu, Pins};

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
    schedule: Schedule,
}

impl ProbeBus {
    fn new(program: &[u8], schedule: Schedule) -> Self {
        let mut memory = vec![0; 0x10000];
        memory[..program.len()].copy_from_slice(program);
        ProbeBus {
            memory,
            port_input: 0x5A,
            tick: 0,
            calls: Vec::new(),
            schedule,
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

    fn wait_requested(&self) -> bool {
        self.schedule.asserted_at(self.tick)
    }
}

/// What the board does with /WAIT while the instruction runs.
#[derive(Clone, Copy)]
enum Schedule {
    /// No device pulls the line.
    Released,
    /// Asserted for `length` consecutive ticks from `from`.
    Held { from: usize, length: usize },
}

impl Schedule {
    fn asserted_at(self, tick: usize) -> bool {
        match self {
            Schedule::Released => false,
            Schedule::Held { from, length } => (from..from + length).contains(&tick),
        }
    }
}

/// One instruction run to retirement: the CPU it left, the bus calls made,
/// the T-state count, and the recorded snapshots.
struct Run {
    cpu: Cpu,
    calls: Vec<Call>,
    ticks: usize,
    trace: Vec<BusCycle>,
}

fn run(program: &[u8], schedule: Schedule, prepare: impl FnOnce(&mut Cpu)) -> Run {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0000;
    prepare(&mut cpu);
    let mut bus = ProbeBus::new(program, schedule);

    let mut ticks = 0;
    loop {
        bus.tick = ticks;
        cpu.tick(&mut bus);
        ticks += 1;
        assert!(ticks < 200, "instruction never retired");
        if cpu.at_instruction_boundary() {
            break;
        }
    }
    assert_eq!(ticks, cpu.bus_trace().len(), "ticks vs recorded T-states");

    let trace = cpu.bus_trace().to_vec();
    Run {
        cpu,
        calls: bus.calls,
        ticks,
        trace,
    }
}

/// The tick the `n`th call of `access` lands on with the line at rest — every
/// expected stretch is measured from here rather than assumed.
fn access_tick(run: &Run, access: Access, index: usize) -> usize {
    run.calls
        .iter()
        .filter(|(_, kind, _)| *kind == access)
        .nth(index)
        .expect("access not found")
        .0
}

/// A wait state holds the last driven address with the data pins off and no
/// control pin asserted.
fn held(address: u16) -> BusCycle {
    BusCycle {
        address,
        data: None,
        pins: Pins::default(),
    }
}

/// OUT (n),A — the port write's own cycle stretches, and the I/O call still
/// lands on the access tick.
#[test]
fn output_to_immediate_port_stalls_from_the_access() {
    let resting = run(&[0xD3, 0x10], Schedule::Released, |cpu| cpu.a = 0x42);
    assert_eq!(resting.ticks, 11);
    let access = access_tick(&resting, Access::IoWrite, 0);
    assert_eq!(access, 9);

    for length in 1..=4 {
        let stalled = run(
            &[0xD3, 0x10],
            Schedule::Held {
                from: access,
                length,
            },
            |cpu| cpu.a = 0x42,
        );
        assert_eq!(stalled.ticks, resting.ticks + length);
        assert_eq!(stalled.calls, resting.calls);
        assert_eq!(
            &stalled.trace[access + 1..access + 1 + length],
            &vec![held(0x4210); length]
        );
    }
}

/// LD A,(nn) — a memory read stretches the same way, and the byte still
/// reaches A.
#[test]
fn load_accumulator_absolute_stalls_on_its_read() {
    let program = [0x3A, 0x34, 0x12];
    let resting = run(&program, Schedule::Released, |_| {});
    assert_eq!(resting.ticks, 13);
    let access = access_tick(&resting, Access::MemRead, 3);
    assert_eq!(access, 11);

    for length in 1..=4 {
        let stalled = run(
            &program,
            Schedule::Held {
                from: access,
                length,
            },
            |_| {},
        );
        assert_eq!(stalled.ticks, resting.ticks + length);
        assert_eq!(stalled.calls, resting.calls);
        assert_eq!(stalled.cpu.a, resting.cpu.a);
        assert_eq!(
            &stalled.trace[access + 1..access + 1 + length],
            &vec![held(0x1234); length]
        );
    }
}

/// One T-state of assertion buys exactly one wait state: the sample at its
/// end sees the line released and the schedule resumes.
#[test]
fn release_after_one_wait_state() {
    let resting = run(&[0x7E], Schedule::Released, |cpu| {
        cpu.h = 0x12;
        cpu.l = 0x34;
    });
    assert_eq!(resting.ticks, 7);
    let access = access_tick(&resting, Access::MemRead, 1);

    let stalled = run(
        &[0x7E],
        Schedule::Held {
            from: access,
            length: 1,
        },
        |cpu| {
            cpu.h = 0x12;
            cpu.l = 0x34;
        },
    );
    assert_eq!(stalled.ticks, 8);
    assert_eq!(stalled.trace[access + 1], held(0x1234));
    assert_eq!(stalled.trace[access + 2..], resting.trace[access + 1..]);
}

/// M1's refresh T-states carry no sample point, so a line asserted across
/// them alone changes nothing.
#[test]
fn refresh_states_are_not_sampled() {
    let resting = run(&[0x00], Schedule::Released, |_| {});
    assert_eq!(resting.ticks, 4);

    let across_refresh = run(&[0x00], Schedule::Held { from: 2, length: 2 }, |_| {});
    assert_eq!(across_refresh.ticks, resting.ticks);
    assert_eq!(across_refresh.trace, resting.trace);
}

/// An internal cycle never samples either — the line asserted across the
/// five internal T-states of an (IX+d) operand leaves the instruction's
/// length alone.
#[test]
fn internal_cycles_are_not_sampled() {
    let program = [0xDD, 0x7E, 0x05];
    let resting = run(&program, Schedule::Released, |cpu| cpu.ix = 0x1230);
    assert_eq!(resting.ticks, 19);

    let across_padding = run(
        &program,
        Schedule::Held {
            from: 11,
            length: 5,
        },
        |cpu| cpu.ix = 0x1230,
    );
    assert_eq!(across_padding.ticks, resting.ticks);
    assert_eq!(across_padding.trace, resting.trace);
}
