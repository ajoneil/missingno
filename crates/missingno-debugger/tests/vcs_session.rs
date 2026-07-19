//! With the `vcs` feature, drive a Session over a minimal Atari VCS ROM through
//! the same factory the server uses, exercising the sub-instruction tick seam.

#![cfg(feature = "vcs")]

use std::path::Path;

use missingno_core::inspect::WatchTerm;
use missingno_debugger::{Session, factory};

fn value_term(key: &str, value: u32) -> WatchTerm {
    WatchTerm {
        key: key.to_string(),
        address: None,
        value: Some(value),
    }
}

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
fn watchables_list_the_pc_and_cart_bank_keys() {
    let session = session();
    let keys: Vec<&str> = session.watchables().iter().map(|w| w.key).collect();
    assert!(keys.contains(&"pc"));
    assert!(keys.contains(&"cart-bank"));
}

#[test]
fn compound_pc_bank_watch_round_trips() {
    let mut session = session();
    let compound = vec![value_term("pc", 0xF006), value_term("cart-bank", 1)];
    let added = session
        .add_watch(compound.clone())
        .expect("compound validates against the watchables");
    assert!(session.watches().contains(&added));
    session
        .remove_watch(compound)
        .expect("removes the compound");
    assert!(session.watches().is_empty());
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
