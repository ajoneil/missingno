//! The DMG save-state round-trip gate: run a game, save, run K frames recording
//! the frame hashes, load the save, and run K frames again — the two
//! continuations must produce an identical frame-hash sequence, and the record
//! must round-trip exactly at the save boundary. Plus the cross-boundary error
//! cases (corrupt, unsupported version, wrong ROM).
//!
//! Saves are boundary-faithful (Tier-2a): the machine record round-trips
//! exactly at a frame boundary, and a static continuation reproduces the
//! frame-hash sequence bit-for-bit. When the save catches the display
//! mid-animation, the volatile pixel-pipeline latches (deliberately not
//! captured — the deferred Tier-2b residue) leave the very first post-restore
//! frame transiently different before the sequence reconverges.

use missingno_core::system::{StateError, SystemConsole};
use missingno_gb::system::{GbConsole, create_console};
use missingno_test_support::roundtrip;

/// Wrap a freshly booted DMG console in the system seam.
fn dmg_console(rom: &str) -> GbConsole<missingno_gb::Dmg> {
    let run = missingno_gb::test_support::load_rom(rom);
    create_console(run.gb, |_| None)
}

fn assert_round_trips(rom: &str, warmup: usize, run: usize, converge_after: usize, lively: bool) {
    roundtrip::assert_round_trips(&mut dmg_console(rom), warmup, run, converge_after, lively);
}

#[test]
fn dmg_save_state_round_trips_static_continuation() {
    // A save deep into execution, where the screen is static: the whole
    // frame-hash sequence reproduces bit-for-bit — the strict round-trip gate.
    assert_round_trips(
        "blargg/cpu_instrs/individual/06-ld r,r.gb",
        40,
        15,
        0,
        false,
    );
}

#[test]
fn dmg_save_state_round_trips_animated() {
    // A save at a frame boundary during boot, where cpu_instrs is still printing
    // — the continuation animates, and the frame-hash sequence reconverges after
    // the one-frame pixel-pipeline transient (the Tier-2a residue).
    assert_round_trips("blargg/cpu_instrs/individual/01-special.gb", 1, 20, 1, true);
}

#[test]
fn dmg_save_state_captures_progress() {
    // The state after running differs from a fresh boot — the save carries real
    // progress, not power-on defaults.
    let fresh = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    let boot = fresh.read_state().unwrap();

    let mut advanced = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    for _ in 0..30 {
        advanced.step_frame();
    }
    assert_ne!(boot, advanced.read_state().unwrap());
}

#[test]
fn load_rejects_a_corrupt_file() {
    let mut console = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    assert_eq!(
        console.load_state(b"not a save file at all"),
        Err(StateError::Corrupt)
    );
}

#[test]
fn load_rejects_an_unsupported_version() {
    let mut console = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    let mut save = console.save_state().unwrap();
    // Byte 4 is the container version; corrupt it to an unknown value.
    save[4] = 0xEE;
    assert_eq!(console.load_state(&save), Err(StateError::VersionMismatch));
}

#[test]
fn save_and_restore_are_refused_mid_instruction() {
    use missingno_core::system::SystemConsole as _;

    let mut console = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    // Settle onto an instruction boundary and take a good save there.
    for _ in 0..2 {
        console.step_frame();
    }
    let good_save = console.save_state().expect("a boundary save");

    let mut dbg = Box::new(console).into_debugger();

    // Ticking off the boundary, the save is refused the moment the CPU is inside
    // an instruction (past its fetch M-cycle) — and restoring a good save there
    // is an honest boundary error, distinctly not the generic corrupt case.
    let mut mid_instruction_error = None;
    for _ in 0..64 {
        dbg.step_tick();
        if dbg.save_state().is_none() {
            mid_instruction_error = Some(dbg.load_state(&good_save).unwrap_err());
            break;
        }
    }
    let err = mid_instruction_error.expect("a mid-instruction save must be refused");
    assert_eq!(err, StateError::NotAtBoundary);
    assert_ne!(err, StateError::Corrupt);
}

#[test]
fn load_rejects_a_state_for_a_different_rom() {
    let mut console = dmg_console("blargg/cpu_instrs/individual/06-ld r,r.gb");
    let other = dmg_console("blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb");
    let other_save = other.save_state().unwrap();
    assert_eq!(
        console.load_state(&other_save),
        Err(StateError::IncompatibleRom)
    );
}
