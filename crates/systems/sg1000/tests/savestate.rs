//! Save-state round-trip and refusal coverage.
//!
//! A save is taken at an instruction boundary, where the Z80 holds no
//! sequencer residue, and the VDP and PSG are captured wherever that boundary
//! leaves them — mid-line included. So a restore is bit-exact: every raster
//! emitted after it matches the un-saved run, whether the save was taken at a
//! frame handoff or partway down the picture.

use missingno_core::machine::BoundaryState;
use missingno_core::state_file::{StateMeta, write_state_file};
use missingno_core::system::{StateError, SystemDebugger};
use missingno_sg1000::cartridge::CartType;
use missingno_sg1000::console::{Sg1000, TSTATES_PER_FRAME};
use missingno_sg1000::debug::create_console;
use missingno_sg1000::snapshot::{capture, restore};
use missingno_sg1000::state_schema::sg1000_state_schema;

/// The ti-vdp conformance corpus, borrowed here for scenes that put a picture
/// up: the board is what these tests exercise, not the chip's verdicts.
const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../chips/ti-vdp/tests/accuracy/roms/"
);

const FRAME_BUDGET: u32 = 4 * TSTATES_PER_FRAME;

fn image(relative: &str) -> Vec<u8> {
    let path = format!("{CORPUS}{relative}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn load(relative: &str) -> Sg1000 {
    Sg1000::new(&image(relative), None).expect("flat cartridge image")
}

/// Run `frames` frames, each landing at an instruction boundary.
fn run_frames(console: &mut Sg1000, frames: usize) {
    for _ in 0..frames {
        console.step_frame(FRAME_BUDGET);
    }
}

fn next_raster(console: &mut Sg1000) -> Vec<u8> {
    console
        .step_frame(FRAME_BUDGET)
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

/// Step to the first instruction boundary on a given raster line. No Z80
/// instruction spans a line, so every line is reachable.
fn run_to_line(console: &mut Sg1000, line: u16) {
    for _ in 0..TSTATES_PER_FRAME as usize {
        console.step_instruction();
        if console.vdp().line() == line {
            return;
        }
    }
    panic!("the raster never reached line {line}");
}

/// Save `original` where it stands, restore into a fresh console, and require
/// the next `frames` rasters to match the un-saved run byte for byte.
fn assert_continues_identically(original: &mut Sg1000, relative: &str, frames: usize) {
    let state = capture(original).expect("a boundary save");
    let mut restored = load(relative);
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

/// A settled scene, saved where the last frame was handed out.
fn assert_frame_handoff_continues(relative: &str, frames: usize) {
    let mut original = load(relative);
    run_frames(&mut original, 90);
    assert_continues_identically(&mut original, relative, frames);
}

/// A settled scene, saved on a display line — with rows of the field already
/// emitted, a row half composited under the raster, and a fetch latched.
fn assert_mid_picture_continues(relative: &str, line: u16, frames: usize) {
    assert!(line < missingno_ti_vdp::ACTIVE_LINES, "a display line");
    let mut original = load(relative);
    run_frames(&mut original, 90);
    run_to_line(&mut original, line);
    assert_continues_identically(&mut original, relative, frames);
}

#[test]
fn a_scene_restored_at_a_frame_handoff_continues_identically() {
    assert_frame_handoff_continues("modes/graphic1.sg", 3);
}

/// Partway down the picture: the rows already emitted, the row under the
/// raster, the latched fetch and the sprite plane all ride the save.
#[test]
fn a_scene_restored_mid_picture_continues_identically() {
    assert_mid_picture_continues("sprites/priority.sg", 100, 3);
}

/// A sweep that leans on the CPU-access schedule, so the port engine's
/// in-flight access and pending flag are live at the save point.
#[test]
fn a_vram_sweep_restored_mid_picture_continues_identically() {
    assert_mid_picture_continues("timing/steal-sweep.sg", 64, 2);
}

/// The interrupt line and the frame flag: the save carries IFF1, the sampled
/// /INT level and the instant F was set.
#[test]
fn an_interrupt_driven_scene_continues_identically() {
    assert_frame_handoff_continues("interrupt/cadence.sg", 3);
}

/// The kilobyte of work RAM travels with the state — a fresh console wakes
/// with it zeroed.
#[test]
fn the_work_ram_rides_the_save() {
    let mut original = load("modes/graphic1.sg");
    run_frames(&mut original, 90);
    let written = (0..0x400).map(|offset| original.peek(0xC000 + offset));
    assert!(written.clone().any(|byte| byte != 0), "the scene used RAM");

    let state = capture(&original).expect("a boundary save");
    let mut restored = load("modes/graphic1.sg");
    restore(
        &mut restored,
        &state.record,
        &owned(&state),
        state.frame.as_ref(),
    )
    .expect("restore succeeds");

    let read_back: Vec<u8> = (0..0x400)
        .map(|offset| restored.peek(0xC000 + offset))
        .collect();
    assert_eq!(read_back, written.collect::<Vec<u8>>());
}

/// A RAM-bearing board's own store rides the save the work RAM does: a fresh
/// console of the same board wakes with it cleared, and the restore fills it.
#[test]
fn cart_ram_rides_the_save() {
    // LD A,$5A / LD ($8000),A / LD A,$A5 / LD ($87FF),A / spin.
    let mut rom = vec![0u8; 0x8000];
    rom[..13].copy_from_slice(&[
        0x3E, 0x5A, 0x32, 0x00, 0x80, 0x3E, 0xA5, 0x32, 0xFF, 0x87, 0xC3, 0x0A, 0x00,
    ]);
    let board = Some(CartType::OthelloRam);

    let mut original = Sg1000::new(&rom, board).expect("an image the board holds");
    for _ in 0..8 {
        original.step_instruction();
    }
    assert_eq!(original.peek(0x8000), 0x5A);
    assert_eq!(original.peek(0x87FF), 0xA5);

    let state = capture(&original).expect("a boundary save");
    let mut restored = Sg1000::new(&rom, board).expect("an image the board holds");
    assert_eq!(restored.peek(0x8000), 0x00, "cart RAM wakes cleared");
    restore(
        &mut restored,
        &state.record,
        &owned(&state),
        state.frame.as_ref(),
    )
    .expect("restore succeeds");

    assert_eq!(restored.cart_ram(), original.cart_ram());
    assert_eq!(restored.peek(0x8000), 0x5A);
    assert_eq!(restored.peek(0x87FF), 0xA5);
}

/// A board with no RAM of its own contributes no region.
#[test]
fn a_flat_board_carries_no_cart_ram_region() {
    let console = load("modes/graphic1.sg");
    let state = capture(&console).expect("a boundary save");
    assert!(!state.memory.iter().any(|(name, _)| *name == "cart_ram"));
}

#[test]
fn a_captured_record_validates_against_the_schema() {
    let mut console = load("modes/graphic2.sg");
    run_frames(&mut console, 30);
    let state = capture(&console).expect("a boundary save");
    assert_eq!(state.record.validate(sg1000_state_schema()), Ok(()));
    assert!(
        state.frame.is_some_and(|frame| !frame.data.is_empty()),
        "the save carries the field being emitted"
    );
}

#[test]
fn a_save_is_refused_mid_instruction() {
    let mut console = load("modes/graphic1.sg");
    run_frames(&mut console, 4);
    console.step_tstate();
    assert!(!console.at_instruction_boundary());
    assert!(matches!(capture(&console), Err(StateError::NotAtBoundary)));
}

// ── Through the seam ──────────────────────────────────────────────

fn seam_console(relative: &str) -> Box<dyn SystemDebugger> {
    let console =
        create_console(&image(relative), "test".into(), None).expect("flat cartridge image");
    let mut debugger = console.into_debugger();
    for _ in 0..20 {
        debugger.step_frame();
    }
    debugger
}

#[test]
fn the_seam_saves_and_loads_a_state() {
    let saved = seam_console("modes/graphic1.sg");
    let bytes = saved.save_state().expect("a save at a frame handoff");

    let mut loaded = seam_console("modes/graphic1.sg");
    loaded.load_state(&bytes).expect("a matching state loads");
}

#[test]
fn loading_refuses_a_state_from_another_system() {
    let meta = StateMeta {
        system: "dmg",
        rom_sha256: None,
        emulator: "missingno",
        emulator_version: "0",
    };
    let bytes =
        write_state_file(&meta, &missingno_core::state::StateRecord::new(), &[], None).unwrap();

    let mut console = seam_console("modes/graphic1.sg");
    assert_eq!(console.load_state(&bytes), Err(StateError::WrongSystem));
}

#[test]
fn loading_refuses_a_state_from_another_cartridge() {
    let saved = seam_console("modes/graphic1.sg");
    let bytes = saved.save_state().expect("a save at a frame handoff");

    let mut other = seam_console("modes/text.sg");
    assert_eq!(other.load_state(&bytes), Err(StateError::IncompatibleRom));
}

#[test]
fn loading_refuses_corrupt_bytes_and_another_container_version() {
    let mut console = seam_console("modes/graphic1.sg");
    assert_eq!(console.load_state(b"not a state"), Err(StateError::Corrupt));

    let mut bytes = console.save_state().expect("a save at a frame handoff");
    bytes[4] = 0xFE;
    assert_eq!(console.load_state(&bytes), Err(StateError::VersionMismatch));
}
