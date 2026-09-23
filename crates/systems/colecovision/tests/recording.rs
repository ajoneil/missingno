//! The recording replay gate: record a session with scripted controller input
//! at known frames, replay it against a fresh console, and require every
//! frame-hash checkpoint to match. Runs the real BIOS and skips without
//! `COLECOVISION_BIOS`.

use missingno_colecovision::console::PORT1;
use missingno_colecovision::debug::create_console;
use missingno_colecovision::firmware::BIOS_SIZE;
use missingno_core::recording::Recording;
use missingno_core::system::{ControlId, ControlRole, SystemConsole};
use missingno_test_support::roundtrip::{
    assert_replay_refuses_other_cartridge, assert_replays_deterministically, record_scripted,
};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/accuracy/roms/");

/// Scripted changes to controller 1: (frame boundary, role, pressed).
const SCRIPT: &[(u64, ControlRole, bool)] = &[
    (1, ControlRole::Up, true),
    (3, ControlRole::Up, false),
    (3, ControlRole::Action(0), true),
    (5, ControlRole::Key(4), true),
    (6, ControlRole::Action(0), false),
    (7, ControlRole::Left, true),
];

fn bios(test: &str) -> Option<[u8; BIOS_SIZE]> {
    let Some(path) = std::env::var_os("COLECOVISION_BIOS") else {
        println!("{test}: skipped — set COLECOVISION_BIOS");
        return None;
    };
    let bytes = std::fs::read(&path).expect("the BIOS image reads");
    Some(bytes.try_into().expect("an 8 KB BIOS image"))
}

fn console(relative: &str, bios: [u8; BIOS_SIZE]) -> Box<dyn SystemConsole> {
    let path = format!("{CORPUS}{relative}");
    let rom = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    create_console(&rom, "test".into(), None, bios).expect("a flat cartridge image")
}

fn record(
    relative: &str,
    warmup: usize,
    frames: u64,
    interval: u64,
    bios: [u8; BIOS_SIZE],
) -> Recording {
    let mut console = console(relative, bios);
    for _ in 0..warmup {
        console.step_frame();
    }
    record_scripted(console.as_mut(), SCRIPT, frames, interval, |role| {
        ControlId::port(PORT1, role)
    })
}

#[test]
fn a_recording_replays_deterministically() {
    let Some(bios) = bios("a_recording_replays_deterministically") else {
        return;
    };
    let rom = "modes/graphic1.col";
    let recording = record(rom, 40, 24, 4, bios);
    assert_replays_deterministically(&recording, console(rom, bios).as_mut(), 24);
}

#[test]
fn replay_refuses_a_recording_from_another_cartridge() {
    let Some(bios) = bios("replay_refuses_a_recording_from_another_cartridge") else {
        return;
    };
    let recording = record("modes/graphic1.col", 20, 8, 4, bios);
    assert_replay_refuses_other_cartridge(
        &recording,
        console("sprites/priority.col", bios).as_mut(),
    );
}
