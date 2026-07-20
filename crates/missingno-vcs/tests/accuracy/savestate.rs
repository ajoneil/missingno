//! Save-state round-trip and error-case coverage.
//!
//! A VCS save is taken at an instruction boundary, where the CPU carries no
//! micro-sequencer residue and the φ0 grid phase follows the captured beam —
//! no Tier-2b residue. So a restore is bit-exact: every scanline emitted after
//! the restore matches the un-saved run. These tests save at a frame boundary
//! (the frame-assembly buffer empty) and assert the continuation frames are
//! byte-identical.

use crate::common::rom_path;
use missingno_core::state_file::{StateMeta, read_state_file, write_state_file};
use missingno_core::system::StateError;
use missingno_vcs::console::{Frame, Vcs};
use missingno_vcs::debug::create_console;
use missingno_vcs::snapshot::{capture_memory, read_state, restore};
use missingno_vcs::state_schema::vcs_state_schema;
use missingno_vcs::{CartType, TvStandard};

const FRAME_LINE_BUDGET: usize = 400;

fn load(relative: &str, cart: CartType) -> Vcs {
    let rom = std::fs::read(rom_path(relative)).unwrap();
    Vcs::new(&rom, TvStandard::Ntsc, Some(cart)).unwrap()
}

fn owned_memory(vcs: &Vcs) -> Vec<(String, Vec<u8>)> {
    capture_memory(vcs)
        .into_iter()
        .map(|(name, bytes)| (name.to_owned(), bytes))
        .collect()
}

fn frame_lines(frame: &Frame) -> Vec<[u8; 160]> {
    frame.lines.clone()
}

/// Settle the console, land it at a frame boundary (frame-assembly buffer just
/// drained) and then at an instruction boundary, and assert a save/restore into
/// a fresh console reproduces the next `frames` fields byte-for-byte.
fn assert_round_trip(relative: &str, cart: CartType, frames: usize) {
    let mut original = load(relative, cart);
    for _ in 0..8 {
        original.step_frame(FRAME_LINE_BUDGET);
    }
    // Complete a frame (the completed-line accumulator is now empty), then reach
    // an instruction boundary — still inside the VSYNC region, so the buffer
    // stays empty and the save point is a clean field boundary.
    original.step_frame(FRAME_LINE_BUDGET).expect("a frame");
    original.step_instruction();
    assert!(
        original.at_instruction_boundary(),
        "{relative}: save point is an instruction boundary"
    );

    let record = read_state(&original);
    let memory = owned_memory(&original);

    let mut restored = load(relative, cart);
    restore(&mut restored, &record, &memory).expect("restore succeeds");

    for i in 0..frames {
        let a = original
            .step_frame(FRAME_LINE_BUDGET)
            .expect("original frame");
        let b = restored
            .step_frame(FRAME_LINE_BUDGET)
            .expect("restored frame");
        assert_eq!(
            frame_lines(&a),
            frame_lines(&b),
            "{relative}: frame {i} after restore diverges"
        );
    }
}

#[test]
fn round_trip_draw_delay_is_bit_exact() {
    // A screenshot-class kernel with a sustained picture — the object counters,
    // ring phases, and serialiser latches are all live at the save point.
    assert_round_trip("tia-render/draw-delay_ntsc.a26", CartType::Plain4K, 4);
}

#[test]
fn round_trip_bank_f8_restores_the_bank() {
    // An F8 banked cart: the round-trip also proves the selected ROM bank is
    // restored (a fresh console would wake in bank 0).
    let mut original = load("cartridge/bank-f8_ntsc.a26", CartType::F8);
    for _ in 0..16 {
        original.step_frame(FRAME_LINE_BUDGET);
    }
    original.step_instruction();
    let saved_bank = original.cartridge().selected_bank();

    let record = read_state(&original);
    let memory = owned_memory(&original);
    let mut restored = load("cartridge/bank-f8_ntsc.a26", CartType::F8);
    restore(&mut restored, &record, &memory).unwrap();
    assert_eq!(
        restored.cartridge().selected_bank(),
        saved_bank,
        "the selected bank is restored"
    );

    assert_round_trip("cartridge/bank-f8_ntsc.a26", CartType::F8, 3);
}

#[test]
fn round_trip_bank_fa_restores_cart_ram() {
    // The CBS RAM Plus (FA) board has 256 bytes of cart RAM; the round-trip
    // carries it as a memory span. Confirm the captured RAM is non-trivial and
    // restores to the same bytes, and the continuation is bit-exact.
    let mut original = load("cartridge/bank-fa_ntsc.a26", CartType::Fa);
    for _ in 0..16 {
        original.step_frame(FRAME_LINE_BUDGET);
    }
    original.step_instruction();
    let memory = owned_memory(&original);
    let cart_ram = memory
        .iter()
        .find(|(name, _)| name == "cart_ram")
        .map(|(_, bytes)| bytes.clone())
        .expect("FA board carries a cart_ram span");
    assert_eq!(cart_ram.len(), 0x100, "CBS RAM Plus is 256 bytes");

    let record = read_state(&original);
    let mut restored = load("cartridge/bank-fa_ntsc.a26", CartType::Fa);
    restore(&mut restored, &record, &memory).unwrap();
    let restored_ram = owned_memory(&restored)
        .into_iter()
        .find(|(name, _)| name == "cart_ram")
        .map(|(_, bytes)| bytes)
        .unwrap();
    assert_eq!(restored_ram, cart_ram, "cart RAM restores byte-for-byte");

    assert_round_trip("cartridge/bank-fa_ntsc.a26", CartType::Fa, 3);
}

#[test]
fn read_state_validates_against_the_schema() {
    let mut vcs = load("tia-render/draw-delay_ntsc.a26", CartType::Plain4K);
    for _ in 0..4 {
        vcs.step_frame(FRAME_LINE_BUDGET);
    }
    vcs.step_instruction();
    let record = read_state(&vcs);
    assert_eq!(record.validate(vcs_state_schema()), Ok(()));
}

// ── Seam save/load + error cases ─────────────────────────────────

fn boundary_console(relative: &str) -> Box<dyn missingno_core::system::SystemDebugger> {
    let rom = std::fs::read(rom_path(relative)).unwrap();
    let console = create_console(&rom, "test".into(), Some(TvStandard::Ntsc), None).unwrap();
    let mut dbg = console.into_debugger().ok().expect("debugger backend");
    // Step instructions to settle and land on an instruction boundary.
    for _ in 0..20_000 {
        dbg.step();
    }
    dbg
}

#[test]
fn seam_save_and_load_round_trips() {
    let a = boundary_console("tia-render/draw-delay_ntsc.a26");
    let bytes = a.save_state().expect("save at an instruction boundary");

    let rom = std::fs::read(rom_path("tia-render/draw-delay_ntsc.a26")).unwrap();
    let console = create_console(&rom, "test".into(), Some(TvStandard::Ntsc), None).unwrap();
    let mut b = console.into_debugger().ok().expect("debugger backend");
    b.load_state(&bytes).expect("load a matching state");
}

#[test]
fn load_rejects_a_wrong_system_state() {
    // A state file that declares another system is refused before any record
    // parsing — the DMG↔VCS wrong-system guard, symmetric on both cores.
    let meta = StateMeta {
        system: "dmg",
        rom_sha256: None,
        emulator: "missingno",
        emulator_version: "0",
    };
    let record = missingno_core::state::StateRecord::new();
    let bytes = write_state_file(&meta, &record, &[], None);

    let mut console = boundary_console("tia-render/draw-delay_ntsc.a26");
    assert_eq!(console.load_state(&bytes), Err(StateError::WrongSystem));
}

#[test]
fn load_rejects_a_wrong_rom_state() {
    let meta = StateMeta {
        system: "vcs",
        rom_sha256: Some("00ff00ff"),
        emulator: "missingno",
        emulator_version: "0",
    };
    let record = missingno_core::state::StateRecord::new();
    let bytes = write_state_file(&meta, &record, &[], None);

    let mut console = boundary_console("tia-render/draw-delay_ntsc.a26");
    assert_eq!(console.load_state(&bytes), Err(StateError::IncompatibleRom));
}

#[test]
fn load_rejects_corrupt_and_wrong_version() {
    let mut console = boundary_console("tia-render/draw-delay_ntsc.a26");
    assert_eq!(console.load_state(b"not a state"), Err(StateError::Corrupt));

    // A real save with its version byte mutated is a version mismatch.
    let mut bytes = console.save_state().expect("save at a boundary");
    bytes[4] = 0xFE;
    assert_eq!(console.load_state(&bytes), Err(StateError::VersionMismatch));
}

#[test]
fn state_file_round_trips_the_record() {
    // The full container path: capture → write → read → rebuild the record.
    let mut vcs = load("cartridge/bank-f8_ntsc.a26", CartType::F8);
    for _ in 0..8 {
        vcs.step_frame(FRAME_LINE_BUDGET);
    }
    vcs.step_instruction();
    let record = read_state(&vcs);
    let memory = capture_memory(&vcs);
    let meta = StateMeta {
        system: "vcs",
        rom_sha256: Some("abc"),
        emulator: "missingno",
        emulator_version: "0",
    };
    let bytes = write_state_file(&meta, &record, &memory, None);
    let file = read_state_file(&bytes).unwrap();
    assert_eq!(file.system, "vcs");
    let rebuilt = vcs_state_schema().record_from(file.fields).unwrap();
    assert_eq!(rebuilt, record);
}
