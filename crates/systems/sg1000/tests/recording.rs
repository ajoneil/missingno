//! The recording replay gate: record a session with scripted pad input at
//! known frames, replay it against a fresh console, and require every
//! frame-hash checkpoint to match. A save carries no sequencer residue, so the
//! replay is bit-exact.

use missingno_core::recording::Recording;
use missingno_core::system::{ControlId, ControlRole, SystemConsole};
use missingno_sg1000::console::JOY1;
use missingno_sg1000::debug::create_console;
use missingno_test_support::roundtrip::{
    assert_replay_refuses_other_cartridge, assert_replays_deterministically, record_scripted,
};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../chips/ti-vdp/tests/accuracy/roms/"
);

/// Scripted changes to player 1's pad: (frame boundary, role, pressed).
const SCRIPT: &[(u64, ControlRole, bool)] = &[
    (1, ControlRole::Up, true),
    (3, ControlRole::Up, false),
    (3, ControlRole::Action(0), true),
    (6, ControlRole::Action(0), false),
    (7, ControlRole::Left, true),
];

fn console(relative: &str) -> Box<dyn SystemConsole> {
    let path = format!("{CORPUS}{relative}");
    let rom = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    create_console(&rom, "test".into(), None).expect("flat cartridge image")
}

fn record(relative: &str, warmup: usize, frames: u64, interval: u64) -> Recording {
    let mut console = console(relative);
    for _ in 0..warmup {
        console.step_frame();
    }

    record_scripted(console.as_mut(), SCRIPT, frames, interval, |role| {
        ControlId::port(JOY1, role)
    })
}

#[test]
fn a_recording_replays_deterministically() {
    let rom = "modes/graphic1.sg";
    let recording = record(rom, 40, 24, 4);
    assert_replays_deterministically(&recording, console(rom).as_mut(), 24);
}

#[test]
fn replay_refuses_a_recording_from_another_cartridge() {
    let recording = record("modes/graphic1.sg", 20, 8, 4);
    assert_replay_refuses_other_cartridge(&recording, console("modes/text.sg").as_mut());
}
