//! A ColecoVision boots through the factory with its BIOS supplied from the
//! firmware folder alone — the path the curator and the headless debugger
//! take. Skips without `COLECOVISION_BIOS`.

#![cfg(feature = "colecovision")]

use std::path::{Path, PathBuf};

use missingno_core::launch::{LaunchValues, TV_STANDARD};
use missingno_session::factory;
use missingno_session::{FirmwareDefaults, FirmwareLibrary};

/// A firmware folder holding only the image `COLECOVISION_BIOS` names, or
/// `None` where the variable is unset or names no dump a socket knows.
fn folder_with_bios(test: &str) -> Option<PathBuf> {
    let Some(path) = std::env::var_os("COLECOVISION_BIOS") else {
        println!("{test}: skipped — set COLECOVISION_BIOS");
        return None;
    };
    let bytes = std::fs::read(&path).expect("COLECOVISION_BIOS names a readable file");
    let slots = factory::firmware_slots();
    if !slots.iter().any(|slot| slot.identify(&bytes).is_some()) {
        println!("{test}: skipped — COLECOVISION_BIOS is no dump a socket knows");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("missingno-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("dropped-in-under-any-name.bin"), bytes).unwrap();
    Some(dir)
}

/// A flat cartridge: the game magic the BIOS looks for, then nothing.
fn dump() -> Vec<u8> {
    let mut rom = vec![0x00; 0x1000];
    rom[..2].copy_from_slice(&[0xAA, 0x55]);
    rom
}

#[test]
fn a_bios_in_the_folder_boots_a_colecovision_nobody_chose_firmware_for() {
    let Some(dir) = folder_with_bios("bios_in_folder_boots") else {
        return;
    };
    let firmware = FirmwareLibrary::scan(dir.clone(), &factory::firmware_slots());
    for standard in [None, Some("ntsc"), Some("pal")] {
        let mut launch = LaunchValues::default();
        if let Some(standard) = standard {
            launch.set_choice(TV_STANDARD, standard);
        }
        factory::create_console_with(
            Path::new("game.col"),
            &dump(),
            &launch,
            &firmware,
            &FirmwareDefaults::default(),
        )
        .unwrap_or_else(|error| panic!("standard {standard:?}: {error}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
