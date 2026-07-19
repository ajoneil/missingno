//! With the `gb` feature, drive a Session over a minimal Game Boy ROM through
//! the same factory the server uses — no HTTP sockets involved.

#![cfg(feature = "gb")]

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

/// The value of a named PPU-section row, if present.
fn ppu_row(session: &Session, label: &str) -> Option<String> {
    use missingno_core::inspect::SectionBlock;
    let sections = session.sidebar_sections();
    let ppu = sections.iter().find(|section| section.name == "PPU")?;
    for block in &ppu.blocks {
        if let SectionBlock::Rows(rows) = block {
            if let Some(row) = rows.iter().find(|row| row.label == label) {
                return Some(row.value.clone());
            }
        }
    }
    None
}

#[test]
fn gb_advertises_a_dot_tick_finer_than_an_instruction() {
    let mut session = session();
    // The Game Boy's sub-instruction tick is the dot (T-cycle).
    assert_eq!(session.tick_name(), Some("dot"));

    // A NOP is four dots: a single dot does not complete it, and the fourth
    // advances the PC by one — the tick is genuinely sub-instruction.
    let pc0 = session.pc();
    session.step_tick();
    assert_eq!(session.pc(), pc0, "one dot must not finish a four-dot NOP");
    for _ in 0..3 {
        session.step_tick();
    }
    assert_eq!(session.pc(), pc0 + 1, "four dots complete exactly one NOP");
}

#[test]
fn ppu_section_carries_folded_stat_and_lyc() {
    let session = session();
    // The former `gb_ppu_state` STAT and LYC fields now live in the PPU
    // sidebar section the generic `describe_machine` renders.
    assert!(ppu_row(&session, "stat").is_some());
    assert!(ppu_row(&session, "lyc").is_some());
}

#[test]
fn watchables_list_the_pc_and_bank_keys() {
    // The dynamic listing /watchables and the MCP watch tool description render.
    let session = session();
    let keys: Vec<&str> = session.watchables().iter().map(|w| w.key).collect();
    assert!(keys.contains(&"pc"));
    assert!(keys.contains(&"rom-bank"));
    assert!(keys.contains(&"sram-bank"));
}

#[test]
fn compound_pc_bank_watch_round_trips() {
    // The gutter's `{pc, bank}` compound validates and survives the watch list.
    let mut session = session();
    let compound = vec![value_term("pc", 0x4000), value_term("rom-bank", 3)];
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
