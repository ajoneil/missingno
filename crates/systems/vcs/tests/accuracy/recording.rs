//! The VCS recording replay gate: record a session with scripted joystick and
//! fire events at known frames, replay it against a fresh console, and require
//! the frame-hash checkpoints to match end-to-end. A VCS save carries no
//! Tier-2b residue, so replay is bit-exact.

use crate::common::seam_console;
use missingno_core::recording::Recording;
use missingno_core::system::{ControlId, ControlRole};
use missingno_test_support::roundtrip::{
    assert_replay_refuses_other_cartridge, assert_replays_deterministically, record_scripted,
};

/// Scripted changes to the left joystick: (frame boundary, role, pressed).
const SCRIPT: &[(u64, ControlRole, bool)] = &[
    (1, ControlRole::Up, true),
    (3, ControlRole::Up, false),
    (3, ControlRole::Action(0), true),
    (6, ControlRole::Action(0), false),
    (7, ControlRole::Left, true),
];

fn record(relative: &str, warmup: usize, frames: u64, interval: u64) -> Recording {
    let mut console = seam_console(relative);
    for _ in 0..warmup {
        console.step_frame();
    }

    record_scripted(console.as_mut(), SCRIPT, frames, interval, |role| {
        ControlId::port(missingno_vcs::debug::LEFT_PORT, role)
    })
}

#[test]
fn vcs_recording_replays_deterministically() {
    let rom = "tia-render/draw-delay_ntsc.a26";
    let recording = record(rom, 8, 24, 4);
    assert_replays_deterministically(&recording, seam_console(rom).as_mut(), 24);
}

#[test]
fn replay_rejects_a_recording_for_a_different_rom() {
    let recording = record("tia-render/draw-delay_ntsc.a26", 8, 8, 4);
    assert_replay_refuses_other_cartridge(
        &recording,
        seam_console("tia-render/colors_ntsc.a26").as_mut(),
    );
}
