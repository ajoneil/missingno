//! The save-state and recording round-trip shapes the cores share.

use std::hash::{Hash, Hasher};

use missingno_core::recording::{Recorder, Recording};
use missingno_core::system::{ControlId, ControlInput, ControlRole, SystemConsole};
use missingno_core::video::Frame;

/// Hash a displayed frame — the currency the round-trip gates compare
/// continuations in.
pub fn frame_hash(frame: &Frame) -> u64 {
    let rgba = frame.resolve_rgba();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rgba.width.hash(&mut hasher);
    rgba.height.hash(&mut hasher);
    rgba.pixels.hash(&mut hasher);
    hasher.finish()
}

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
