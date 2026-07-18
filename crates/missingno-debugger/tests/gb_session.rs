//! With the `gb` feature, drive a Session over a minimal Game Boy ROM through
//! the same factory the server uses — no HTTP sockets involved.

#![cfg(feature = "gb")]

use std::path::Path;

use missingno_debugger::{Session, factory};

/// A 32 KiB all-NOP ROM. It carries no boot logo, so the `.gb` extension is
/// what makes the registry claim it; the DMG core boots to PC 0x0100.
fn minimal_rom() -> Vec<u8> {
    vec![0x00; 0x8000]
}

fn session() -> Session {
    let rom = minimal_rom();
    let console = factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory should claim a .gb ROM");
    let debugger = console
        .into_debugger()
        .ok()
        .expect("gb has a debugger backend");
    Session::new(debugger)
}

#[test]
fn steps_and_reads_state() {
    let mut session = session();

    // The DMG boots to the cartridge entry point.
    let start = session.pc();
    assert_eq!(start, 0x0100);

    // A single instruction advances the PC (NOP is one byte).
    session.step();
    assert_eq!(session.pc(), start + 1);

    // Registers and memory regions come through the schema.
    assert!(!session.register_groups().is_empty());
    assert!(!session.memory_regions().is_empty());

    // Peek reads the ROM: byte at the entry point is our NOP.
    assert_eq!(session.peek(0x0100), 0x00);
    assert_eq!(session.memory(0x0100, 4), vec![0x00, 0x00, 0x00, 0x00]);
}

#[test]
fn breakpoints_round_trip() {
    let mut session = session();
    session.set_breakpoint(0x0150);
    assert!(session.breakpoints().contains(&0x0150));
    session.clear_breakpoint(0x0150);
    assert!(!session.breakpoints().contains(&0x0150));
}

#[test]
fn disassembly_window_decodes_from_pc() {
    let session = session();
    let lines = session
        .disassembly(session.pc(), 4)
        .expect("gb core has a disassembler");
    assert_eq!(lines.len(), 4);
    // NOP decodes to a one-byte instruction row.
    assert!(!lines[0].is_data);
    assert_eq!(lines[0].length, 1);
}
