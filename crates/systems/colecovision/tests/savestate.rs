//! Save-state round-trip and refusal coverage.
//!
//! A save is taken at an instruction boundary, where the Z80 holds no
//! sequencer residue and no M1 wait is in flight; the VDP and PSG are captured
//! wherever that boundary leaves them. So a restore is bit-exact: every raster
//! emitted after it matches the un-saved run. Every test runs the real BIOS and
//! skips without `COLECOVISION_BIOS`.

use missingno_colecovision::console::{ColecoVision, tstates_per_frame};
use missingno_colecovision::debug::create_console;
use missingno_colecovision::firmware::BIOS_SIZE;
use missingno_colecovision::snapshot::{capture, restore};
use missingno_colecovision::state_schema::colecovision_state_schema;
use missingno_core::machine::BoundaryState;
use missingno_core::state::StateValue;
use missingno_core::state_file::{StateMeta, write_state_file};
use missingno_core::system::{StateError, SystemDebugger};
use missingno_ti_vdp::Standard;

/// The `.col` corpus, borrowed for scenes that put a picture up: the board is
/// what these tests exercise, not the chip's verdicts.
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/accuracy/roms/");

fn bios(test: &str) -> Option<[u8; BIOS_SIZE]> {
    let Some(path) = std::env::var_os("COLECOVISION_BIOS") else {
        println!("{test}: skipped — set COLECOVISION_BIOS");
        return None;
    };
    let bytes = std::fs::read(&path).expect("the BIOS image reads");
    Some(bytes.try_into().expect("an 8 KB BIOS image"))
}

fn image(relative: &str) -> Vec<u8> {
    let path = format!("{CORPUS}{relative}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn load_cut_for(relative: &str, standard: Standard, bios: [u8; BIOS_SIZE]) -> ColecoVision {
    ColecoVision::new(&image(relative), standard, bios).expect("a flat cartridge image")
}

fn frame_budget(console: &ColecoVision) -> u32 {
    4 * tstates_per_frame(console.standard())
}

fn run_frames(console: &mut ColecoVision, frames: usize) {
    for _ in 0..frames {
        console.step_frame(frame_budget(console));
    }
}

fn next_raster(console: &mut ColecoVision) -> Vec<u8> {
    console
        .step_frame(frame_budget(console))
        .expect("a frame completes")
        .pixels
        .clone()
}

fn owned(state: &BoundaryState) -> Vec<(String, Vec<u8>)> {
    state
        .memory
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), bytes.clone()))
        .collect()
}

/// Step to the first instruction boundary on a given raster line.
fn run_to_line(console: &mut ColecoVision, line: u16) {
    for _ in 0..tstates_per_frame(console.standard()) as usize {
        console.step_instruction();
        if console.vdp().line() == line {
            return;
        }
    }
    panic!("the raster never reached line {line}");
}

/// Save `original` where it stands, restore into a fresh console, and require
/// the next `frames` rasters to match the un-saved run byte for byte.
fn assert_continues_identically(
    original: &mut ColecoVision,
    relative: &str,
    frames: usize,
    bios: [u8; BIOS_SIZE],
) {
    let state = capture(original).expect("a boundary save");
    let mut restored = load_cut_for(relative, original.standard(), bios);
    restore(
        &mut restored,
        &state.record,
        &owned(&state),
        state.frame.as_ref(),
    )
    .expect("restore succeeds");

    for index in 0..frames {
        assert_eq!(
            next_raster(original),
            next_raster(&mut restored),
            "{relative}: raster {index} after restore diverges"
        );
    }
}

#[test]
fn a_scene_restored_at_a_frame_handoff_continues_identically() {
    let Some(bios) = bios("a_scene_restored_at_a_frame_handoff_continues_identically") else {
        return;
    };
    let relative = "modes/graphic1.col";
    let mut original = load_cut_for(relative, Standard::Ntsc, bios);
    run_frames(&mut original, 90);
    assert_continues_identically(&mut original, relative, 3, bios);
}

/// Partway down the picture: the rows already emitted, the row under the
/// raster, the latched fetch and the sprite plane all ride the save.
#[test]
fn a_scene_restored_mid_picture_continues_identically() {
    let Some(bios) = bios("a_scene_restored_mid_picture_continues_identically") else {
        return;
    };
    let relative = "sprites/priority.col";
    let mut original = load_cut_for(relative, Standard::Ntsc, bios);
    run_frames(&mut original, 90);
    run_to_line(&mut original, 100);
    assert_continues_identically(&mut original, relative, 3, bios);
}

/// The TMS9929A's taller picture rides the save as the TMS9928A's does.
#[test]
fn a_pal_scene_restored_mid_picture_continues_identically() {
    let Some(bios) = bios("a_pal_scene_restored_mid_picture_continues_identically") else {
        return;
    };
    let relative = "sprites/priority.col";
    let mut original = load_cut_for(relative, Standard::Pal, bios);
    run_frames(&mut original, 90);
    run_to_line(&mut original, 100);
    let state = capture(&original).expect("a boundary save");
    assert_eq!(state.frame.and_then(|frame| frame.height), Some(294));
    assert_continues_identically(&mut original, relative, 3, bios);
}

/// The kilobyte of RAM travels with the state — a fresh console wakes with it
/// zeroed.
#[test]
fn the_ram_rides_the_save() {
    let Some(bios) = bios("the_ram_rides_the_save") else {
        return;
    };
    let relative = "modes/graphic1.col";
    let mut original = load_cut_for(relative, Standard::Ntsc, bios);
    run_frames(&mut original, 90);
    let written: Vec<u8> = (0..0x400)
        .map(|offset| original.peek(0x6000 + offset))
        .collect();
    assert!(written.iter().any(|&byte| byte != 0), "the scene used RAM");

    let state = capture(&original).expect("a boundary save");
    let mut restored = load_cut_for(relative, Standard::Ntsc, bios);
    restore(
        &mut restored,
        &state.record,
        &owned(&state),
        state.frame.as_ref(),
    )
    .expect("restore succeeds");

    let read_back: Vec<u8> = (0..0x400)
        .map(|offset| restored.peek(0x6000 + offset))
        .collect();
    assert_eq!(read_back, written);
}

#[test]
fn a_captured_record_validates_against_the_schema() {
    let Some(bios) = bios("a_captured_record_validates_against_the_schema") else {
        return;
    };
    let mut console = load_cut_for("modes/graphic1.col", Standard::Ntsc, bios);
    run_frames(&mut console, 30);
    let state = capture(&console).expect("a boundary save");
    assert_eq!(state.record.validate(colecovision_state_schema()), Ok(()));
    assert!(
        state.frame.is_some_and(|frame| !frame.data.is_empty()),
        "the save carries the field being emitted"
    );
    assert!(matches!(
        state.record.get("nmi_line"),
        Some(StateValue::Bool(_))
    ));
}

#[test]
fn a_save_is_refused_mid_instruction() {
    let Some(bios) = bios("a_save_is_refused_mid_instruction") else {
        return;
    };
    let mut console = load_cut_for("modes/graphic1.col", Standard::Ntsc, bios);
    run_frames(&mut console, 4);
    console.step_tstate();
    assert!(!console.at_instruction_boundary());
    assert!(matches!(capture(&console), Err(StateError::NotAtBoundary)));
}

// ── Through the seam ──────────────────────────────────────────────

fn seam_console(relative: &str, bios: [u8; BIOS_SIZE]) -> Box<dyn SystemDebugger> {
    let console = create_console(&image(relative), "test".into(), None, bios)
        .expect("a flat cartridge image");
    let mut debugger = console.into_debugger();
    for _ in 0..20 {
        debugger.step_frame();
    }
    debugger
}

#[test]
fn the_seam_saves_and_loads_a_state() {
    let Some(bios) = bios("the_seam_saves_and_loads_a_state") else {
        return;
    };
    let saved = seam_console("modes/graphic1.col", bios);
    let bytes = saved.save_state().expect("a save at a frame handoff");

    let mut loaded = seam_console("modes/graphic1.col", bios);
    loaded.load_state(&bytes).expect("a matching state loads");
}

#[test]
fn loading_refuses_a_state_from_another_system() {
    let Some(bios) = bios("loading_refuses_a_state_from_another_system") else {
        return;
    };
    let meta = StateMeta {
        system: "sg1000",
        rom_sha256: None,
        emulator: "missingno",
        emulator_version: "0",
    };
    let bytes =
        write_state_file(&meta, &missingno_core::state::StateRecord::new(), &[], None).unwrap();

    let mut console = seam_console("modes/graphic1.col", bios);
    assert_eq!(console.load_state(&bytes), Err(StateError::WrongSystem));
}

#[test]
fn loading_refuses_a_state_from_another_cartridge() {
    let Some(bios) = bios("loading_refuses_a_state_from_another_cartridge") else {
        return;
    };
    let saved = seam_console("modes/graphic1.col", bios);
    let bytes = saved.save_state().expect("a save at a frame handoff");

    let mut other = seam_console("sprites/priority.col", bios);
    assert_eq!(other.load_state(&bytes), Err(StateError::IncompatibleRom));
}

#[test]
fn loading_refuses_corrupt_bytes_and_another_container_version() {
    let Some(bios) = bios("loading_refuses_corrupt_bytes_and_another_container_version") else {
        return;
    };
    let mut console = seam_console("modes/graphic1.col", bios);
    assert_eq!(console.load_state(b"not a state"), Err(StateError::Corrupt));

    let mut bytes = console.save_state().expect("a save at a frame handoff");
    bytes[4] = 0xFE;
    assert_eq!(console.load_state(&bytes), Err(StateError::VersionMismatch));
}
