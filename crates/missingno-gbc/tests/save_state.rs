//! CGB save-state coverage: the colour console captures a schema-complete
//! record (the shared Game Boy state plus the CGB delta), and the cross-system
//! guard rejects a DMG state loaded into a CGB session.
//!
//! Restore is not yet wired for the colour core — the CGB banked VRAM/WRAM and
//! palette RAM need capture and reconstruction the boundary bridge does not yet
//! cover — so `load_state` of a CGB save reports `Unsupported` rather than
//! mis-restoring. The capture side proves the schema is genuinely general.

use missingno_core::state_file::read_state_file;
use missingno_core::system::{StateError, SystemConsole};
use missingno_gb::cartridge::Cartridge;
use missingno_gb::system::GbConsole;
use missingno_gb::{Dmg, GameBoy};
use missingno_gbc::{Cgb, GameBoyColor};

fn cgb_console() -> GbConsole<Cgb> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/accuracy/roms/cgb-acid2/cgb-acid2.gbc"
    );
    let rom = std::fs::read(path).expect("cgb-acid2 ROM present");
    let mut gbc = GameBoyColor::new(Cartridge::new(rom, None), None);
    missingno_gb::test_support::run_boot_rom(&mut gbc);
    GbConsole::new(gbc, |_| None)
}

fn dmg_console() -> GbConsole<Dmg> {
    // A synthetic all-NOP DMG cartridge — enough to produce a DMG save state.
    let gb = GameBoy::new(Cartridge::new(vec![0u8; 0x8000], None), None);
    GbConsole::new(gb, |_| None)
}

#[test]
fn cgb_save_captures_a_schema_complete_colour_record() {
    let mut console = cgb_console();
    for _ in 0..10 {
        console.step_frame();
    }

    let bytes = console.save_state().expect("CGB authors a save state");
    let file = read_state_file(&bytes).expect("the save file parses");
    assert_eq!(file.system, "cgb");

    // The record rebuilds and validates against the CGB schema — every
    // non-nullable field, shared and colour-delta, is present and well-typed.
    let schema = missingno_gbc::state_schema::cgb_state_schema();
    let record = schema
        .record_from(file.fields)
        .expect("the CGB record validates against its schema");

    // The colour delta is captured.
    for field in [
        "double_speed",
        "svbk",
        "vbk",
        "opri",
        "bcps",
        "ocps",
        "hdma_active",
    ] {
        assert!(record.get(field).is_some(), "missing colour field {field}");
    }
}

#[test]
fn cgb_rejects_a_dmg_state() {
    let dmg_save = dmg_console()
        .save_state()
        .expect("DMG authors a save state");
    let mut cgb = cgb_console();
    assert_eq!(cgb.load_state(&dmg_save), Err(StateError::WrongSystem));
}
