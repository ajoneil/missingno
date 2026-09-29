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

/// A cartridge that waits for each VBlank the way most games do: flag a byte
/// in HRAM, HALT, and loop until the VBlank handler has cleared it.
fn frame_waiting_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    // VBlank vector: count frames at $C000, clear the wait flag, return.
    rom[0x40..0x48].copy_from_slice(&[0x21, 0x00, 0xC0, 0x34, 0xAF, 0xE0, 0x80, 0xD9]);
    // Entry: jump over the header.
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]);
    rom[0x150..0x163].copy_from_slice(&[
        0x3E, 0x01, 0xE0, 0xFF, // ld a,1; ldh [IE],a
        0xFB, // ei
        0x3E, 0x01, 0xE0, 0x80, // wait: ld a,1; ldh [$80],a
        0x76, // halt
        0xF0, 0x80, 0xA7, 0x20, 0xFA, // ldh a,[$80]; and a; jr nz,halt
        0x04, 0x05, // inc b; dec b
        0x18, 0xF2, // jr wait
    ]);
    let checksum = rom[0x134..0x14D]
        .iter()
        .fold(0u8, |c, b| c.wrapping_sub(*b).wrapping_sub(1));
    rom[0x14D] = checksum;
    rom
}

/// Save at the end of each frame — the CPU halted and waking into the VBlank
/// interrupt — restore into a fresh console, and require it to follow the
/// original exactly: the same T-cycles per step and the same record after
/// every step, across the next frame.
#[test]
fn dmg_restore_at_a_frame_end_runs_in_lockstep() {
    use missingno_core::state::StateRecord;
    use missingno_gb::GameBoy;
    use missingno_gb::cartridge::Cartridge;
    use missingno_gb::snapshot::{capture_memory, read_shared_record};

    const FRAMES: usize = 30;
    const FOLLOW: usize = 4000;

    fn console() -> GameBoy {
        GameBoy::new(
            Cartridge::new(frame_waiting_rom(), None, None).unwrap(),
            None,
        )
    }
    fn synced_record(gb: &mut GameBoy) -> StateRecord {
        gb.sync_audio();
        gb.sync_ppu();
        read_shared_record(gb)
    }

    let mut original = console();
    let mut saves = Vec::new();
    let mut trail = Vec::new();
    let mut last_save = 0;
    while saves.len() < FRAMES || trail.len() < last_save + 1 + FOLLOW {
        let result = original.step();
        let record = synced_record(&mut original);
        if result.new_screen && saves.len() < FRAMES {
            assert!(
                original.cpu().is_halted(),
                "the frame ends with the CPU halted"
            );
            last_save = trail.len();
            saves.push((last_save, record.clone(), capture_memory(&original)));
        }
        trail.push((result.tcycles, record));
    }

    for (at, record, memory) in saves {
        let mut restored = console();
        let memory = memory
            .into_iter()
            .map(|(n, b)| (n.to_string(), b))
            .collect();
        restored.restore_boundary(&record, memory, None).unwrap();
        for (step, (tcycles, expected)) in trail[at + 1..at + 1 + FOLLOW].iter().enumerate() {
            assert_eq!(
                restored.step().tcycles,
                *tcycles,
                "save at step {at}: T-cycles differ {step} steps after the restore"
            );
            assert_eq!(
                &synced_record(&mut restored),
                expected,
                "save at step {at}: records differ {step} steps after the restore"
            );
        }
    }
}

/// Where the machine is after a frame: the T-cycles the frame took, the
/// picture, work and high RAM, and the timer's internal counter.
fn frame_fingerprint(gb: &mut missingno_gb::GameBoy) -> (u64, Vec<u8>, Vec<u8>, u16) {
    let mut tcycles = 0u64;
    loop {
        let step = gb.step();
        tcycles += step.tcycles as u64;
        if step.new_screen {
            break;
        }
    }
    let picture = gb
        .screen()
        .front()
        .pixels
        .iter()
        .flatten()
        .map(|p| p.0)
        .collect();
    let mut ram = gb.peek_range(0xC000, 0x2000);
    ram.extend(gb.peek_range(0xFF80, 0x7F));
    (tcycles, picture, ram, gb.timers().internal_counter())
}

/// A save taken through the DMG's own state, between frames or `into_frame`
/// instructions into one, written and read back as a state file, lands on a
/// fresh console that then runs in step with the original, frame for frame. Needs a ROM whose behaviour hangs on
/// hardware timing (Pokémon Red seeds its random numbers from DIV); give its
/// path in `DMG_TIMING_ROM`.
#[test]
#[ignore = "needs a commercial ROM in DMG_TIMING_ROM"]
fn a_restore_runs_in_step_with_the_original() {
    use missingno_core::state_file::{StateMeta, read_state_file, write_state_file};
    use missingno_gb::cartridge::Cartridge;
    use missingno_gb::system::ConsoleUi;
    use missingno_gb::{Dmg, GameBoy};

    let path = std::env::var("DMG_TIMING_ROM").expect("DMG_TIMING_ROM names a ROM");
    let rom = std::fs::read(&path).unwrap();
    let boot = || GameBoy::new(Cartridge::new(rom.clone(), None, None).unwrap(), None);

    let mut original = boot();
    let mut frame = 0;
    let saves = [
        (150, 0),
        (600, 0),
        (1400, 0),
        (1800, 100),
        (2200, 400),
        (2600, 900),
        (3000, 1500),
        (3400, 2500),
        (3800, 3300),
        (4200, 4100),
    ];
    for (save_at, into_frame) in saves {
        while frame < save_at {
            frame_fingerprint(&mut original);
            frame += 1;
        }
        for _ in 0..into_frame {
            original.step();
        }
        let record = <Dmg as ConsoleUi>::read_state(&original).unwrap();
        let memory = <Dmg as ConsoleUi>::capture_memory(&original);
        let meta = StateMeta {
            system: "dmg",
            rom_sha256: None,
            emulator: "missingno",
            emulator_version: "",
        };
        let file =
            read_state_file(&write_state_file(&meta, &record, &memory, None).unwrap()).unwrap();
        let record = missingno_gb::state_schema::dmg_state_schema()
            .record_from(file.fields)
            .unwrap();

        let mut restored = boot();
        for _ in 0..30 {
            frame_fingerprint(&mut restored);
        }
        restored
            .restore_boundary(&record, file.memory, None)
            .unwrap();
        for after in 0..400 {
            let (a, b) = (
                frame_fingerprint(&mut original),
                frame_fingerprint(&mut restored),
            );
            let parted = [
                ("T-cycles", a.0 != b.0),
                // A save doesn't carry the lines of the frame being drawn.
                ("picture", a.1 != b.1 && (after > 0 || into_frame == 0)),
                ("RAM", a.2 != b.2),
                ("timer", a.3 != b.3),
            ];
            let parted: Vec<_> = parted.iter().filter(|p| p.1).map(|p| p.0).collect();
            assert!(
                parted.is_empty(),
                "restored from frame {save_at}+{into_frame}, parted {after} frames later: {parted:?} ({} vs {} T-cycles)",
                a.0,
                b.0
            );
            frame += 1;
        }
    }
}
