//! Core selection for the Game Boy family: the cartridge header names the
//! console it is slotted into, and this crate is the one that knows both.

use missingno_core::launch::{
    LaunchChoice, LaunchOptionDescriptor, LaunchOptionKind, LaunchValues,
};
use missingno_gb::serial_transfer::SerialLink;
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge};

use crate::GameBoyColor;

/// The console to run the cartridge on.
pub const RUNNER: &str = "runner";
/// The boot ROM to map over the cartridge.
pub const BOOT_ROM: &str = "boot-rom";

/// The options the Game Boy family accepts at launch.
pub fn launch_options() -> Vec<LaunchOptionDescriptor> {
    vec![
        LaunchOptionDescriptor {
            id: RUNNER,
            label: "Console",
            kind: LaunchOptionKind::Choice {
                choices: vec![
                    LaunchChoice {
                        value: "dmg",
                        label: "Game Boy (DMG)",
                    },
                    LaunchChoice {
                        value: "cgb",
                        label: "Game Boy Color (CGB)",
                    },
                ],
            },
        },
        LaunchOptionDescriptor {
            id: BOOT_ROM,
            label: "Boot ROM",
            kind: LaunchOptionKind::File {
                label: "Boot ROM image",
            },
        },
    ]
}

/// Receives the console [`console`] selects. Two concrete arms rather than one
/// generic method so a caller can require its own model traits on each.
pub trait GbLaunch {
    type Output;
    fn dmg(self, console: GameBoy) -> Self::Output;
    fn cgb(self, console: GameBoyColor) -> Self::Output;
}

/// What became of a candidate boot ROM. A boot ROM only boots the model it was
/// dumped from, so one that does not match the selected core is dropped rather
/// than forced on it; whether to say so is the caller's policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootRomFit {
    /// The candidate matches the selected core, or none was offered.
    Kept,
    /// The candidate was dumped from the other model, and was dropped.
    Dropped,
}

/// Which console of the family a cartridge is slotted into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunnerPreference {
    /// The cartridge header decides, as the hardware it is slotted into would.
    #[default]
    Auto,
    /// The DMG, which a cartridge whose header requires the CGB refuses.
    Dmg,
    /// The CGB, which runs DMG cartridges in compatibility mode.
    Cgb,
}

impl RunnerPreference {
    /// The console the launch values ask for; `Err` carries a value that names
    /// none.
    pub fn from_launch(values: &LaunchValues) -> Result<RunnerPreference, &str> {
        match values.choice(RUNNER) {
            None => Ok(RunnerPreference::Auto),
            Some("dmg") => Ok(RunnerPreference::Dmg),
            Some("cgb") => Ok(RunnerPreference::Cgb),
            Some(other) => Err(other),
        }
    }
}

/// Why the console a caller asked for cannot run this cartridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerRefused {
    /// The header's $C0 CGB flag: the game runs on no Game Boy but a Color.
    CgbOnlyCartridge,
}

impl std::fmt::Display for RunnerRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerRefused::CgbOnlyCartridge => {
                f.write_str("the cartridge requires a Game Boy Color")
            }
        }
    }
}

/// The one DMG-vs-CGB selection point: left to the header, CGB-aware media —
/// enhanced or required — boots the CGB core, like a cartridge slotted into a
/// real GBC, and DMG-only media boots the DMG core. A caller may name the
/// console instead; only a CGB-only cartridge on the DMG is refused. Any serial
/// peripheral goes on the selected console's link port.
pub fn console<L: GbLaunch>(
    cartridge: Cartridge,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
    runner: RunnerPreference,
    launcher: L,
) -> Result<(L::Output, BootRomFit), RunnerRefused> {
    let cgb_core = match runner {
        RunnerPreference::Auto => cartridge.is_cgb(),
        RunnerPreference::Cgb => true,
        RunnerPreference::Dmg if cartridge.requires_cgb() => {
            return Err(RunnerRefused::CgbOnlyCartridge);
        }
        RunnerPreference::Dmg => false,
    };
    let (boot_rom, fit) = match (&boot_rom, cgb_core) {
        (Some(BootRom::Dmg(_)), true) | (Some(BootRom::Cgb(_)), false) => {
            (None, BootRomFit::Dropped)
        }
        _ => (boot_rom, BootRomFit::Kept),
    };
    let output = if cgb_core {
        let mut console = GameBoyColor::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.cgb(console)
    } else {
        let mut console = GameBoy::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.dmg(console)
    };
    Ok((output, fit))
}
