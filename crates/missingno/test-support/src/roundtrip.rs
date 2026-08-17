//! The save-state and recording round-trip shapes the cores share.

use std::collections::HashSet;

use missingno_core::recording::{
    Recorder, Recording, ReplayError, ReplayOutcome, frame_hash, replay,
};
use missingno_core::system::{ControlId, ControlInput, ControlRole, StateError, SystemConsole};

/// Step one frame and hash whatever it displayed; a step the display produced
/// no frame for hashes as zero.
pub fn step_frame_hash(console: &mut dyn SystemConsole) -> u64 {
    match console.step_frame().display {
        Some(frame) => frame_hash(&frame),
        None => 0,
    }
}

/// Record `frames` frames, applying each scripted `(frame, role, pressed)`
/// change at its boundary and checkpointing a frame hash every `interval`
/// frames. `seat` names the control site the roles belong to.
pub fn record_scripted(
    console: &mut dyn SystemConsole,
    script: &[(u64, ControlRole, bool)],
    frames: u64,
    interval: u64,
    seat: impl Fn(ControlRole) -> ControlId,
) -> Recording {
    let mut recorder =
        Recorder::start(&mut *console, interval).expect("the console authors a save state");

    for frame in 0..frames {
        for &(at, role, pressed) in script {
            if at == frame {
                let control = seat(role);
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

/// The save-state round-trip gate: `warmup` frames, save, `run` frames
/// capturing frame hashes, load, `run` frames again. The record read straight
/// back after the load must equal the record at save, and the two frame-hash
/// continuations must agree from `converge_after` onwards — one frame of
/// tolerance when the save caught the display mid-animation, zero for a static
/// continuation. `lively` additionally requires the continuation to have
/// animated at all.
pub fn assert_round_trips(
    console: &mut dyn SystemConsole,
    warmup: usize,
    run: usize,
    converge_after: usize,
    lively: bool,
) {
    for _ in 0..warmup {
        console.step_frame();
    }

    let save = console
        .save_state()
        .expect("the console authors a save state");
    let record_at_save = console.read_state().expect("the console reads its state");

    let baseline: Vec<u64> = (0..run).map(|_| step_frame_hash(console)).collect();

    console.load_state(&save).expect("the save loads back");

    assert_eq!(
        console.read_state(),
        Some(record_at_save),
        "the record after load differs from the record at save"
    );

    let replayed: Vec<u64> = (0..run).map(|_| step_frame_hash(console)).collect();

    assert_eq!(
        baseline[converge_after..],
        replayed[converge_after..],
        "the frame-hash continuations diverged"
    );
    if lively {
        assert!(
            baseline.iter().collect::<HashSet<_>>().len() > 1,
            "the continuation should exercise more than one frame"
        );
    }
}

/// The deterministic-replay gate: the recording carries checkpoints and the
/// scripted inputs, round-trips through its container, and replays against a
/// fresh console with every checkpoint verified.
pub fn assert_replays_deterministically(
    recording: &Recording,
    fresh: &mut dyn SystemConsole,
    frames: u64,
) {
    assert!(
        !recording.checks.is_empty(),
        "the recording should carry frame-hash checkpoints"
    );
    assert!(
        !recording.events.is_empty(),
        "the recording should carry the scripted inputs"
    );

    let bytes = recording.to_bytes().unwrap();
    let parsed = Recording::from_bytes(&bytes).expect("the recording round-trips through bytes");

    let outcome = replay(&parsed, fresh).expect("the recording replays");
    assert_eq!(
        outcome,
        ReplayOutcome {
            frames,
            checks_verified: parsed.checks.len() as u64,
        }
    );
}

/// A recording made against one cartridge must not replay against another.
pub fn assert_replay_refuses_other_cartridge(recording: &Recording, other: &mut dyn SystemConsole) {
    assert_eq!(
        replay(recording, other),
        Err(ReplayError::State(StateError::IncompatibleRom))
    );
}
