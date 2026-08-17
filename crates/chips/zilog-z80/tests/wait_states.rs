//! The /WAIT channel: a board holding the line low at a transfer cycle's
//! sample point stretches that cycle. The board answers for the pin through
//! `Bus::wait_requested`, whose default is a released line — the
//! SingleStepTests sweep rides that default, so this probe schedules
//! assertions against the tick the access lands on and counts the T-states
//! that follow.

use missingno_zilog_z80::{BusCycle, Pins};

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
