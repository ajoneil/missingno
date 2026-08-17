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

use missingno_core::launch::{LaunchOptionDescriptor, LaunchValues};
use missingno_core::system::SystemConsole;

/// Whether a path and its contents are this core's media.
pub type IsRom = fn(&Path, &[u8]) -> bool;
/// Build this core's console from a ROM's path and contents, honouring the
/// launch options the core published.
pub type Create = fn(&Path, &[u8], &LaunchValues) -> Result<Box<dyn SystemConsole>, LoadError>;

/// Why media did not become a console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// No registered core claims this media.
    UnrecognizedMedia,
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
    /// The launch options this core publishes.
    pub options: fn() -> Vec<LaunchOptionDescriptor>,
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
    use missingno_gb::{BootRom, GameBoy, media};
    use missingno_gbc::GameBoyColor;
    use missingno_gbc::launch::{self, BOARD, BOOT_ROM, GbLaunch, RUNNER, RunnerPreference};

    /// The headless build persists no battery save; the format is frontend
    /// policy the GUI owns.
    fn no_battery(_: &Cartridge) -> Option<Vec<u8>> {
        None
    }

    /// No battery save, no link peripheral, and a mismatched boot ROM dropped
    /// without a word — this load path has no user to tell.
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
        let board = launch::board_from_launch(launch).map_err(|value| LoadError::InvalidValue {
            option: BOARD.to_string(),
            value: value.to_string(),
        })?;
        let cartridge = Cartridge::new(rom.to_vec(), board, None)
            .map_err(|refusal| LoadError::Core(refusal.to_string()))?;
        let boot_rom = match launch.file(BOOT_ROM) {
            Some(bytes) => Some(BootRom::from_bytes(bytes.to_vec()).map_err(|length| {
                LoadError::InvalidValue {
                    option: BOOT_ROM.to_string(),
                    value: format!("{length}-byte image"),
                }
            })?),
            None => None,
        };
        let runner =
            RunnerPreference::from_launch(launch).map_err(|value| LoadError::InvalidValue {
                option: RUNNER.to_string(),
                value: value.to_string(),
            })?;
        let (console, _) =
            launch::console(cartridge, boot_rom, None, runner, Boxed).map_err(|refusal| {
                LoadError::IncompatibleOption {
                    option: RUNNER.to_string(),
                    reason: refusal.to_string(),
                }
            })?;
        Ok(console)
    }

    pub fn options() -> Vec<LaunchOptionDescriptor> {
        launch::launch_options()
    }

    pub fn is_rom(path: &Path, rom: &[u8]) -> bool {
        media::is_family_rom(path, rom)
    }
}

#[cfg(feature = "vcs")]
mod vcs {
    use super::*;
    use missingno_core::system::SystemConsole;

    use missingno_vcs::debug::{BOARD, OVERDUMP, TV_STANDARD};
    use missingno_vcs::{CartType, TvStandard};

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
                    TvStandard::from_code(name).ok_or_else(|| LoadError::InvalidValue {
                        option: TV_STANDARD.to_string(),
                        value: name.to_string(),
                    })?,
                )
            }
            None => None,
        };
        let board = match launch.choice(BOARD) {
            Some(code) if CartType::from_code(code).is_none() => {
                return Err(LoadError::InvalidValue {
                    option: BOARD.to_string(),
                    value: code.to_string(),
                });
            }
            board => board,
        };
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

    pub fn options() -> Vec<LaunchOptionDescriptor> {
        missingno_vcs::debug::launch_options()
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

    pub fn create(
        path: &Path,
        rom: &[u8],
        _launch: &LaunchValues,
    ) -> Result<Box<dyn SystemConsole>, LoadError> {
        missingno_sg1000::debug::create_console(rom, title_for(path))
            .map_err(|error| LoadError::Core(format!("{error:?}")))
    }

    pub fn is_rom(path: &Path, _rom: &[u8]) -> bool {
        missingno_sg1000::debug::is_sg1000_rom(path)
    }
}

/// Every registered core, in claim order.
pub static FACTORIES: &[CoreFactory] = &[
    #[cfg(feature = "gb")]
    CoreFactory {
        name: "Game Boy",
        is_rom: gb::is_rom,
        create: gb::create,
        options: gb::options,
    },
    #[cfg(feature = "vcs")]
    CoreFactory {
        name: "Atari VCS",
        is_rom: vcs::is_rom,
        create: vcs::create,
        options: vcs::options,
    },
    #[cfg(feature = "nes")]
    CoreFactory {
        name: "NES",
        is_rom: nes::is_rom,
        create: nes::create,
        options: Vec::new,
    },
    #[cfg(feature = "sms")]
    CoreFactory {
        name: "Master System",
        is_rom: sms::is_rom,
        create: sms::create,
        options: Vec::new,
    },
    #[cfg(feature = "sg1000")]
    CoreFactory {
        name: "SG-1000",
        is_rom: sg1000::is_rom,
        create: sg1000::create,
        options: Vec::new,
    },
];

/// The factory whose media this is, if any core in this build claims it.
pub fn factory_for(path: &Path, rom: &[u8]) -> Option<&'static CoreFactory> {
    FACTORIES.iter().find(|factory| (factory.is_rom)(path, rom))
}

/// Build a console from a ROM's path and contents, leaving every launch option
/// to the core that claims it.
pub fn create_console(path: &Path, rom: &[u8]) -> Result<Box<dyn SystemConsole>, LoadError> {
    create_console_with(path, rom, &LaunchValues::default())
}

/// Build a console from the launch values a loader collected. Recognition is
/// unaffected by them.
pub fn create_console_with(
    path: &Path,
    rom: &[u8],
    launch: &LaunchValues,
) -> Result<Box<dyn SystemConsole>, LoadError> {
    let factory = factory_for(path, rom).ok_or(LoadError::UnrecognizedMedia)?;
    (factory.create)(path, rom, launch)
}
