//! Naming the console outright: what recognition cannot settle — a `.bin` dump
//! is any core's — a caller states through the factory's own launch option.

#![cfg(all(feature = "sg1000", feature = "vcs"))]

use std::path::Path;

use missingno_core::launch::LaunchValues;
use missingno_session::factory::{self, LoadError, SYSTEM};

/// No firmware folder: every test here supplies its own image or none at all.
fn no_firmware() -> missingno_session::FirmwareLibrary {
    missingno_session::FirmwareLibrary::scan(std::path::PathBuf::new(), &[])
}

/// 4 KiB of NOPs: both a bare VCS ROM size and a flat SG-1000 image.
fn dump() -> Vec<u8> {
    vec![0x00; 0x1000]
}

#[test]
fn a_stated_system_builds_its_core_without_consulting_a_predicate() {
    let mut launch = LaunchValues::default();
    launch.set_choice(SYSTEM, "SG-1000");
    let console =
        factory::create_console_with(Path::new("dump.bin"), &dump(), &launch, &no_firmware())
            .expect("a stated system settles which core builds the media");
    assert_eq!(console.game_title(), "dump");
}

#[test]
fn a_stated_system_matches_a_registered_name_whatever_its_case() {
    assert_eq!(
        factory::factory_named("sg-1000").map(|factory| factory.name),
        Some("SG-1000")
    );
}

#[test]
fn a_system_no_core_answers_to_names_the_candidates() {
    let mut launch = LaunchValues::default();
    launch.set_choice(SYSTEM, "Jaguar");
    let Err(error) =
        factory::create_console_with(Path::new("dump.bin"), &dump(), &launch, &no_firmware())
    else {
        panic!("no core is registered as a Jaguar");
    };
    assert_eq!(error, LoadError::UnknownSystem("Jaguar".to_string()));
    let message = error.to_string();
    for name in factory::factory_names() {
        assert!(message.contains(name), "{message} should list {name}");
    }
}

#[test]
fn an_unstated_bin_dump_is_no_cores_media() {
    let Err(error) = factory::create_console(Path::new("dump.bin"), &dump()) else {
        panic!("a generic dump extension identifies no core");
    };
    assert_eq!(error, LoadError::UnrecognizedMedia);
}
