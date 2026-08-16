//! The per-core ROM→console registry: the one point that knows concrete
//! cores. Each entry pairs a media-recognition predicate with a constructor
//! that builds a `Box<dyn SystemConsole>`; everything downstream is generic.
//!
//! Entries are feature-gated, so a build carries only the cores it selected.
//! Off-chip Game Boy peripherals (serial link, printer, battery save) are
//! frontend policy — this factory constructs without them.

use std::path::Path;

use missingno_core::system::SystemConsole;

/// Whether a path and its contents are this core's media.
pub type IsRom = fn(&Path, &[u8]) -> bool;
/// Build this core's console from a ROM's path and contents, honouring any
/// construction options a core recognises.
pub type Create = fn(&Path, &[u8], &LoadOptions) -> Option<Box<dyn SystemConsole>>;

/// Optional construction overrides a core may honour. Generic by design: a core
/// that does not recognise an option ignores it.
#[derive(Clone, Default)]
pub struct LoadOptions {
    /// A broadcast-standard override ("ntsc"/"pal"/"secam", case-insensitive);
    /// `None` lets the core auto-detect. Read by the Atari VCS core.
    pub tv_standard: Option<String>,
    /// A boot ROM's contents, so a session can observe the boot sequence
    /// rather than the post-boot state the core otherwise seeds. Read by the
    /// Game Boy family.
    pub boot_rom: Option<Vec<u8>>,
    /// A cartridge board override ("F8", "F6SC", …); `None` lets the core
    /// size-detect. Read by the Atari VCS core, whose carts have no header.
    pub cart_type: Option<String>,
    /// The catalogue records this dump as an overdump, so the image runs past
    /// the cartridge's silicon and the stated board says where it ends. Read
    /// by the Atari VCS core.
    pub overdump: bool,
}

/// A registered core: how its media is recognised, and how a console is built.
pub struct CoreFactory {
    pub name: &'static str,
    pub is_rom: IsRom,
    pub create: Create,
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
    use missingno_gb::system::GbConsole;
    use missingno_gb::{BootRom, GameBoy, media};
    use missingno_gbc::GameBoyColor;

    /// The headless build persists no battery save; the format is frontend
    /// policy the GUI owns.
    fn no_battery(_: &Cartridge) -> Option<Vec<u8>> {
        None
    }

    /// The one DMG-vs-CGB selection point: CGB-aware media boots the CGB core,
    /// like a cartridge slotted into a real GBC; DMG media boots the DMG core.
    pub fn create(
        _path: &Path,
        rom: &[u8],
        options: &LoadOptions,
    ) -> Option<Box<dyn SystemConsole>> {
        let cartridge = Cartridge::new(rom.to_vec(), None);
        let boot_rom = boot_rom_for(options, cartridge.is_cgb());
        Some(if cartridge.is_cgb() {
            Box::new(GbConsole::new(
                GameBoyColor::new(cartridge, boot_rom),
                no_battery,
            ))
        } else {
            Box::new(GbConsole::new(
                GameBoy::new(cartridge, boot_rom),
                no_battery,
            ))
        })
    }

    /// A boot ROM only boots the model it was dumped from, so one that does not
    /// match the core the header selected is dropped rather than forced on it.
    fn boot_rom_for(options: &LoadOptions, cgb_core: bool) -> Option<BootRom> {
        let boot_rom = BootRom::from_bytes(options.boot_rom.clone()?).ok()?;
        match (&boot_rom, cgb_core) {
            (BootRom::Dmg(_), true) | (BootRom::Cgb(_), false) => None,
            _ => Some(boot_rom),
        }
    }

    pub fn is_rom(path: &Path, rom: &[u8]) -> bool {
        media::is_family_rom(path, rom)
    }
}

#[cfg(feature = "vcs")]
mod vcs {
    use super::*;
    use missingno_core::system::SystemConsole;

    pub fn create(
        path: &Path,
        rom: &[u8],
        options: &LoadOptions,
    ) -> Option<Box<dyn SystemConsole>> {
        let standard = options.tv_standard.as_deref().and_then(parse_tv_standard);
        missingno_vcs::debug::create_console(
            rom,
            title_for(path),
            standard,
            options.cart_type.as_deref(),
            options.overdump,
        )
        .ok()
    }

    pub fn is_rom(path: &Path, rom: &[u8]) -> bool {
        missingno_vcs::debug::is_vcs_rom(path, rom)
    }

    fn parse_tv_standard(name: &str) -> Option<missingno_vcs::TvStandard> {
        use missingno_vcs::TvStandard;
        match name.trim().to_ascii_lowercase().as_str() {
            "ntsc" => Some(TvStandard::Ntsc),
            "pal" => Some(TvStandard::Pal),
            "secam" => Some(TvStandard::Secam),
            _ => None,
        }
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
        _options: &LoadOptions,
    ) -> Option<Box<dyn SystemConsole>> {
        let nes = Nes::new(rom).ok()?;
        Some(Box::new(MachineConsole::<NesSystem>::new(
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
        _options: &LoadOptions,
    ) -> Option<Box<dyn SystemConsole>> {
        let sms = Sms::new(rom).ok()?;
        Some(Box::new(MachineConsole::<SmsSystem>::new(
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
    use missingno_core::machine::MachineConsole;
    use missingno_core::system::SystemConsole;
    use missingno_sg1000::console::Sg1000;
    use missingno_sg1000::debug::Sg1000System;

    pub fn create(
        path: &Path,
        rom: &[u8],
        _options: &LoadOptions,
    ) -> Option<Box<dyn SystemConsole>> {
        let sg1000 = Sg1000::new(rom).ok()?;
        Some(Box::new(MachineConsole::<Sg1000System>::new(
            sg1000,
            title_for(path),
        )))
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
    },
    #[cfg(feature = "vcs")]
    CoreFactory {
        name: "Atari VCS",
        is_rom: vcs::is_rom,
        create: vcs::create,
    },
    #[cfg(feature = "nes")]
    CoreFactory {
        name: "NES",
        is_rom: nes::is_rom,
        create: nes::create,
    },
    #[cfg(feature = "sms")]
    CoreFactory {
        name: "Master System",
        is_rom: sms::is_rom,
        create: sms::create,
    },
    #[cfg(feature = "sg1000")]
    CoreFactory {
        name: "SG-1000",
        is_rom: sg1000::is_rom,
        create: sg1000::create,
    },
];

/// The factory whose media this is, if any core in this build claims it.
pub fn factory_for(path: &Path, rom: &[u8]) -> Option<&'static CoreFactory> {
    FACTORIES.iter().find(|factory| (factory.is_rom)(path, rom))
}

/// Build a console from a ROM's path and contents. `Ok(None)` when no core
/// recognises the media; `Err` when a core claimed it but construction failed.
pub fn create_console(path: &Path, rom: &[u8]) -> Result<Option<Box<dyn SystemConsole>>, String> {
    create_console_with(path, rom, &LoadOptions::default())
}

/// Build a console, passing construction options a core may honour.
/// Recognition is unaffected by the options.
pub fn create_console_with(
    path: &Path,
    rom: &[u8],
    options: &LoadOptions,
) -> Result<Option<Box<dyn SystemConsole>>, String> {
    let Some(factory) = factory_for(path, rom) else {
        return Ok(None);
    };
    match (factory.create)(path, rom, options) {
        Some(console) => Ok(Some(console)),
        None => Err(format!(
            "{}: failed to construct console from media",
            factory.name
        )),
    }
}
