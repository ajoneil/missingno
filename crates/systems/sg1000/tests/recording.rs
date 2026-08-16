//! The recording replay gate: record a session with scripted pad input at
//! known frames, replay it against a fresh console, and require every
//! frame-hash checkpoint to match. A save carries no sequencer residue, so the
//! replay is bit-exact.

use missingno_core::recording::{Recording, ReplayError, ReplayOutcome, replay};
use missingno_core::system::{ControlId, ControlRole, StateError, SystemConsole};
use missingno_sg1000::console::JOY1;
use missingno_sg1000::debug::create_console;
use missingno_test_support::roundtrip::record_scripted;

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
    create_console(&rom, "test".into()).expect("flat cartridge image")
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
    assert!(
        !recording.checks.is_empty(),
        "carries frame-hash checkpoints"
    );
    assert!(!recording.events.is_empty(), "carries the scripted inputs");

    let bytes = recording.to_bytes().unwrap();
    let parsed = Recording::from_bytes(&bytes).expect("round-trips through bytes");

    let mut console = console(rom);
    let outcome = replay(&parsed, console.as_mut()).expect("the recording replays");
    assert_eq!(
        outcome,
        ReplayOutcome {
            frames: 24,
            checks_verified: parsed.checks.len() as u64,
        }
    );
}

#[test]
fn replay_refuses_a_recording_from_another_cartridge() {
    let recording = record("modes/graphic1.sg", 20, 8, 4);
    let mut other = console("modes/text.sg");
    assert_eq!(
        replay(&recording, other.as_mut()),
        Err(ReplayError::State(StateError::IncompatibleRom))
    );
}
