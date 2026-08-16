//! The CGB recording replay gate: the colour console records a scripted input
//! session and replays it deterministically through the shared recording path.
//! Confirms the recording machinery is model-generic — the CGB console reaches
//! it with no CGB-specific code.

use missingno_core::recording::{Recording, ReplayOutcome, replay};
use missingno_core::system::{ControlId, ControlRole, SystemConsole};
use missingno_gb::cartridge::Cartridge;
use missingno_gb::system::{GbConsole, create_console};
use missingno_gbc::{Cgb, GameBoyColor};
use missingno_test_support::roundtrip::record_scripted;

fn cgb_console(rom: &str) -> GbConsole<Cgb> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/accuracy/roms/");
    let rom = std::fs::read(format!("{path}{rom}")).expect("ROM present");
    let mut gbc = GameBoyColor::new(Cartridge::new(rom, None), None);
    missingno_gb::test_support::run_boot_rom(&mut gbc);
    create_console(gbc, |_| None)
}

const SCRIPT: &[(u64, ControlRole, bool)] = &[
    (0, ControlRole::Action(0), true),
    (2, ControlRole::Action(0), false),
    (4, ControlRole::Start, true),
];

#[test]
fn cgb_recording_replays_deterministically() {
    let rom = "cgb-acid2/cgb-acid2.gbc";
    let mut console = cgb_console(rom);
    for _ in 0..30 {
        console.step_frame();
    }

    let recording = record_scripted(&mut console, SCRIPT, 16, 4, ControlId::integrated);

    let bytes = recording.to_bytes().unwrap();
    let parsed = Recording::from_bytes(&bytes).expect("round-trips through bytes");

    let mut fresh = cgb_console(rom);
    let outcome = replay(&parsed, &mut fresh).expect("the CGB recording replays");
    assert_eq!(
        outcome,
        ReplayOutcome {
            frames: 16,
            checks_verified: parsed.checks.len() as u64,
        }
    );
}
