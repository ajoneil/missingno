//! Core selection for the Game Boy family: the cartridge header names the
//! console it is slotted into, and this crate is the one that knows both.

use missingno_core::cartridge::{BoardValue, BoardVocabulary};
use missingno_core::launch::{
    LaunchChoice, LaunchOptionDescriptor, LaunchOptionKind, LaunchValue, LaunchValues, board_option,
};
use missingno_gb::cartridge::GbCartType;
use missingno_gb::serial_transfer::SerialLink;
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge};

use crate::GameBoyColor;

/// The console to run the cartridge on.
pub const RUNNER: &str = "runner";
/// The boot ROM to map over the cartridge.
pub const BOOT_ROM: &str = "boot-rom";
/// The board the cartridge is built on, for media whose header misdeclares it.
pub const BOARD: &str = "board";
/// The console variants the cartridge is played with; where no console is
/// chosen, these name the one it boots.
pub const ENHANCEMENTS: &str = "enhancements";
pub const ENHANCEMENT_SGB: &str = "sgb";
pub const ENHANCEMENT_CGB: &str = "cgb";

/// The options the Game Boy family accepts at launch for this cartridge. The
/// console is a choice only for media both can run: a Color runs a DMG
/// cartridge, but one whose header requires the Color leaves nothing to pick —
/// and with nothing to pick there is no enhancement left to state either.
pub fn launch_options(rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
    let both_consoles = !Cartridge::peek_cgb_only(rom);
    let runner = both_consoles.then_some(LaunchOptionDescriptor {
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
    let enhancements = both_consoles.then_some(LaunchOptionDescriptor {
        id: ENHANCEMENTS,
        label: "Enhancements",
        kind: LaunchOptionKind::Flags {
            flags: vec![
                LaunchChoice {
                    value: ENHANCEMENT_CGB,
                    label: "Game Boy Color",
                },
                LaunchChoice {
                    value: ENHANCEMENT_SGB,
                    label: "Super Game Boy",
                },
            ],
        },
    });
    let fixed = [
        board_option(BOARD, GbCartType::catalogue().iter().cloned()),
        LaunchOptionDescriptor {
            id: BOOT_ROM,
            label: "Boot ROM",
            kind: LaunchOptionKind::File {
                label: "Boot ROM image",
            },
        },
    ];
    runner
        .into_iter()
        .chain(enhancements)
        .chain(fixed)
        .collect()
}

/// The board the launch values state, or `None` where the header decides. A
/// catalogue states the whole board, sizes and all; a bare name states one with
/// nothing beside it, which only a board whose wiring fixes its parts accepts.
pub fn board_from_launch(values: &LaunchValues) -> Result<Option<GbCartType>, String> {
    match values.value(BOARD) {
        None => Ok(None),
        Some(LaunchValue::Board(board)) => GbCartType::from_value(board).map(Some),
        Some(LaunchValue::Choice(name)) => GbCartType::from_value(&BoardValue::new(name)).map(Some),
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
    /// none. A named console is the whole answer; failing that, a stated
    /// enhancement set names one — a set without the Color plays on a Game Boy
    /// however the header is flagged.
    pub fn from_launch(values: &LaunchValues) -> Result<RunnerPreference, &str> {
        match values.choice(RUNNER) {
            Some("dmg") => Ok(RunnerPreference::Dmg),
            Some("cgb") => Ok(RunnerPreference::Cgb),
            Some(other) => Err(other),
            None => Ok(match values.flags(ENHANCEMENTS) {
                Some(enhancements) if enhancements.contains(ENHANCEMENT_CGB) => {
                    RunnerPreference::Cgb
                }
                Some(_) => RunnerPreference::Dmg,
                None => RunnerPreference::Auto,
            }),
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
    use missingno_gb::cartridge::{GbRamSize, GbRomSize};

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

    fn enhancement_flags(rom: &[u8]) -> Option<Vec<&'static str>> {
        let published = launch_options(rom)
            .into_iter()
            .find(|option| option.id == ENHANCEMENTS)?;
        match published.kind {
            LaunchOptionKind::Flags { flags } => {
                Some(flags.into_iter().map(|flag| flag.value).collect())
            }
            _ => panic!("the enhancements option is a set of flags"),
        }
    }

    #[test]
    fn media_both_consoles_run_states_the_enhancements_it_exploits() {
        for cgb_flag in [0x00, 0x80] {
            assert_eq!(
                enhancement_flags(&rom(cgb_flag)),
                Some(vec![ENHANCEMENT_CGB, ENHANCEMENT_SGB]),
                "cgb flag ${cgb_flag:02X}"
            );
        }
    }

    #[test]
    fn a_cgb_only_cartridge_publishes_no_enhancements() {
        assert_eq!(enhancement_flags(&rom(0xC0)), None);
    }

    #[test]
    fn stated_enhancements_name_the_console_where_none_was_chosen() {
        let set = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();

        let mut values = LaunchValues::default();
        assert_eq!(
            RunnerPreference::from_launch(&values),
            Ok(RunnerPreference::Auto)
        );

        values.set_flags(ENHANCEMENTS, set(&[ENHANCEMENT_CGB, ENHANCEMENT_SGB]));
        assert_eq!(
            RunnerPreference::from_launch(&values),
            Ok(RunnerPreference::Cgb)
        );

        values.set_flags(ENHANCEMENTS, set(&[ENHANCEMENT_SGB]));
        assert_eq!(
            RunnerPreference::from_launch(&values),
            Ok(RunnerPreference::Dmg)
        );

        // Nothing exploited is still a statement: the plain Game Boy.
        values.set_flags(ENHANCEMENTS, set(&[]));
        assert_eq!(
            RunnerPreference::from_launch(&values),
            Ok(RunnerPreference::Dmg)
        );

        values.set_choice(RUNNER, "cgb");
        assert_eq!(
            RunnerPreference::from_launch(&values),
            Ok(RunnerPreference::Cgb)
        );
    }

    #[test]
    fn a_bare_name_and_a_stated_board_both_reach_the_core() {
        let mut values = LaunchValues::default();
        values.set_choice(BOARD, "Mbc6");
        assert_eq!(board_from_launch(&values), Ok(Some(GbCartType::Mbc6)));

        let mbc5 = GbCartType::Mbc5 {
            rom: GbRomSize::Mb1,
            ram: Some(GbRamSize::Kb32),
            battery: true,
            rumble: false,
        };
        values.set_board(BOARD, mbc5.to_value());
        assert_eq!(board_from_launch(&values), Ok(Some(mbc5)));
    }

    #[test]
    fn a_name_alone_states_no_board_whose_sizes_vary() {
        let mut values = LaunchValues::default();
        values.set_choice(BOARD, "Mbc5");
        assert!(
            board_from_launch(&values)
                .unwrap_err()
                .contains("needs a \"rom\" attribute")
        );

        values.set_choice(BOARD, "Mbc9");
        assert!(
            board_from_launch(&values)
                .unwrap_err()
                .contains("unknown Game Boy board")
        );
    }

    #[test]
    fn the_board_option_offers_the_whole_vocabulary() {
        let board = launch_options(&rom(0x00))
            .into_iter()
            .find(|option| option.id == BOARD)
            .expect("the board option is published");
        let offered: Vec<&str> = match &board.kind {
            LaunchOptionKind::Board { boards } => boards.iter().map(|spec| spec.name).collect(),
            _ => panic!("the board option states a board"),
        };
        let catalogued: Vec<&str> = GbCartType::catalogue()
            .iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(offered, catalogued);
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
