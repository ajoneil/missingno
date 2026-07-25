//! The VCS recording replay gate: record a session with scripted joystick and
//! fire events at known frames, replay it against a fresh console, and require
//! the frame-hash checkpoints to match end-to-end. A VCS save carries no
//! Tier-2b residue, so replay is bit-exact.

use crate::common::rom_path;
use missingno_core::recording::{Recorder, Recording, ReplayError, ReplayOutcome, replay};
use missingno_core::system::{ControlId, ControlInput, ControlRole, StateError, SystemConsole};
use missingno_vcs::TvStandard;
use missingno_vcs::debug::create_console;

fn console(relative: &str) -> Box<dyn SystemConsole> {
    let rom = std::fs::read(rom_path(relative)).unwrap();
    create_console(&rom, "test".into(), Some(TvStandard::Ntsc), None).unwrap()
}

/// Scripted changes to the left joystick: (frame boundary, role, pressed).
const SCRIPT: &[(u64, ControlRole, bool)] = &[
    (1, ControlRole::Up, true),
    (3, ControlRole::Up, false),
    (3, ControlRole::Action(0), true),
    (6, ControlRole::Action(0), false),
    (7, ControlRole::Left, true),
];

fn record(relative: &str, warmup: usize, frames: u64, interval: u64) -> Recording {
    let mut console = console(relative);
    for _ in 0..warmup {
        console.step_frame();
    }

    let mut recorder = Recorder::start(console.as_mut(), interval).expect("VCS saves state");

    for frame in 0..frames {
        for &(at, role, pressed) in SCRIPT {
            if at == frame {
                let control = ControlId::port(missingno_vcs::debug::LEFT_PORT, role);
                let input = ControlInput::Digital(pressed);
                console.set_control(control, input);
                recorder.note_input(control, input);
            }
        }
        let produced = console.step_frame().display;
        recorder.note_frame(produced.as_ref());
    }

    recorder.finish()
}

#[test]
fn vcs_recording_replays_deterministically() {
    let rom = "tia-render/draw-delay_ntsc.a26";
    let recording = record(rom, 8, 24, 4);
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
fn replay_rejects_a_recording_for_a_different_rom() {
    let recording = record("tia-render/draw-delay_ntsc.a26", 8, 8, 4);
    let mut other = console("tia-render/colors_ntsc.a26");
    assert_eq!(
        replay(&recording, other.as_mut()),
        Err(ReplayError::State(StateError::IncompatibleRom))
    );
}
