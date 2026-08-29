//! Core selection for the Game Boy family: the cartridge header names the
//! console it is slotted into, and this crate is the one that knows both.

use missingno_core::cartridge::BoardVocabulary;
use missingno_core::launch::{
    LaunchChoice, LaunchOptionDescriptor, LaunchOptionKind, LaunchValue, LaunchValues, board_option,
};
use missingno_gb::cartridge::{GbCartType, GbRamSize, GbRomSize};
use missingno_gb::serial_transfer::SerialLink;
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge};

use crate::GameBoyColor;

/// The console to run the cartridge on.
pub const RUNNER: &str = "runner";
/// The boot ROM to map over the cartridge.
pub const BOOT_ROM: &str = "boot-rom";
/// The board the cartridge is built on, for media whose header misdeclares it.
pub const BOARD: &str = "board";

/// One board a picker can name, over the combinations a `$0147` byte declares
/// plus the three boards no byte names. A picker states a board and not the
/// sizes beside it, so each preset carries the size its silicon commonly comes
/// in; a catalogue that knows the sizes states the whole board instead.
struct Preset {
    name: &'static str,
    label: &'static str,
    board: GbCartType,
}

const fn preset(name: &'static str, label: &'static str, board: GbCartType) -> Preset {
    Preset { name, label, board }
}

/// The ROM chip a preset carries where the header names no size.
const PRESET_ROM: GbRomSize = GbRomSize::Mb1;
/// The RAM chip a preset carries where the board takes one.
const PRESET_RAM: Option<GbRamSize> = Some(GbRamSize::Kb32);

const fn rom(ram: Option<GbRamSize>, battery: bool) -> GbCartType {
    GbCartType::Rom { ram, battery }
}

const fn mbc1(ram: Option<GbRamSize>, battery: bool) -> GbCartType {
    GbCartType::Mbc1 {
        rom: PRESET_ROM,
        ram,
        battery,
    }
}

const fn mbc3(ram: Option<GbRamSize>, battery: bool, rtc: bool) -> GbCartType {
    GbCartType::Mbc3 {
        rom: PRESET_ROM,
        ram,
        battery,
        rtc,
    }
}

const fn mbc5(ram: Option<GbRamSize>, battery: bool, rumble: bool) -> GbCartType {
    GbCartType::Mbc5 {
        rom: PRESET_ROM,
        ram,
        battery,
        rumble,
    }
}

const PRESETS: &[Preset] = &[
    preset("Rom", "ROM only", rom(None, false)),
    preset("RomRam", "ROM + RAM", rom(Some(GbRamSize::Kb8), false)),
    preset(
        "RomRamBattery",
        "ROM + RAM + battery",
        rom(Some(GbRamSize::Kb8), true),
    ),
    preset("Mbc1", "MBC1", mbc1(None, false)),
    preset("Mbc1Ram", "MBC1 + RAM", mbc1(PRESET_RAM, false)),
    preset(
        "Mbc1RamBattery",
        "MBC1 + RAM + battery",
        mbc1(PRESET_RAM, true),
    ),
    preset(
        "Mbc1Multicart",
        "MBC1 multicart",
        GbCartType::Mbc1Multicart { rom: PRESET_ROM },
    ),
    preset(
        "Mbc2",
        "MBC2",
        GbCartType::Mbc2 {
            rom: GbRomSize::Kb256,
            battery: false,
        },
    ),
    preset(
        "Mbc2Battery",
        "MBC2 + battery",
        GbCartType::Mbc2 {
            rom: GbRomSize::Kb256,
            battery: true,
        },
    ),
    preset(
        "Mbc3TimerBattery",
        "MBC3 + timer + battery",
        mbc3(None, true, true),
    ),
    preset(
        "Mbc3TimerRamBattery",
        "MBC3 + timer + RAM + battery",
        mbc3(PRESET_RAM, true, true),
    ),
    preset("Mbc3", "MBC3", mbc3(None, false, false)),
    preset("Mbc3Ram", "MBC3 + RAM", mbc3(PRESET_RAM, false, false)),
    preset(
        "Mbc3RamBattery",
        "MBC3 + RAM + battery",
        mbc3(PRESET_RAM, true, false),
    ),
    preset(
        "Mbc30",
        "MBC30",
        GbCartType::Mbc30 {
            rom: GbRomSize::Mb2,
            ram: Some(GbRamSize::Kb64),
            battery: true,
            rtc: true,
        },
    ),
    preset("Mbc5", "MBC5", mbc5(None, false, false)),
    preset("Mbc5Ram", "MBC5 + RAM", mbc5(PRESET_RAM, false, false)),
    preset(
        "Mbc5RamBattery",
        "MBC5 + RAM + battery",
        mbc5(PRESET_RAM, true, false),
    ),
    preset("Mbc5Rumble", "MBC5 + rumble", mbc5(None, false, true)),
    preset(
        "Mbc5RumbleRam",
        "MBC5 + rumble + RAM",
        mbc5(PRESET_RAM, false, true),
    ),
    preset(
        "Mbc5RumbleRamBattery",
        "MBC5 + rumble + RAM + battery",
        mbc5(PRESET_RAM, true, true),
    ),
    preset("Mbc6", "MBC6", GbCartType::Mbc6),
    preset("Mbc7", "MBC7", GbCartType::Mbc7 { rom: PRESET_ROM }),
    preset(
        "Huc3",
        "HuC-3",
        GbCartType::Huc3 {
            rom: PRESET_ROM,
            ram: PRESET_RAM,
        },
    ),
    preset(
        "Huc1",
        "HuC-1",
        GbCartType::Huc1 {
            rom: PRESET_ROM,
            ram: PRESET_RAM,
        },
    ),
    preset(
        "DbzTrans",
        "DBZ Trans (unlicensed)",
        GbCartType::DbzTrans { ram: PRESET_RAM },
    ),
];

/// The options the Game Boy family accepts at launch for this cartridge. The
/// console is a choice only for media both can run: a Color runs a DMG
/// cartridge, but one whose header requires the Color leaves nothing to pick.
pub fn launch_options(rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
    let runner = (!Cartridge::peek_cgb_only(rom)).then_some(LaunchOptionDescriptor {
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
    });
    let fixed = [
        board_option(
            BOARD,
            PRESETS.iter().map(|preset| LaunchChoice {
                value: preset.name,
                label: preset.label,
            }),
        ),
        LaunchOptionDescriptor {
            id: BOOT_ROM,
            label: "Boot ROM",
            kind: LaunchOptionKind::File {
                label: "Boot ROM image",
            },
        },
    ];
    runner.into_iter().chain(fixed).collect()
}

/// The board the launch values state, or `None` where the header decides. A
/// catalogue states the whole board, sizes and all; a picker names a preset.
pub fn board_from_launch(values: &LaunchValues) -> Result<Option<GbCartType>, String> {
    match values.value(BOARD) {
        None => Ok(None),
        Some(LaunchValue::Board(board)) => GbCartType::from_value(board).map(Some),
        Some(LaunchValue::Choice(name)) => PRESETS
            .iter()
            .find(|preset| preset.name == name)
            .map(|preset| Some(preset.board))
            .ok_or_else(|| format!("unknown Game Boy board \"{name}\"")),
        Some(_) => Err(format!("the {BOARD} option states a board")),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A cartridge image whose header carries `cgb_flag` at $0143 and names a
    /// mapperless board at $0147.
    fn rom(cgb_flag: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x143] = cgb_flag;
        rom
    }

    fn runner_choices(rom: &[u8]) -> Vec<&'static str> {
        let runner = launch_options(rom)
            .into_iter()
            .find(|option| option.id == RUNNER)
            .expect("the console option is published");
        match runner.kind {
            LaunchOptionKind::Choice { choices } => {
                choices.into_iter().map(|choice| choice.value).collect()
            }
            _ => panic!("the console option is a choice"),
        }
    }

    #[test]
    fn a_cgb_only_cartridge_publishes_no_console_choice() {
        assert!(
            !launch_options(&rom(0xC0))
                .iter()
                .any(|option| option.id == RUNNER)
        );
    }

    #[test]
    fn a_cgb_enhanced_cartridge_offers_both_consoles() {
        assert_eq!(runner_choices(&rom(0x80)), ["dmg", "cgb"]);
    }

    #[test]
    fn a_dmg_cartridge_offers_both_consoles() {
        assert_eq!(runner_choices(&rom(0x00)), ["dmg", "cgb"]);
    }

    #[test]
    fn a_named_preset_and_a_stated_board_both_reach_the_core() {
        let mut values = LaunchValues::default();
        values.set_choice(BOARD, "Mbc5RamBattery");
        let named = board_from_launch(&values).unwrap().expect("a board");
        assert_eq!(named.name(), "Mbc5");
        assert!(named.has_battery());

        values.set_board(BOARD, named.to_value());
        assert_eq!(board_from_launch(&values), Ok(Some(named)));

        values.set_choice(BOARD, "Mbc9");
        assert!(board_from_launch(&values).is_err());
    }

    #[test]
    fn every_preset_names_a_board_the_vocabulary_reads_back() {
        for preset in PRESETS {
            assert_eq!(
                GbCartType::from_value(&preset.board.to_value()),
                Ok(preset.board),
                "{}",
                preset.name
            );
        }
    }

    #[test]
    fn a_dmg_choice_kept_from_elsewhere_is_still_refused() {
        struct Named;
        impl GbLaunch for Named {
            type Output = &'static str;
            fn dmg(self, _: GameBoy) -> &'static str {
                "dmg"
            }
            fn cgb(self, _: GameBoyColor) -> &'static str {
                "cgb"
            }
        }
        let cartridge = Cartridge::new(rom(0xC0), None, None).expect("the header names a board");
        let launched = console(cartridge, None, None, RunnerPreference::Dmg, Named);
        assert_eq!(launched.err(), Some(RunnerRefused::CgbOnlyCartridge));
    }
}
