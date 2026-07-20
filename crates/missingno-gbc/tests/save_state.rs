//! CGB save-state coverage: the colour console captures a schema-complete
//! record (the shared Game Boy state plus the CGB delta) and restores it
//! bit-exactly at an instruction boundary, and the cross-system guard rejects a
//! DMG state loaded into a CGB session.
//!
//! Restore is boundary-faithful (Tier-2a): the machine record round-trips
//! exactly at a frame boundary, and a static continuation reproduces the
//! frame-hash sequence bit-for-bit. When the save catches the display
//! mid-animation, the volatile pixel-pipeline latches (deliberately not
//! captured — the deferred Tier-2b residue) leave the first post-restore frame
//! transiently different before the sequence reconverges. Double-speed saves
//! carry no boundary-observable dot-phase alignment, so restore refuses them.

use std::hash::{Hash, Hasher};

use missingno_core::state_file::read_state_file;
use missingno_core::system::{StateError, SystemConsole};
use missingno_core::video::Frame;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::system::GbConsole;
use missingno_gb::{Dmg, GameBoy};
use missingno_gbc::{Cgb, GameBoyColor};

/// A booted CGB console wrapped in the system seam, from a gbc-crate ROM.
fn cgb_console(rom: &str) -> GbConsole<Cgb> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/accuracy/roms/");
    let rom = std::fs::read(format!("{path}{rom}")).expect("ROM present");
    let mut gbc = GameBoyColor::new(Cartridge::new(rom, None), None);
    missingno_gb::test_support::run_boot_rom(&mut gbc);
    GbConsole::new(gbc, |_| None)
}

fn cgb_acid2() -> GbConsole<Cgb> {
    cgb_console("cgb-acid2/cgb-acid2.gbc")
}

fn dmg_console() -> GbConsole<Dmg> {
    // A synthetic all-NOP DMG cartridge — enough to produce a DMG save state.
    let gb = GameBoy::new(Cartridge::new(vec![0u8; 0x8000], None), None);
    GbConsole::new(gb, |_| None)
}

fn frame_hash(frame: &Frame) -> u64 {
    let rgba = frame.resolve_rgba();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rgba.width.hash(&mut hasher);
    rgba.height.hash(&mut hasher);
    rgba.pixels.hash(&mut hasher);
    hasher.finish()
}

fn step_frame_hash(console: &mut dyn SystemConsole) -> u64 {
    match console.step_frame().display {
        Some(frame) => frame_hash(&frame),
        None => 0,
    }
}

/// The round-trip: `warmup` frames, save, `run` frames capturing hashes, load,
/// `run` frames again. The frame-hash continuations must match once the display
/// reconverges, and the record read straight back after the load must equal the
/// record at save (a faithful boundary restore).
fn assert_round_trips(
    mut console: GbConsole<Cgb>,
    warmup: usize,
    run: usize,
    converge_after: usize,
    lively: bool,
) {
    for _ in 0..warmup {
        console.step_frame();
    }

    let save = console.save_state().expect("CGB authors a save state");
    let record_at_save = console.read_state().expect("CGB reads its state");

    let baseline: Vec<u64> = (0..run).map(|_| step_frame_hash(&mut console)).collect();

    console.load_state(&save).expect("the save loads back");

    assert_eq!(
        console.read_state(),
        Some(record_at_save),
        "the record after load differs from the record at save"
    );

    let replay: Vec<u64> = (0..run).map(|_| step_frame_hash(&mut console)).collect();

    assert_eq!(
        baseline[converge_after..],
        replay[converge_after..],
        "the frame-hash continuations diverged"
    );
    if lively {
        assert!(
            baseline
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1,
            "the continuation should exercise more than one frame"
        );
    }
}

#[test]
fn cgb_save_captures_a_schema_complete_colour_record() {
    let mut console = cgb_acid2();
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

    // The bank-complete memory spans are captured: both VRAM banks, all eight
    // WRAM banks, and both palette RAMs.
    let span = |name: &str| {
        file.memory
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.len())
    };
    assert_eq!(span("vram"), Some(0x4000));
    assert_eq!(span("wram"), Some(0x8000));
    assert_eq!(span("cram_bg"), Some(64));
    assert_eq!(span("cram_obj"), Some(64));
}

#[test]
fn cgb_save_state_round_trips_static_continuation() {
    // cgb-acid2 settles to a static test screen: the whole frame-hash sequence
    // reproduces bit-for-bit — the strict round-trip gate.
    assert_round_trips(cgb_acid2(), 30, 15, 0, false);
}

#[test]
fn cgb_save_state_round_trips_animated() {
    // Blargg's interrupt_time prints its results while running, so a save early
    // in the run catches the display mid-animation. The continuation reconverges
    // after the one-frame pixel-pipeline transient (the Tier-2a residue). The
    // save lands single-speed (early boot, before the test's speed switch).
    let console = cgb_console("blargg/interrupt_time.gb");
    assert_round_trips(console, 2, 20, 1, true);
}

#[test]
fn cgb_rejects_a_dmg_state() {
    let dmg_save = dmg_console()
        .save_state()
        .expect("DMG authors a save state");
    let mut cgb = cgb_acid2();
    assert_eq!(cgb.load_state(&dmg_save), Err(StateError::WrongSystem));
}

#[test]
fn cgb_refuses_a_double_speed_save() {
    // A cartridge that arms KEY1 and STOPs to engage double speed, then loops.
    let mut rom = vec![0u8; 0x8000];
    rom[0x143] = 0xC0; // CGB cartridge
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xc3, 0x50, 0x01]); // NOP; JP $0150
    rom[0x150..0x158].copy_from_slice(&[
        0x3e, 0x01, // LD A,$01
        0xe0, 0x4d, // LDH ($4D),A — arm KEY1
        0x10, 0x00, // STOP — engage the speed switch
        0x18, 0xfe, // JR -2 — loop at double speed
    ]);
    let mut gbc = GameBoyColor::new(Cartridge::new(rom, None), None);
    missingno_gb::test_support::run_boot_rom(&mut gbc);
    let mut console = GbConsole::new(gbc, |_| None);

    for _ in 0..5 {
        console.step_frame();
    }

    // The console is now at double speed, and the save captures that.
    let record = console.read_state().expect("CGB reads its state");
    assert_eq!(
        record.get("double_speed"),
        Some(&missingno_core::state::StateValue::Bool(true)),
        "the test ROM should have engaged double speed"
    );

    // The save writes fine, but restoring it refuses — the dot-phase alignment
    // a speed switch leaves is not boundary-observable.
    let save = console.save_state().expect("CGB authors a save state");
    assert_eq!(
        console.load_state(&save),
        Err(StateError::DoubleSpeedBoundary)
    );
}
