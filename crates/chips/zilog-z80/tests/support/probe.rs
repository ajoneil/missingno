//! A tick-at-a-time probe bus and the loop that runs one instruction against
//! it. Included via `#[path]` — integration tests are separate crates, so each
//! pulls in its own copy and uses the subset it needs.
#![allow(dead_code)]

use missingno_zilog_z80::{Bus, BusCycle, Cpu};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    MemRead,
    MemWrite,
    IoRead,
    IoWrite,
}

/// One bus call: the tick it arrived on, what it was, and the address.
pub type Call = (usize, Access, u16);

/// What the board does with /WAIT while the instruction runs.
#[derive(Clone, Copy)]
pub enum Schedule {
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

pub struct ProbeBus {
    pub memory: Vec<u8>,
    pub port_input: u8,
    pub tick: usize,
    pub calls: Vec<Call>,
    pub schedule: Schedule,
}

impl ProbeBus {
    pub fn new(program: &[u8]) -> Self {
        ProbeBus::with_schedule(program, Schedule::Released)
    }

    pub fn with_schedule(program: &[u8], schedule: Schedule) -> Self {
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

/// One instruction run to retirement: the CPU it left, the bus calls made,
/// the T-state count, and the recorded snapshots.
pub struct Run {
    pub cpu: Cpu,
    pub calls: Vec<Call>,
    pub ticks: usize,
    pub trace: Vec<BusCycle>,
}

impl Run {
    /// The ticks whose recorded snapshot asserts a transfer's pins, in order.
    pub fn asserted(&self) -> Vec<Call> {
        self.trace
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
            .collect()
    }

    /// The tick the `n`th call of `access` lands on.
    pub fn access_tick(&self, access: Access, index: usize) -> usize {
        self.calls
            .iter()
            .filter(|(_, kind, _)| *kind == access)
            .nth(index)
            .expect("access not found")
            .0
    }
}

pub fn run(program: &[u8], schedule: Schedule, prepare: impl FnOnce(&mut Cpu)) -> Run {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0000;
    prepare(&mut cpu);
    let mut bus = ProbeBus::with_schedule(program, schedule);

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
