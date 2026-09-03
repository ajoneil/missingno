//! The board a cartridge is built on: a mapper chip and the parts populated
//! beside it.
//!
//! A Game Boy cartridge states its own board across three header bytes — the
//! mapper and its extras at `$0147`, the ROM's size at `$0148`, the RAM chip's
//! at `$0149` — so the header is the normal path. No byte names a few boards —
//! a multicart, the MBC30 chip, an unlicensed mapper hiding behind a borrowed
//! byte — so a caller that knows better can state the board instead, and a
//! stated board is a whole statement: it replaces the header's word, parts and
//! all.

use missingno_core::cartridge::{
    AttributeKind, AttributeSpec, AttributeValue, BoardSpec, BoardValue, BoardVocabulary,
};

/// The cartridge's objection to media it cannot be built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbCartridgeError {
    /// A `$0147` value naming no board this core carries.
    UnknownMapper(u8),
    /// A stated board whose ROM is larger than the image, so the dump is
    /// missing silicon rather than carrying padding past it.
    ImageShorterThanBoard { board: usize, image: usize },
}

impl std::fmt::Display for GbCartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GbCartridgeError::UnknownMapper(byte) => {
                write!(f, "unsupported cartridge type ${byte:02x}")
            }
            GbCartridgeError::ImageShorterThanBoard { board, image } => write!(
                f,
                "the board holds {board} bytes but the image is only {image}"
            ),
        }
    }
}

impl std::error::Error for GbCartridgeError {}

/// The ROM chip's size, as the ladder at `$0148` names it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbRomSize {
    Kb32,
    Kb64,
    Kb128,
    Kb256,
    Kb512,
    Mb1,
    Mb2,
    Mb4,
    Mb8,
}

/// Every ROM size, smallest first; the position in this ladder is the value
/// `$0148` carries.
const ROM_LADDER: &[GbRomSize] = &[
    GbRomSize::Kb32,
    GbRomSize::Kb64,
    GbRomSize::Kb128,
    GbRomSize::Kb256,
    GbRomSize::Kb512,
    GbRomSize::Mb1,
    GbRomSize::Mb2,
    GbRomSize::Mb4,
    GbRomSize::Mb8,
];

/// The ROM sizes' codes, in the ladder's order.
const ROM_CODES: &[&str] = &["32K", "64K", "128K", "256K", "512K", "1M", "2M", "4M", "8M"];

impl GbRomSize {
    fn rung(self) -> usize {
        ROM_LADDER
            .iter()
            .position(|size| *size == self)
            .expect("every ROM size is on the ladder")
    }

    /// The size in bytes: 32 KB doubling up the ladder.
    pub fn bytes(self) -> usize {
        (32 * 1024) << self.rung()
    }

    /// The code the size goes by wherever a board is stated untyped.
    pub fn code(self) -> &'static str {
        ROM_CODES[self.rung()]
    }

    fn from_code(code: &str) -> Option<GbRomSize> {
        ROM_CODES
            .iter()
            .position(|listed| *listed == code)
            .map(|rung| ROM_LADDER[rung])
    }

    /// The size `$0148` declares; `None` for a value off the ladder.
    fn from_header(byte: u8) -> Option<GbRomSize> {
        ROM_LADDER.get(byte as usize).copied()
    }

    /// The smallest chip an image of this length fits on, for a header that
    /// declares no size the ladder names.
    fn for_image(len: usize) -> GbRomSize {
        ROM_LADDER
            .iter()
            .copied()
            .find(|size| size.bytes() >= len)
            .unwrap_or(GbRomSize::Mb8)
    }
}

/// The cartridge RAM chip's size, as `$0149` names it. The unofficial 2 KB
/// value names no chip this core builds.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbRamSize {
    Kb8,
    Kb32,
    Kb64,
    Kb128,
}

impl GbRamSize {
    /// The 8 KB banks the chip fills.
    pub fn banks(self) -> usize {
        match self {
            GbRamSize::Kb8 => 1,
            GbRamSize::Kb32 => 4,
            GbRamSize::Kb64 => 8,
            GbRamSize::Kb128 => 16,
        }
    }

    /// The code the size goes by wherever a board is stated untyped.
    pub fn code(self) -> &'static str {
        match self {
            GbRamSize::Kb8 => "8K",
            GbRamSize::Kb32 => "32K",
            GbRamSize::Kb64 => "64K",
            GbRamSize::Kb128 => "128K",
        }
    }

    fn from_code(code: &str) -> Option<GbRamSize> {
        match code {
            "8K" => Some(GbRamSize::Kb8),
            "32K" => Some(GbRamSize::Kb32),
            "64K" => Some(GbRamSize::Kb64),
            "128K" => Some(GbRamSize::Kb128),
            _ => None,
        }
    }

    /// The chip `$0149` declares; `None` where the board carries none.
    pub(crate) fn from_header(byte: u8) -> Option<GbRamSize> {
        match byte {
            2 => Some(GbRamSize::Kb8),
            3 => Some(GbRamSize::Kb32),
            4 => Some(GbRamSize::Kb128),
            5 => Some(GbRamSize::Kb64),
            _ => None,
        }
    }
}

/// A board the core can build: a mapper chip and the parts populated beside it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbCartType {
    /// No mapper, so one 32 KB image and at most the single RAM chip the
    /// address lines reach.
    Rom {
        ram: Option<GbRamSize>,
        battery: bool,
    },
    Mbc1 {
        rom: GbRomSize,
        ram: Option<GbRamSize>,
        battery: bool,
    },
    /// Several games on one MBC1 board, wired to bank in 4 Mbit slots. No header
    /// byte names it; the header path reads the second game's logo instead.
    Mbc1Multicart { rom: GbRomSize },
    /// The 512×4-bit store sits on the mapper die, so no RAM chip is populated
    /// beside it.
    Mbc2 { rom: GbRomSize, battery: bool },
    Mbc3 {
        rom: GbRomSize,
        ram: Option<GbRamSize>,
        battery: bool,
        rtc: bool,
    },
    /// The MBC3 successor, whose wider bank register reaches 256 ROM banks and
    /// 8 RAM banks. It shares MBC3's header bytes, so only the sizes give it
    /// away.
    Mbc30 {
        rom: GbRomSize,
        ram: Option<GbRamSize>,
        battery: bool,
        rtc: bool,
    },
    Mbc5 {
        rom: GbRomSize,
        ram: Option<GbRamSize>,
        battery: bool,
        rumble: bool,
    },
    /// One title's board, its ROM, flash and RAM all fixed by the wiring.
    Mbc6,
    /// The EEPROM store, the tilt sensor and the rumble motor are all inherent.
    Mbc7 { rom: GbRomSize },
    /// The infrared port is inherent.
    Huc1 {
        rom: GbRomSize,
        ram: Option<GbRamSize>,
    },
    /// The clock is inherent.
    Huc3 {
        rom: GbRomSize,
        ram: Option<GbRamSize>,
    },
    /// The unlicensed "GB DBZ GOKOU 2" mapper, which declares MBC5 in its header
    /// and adds an independent half-bank switch.
    DbzTrans { ram: Option<GbRamSize> },
    /// Sachen's own mapper. The board scrambles the address lines the header
    /// sits behind, so the logo and the type byte read as noise unless
    /// descrambled — which is what identifies the board, no header naming it.
    /// Catalogued but not modelled: stating it names the silicon, and the
    /// image still runs as an MBC1.
    SachenMmc1 { rom: GbRomSize },
}

/// The name refusals say this vocabulary by.
const VOCABULARY: &str = "Game Boy";

/// A mapper with no bank lines to RAM reaches one chip.
const ONE_RAM_CHIP: &[&str] = &["8K"];
/// Two bank bits reach four.
const FOUR_RAM_BANKS: &[&str] = &["8K", "32K"];
/// The whole `$0149` ladder, for the mappers whose register spans it.
const RAM_SIZES: &[&str] = &["8K", "32K", "64K", "128K"];

const ROM: AttributeSpec = AttributeSpec {
    key: "rom",
    label: "ROM",
    kind: AttributeKind::Choice { names: ROM_CODES },
    optional: false,
};
const BATTERY: AttributeSpec = AttributeSpec {
    key: "battery",
    label: "Battery",
    kind: AttributeKind::Toggle,
    optional: true,
};
const RTC: AttributeSpec = AttributeSpec {
    key: "rtc",
    label: "Timer",
    kind: AttributeKind::Toggle,
    optional: true,
};
const RUMBLE: AttributeSpec = AttributeSpec {
    key: "rumble",
    label: "Rumble",
    kind: AttributeKind::Toggle,
    optional: true,
};

/// The RAM chip populated beside the mapper, over the sizes its bank lines
/// reach; unstated where the board carries none.
const fn ram(names: &'static [&'static str]) -> AttributeSpec {
    AttributeSpec {
        key: "ram",
        label: "RAM",
        kind: AttributeKind::Choice { names },
        optional: true,
    }
}

/// The whole board vocabulary, one row per mapper, each listing the parts its
/// silicon can vary. Every name a board answers to derives from here.
const CATALOGUE: &[BoardSpec] = &[
    BoardSpec {
        name: "Rom",
        display: "ROM only",
        attributes: &[ram(ONE_RAM_CHIP), BATTERY],
    },
    BoardSpec {
        name: "Mbc1",
        display: "MBC1",
        attributes: &[ROM, ram(FOUR_RAM_BANKS), BATTERY],
    },
    BoardSpec {
        name: "Mbc1Multicart",
        display: "MBC1 multicart",
        attributes: &[ROM],
    },
    BoardSpec {
        name: "Mbc2",
        display: "MBC2",
        attributes: &[ROM, BATTERY],
    },
    BoardSpec {
        name: "Mbc3",
        display: "MBC3",
        attributes: &[ROM, ram(FOUR_RAM_BANKS), BATTERY, RTC],
    },
    BoardSpec {
        name: "Mbc30",
        display: "MBC30",
        attributes: &[ROM, ram(RAM_SIZES), BATTERY, RTC],
    },
    BoardSpec {
        name: "Mbc5",
        display: "MBC5",
        attributes: &[ROM, ram(RAM_SIZES), BATTERY, RUMBLE],
    },
    BoardSpec {
        name: "Mbc6",
        display: "MBC6",
        attributes: &[],
    },
    BoardSpec {
        name: "Mbc7",
        display: "MBC7",
        attributes: &[ROM],
    },
    BoardSpec {
        name: "Huc1",
        display: "HuC-1",
        attributes: &[ROM, ram(FOUR_RAM_BANKS)],
    },
    BoardSpec {
        name: "Huc3",
        display: "HuC-3",
        attributes: &[ROM, ram(FOUR_RAM_BANKS)],
    },
    BoardSpec {
        name: "DbzTrans",
        display: "DBZ Trans (unlicensed)",
        attributes: &[ram(RAM_SIZES)],
    },
    BoardSpec {
        name: "SachenMmc1",
        display: "Sachen MMC1 (unlicensed)",
        attributes: &[ROM],
    },
];

impl GbCartType {
    /// This board's variant name, the string every channel says it by.
    pub fn name(&self) -> &'static str {
        match self {
            GbCartType::Rom { .. } => "Rom",
            GbCartType::Mbc1 { .. } => "Mbc1",
            GbCartType::Mbc1Multicart { .. } => "Mbc1Multicart",
            GbCartType::Mbc2 { .. } => "Mbc2",
            GbCartType::Mbc3 { .. } => "Mbc3",
            GbCartType::Mbc30 { .. } => "Mbc30",
            GbCartType::Mbc5 { .. } => "Mbc5",
            GbCartType::Mbc6 => "Mbc6",
            GbCartType::Mbc7 { .. } => "Mbc7",
            GbCartType::Huc1 { .. } => "Huc1",
            GbCartType::Huc3 { .. } => "Huc3",
            GbCartType::DbzTrans { .. } => "DbzTrans",
            GbCartType::SachenMmc1 { .. } => "SachenMmc1",
        }
    }

    /// This board's catalogue row.
    fn spec(&self) -> &'static BoardSpec {
        let name = self.name();
        CATALOGUE
            .iter()
            .find(|spec| spec.name == name)
            .expect("every board has a catalogue row")
    }

    /// The board the three header bytes declare; `Err` carries a `$0147` value
    /// naming none. A header off the `$0148` ladder leaves the image's own
    /// length to say how big the chip is.
    pub fn from_header(rom: &[u8]) -> Result<GbCartType, u8> {
        let mapper = rom[0x147];
        let rom_size =
            GbRomSize::from_header(rom[0x148]).unwrap_or_else(|| GbRomSize::for_image(rom.len()));
        let ram = GbRamSize::from_header(rom[0x149]);
        // A mapper's bank lines cap the chip beside it, so a header naming a
        // larger one names a board this mapper is not.
        let reachable = |names: &[&str]| ram.filter(|chip| names.contains(&chip.code()));
        // Only a ROM past 2 MB or a RAM chip past four banks needs the MBC30's
        // wider registers, and no header byte tells the two chips apart.
        let mbc30 = rom.len() > 0x200000 || matches!(rom[0x149], 4 | 5);

        let mbc3 = |battery, rtc| match mbc30 {
            true => GbCartType::Mbc30 {
                rom: rom_size,
                ram,
                battery,
                rtc,
            },
            false => GbCartType::Mbc3 {
                rom: rom_size,
                ram: reachable(FOUR_RAM_BANKS),
                battery,
                rtc,
            },
        };
        let mbc5 = |battery, rumble| GbCartType::Mbc5 {
            rom: rom_size,
            ram,
            battery,
            rumble,
        };

        Ok(match mapper {
            0x00 | 0x08 => GbCartType::Rom {
                ram: reachable(ONE_RAM_CHIP),
                battery: false,
            },
            0x09 => GbCartType::Rom {
                ram: reachable(ONE_RAM_CHIP),
                battery: true,
            },
            0x01 | 0x02 => GbCartType::Mbc1 {
                rom: rom_size,
                ram: reachable(FOUR_RAM_BANKS),
                battery: false,
            },
            0x03 => GbCartType::Mbc1 {
                rom: rom_size,
                ram: reachable(FOUR_RAM_BANKS),
                battery: true,
            },
            0x05 => GbCartType::Mbc2 {
                rom: rom_size,
                battery: false,
            },
            0x06 => GbCartType::Mbc2 {
                rom: rom_size,
                battery: true,
            },
            0x0f | 0x10 => mbc3(true, true),
            0x11 | 0x12 => mbc3(false, false),
            0x13 => mbc3(true, false),
            0x19 | 0x1a => mbc5(false, false),
            0x1b => mbc5(true, false),
            0x1c | 0x1d => mbc5(false, true),
            0x1e => mbc5(true, true),
            0x20 => GbCartType::Mbc6,
            0x22 => GbCartType::Mbc7 { rom: rom_size },
            0xfe => GbCartType::Huc3 {
                rom: rom_size,
                ram: reachable(FOUR_RAM_BANKS),
            },
            0xff => GbCartType::Huc1 {
                rom: rom_size,
                ram: reachable(FOUR_RAM_BANKS),
            },
            byte => return Err(byte),
        })
    }

    /// The ROM chip's size, where the board doesn't fix one by its wiring.
    pub fn rom_size(&self) -> Option<GbRomSize> {
        match self {
            GbCartType::Mbc1 { rom, .. }
            | GbCartType::Mbc1Multicart { rom }
            | GbCartType::Mbc2 { rom, .. }
            | GbCartType::Mbc3 { rom, .. }
            | GbCartType::Mbc30 { rom, .. }
            | GbCartType::Mbc5 { rom, .. }
            | GbCartType::Mbc7 { rom }
            | GbCartType::Huc1 { rom, .. }
            | GbCartType::Huc3 { rom, .. }
            | GbCartType::SachenMmc1 { rom } => Some(*rom),
            GbCartType::Rom { .. } | GbCartType::Mbc6 | GbCartType::DbzTrans { .. } => None,
        }
    }

    /// The RAM chip populated beside the mapper, where the board takes one.
    pub fn ram_size(&self) -> Option<GbRamSize> {
        match self {
            GbCartType::Rom { ram, .. }
            | GbCartType::Mbc1 { ram, .. }
            | GbCartType::Mbc3 { ram, .. }
            | GbCartType::Mbc30 { ram, .. }
            | GbCartType::Mbc5 { ram, .. }
            | GbCartType::Huc1 { ram, .. }
            | GbCartType::Huc3 { ram, .. }
            | GbCartType::DbzTrans { ram } => *ram,
            GbCartType::Mbc1Multicart { .. }
            | GbCartType::Mbc2 { .. }
            | GbCartType::Mbc6
            | GbCartType::Mbc7 { .. }
            | GbCartType::SachenMmc1 { .. } => None,
        }
    }

    /// Whether a battery is fitted, whatever it holds alive.
    fn battery(&self) -> bool {
        match self {
            GbCartType::Rom { battery, .. }
            | GbCartType::Mbc1 { battery, .. }
            | GbCartType::Mbc2 { battery, .. }
            | GbCartType::Mbc3 { battery, .. }
            | GbCartType::Mbc30 { battery, .. }
            | GbCartType::Mbc5 { battery, .. } => *battery,
            GbCartType::Mbc7 { .. }
            | GbCartType::Huc1 { .. }
            | GbCartType::Huc3 { .. }
            | GbCartType::DbzTrans { .. } => true,
            GbCartType::Mbc1Multicart { .. } | GbCartType::Mbc6 | GbCartType::SachenMmc1 { .. } => {
                false
            }
        }
    }

    fn rumble(&self) -> bool {
        match self {
            GbCartType::Mbc5 { rumble, .. } => *rumble,
            GbCartType::Mbc7 { .. } => true,
            _ => false,
        }
    }

    /// Whether the board keeps a store alive off the cartridge battery, so a
    /// save is worth restoring and worth writing back. A clock-only battery
    /// backs no store, so there is nothing to keep.
    pub fn has_battery(&self) -> bool {
        match self {
            // MBC2's store is on the mapper die and MBC7's is an EEPROM, so
            // neither depends on a RAM chip being populated.
            GbCartType::Mbc2 { battery, .. } => *battery,
            GbCartType::Mbc7 { .. } => true,
            _ => self.battery() && self.ram_size().is_some(),
        }
    }

    /// Whether the board is populated with the MBC3 real-time clock.
    pub fn has_timer(&self) -> bool {
        matches!(
            self,
            GbCartType::Mbc3 { rtc: true, .. } | GbCartType::Mbc30 { rtc: true, .. }
        )
    }
}

impl BoardVocabulary for GbCartType {
    fn catalogue() -> &'static [BoardSpec] {
        CATALOGUE
    }

    fn to_value(&self) -> BoardValue {
        let mut value = BoardValue::new(self.name());
        if let Some(rom) = self.rom_size() {
            value = value.with_choice("rom", rom.code());
        }
        if let Some(ram) = self.ram_size() {
            value = value.with_choice("ram", ram.code());
        }
        for attribute in self.spec().attributes {
            value = match attribute.key {
                "battery" => value.with_toggle("battery", self.battery()),
                "rtc" => value.with_toggle("rtc", self.has_timer()),
                "rumble" => value.with_toggle("rumble", self.rumble()),
                _ => value,
            };
        }
        value
    }

    fn from_value(value: &BoardValue) -> Result<GbCartType, String> {
        let reader = value.read(VOCABULARY, CATALOGUE)?;
        let rom = || {
            GbRomSize::from_code(reader.choice("rom")).expect("the catalogue lists every ROM size")
        };
        let ram = || {
            reader
                .optional_choice("ram")
                .map(|code| GbRamSize::from_code(code).expect("the catalogue lists every RAM size"))
        };
        Ok(match reader.board() {
            "Rom" => GbCartType::Rom {
                ram: ram(),
                battery: reader.toggle("battery"),
            },
            "Mbc1" => GbCartType::Mbc1 {
                rom: rom(),
                ram: ram(),
                battery: reader.toggle("battery"),
            },
            "Mbc1Multicart" => GbCartType::Mbc1Multicart { rom: rom() },
            "Mbc2" => GbCartType::Mbc2 {
                rom: rom(),
                battery: reader.toggle("battery"),
            },
            "Mbc3" => GbCartType::Mbc3 {
                rom: rom(),
                ram: ram(),
                battery: reader.toggle("battery"),
                rtc: reader.toggle("rtc"),
            },
            "Mbc30" => GbCartType::Mbc30 {
                rom: rom(),
                ram: ram(),
                battery: reader.toggle("battery"),
                rtc: reader.toggle("rtc"),
            },
            "Mbc5" => GbCartType::Mbc5 {
                rom: rom(),
                ram: ram(),
                battery: reader.toggle("battery"),
                rumble: reader.toggle("rumble"),
            },
            "Mbc6" => GbCartType::Mbc6,
            "Mbc7" => GbCartType::Mbc7 { rom: rom() },
            "Huc1" => GbCartType::Huc1 {
                rom: rom(),
                ram: ram(),
            },
            "Huc3" => GbCartType::Huc3 {
                rom: rom(),
                ram: ram(),
            },
            "DbzTrans" => GbCartType::DbzTrans { ram: ram() },
            "SachenMmc1" => GbCartType::SachenMmc1 { rom: rom() },
            board => unreachable!("the catalogue names no {board} board"),
        })
    }

    fn display_name(&self) -> String {
        let spec = self.spec();
        let value = self.to_value();
        let mut parts = Vec::new();
        for attribute in spec.attributes {
            match value.attributes.get(attribute.key) {
                Some(AttributeValue::Choice(size)) if attribute.key == "rom" => {
                    parts.push(size.clone())
                }
                Some(AttributeValue::Choice(size)) => {
                    parts.push(format!("{} {size}", attribute.label))
                }
                Some(AttributeValue::Toggle(true)) => parts.push(attribute.label.to_lowercase()),
                _ => {}
            }
        }
        match parts.is_empty() {
            true => spec.display.to_owned(),
            false => format!("{} ({})", spec.display, parts.join(", ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One board of every variant, each carrying parts its catalogue row
    /// declares.
    fn samples() -> Vec<GbCartType> {
        vec![
            GbCartType::Rom {
                ram: None,
                battery: false,
            },
            GbCartType::Rom {
                ram: Some(GbRamSize::Kb8),
                battery: true,
            },
            GbCartType::Mbc1 {
                rom: GbRomSize::Mb1,
                ram: Some(GbRamSize::Kb32),
                battery: true,
            },
            GbCartType::Mbc1Multicart {
                rom: GbRomSize::Mb1,
            },
            GbCartType::Mbc2 {
                rom: GbRomSize::Kb256,
                battery: true,
            },
            GbCartType::Mbc3 {
                rom: GbRomSize::Mb2,
                ram: Some(GbRamSize::Kb32),
                battery: true,
                rtc: true,
            },
            GbCartType::Mbc30 {
                rom: GbRomSize::Mb2,
                ram: Some(GbRamSize::Kb64),
                battery: true,
                rtc: true,
            },
            GbCartType::Mbc5 {
                rom: GbRomSize::Mb4,
                ram: Some(GbRamSize::Kb128),
                battery: true,
                rumble: true,
            },
            GbCartType::Mbc6,
            GbCartType::Mbc7 {
                rom: GbRomSize::Kb256,
            },
            GbCartType::Huc1 {
                rom: GbRomSize::Mb1,
                ram: Some(GbRamSize::Kb32),
            },
            GbCartType::Huc3 {
                rom: GbRomSize::Mb2,
                ram: Some(GbRamSize::Kb32),
            },
            GbCartType::DbzTrans {
                ram: Some(GbRamSize::Kb32),
            },
            GbCartType::SachenMmc1 {
                rom: GbRomSize::Kb256,
            },
        ]
    }

    /// A header naming `cartridge_type`, with the two size bytes beside it.
    fn rom_declaring(cartridge_type: u8, rom_size: u8, ram_size: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = cartridge_type;
        rom[0x148] = rom_size;
        rom[0x149] = ram_size;
        rom
    }

    #[test]
    fn every_catalogue_row_has_a_sample() {
        for spec in CATALOGUE {
            assert!(
                samples().iter().any(|board| board.name() == spec.name),
                "{} has no sample",
                spec.name
            );
        }
    }

    #[test]
    fn every_board_round_trips_through_its_value() {
        for board in samples() {
            let value = board.to_value();
            assert_eq!(value.board, board.name());
            assert_eq!(GbCartType::from_value(&value), Ok(board));
        }
    }

    #[test]
    fn every_board_round_trips_through_ron() {
        for board in samples() {
            let text = ron::to_string(&board).expect("a board serialises");
            assert!(text.starts_with(board.name()), "{text}");
            assert_eq!(ron::from_str::<GbCartType>(&text), Ok(board));
        }
    }

    #[test]
    fn a_boards_parts_are_shown_beside_its_name() {
        let mbc5 = GbCartType::Mbc5 {
            rom: GbRomSize::Mb1,
            ram: Some(GbRamSize::Kb32),
            battery: false,
            rumble: true,
        };
        assert_eq!(mbc5.display_name(), "MBC5 (1M, RAM 32K, rumble)");
        assert_eq!(GbCartType::Mbc6.display_name(), "MBC6");
    }

    #[test]
    fn a_statement_the_vocabulary_cannot_read_is_refused() {
        let unknown_board = BoardValue::new("Mbc9");
        assert!(
            GbCartType::from_value(&unknown_board)
                .unwrap_err()
                .contains("unknown Game Boy board")
        );

        let no_such_part = BoardValue::new("Mbc2")
            .with_choice("rom", "256K")
            .with_choice("ram", "8K");
        assert!(
            GbCartType::from_value(&no_such_part)
                .unwrap_err()
                .contains("carries no \"ram\" attribute")
        );

        let no_rom_size = BoardValue::new("Mbc5");
        assert!(
            GbCartType::from_value(&no_rom_size)
                .unwrap_err()
                .contains("needs a \"rom\" attribute")
        );

        let ram_past_the_bank_lines = BoardValue::new("Mbc1")
            .with_choice("rom", "1M")
            .with_choice("ram", "128K");
        assert!(
            GbCartType::from_value(&ram_past_the_bank_lines)
                .unwrap_err()
                .contains("8K, 32K")
        );
    }

    #[test]
    fn every_declared_mapper_names_a_board() {
        const DECLARED: &[(u8, &str)] = &[
            (0x00, "Rom"),
            (0x08, "Rom"),
            (0x09, "Rom"),
            (0x01, "Mbc1"),
            (0x02, "Mbc1"),
            (0x03, "Mbc1"),
            (0x05, "Mbc2"),
            (0x06, "Mbc2"),
            (0x0f, "Mbc3"),
            (0x10, "Mbc3"),
            (0x11, "Mbc3"),
            (0x12, "Mbc3"),
            (0x13, "Mbc3"),
            (0x19, "Mbc5"),
            (0x1a, "Mbc5"),
            (0x1b, "Mbc5"),
            (0x1c, "Mbc5"),
            (0x1d, "Mbc5"),
            (0x1e, "Mbc5"),
            (0x20, "Mbc6"),
            (0x22, "Mbc7"),
            (0xfe, "Huc3"),
            (0xff, "Huc1"),
        ];
        for (byte, name) in DECLARED {
            let board = GbCartType::from_header(&rom_declaring(*byte, 0, 0))
                .expect("the byte names a board");
            assert_eq!(board.name(), *name, "${byte:02x}");
        }
        for byte in 0..=u8::MAX {
            let declared = DECLARED.iter().any(|(listed, _)| listed == &byte);
            assert_eq!(
                GbCartType::from_header(&rom_declaring(byte, 0, 0)).is_ok(),
                declared,
                "${byte:02x}"
            );
        }
    }

    #[test]
    fn every_header_board_states_itself_back() {
        for mapper in 0..=u8::MAX {
            for size in 0..=6u8 {
                let Ok(board) = GbCartType::from_header(&rom_declaring(mapper, 0, size)) else {
                    continue;
                };
                assert_eq!(
                    GbCartType::from_value(&board.to_value()),
                    Ok(board),
                    "${mapper:02x} / $0149 = {size}"
                );
            }
        }
    }

    #[test]
    fn no_header_byte_names_a_stated_only_board() {
        for byte in 0..=u8::MAX {
            let Ok(board) = GbCartType::from_header(&rom_declaring(byte, 0, 0)) else {
                continue;
            };
            assert!(!matches!(
                board,
                GbCartType::Mbc1Multicart { .. }
                    | GbCartType::DbzTrans { .. }
                    | GbCartType::SachenMmc1 { .. }
            ));
        }
    }

    #[test]
    fn the_header_sizes_reach_the_board() {
        let board = GbCartType::from_header(&rom_declaring(0x1b, 5, 3)).unwrap();
        assert_eq!(board.rom_size(), Some(GbRomSize::Mb1));
        assert_eq!(board.ram_size(), Some(GbRamSize::Kb32));

        // The unofficial 2 KB value names no chip this core builds.
        let no_chip = GbCartType::from_header(&rom_declaring(0x03, 0, 1)).unwrap();
        assert_eq!(no_chip.ram_size(), None);
    }

    #[test]
    fn a_rom_size_off_the_ladder_falls_back_to_the_image() {
        let mut rom = vec![0u8; 0x20000];
        rom[0x147] = 0x01;
        rom[0x148] = 0x52;
        let board = GbCartType::from_header(&rom).unwrap();
        assert_eq!(board.rom_size(), Some(GbRomSize::Kb128));
    }

    #[test]
    fn the_wider_chip_is_read_from_the_sizes_mbc3_shares() {
        let banked_ram = GbCartType::from_header(&rom_declaring(0x10, 3, 4)).unwrap();
        assert_eq!(banked_ram.name(), "Mbc30");

        let mut wide_rom = vec![0u8; 0x400000];
        wide_rom[0x147] = 0x13;
        wide_rom[0x148] = 7;
        wide_rom[0x149] = 3;
        assert_eq!(GbCartType::from_header(&wide_rom).unwrap().name(), "Mbc30");

        let narrow = GbCartType::from_header(&rom_declaring(0x13, 0, 3)).unwrap();
        assert_eq!(narrow.name(), "Mbc3");
    }

    #[test]
    fn a_board_keeps_a_save_when_the_battery_holds_a_store() {
        // $0F is MBC3 + timer + battery with no cartridge RAM: the battery
        // backs the clock alone, so there is no save to keep.
        const BATTERY_BYTES: &[u8] = &[0x03, 0x06, 0x09, 0x10, 0x13, 0x1b, 0x1e, 0x22, 0xfe, 0xff];
        for byte in 0..=u8::MAX {
            // $0149 = 2 populates the one RAM chip every RAM-bearing board takes.
            let Ok(board) = GbCartType::from_header(&rom_declaring(byte, 0, 2)) else {
                continue;
            };
            let expected = BATTERY_BYTES.contains(&byte) || byte == 0x0f;
            assert_eq!(board.has_battery(), expected, "${byte:02x}");
        }

        let clock_only = GbCartType::from_header(&rom_declaring(0x0f, 0, 0)).unwrap();
        assert!(!clock_only.has_battery());
        assert!(clock_only.has_timer());
    }
}
