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

/// A CGB-header ROM: the CGB flag at 0x143 makes the factory boot the colour
/// core, like a CGB cartridge in a real GBC.
fn cgb_rom() -> Vec<u8> {
    let mut rom = vec![0x00; 0x8000];
    rom[0x143] = 0xC0;
    rom
}

fn session_from(path: &str, rom: &[u8]) -> Session {
    let console = factory::create_console(Path::new(path), rom)
        .expect("factory should not error")
        .expect("gb factory should claim the ROM");
    let debugger = console
        .into_debugger()
        .ok()
        .expect("gb has a debugger backend");
    Session::new(debugger)
}

#[test]
fn dmg_drives_a_passive_stn_lcd() {
    use missingno_core::video::{DisplayTechnology, LcdPanel};
    match session().video_out() {
        DisplayTechnology::Lcd {
            native,
            panel,
            pixel_aspect,
        } => {
            assert_eq!(native, (160, 144));
            assert_eq!(panel, LcdPanel::PassiveStn);
            assert_eq!(pixel_aspect, 1.0);
        }
        other => panic!("DMG should drive an LCD, got {other:?}"),
    }
}

#[test]
fn cgb_drives_an_active_tft_lcd() {
    use missingno_core::video::{DisplayTechnology, LcdPanel};
    let session = session_from("test.gbc", &cgb_rom());
    match session.video_out() {
        DisplayTechnology::Lcd {
            native,
            panel,
            pixel_aspect,
        } => {
            assert_eq!(native, (160, 144));
            assert_eq!(panel, LcdPanel::ActiveTft);
            assert_eq!(pixel_aspect, 1.0);
        }
        other => panic!("CGB should drive an LCD, got {other:?}"),
    }
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
    session.set_breakpoint(0x0150).unwrap();
    assert!(session.breakpoints().contains(&0x0150));
    session.clear_breakpoint(0x0150);
    assert!(!session.breakpoints().contains(&0x0150));
}

#[test]
fn synthetic_breakpoint_rejected() {
    let mut session = session();
    // A synthetic bank-complete address is not a bus address; setting a
    // breakpoint there must be rejected, not truncated into a phantom stop.
    assert!(session.set_breakpoint(0x0200_8123).is_err());
    assert!(session.breakpoints().is_empty());
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

/// A four-bank MBC5 ROM, each 16 KiB bank stamped with its index.
fn banked_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 4 * 0x4000];
    for (i, bank) in rom.chunks_mut(0x4000).enumerate() {
        bank.fill(i as u8);
    }
    rom[0x147] = 0x19; // MBC5
    rom[0x148] = 0x01; // 64 KiB
    rom
}

fn banked_session() -> Session {
    let rom = banked_rom();
    let console = factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory should claim a .gb ROM");
    Session::new(console.into_debugger().ok().expect("gb has a debugger"))
}

#[test]
fn disassembly_at_synthetic_anchor_decodes_unmapped_bank() {
    // The synthetic ROM base mirrors the debugger's bank-complete space.
    const ROM_BASE: u32 = 0x0200_0000;
    let session = banked_session();

    // The CPU bus pages bank 1 (all 0x01) into $4000; bank 3 (all 0x03) is
    // reachable only through the synthetic space. 0x01 is LD BC,d16 (3 bytes),
    // 0x03 is INC BC (1 byte), so the decode proves which bank was read.
    let anchor = ROM_BASE + 3 * 0x4000;
    let lines = session
        .disassembly(anchor, 3)
        .expect("gb core has a disassembler");
    assert_eq!(
        lines[0].address, anchor,
        "the anchor keeps its synthetic base"
    );
    assert_eq!(lines[0].bytes, vec![0x03]);
    assert_eq!(lines[0].length, 1);
    // Successive rows step by one and stay in the synthetic ROM decade.
    assert_eq!(lines[1].address, anchor + 1);
    assert_eq!(lines[2].address, anchor + 2);
}
