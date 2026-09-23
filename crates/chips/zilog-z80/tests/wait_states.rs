//! The /WAIT channel: a board holding the line low at a transfer cycle's
//! sample point stretches that cycle. The board answers for the pin through
//! `Bus::wait_requested`, whose default is a released line — the
//! SingleStepTests sweep rides that default, so this probe schedules
//! assertions against the tick the access lands on and counts the T-states
//! that follow.

use missingno_zilog_z80::{BusCycle, Cpu, InterruptMode, Pins};

#[path = "support/probe.rs"]
mod support;

use support::{Access, Schedule, run};

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
    let access = resting.access_tick(Access::IoWrite, 0);
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
    let access = resting.access_tick(Access::MemRead, 3);
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
    let access = resting.access_tick(Access::MemRead, 1);

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

/// Ticks for one instruction released, then with one wait state bought at
/// every M1 cycle's T2.
fn m1_waited(program: &[u8], prepare: fn(&mut Cpu)) -> (usize, usize) {
    let resting = run(program, Schedule::Released, prepare);
    let waited = run(program, Schedule::EveryM1, prepare);
    (resting.ticks, waited.ticks)
}

#[test]
fn an_m1_wait_stretches_the_opcode_fetch() {
    assert_eq!(m1_waited(&[0x00], |_| {}), (4, 5));
}

/// The operand reads are memory cycles, not M1: only the fetch pays.
#[test]
fn an_m1_wait_leaves_the_operand_reads_alone() {
    assert_eq!(m1_waited(&[0x3A, 0x34, 0x12], |_| {}), (13, 14));
}

/// A prefix spends a second M1 on the opcode it names, and that one pays too.
#[test]
fn a_prefixed_opcode_pays_for_both_fetches() {
    assert_eq!(m1_waited(&[0xCB, 0x00], |_| {}), (8, 10));
}

/// The acknowledge is an M1 cycle, so accepting an interrupt pays once.
#[test]
fn an_m1_wait_stretches_the_nmi_acknowledge() {
    assert_eq!(m1_waited(&[0x00], |cpu| cpu.trigger_nmi()), (11, 12));
}

#[test]
fn an_m1_wait_stretches_the_irq_acknowledge() {
    let interrupting = |cpu: &mut Cpu| {
        let mut state = cpu.boundary_state().expect("at a boundary");
        state.iff1 = true;
        state.iff2 = true;
        state.interrupt_mode = InterruptMode::Mode1;
        state.irq_line = true;
        state.irq_sampled = true;
        cpu.restore_boundary(&state);
    };
    assert_eq!(m1_waited(&[0x00], interrupting), (13, 14));
}
