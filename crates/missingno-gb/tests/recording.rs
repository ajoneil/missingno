//! The DMG recording replay gate: record a session with scripted input events
//! at known frames, replay it against a fresh console, and require the
//! frame-hash checkpoints to match end-to-end — deterministic by construction.
//! Plus the replay error cases (wrong ROM, container version mismatch).
//!
//! Recording re-seats the console from its own boundary save at start, so the
//! recorded timeline is exactly the continuation replay reproduces: the
//! frame-hash checks agree bit-for-bit, no convergence tolerance.

use missingno_core::recording::{
    Recorder, Recording, RecordingError, ReplayError, ReplayOutcome, replay,
};
use missingno_core::system::{ControlId, ControlInput, StateError, SystemConsole};
use missingno_gb::system::GbConsole;

fn dmg_console(rom: &str) -> GbConsole<missingno_gb::Dmg> {
    let run = missingno_gb::test_support::load_rom(rom);
    GbConsole::new(run.gb, |_| None)
}

/// Scripted control changes: (frame boundary, control id, pressed).
const SCRIPT: &[(u64, u8, bool)] = &[
    (0, 2, true),  // A down
    (2, 2, false), // A up
    (3, 0, true),  // Start down
    (6, 0, false), // Start up
    (8, 4, true),  // D-pad up down
];

/// Record `frames` frames of the ROM, applying the scripted inputs at their
/// boundaries and checkpointing a frame hash every `interval` frames.
fn record(rom: &str, warmup: usize, frames: u64, interval: u64) -> Recording {
    let mut console = dmg_console(rom);
    for _ in 0..warmup {
        console.step_frame();
    }

    let mut recorder =
        Recorder::start(&mut console, interval).expect("DMG authors a save-state backend");

    for frame in 0..frames {
        for &(at, control, pressed) in SCRIPT {
            if at == frame {
                let input = ControlInput::Digital(pressed);
                console.set_control(ControlId(control), input);
                recorder.note_input(ControlId(control), input);
            }
        }
        let produced = console.step_frame().display;
        recorder.note_frame(produced.as_ref());
    }

    recorder.finish()
}

#[test]
fn dmg_recording_replays_deterministically() {
    let rom = "blargg/cpu_instrs/individual/01-special.gb";
    let recording = record(rom, 1, 24, 4);
    assert!(
        !recording.checks.is_empty(),
        "the recording should carry frame-hash checkpoints"
    );
    assert!(
        !recording.inputs.is_empty(),
        "the recording should carry the scripted inputs"
    );

    // Exercise the container round-trip, then replay the parsed recording.
    let bytes = recording.to_bytes().unwrap();
    let parsed = Recording::from_bytes(&bytes).expect("the recording round-trips through bytes");

    let mut console = dmg_console(rom);
    let outcome = replay(&parsed, &mut console).expect("the recording replays");
    assert_eq!(
        outcome,
        ReplayOutcome {
            frames: 24,
            checks_verified: parsed.checks.len() as u64,
        }
    );
}

#[test]
fn replay_rejects_a_recording_for_a_different_rom() {
    let recording = record("blargg/cpu_instrs/individual/01-special.gb", 1, 8, 4);
    let mut other = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    assert_eq!(
        replay(&recording, &mut other),
        Err(ReplayError::State(StateError::IncompatibleRom))
    );
}

#[test]
fn replay_rejects_a_version_mismatched_initial_state() {
    let recording = record("blargg/cpu_instrs/individual/06-ld r,r.gb", 1, 8, 4);
    // The initial-state save file's version byte (past the 4-byte MPSV magic).
    let mut tampered = recording.clone();
    tampered.initial_state[4] = 0xEE;
    let mut console = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    assert_eq!(
        replay(&tampered, &mut console),
        Err(ReplayError::State(StateError::VersionMismatch))
    );
}

#[test]
fn recording_container_rejects_an_unsupported_version() {
    let mut bytes = record("blargg/cpu_instrs/individual/06-ld r,r.gb", 1, 4, 2)
        .to_bytes()
        .unwrap();
    bytes[4] = 0xEE;
    assert_eq!(
        Recording::from_bytes(&bytes),
        Err(RecordingError::UnsupportedVersion(0xEE))
    );
}
