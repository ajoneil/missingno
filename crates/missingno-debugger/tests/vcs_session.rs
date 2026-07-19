//! With the `vcs` feature, drive a Session over a minimal Atari VCS ROM through
//! the same factory the server uses, exercising the sub-instruction tick seam.

#![cfg(feature = "vcs")]

use std::path::Path;

use missingno_debugger::{Session, factory};

/// A 4 KiB ROM whose reset vector points at its origin ($F000). The bytes
/// there decode to whatever; the beam advances per colour clock regardless of
/// what the CPU executes.
fn minimal_rom() -> Vec<u8> {
    let mut rom = vec![0xEA; 0x1000]; // NOPs
    rom[0xFFC] = 0x00;
    rom[0xFFD] = 0xF0;
    rom
}

fn session() -> Session {
    let rom = minimal_rom();
    let console = factory::create_console(Path::new("test.a26"), &rom)
        .expect("factory should not error")
        .expect("vcs factory should claim an .a26 ROM");
    let debugger = console
        .into_debugger()
        .ok()
        .expect("vcs has a debugger backend");
    Session::new(debugger)
}

/// The beam position from the running-status video summary ("beam N · line M").
fn beam(session: &Session) -> u32 {
    let summary = session.running_status().video_summary;
    let rest = summary
        .strip_prefix("beam ")
        .expect("summary starts with the beam position");
    let number = rest.split(' ').next().expect("a beam number");
    number.parse().expect("beam is numeric")
}

#[test]
fn vcs_advertises_a_colour_clock_tick_that_steps_one_clock() {
    let mut session = session();
    assert_eq!(session.tick_name(), Some("colour clock"));

    // One colour clock advances the beam by exactly one, wrapping at line end.
    let before = beam(&session);
    session.step_tick();
    let after = beam(&session);
    assert!(
        after == before + 1 || after < before,
        "beam {before} -> {after} should advance by one colour clock"
    );
}
