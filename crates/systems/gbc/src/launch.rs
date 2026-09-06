//! Core selection for the Game Boy family: the cartridge header names the
//! console it is slotted into, and this crate is the one that knows both.

use missingno_core::cartridge::{BoardValue, BoardVocabulary};
use missingno_core::firmware::{FirmwareSlot, FirmwareValue};
use missingno_core::launch::{
    LaunchChoice, LaunchOptionDescriptor, LaunchOptionKind, LaunchValue, LaunchValues, board_option,
};
use missingno_gb::cartridge::GbCartType;
use missingno_gb::serial_transfer::SerialLink;
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge};

use crate::GameBoyColor;

/// The console to run the cartridge on.
pub const RUNNER: &str = "runner";
/// The board the cartridge is built on, for media whose header misdeclares it.
pub const BOARD: &str = "board";
/// The console variants the cartridge is played with; where no console is
/// chosen, these name the one it boots.
pub const ENHANCEMENTS: &str = "enhancements";
pub const ENHANCEMENT_SGB: &str = "sgb";
pub const ENHANCEMENT_CGB: &str = "cgb";

/// The options the Game Boy family accepts at launch for this cartridge, given
/// the caller's word so far. The console is a choice only for media both can
/// run: a Color runs a DMG cartridge, but one whose header requires the Color
/// leaves nothing to pick — and with nothing to pick there is no enhancement
/// left to state either.
pub fn launch_options(rom: &[u8], chosen: &LaunchValues) -> Vec<LaunchOptionDescriptor> {
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
        firmware_option(selected_boot_rom_slot(rom, chosen)),
    ];
    runner
        .into_iter()
        .chain(enhancements)
        .chain(fixed)
        .collect()
}

/// Whether media carrying these header flags runs on the Color: a named console
/// is the answer, the header answers where the caller left it open, and a
/// cartridge no Game Boy but a Color runs is on the Color whichever was named.
fn runs_on_cgb(cgb: bool, cgb_only: bool, runner: RunnerPreference) -> bool {
    match runner {
        RunnerPreference::Auto => cgb,
        RunnerPreference::Cgb => true,
        RunnerPreference::Dmg => cgb_only,
    }
}

/// The socket of the console these values boot. A value naming no console
/// leaves the launch itself to object.
fn selected_boot_rom_slot(rom: &[u8], chosen: &LaunchValues) -> FirmwareSlot {
    let runner = RunnerPreference::from_launch(chosen).unwrap_or_default();
    match runs_on_cgb(
        Cartridge::peek_cgb(rom),
        Cartridge::peek_cgb_only(rom),
        runner,
    ) {
        true => crate::firmware::boot_rom_slot(),
        false => missingno_gb::firmware::boot_rom_slot(),
    }
}

/// A firmware socket as a launch option, named by the slot itself.
fn firmware_option(slot: FirmwareSlot) -> LaunchOptionDescriptor {
    LaunchOptionDescriptor {
        id: slot.id,
        label: slot.label,
        kind: LaunchOptionKind::Firmware { slot },
    }
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

/// The boot ROM offered for each console of the family. A boot ROM only boots
/// the model it was dumped from, so the family fills one socket per console and
/// whichever console boots reads its own.
#[derive(Clone, Default)]
pub struct BootRoms {
    pub dmg: Option<BootRom>,
    pub cgb: Option<BootRom>,
}

/// The boot ROM each firmware slot was given. `Err` names the slot and the
/// image found in it, which is no image that socket takes.
pub fn boot_roms_from_launch(values: &LaunchValues) -> Result<BootRoms, (&'static str, String)> {
    Ok(BootRoms {
        dmg: slot_image(values, &missingno_gb::firmware::boot_rom_slot())?,
        cgb: slot_image(values, &crate::firmware::boot_rom_slot())?,
    })
}

fn slot_image(
    values: &LaunchValues,
    slot: &FirmwareSlot,
) -> Result<Option<BootRom>, (&'static str, String)> {
    let refuse = |what: String| Err((slot.id, what));
    match values.firmware(slot.id) {
        None | Some(FirmwareValue::None) => Ok(None),
        // Only bytes boot: a frontend that named an image and never resolved it
        // would otherwise start silently with no firmware at all.
        Some(FirmwareValue::Image(_)) => refuse("image not supplied".to_owned()),
        Some(FirmwareValue::Bytes(bytes)) if bytes.len() != slot.size => {
            refuse(format!("{}-byte image", bytes.len()))
        }
        Some(FirmwareValue::Bytes(bytes)) => match BootRom::from_bytes(bytes.clone()) {
            Ok(image) => Ok(Some(image)),
            Err(length) => refuse(format!("{length}-byte image")),
        },
    }
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
/// peripheral goes on the selected console's link port, and the console that
/// boots maps the boot ROM from its own socket.
pub fn console<L: GbLaunch>(
    cartridge: Cartridge,
    boot_roms: BootRoms,
    link: Option<Box<dyn SerialLink>>,
    runner: RunnerPreference,
    launcher: L,
) -> Result<L::Output, RunnerRefused> {
    if runner == RunnerPreference::Dmg && cartridge.requires_cgb() {
        return Err(RunnerRefused::CgbOnlyCartridge);
    }
    let cgb_core = runs_on_cgb(cartridge.is_cgb(), cartridge.requires_cgb(), runner);
    Ok(if cgb_core {
        let mut console = GameBoyColor::new(cartridge, boot_roms.cgb);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.cgb(console)
    } else {
        let mut console = GameBoy::new(cartridge, boot_roms.dmg);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.dmg(console)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_gb::cartridge::{GbRamSize, GbRomSize};
    use missingno_gb::firmware::DMG_BOOT_ROM;

    /// A cartridge image whose header carries `cgb_flag` at $0143 and names a
    /// mapperless board at $0147.
    fn rom(cgb_flag: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x143] = cgb_flag;
        rom
    }

    fn runner_choices(rom: &[u8]) -> Vec<&'static str> {
        let runner = launch_options(rom, &LaunchValues::default())
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
            !launch_options(&rom(0xC0), &LaunchValues::default())
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
        let published = launch_options(rom, &LaunchValues::default())
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
        let board = launch_options(&rom(0x00), &LaunchValues::default())
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
    fn each_console_reads_the_boot_rom_from_its_own_slot() {
        let mut values = LaunchValues::default();
        values.set_firmware(DMG_BOOT_ROM, FirmwareValue::Bytes(vec![0x00; 0x100]));
        values.set_firmware(
            crate::firmware::CGB_BOOT_ROM,
            FirmwareValue::Bytes(vec![0x00; 0x900]),
        );
        let boot_roms = boot_roms_from_launch(&values).expect("both images fit their slots");
        assert!(matches!(boot_roms.dmg, Some(BootRom::Dmg(_))));
        assert!(matches!(boot_roms.cgb, Some(BootRom::Cgb(_))));

        let empty = boot_roms_from_launch(&LaunchValues::default()).expect("no image is no error");
        assert!(empty.dmg.is_none() && empty.cgb.is_none());
    }

    #[test]
    fn an_image_of_the_other_class_is_refused_by_the_slot_that_names_it() {
        let mut values = LaunchValues::default();
        values.set_firmware(DMG_BOOT_ROM, FirmwareValue::Bytes(vec![0x00; 0x900]));
        assert_eq!(
            boot_roms_from_launch(&values).err(),
            Some((DMG_BOOT_ROM, "2304-byte image".to_owned()))
        );

        let mut values = LaunchValues::default();
        values.set_firmware(
            crate::firmware::CGB_BOOT_ROM,
            FirmwareValue::Bytes(vec![0x00; 0x100]),
        );
        assert_eq!(
            boot_roms_from_launch(&values).err(),
            Some((crate::firmware::CGB_BOOT_ROM, "256-byte image".to_owned()))
        );

        let mut values = LaunchValues::default();
        values.set_firmware(DMG_BOOT_ROM, FirmwareValue::Bytes(vec![0x00; 0x80]));
        assert_eq!(
            boot_roms_from_launch(&values).err(),
            Some((DMG_BOOT_ROM, "128-byte image".to_owned()))
        );
    }

    #[test]
    fn a_named_image_nobody_resolved_is_refused_rather_than_ignored() {
        let mut values = LaunchValues::default();
        values.set_firmware(DMG_BOOT_ROM, FirmwareValue::Image("dmg".to_owned()));
        assert_eq!(
            boot_roms_from_launch(&values).err(),
            Some((DMG_BOOT_ROM, "image not supplied".to_owned()))
        );

        values.set_firmware(DMG_BOOT_ROM, FirmwareValue::None);
        assert!(
            boot_roms_from_launch(&values)
                .expect("an empty socket is no error")
                .dmg
                .is_none()
        );
    }

    /// The one firmware socket published for a cartridge under these values.
    fn boot_rom_row(rom: &[u8], chosen: &LaunchValues) -> &'static str {
        let published: Vec<&str> = launch_options(rom, chosen)
            .iter()
            .filter(|option| matches!(option.kind, LaunchOptionKind::Firmware { .. }))
            .map(|option| option.id)
            .collect();
        assert_eq!(published.len(), 1, "one boot ROM row: {published:?}");
        published[0]
    }

    #[test]
    fn the_boot_rom_row_is_the_one_the_header_leads_to() {
        let automatic = LaunchValues::default();
        assert_eq!(boot_rom_row(&rom(0x00), &automatic), DMG_BOOT_ROM);
        assert_eq!(
            boot_rom_row(&rom(0x80), &automatic),
            crate::firmware::CGB_BOOT_ROM
        );
    }

    #[test]
    fn a_chosen_console_takes_the_boot_rom_row_with_it() {
        let mut values = LaunchValues::default();
        values.set_choice(RUNNER, "dmg");
        assert_eq!(boot_rom_row(&rom(0x80), &values), DMG_BOOT_ROM);

        values.set_choice(RUNNER, "cgb");
        assert_eq!(
            boot_rom_row(&rom(0x00), &values),
            crate::firmware::CGB_BOOT_ROM
        );

        // A name no console answers to is the factory's objection to make.
        values.set_choice(RUNNER, "sgb");
        assert_eq!(boot_rom_row(&rom(0x00), &values), DMG_BOOT_ROM);
    }

    #[test]
    fn stated_enhancements_take_the_boot_rom_row_with_them() {
        let set = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();

        let mut values = LaunchValues::default();
        values.set_flags(ENHANCEMENTS, set(&[ENHANCEMENT_CGB]));
        assert_eq!(
            boot_rom_row(&rom(0x00), &values),
            crate::firmware::CGB_BOOT_ROM
        );

        values.set_flags(ENHANCEMENTS, set(&[ENHANCEMENT_SGB]));
        assert_eq!(boot_rom_row(&rom(0x80), &values), DMG_BOOT_ROM);
    }

    #[test]
    fn a_cgb_only_cartridge_publishes_the_colour_row_whatever_was_chosen() {
        let mut values = LaunchValues::default();
        assert_eq!(
            boot_rom_row(&rom(0xC0), &values),
            crate::firmware::CGB_BOOT_ROM
        );

        values.set_choice(RUNNER, "dmg");
        assert_eq!(
            boot_rom_row(&rom(0xC0), &values),
            crate::firmware::CGB_BOOT_ROM
        );
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
        let launched = console(
            cartridge,
            BootRoms::default(),
            None,
            RunnerPreference::Dmg,
            Named,
        );
        assert_eq!(launched.err(), Some(RunnerRefused::CgbOnlyCartridge));
    }
}
