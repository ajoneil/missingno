//! The CGB recording replay gate: the colour console records a scripted input
//! session and replays it deterministically through the shared recording path.
//! Confirms the recording machinery is model-generic — the CGB console reaches
//! it with no CGB-specific code.

use missingno_core::recording::{Recorder, Recording, ReplayOutcome, replay};
use missingno_core::system::{ControlId, ControlInput, ControlRole, SystemConsole};
use missingno_gb::cartridge::Cartridge;
use missingno_gb::system::{GbConsole, create_console};
use missingno_gbc::{Cgb, GameBoyColor};

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

    let mut recorder = Recorder::start(&mut console, 4).expect("CGB saves state");
    for frame in 0..16u64 {
        for &(at, role, pressed) in SCRIPT {
            if at == frame {
                let control = ControlId::integrated(role);
                let input = ControlInput::Digital(pressed);
                console.set_control(control, input);
                recorder.note_input(control, input);
            }
        }
        let produced = console.step_frame().display;
        recorder.note_frame(produced.as_ref());
    }
    let recording = recorder.finish();

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
