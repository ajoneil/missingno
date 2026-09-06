//! The per-core ROM→console registry: the one point that knows concrete
//! cores. Each entry pairs a media-recognition predicate with the launch
//! options its core publishes and a constructor that builds a
//! `Box<dyn SystemConsole>` from values for them; everything downstream is
//! generic.
//!
//! Entries are feature-gated, so a build carries only the cores it selected.
//! Off-chip Game Boy peripherals (serial link, printer, battery save) are
//! frontend policy — this factory constructs without them.

use std::path::Path;

use missingno_core::firmware::FirmwareSlot;
use missingno_core::launch::{LaunchOptionDescriptor, LaunchValues};
use missingno_core::system::SystemConsole;

/// Whether a path and its contents are this core's media.
type IsRom = fn(&Path, &[u8]) -> bool;
/// Build this core's console from a ROM's path and contents, honouring the
/// launch options the core published.
type Create = fn(&Path, &[u8], &LaunchValues) -> Result<Box<dyn SystemConsole>, LoadError>;

/// The console a caller names outright, bypassing recognition. It selects
/// between cores rather than configuring one, so the factory owns it: a generic
/// dump extension identifies nothing, and what no predicate claims must be
/// stated.
pub const SYSTEM: &str = "system";

/// Why media did not become a console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// No registered core claims this media.
    UnrecognizedMedia,
    /// A stated system no core in this build answers to.
    UnknownSystem(String),
    /// A launch value the core does not accept for that option.
    InvalidValue { option: String, value: String },
    /// A launch value the core accepts, but not for this media.
    IncompatibleOption { option: String, reason: String },
    /// The core's own objection to the media.
    Core(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::UnrecognizedMedia => f.write_str("no core recognises this media"),
            LoadError::UnknownSystem(stated) => write!(
                f,
                "{SYSTEM}: no such system \"{stated}\"; this build has: {}",
                factory_names().join(", ")
            ),
            LoadError::InvalidValue { option, value } => {
                write!(f, "{option}: no such value \"{value}\"")
            }
            LoadError::IncompatibleOption { option, reason } => write!(f, "{option}: {reason}"),
            LoadError::Core(message) => f.write_str(message),
        }
    }
}

/// A registered core: how its media is recognised, what it lets a loader
/// decide, and how a console is built.
pub struct CoreFactory {
    pub name: &'static str,
    pub is_rom: IsRom,
    pub create: Create,
    /// The launch options this core publishes for the media in hand: a choice
    /// the ROM's own header rules out is not among them.
    pub options: fn(&[u8]) -> Vec<LaunchOptionDescriptor>,
    /// Every firmware socket on this core's boards, whichever media is loaded.
    pub firmware: fn() -> Vec<FirmwareSlot>,
}

/// The file stem as a display title, falling back to a generic name. The
/// Game Boy family reads its title from the cartridge header, so only the
/// stem-titled cores use this.
#[cfg(any(feature = "vcs", feature = "nes", feature = "sms", feature = "sg1000"))]
fn title_for(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "ROM".to_string())
}

#[cfg(feature = "gb")]
mod gb {
    use super::*;
    use missingno_core::system::SystemConsole;
    use missingno_gb::cartridge::Cartridge;
    use missingno_gb::system::create_console;
    use missingno_gb::{GameBoy, media};
    use missingno_gbc::GameBoyColor;
    use missingno_gbc::launch::{self, BOARD, GbLaunch, RUNNER, RunnerPreference};

    /// The headless build persists no battery save; the format is frontend
    /// policy the GUI owns.
    fn no_battery(_: &Cartridge) -> Option<Vec<u8>> {
        None
    }

    /// No battery save and no link peripheral: both are frontend policy.
    pub fn create(
        _path: &Path,
        rom: &[u8],
        launch: &LaunchValues,
    ) -> Result<Box<dyn SystemConsole>, LoadError> {
        struct Boxed;
        impl GbLaunch for Boxed {
            type Output = Box<dyn SystemConsole>;
            fn dmg(self, console: GameBoy) -> Self::Output {
                Box::new(create_console(console, no_battery))
            }
            fn cgb(self, console: GameBoyColor) -> Self::Output {
                Box::new(create_console(console, no_battery))
            }
        }
        let board =
            launch::board_from_launch(launch).map_err(|refusal| LoadError::InvalidValue {
                option: BOARD.to_string(),
                value: refusal,
            })?;
        let cartridge = Cartridge::new(rom.to_vec(), board, None)
            .map_err(|refusal| LoadError::Core(refusal.to_string()))?;
        let boot_roms = launch::boot_roms_from_launch(launch).map_err(|(slot, image)| {
            LoadError::InvalidValue {
                option: slot.to_string(),
                value: image,
            }
        })?;
        let runner =
            RunnerPreference::from_launch(launch).map_err(|value| LoadError::InvalidValue {
                option: RUNNER.to_string(),
                value: value.to_string(),
            })?;
        launch::console(cartridge, boot_roms, None, runner, Boxed).map_err(|refusal| {
            LoadError::IncompatibleOption {
                option: RUNNER.to_string(),
                reason: refusal.to_string(),
            }
        })
    }

    pub fn options(rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
        launch::launch_options(rom)
    }

    /// One socket per console of the family; the console that boots reads its
    /// own.
    pub fn firmware() -> Vec<FirmwareSlot> {
        vec![
            missingno_gb::firmware::boot_rom_slot(),
            missingno_gbc::firmware::boot_rom_slot(),
        ]
    }

    pub fn is_rom(path: &Path, rom: &[u8]) -> bool {
        media::is_family_rom(path, rom)
    }
}

#[cfg(feature = "vcs")]
mod vcs {
    use super::*;
    use missingno_core::system::SystemConsole;

    use missingno_vcs::TvStandard;
    use missingno_vcs::debug::{BOARD, OVERDUMP, TV_STANDARD, board_from_launch};

    /// A stated board or standard is the catalogue's word on media that carries
    /// no header of its own — a value the core cannot read is an error, never a
    /// quiet fall back to inference.
    pub fn create(
        path: &Path,
        rom: &[u8],
        launch: &LaunchValues,
    ) -> Result<Box<dyn SystemConsole>, LoadError> {
        let standard = match launch.choice(TV_STANDARD) {
            Some(name) => {
                Some(
                    TvStandard::from_name(name).ok_or_else(|| LoadError::InvalidValue {
                        option: TV_STANDARD.to_string(),
                        value: name.to_string(),
                    })?,
                )
            }
            None => None,
        };
        let board = board_from_launch(launch).map_err(|refusal| LoadError::InvalidValue {
            option: BOARD.to_string(),
            value: refusal,
        })?;
        missingno_vcs::debug::create_console(
            rom,
            title_for(path),
            standard,
            board,
            launch.toggle(OVERDUMP),
        )
        .map_err(|error| LoadError::Core(error.to_string()))
    }

    pub fn is_rom(path: &Path, rom: &[u8]) -> bool {
        missingno_vcs::debug::is_vcs_rom(path, rom)
    }

    pub fn options(rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
        missingno_vcs::debug::launch_options(rom)
    }
}

#[cfg(feature = "nes")]
mod nes {
    use super::*;
    use missingno_core::machine::MachineConsole;
    use missingno_core::system::SystemConsole;
    use missingno_nes::console::Nes;
    use missingno_nes::debug::NesSystem;

    pub fn create(
        path: &Path,
        rom: &[u8],
        _launch: &LaunchValues,
    ) -> Result<Box<dyn SystemConsole>, LoadError> {
        let nes = Nes::new(rom).map_err(|error| LoadError::Core(format!("{error:?}")))?;
        Ok(Box::new(MachineConsole::<NesSystem>::new(
            nes,
            title_for(path),
        )))
    }

    pub fn is_rom(_path: &Path, rom: &[u8]) -> bool {
        missingno_nes::debug::is_nes_rom(rom)
    }
}

#[cfg(feature = "sms")]
mod sms {
    use super::*;
    use missingno_core::machine::MachineConsole;
    use missingno_core::system::SystemConsole;
    use missingno_sms::console::Sms;
    use missingno_sms::debug::SmsSystem;

    pub fn create(
        path: &Path,
        rom: &[u8],
        _launch: &LaunchValues,
    ) -> Result<Box<dyn SystemConsole>, LoadError> {
        let sms = Sms::new(rom).map_err(|error| LoadError::Core(format!("{error:?}")))?;
        Ok(Box::new(MachineConsole::<SmsSystem>::new(
            sms,
            title_for(path),
        )))
    }

    pub fn is_rom(path: &Path, _rom: &[u8]) -> bool {
        missingno_sms::debug::is_sms_rom(path)
    }
}

#[cfg(feature = "sg1000")]
mod sg1000 {
    use super::*;
    use missingno_core::system::SystemConsole;
    use missingno_sg1000::debug::{BOARD, board_from_launch};

    /// A stated board is the catalogue's word on media that carries no header of
    /// its own — a code the core cannot read is an error, never a quiet fall
    /// back to a plain ROM.
    pub fn create(
        path: &Path,
        rom: &[u8],
        launch: &LaunchValues,
    ) -> Result<Box<dyn SystemConsole>, LoadError> {
        let board = board_from_launch(launch).map_err(|refusal| LoadError::InvalidValue {
            option: BOARD.to_string(),
            value: refusal,
        })?;
        missingno_sg1000::debug::create_console(rom, title_for(path), board)
            .map_err(|error| LoadError::Core(error.to_string()))
    }

    pub fn is_rom(path: &Path, _rom: &[u8]) -> bool {
        missingno_sg1000::debug::is_sg1000_rom(path)
    }

    pub fn options(rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
        missingno_sg1000::debug::launch_options(rom)
    }
}

/// Every registered core, in claim order.
static FACTORIES: &[CoreFactory] = &[
    #[cfg(feature = "gb")]
    CoreFactory {
        name: "Game Boy",
        is_rom: gb::is_rom,
        create: gb::create,
        options: gb::options,
        firmware: gb::firmware,
    },
    #[cfg(feature = "vcs")]
    CoreFactory {
        name: "Atari VCS",
        is_rom: vcs::is_rom,
        create: vcs::create,
        options: vcs::options,
        firmware: Vec::new,
    },
    #[cfg(feature = "nes")]
    CoreFactory {
        name: "NES",
        is_rom: nes::is_rom,
        create: nes::create,
        options: |_| Vec::new(),
        firmware: Vec::new,
    },
    #[cfg(feature = "sms")]
    CoreFactory {
        name: "Master System",
        is_rom: sms::is_rom,
        create: sms::create,
        options: |_| Vec::new(),
        firmware: Vec::new,
    },
    #[cfg(feature = "sg1000")]
    CoreFactory {
        name: "SG-1000",
        is_rom: sg1000::is_rom,
        create: sg1000::create,
        options: sg1000::options,
        firmware: Vec::new,
    },
];

/// The factory whose media this is, if any core in this build claims it.
pub fn factory_for(path: &Path, rom: &[u8]) -> Option<&'static CoreFactory> {
    FACTORIES.iter().find(|factory| (factory.is_rom)(path, rom))
}

/// The factory a caller named, matched case-insensitively against the
/// registered names.
pub fn factory_named(name: &str) -> Option<&'static CoreFactory> {
    FACTORIES
        .iter()
        .find(|factory| factory.name.eq_ignore_ascii_case(name))
}

/// Every registered core's name, in claim order: what a caller may state.
pub fn factory_names() -> Vec<&'static str> {
    FACTORIES.iter().map(|factory| factory.name).collect()
}

/// Every firmware socket in this build, in claim order: what a firmware folder
/// is read against before any media is known.
pub fn firmware_slots() -> Vec<FirmwareSlot> {
    FACTORIES
        .iter()
        .flat_map(|factory| (factory.firmware)())
        .collect()
}

/// Build a console from a ROM's path and contents, leaving every launch option
/// to the core that claims it.
pub fn create_console(path: &Path, rom: &[u8]) -> Result<Box<dyn SystemConsole>, LoadError> {
    create_console_with(path, rom, &LaunchValues::default())
}

/// Build a console from the launch values a loader collected. A stated
/// [`SYSTEM`] settles which core builds it; otherwise recognition is unaffected
/// by them.
pub fn create_console_with(
    path: &Path,
    rom: &[u8],
    launch: &LaunchValues,
) -> Result<Box<dyn SystemConsole>, LoadError> {
    let factory = match launch.choice(SYSTEM) {
        Some(stated) => {
            factory_named(stated).ok_or_else(|| LoadError::UnknownSystem(stated.to_string()))?
        }
        None => factory_for(path, rom).ok_or(LoadError::UnrecognizedMedia)?,
    };
    (factory.create)(path, rom, launch)
}
