//! The board a cartridge is built on, as its header declares it.
//!
//! A Game Boy cartridge names its own board in header byte `$0147`, so the
//! header is the normal path. A few boards no byte names — a multicart, the
//! MBC30 chip, an unlicensed mapper hiding behind a borrowed byte — so a caller
//! that knows better can state the board instead.

use missingno_core::cartridge::BoardNames;

/// The cartridge's objection to media it cannot be built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbCartridgeError {
    /// A `$0147` value naming no board this core carries.
    UnknownMapper(u8),
}

impl std::fmt::Display for GbCartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GbCartridgeError::UnknownMapper(byte) => {
                write!(f, "unsupported cartridge type ${byte:02x}")
            }
        }
    }
}

impl std::error::Error for GbCartridgeError {}

/// A board the core can build: a mapper chip and the parts populated beside it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbCartType {
    Rom,
    RomRam,
    RomRamBattery,
    Mbc1,
    Mbc1Ram,
    Mbc1RamBattery,
    /// Several games on one MBC1 board, wired to bank in 4 Mbit slots. No header
    /// byte names it; the header path reads the second game's logo instead.
    Mbc1Multicart,
    Mbc2,
    Mbc2Battery,
    Mbc3TimerBattery,
    Mbc3TimerRamBattery,
    Mbc3,
    Mbc3Ram,
    Mbc3RamBattery,
    /// The MBC3 successor, whose wider bank register reaches 256 ROM banks. It
    /// shares MBC3's header bytes, so only ROM and RAM size distinguish it.
    Mbc30,
    Mbc5,
    Mbc5Ram,
    Mbc5RamBattery,
    Mbc5Rumble,
    Mbc5RumbleRam,
    Mbc5RumbleRamBattery,
    Mbc6,
    Mbc7,
    Huc3,
    Huc1,
    /// The unlicensed "GB DBZ GOKOU 2" mapper, which declares MBC5 in its header
    /// and adds an independent half-bank switch.
    DbzTrans,
}

/// One row of the vocabulary: the `$0147` value declaring this board — `None`
/// where no byte names it — beside the code it goes by in interchange and the
/// name shown to a reader.
const fn row(
    cart_type: GbCartType,
    header: Option<u8>,
    name: &'static str,
    display: &'static str,
) -> BoardNames<GbCartType, Option<u8>> {
    BoardNames {
        board: cart_type,
        declared: header,
        name,
        display,
    }
}

/// The whole board vocabulary, one row per board. Every name a board answers to
/// derives from here.
const BOARD_NAMES: &[BoardNames<GbCartType, Option<u8>>] = &[
    row(GbCartType::Rom, Some(0x00), "Rom", "ROM only"),
    row(GbCartType::RomRam, Some(0x08), "RomRam", "ROM + RAM"),
    row(
        GbCartType::RomRamBattery,
        Some(0x09),
        "RomRamBattery",
        "ROM + RAM + battery",
    ),
    row(GbCartType::Mbc1, Some(0x01), "Mbc1", "MBC1"),
    row(GbCartType::Mbc1Ram, Some(0x02), "Mbc1Ram", "MBC1 + RAM"),
    row(
        GbCartType::Mbc1RamBattery,
        Some(0x03),
        "Mbc1RamBattery",
        "MBC1 + RAM + battery",
    ),
    row(
        GbCartType::Mbc1Multicart,
        None,
        "Mbc1Multicart",
        "MBC1 multicart",
    ),
    row(GbCartType::Mbc2, Some(0x05), "Mbc2", "MBC2"),
    row(
        GbCartType::Mbc2Battery,
        Some(0x06),
        "Mbc2Battery",
        "MBC2 + battery",
    ),
    row(
        GbCartType::Mbc3TimerBattery,
        Some(0x0f),
        "Mbc3TimerBattery",
        "MBC3 + timer + battery",
    ),
    row(
        GbCartType::Mbc3TimerRamBattery,
        Some(0x10),
        "Mbc3TimerRamBattery",
        "MBC3 + timer + RAM + battery",
    ),
    row(GbCartType::Mbc3, Some(0x11), "Mbc3", "MBC3"),
    row(GbCartType::Mbc3Ram, Some(0x12), "Mbc3Ram", "MBC3 + RAM"),
    row(
        GbCartType::Mbc3RamBattery,
        Some(0x13),
        "Mbc3RamBattery",
        "MBC3 + RAM + battery",
    ),
    row(GbCartType::Mbc30, None, "Mbc30", "MBC30"),
    row(GbCartType::Mbc5, Some(0x19), "Mbc5", "MBC5"),
    row(GbCartType::Mbc5Ram, Some(0x1a), "Mbc5Ram", "MBC5 + RAM"),
    row(
        GbCartType::Mbc5RamBattery,
        Some(0x1b),
        "Mbc5RamBattery",
        "MBC5 + RAM + battery",
    ),
    row(
        GbCartType::Mbc5Rumble,
        Some(0x1c),
        "Mbc5Rumble",
        "MBC5 + rumble",
    ),
    row(
        GbCartType::Mbc5RumbleRam,
        Some(0x1d),
        "Mbc5RumbleRam",
        "MBC5 + rumble + RAM",
    ),
    row(
        GbCartType::Mbc5RumbleRamBattery,
        Some(0x1e),
        "Mbc5RumbleRamBattery",
        "MBC5 + rumble + RAM + battery",
    ),
    row(GbCartType::Mbc6, Some(0x20), "Mbc6", "MBC6"),
    row(GbCartType::Mbc7, Some(0x22), "Mbc7", "MBC7"),
    row(GbCartType::Huc3, Some(0xfe), "Huc3", "HuC-3"),
    row(GbCartType::Huc1, Some(0xff), "Huc1", "HuC-1"),
    row(
        GbCartType::DbzTrans,
        None,
        "DbzTrans",
        "DBZ Trans (unlicensed)",
    ),
];

missingno_core::board_vocabulary!(GbCartType, BOARD_NAMES, "unknown Game Boy board code");

impl GbCartType {
    /// The board header byte `$0147` declares; `Err` carries a byte naming none.
    pub fn from_header(byte: u8) -> Result<GbCartType, u8> {
        BOARD_NAMES
            .iter()
            .find(|row| row.declared == Some(byte))
            .map(|row| row.board)
            .ok_or(byte)
    }

    /// Whether the board keeps its RAM alive off the cartridge battery, so a
    /// save is worth restoring and worth writing back.
    pub fn has_battery(self) -> bool {
        matches!(
            self,
            GbCartType::RomRamBattery
                | GbCartType::Mbc1RamBattery
                | GbCartType::Mbc2Battery
                | GbCartType::Mbc3TimerRamBattery
                | GbCartType::Mbc3RamBattery
                | GbCartType::Mbc30
                | GbCartType::Mbc5RamBattery
                | GbCartType::Mbc5RumbleRamBattery
                | GbCartType::Mbc7
                | GbCartType::Huc3
                | GbCartType::Huc1
                | GbCartType::DbzTrans
        )
    }

    /// Whether the board is populated with the MBC3 real-time clock.
    pub fn has_timer(self) -> bool {
        matches!(
            self,
            GbCartType::Mbc3TimerBattery | GbCartType::Mbc3TimerRamBattery
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_board_round_trips_its_name() {
        for row in BOARD_NAMES {
            assert_eq!(GbCartType::from_name(row.name), Some(row.board));
            assert_eq!(row.board.name(), row.name);
        }
    }

    #[test]
    fn every_board_round_trips_through_ron() {
        for board in GbCartType::all() {
            let text = ron::to_string(&board).expect("a board serialises");
            // The vocabulary's name and the serialised variant are one string:
            // if a row drifts from its variant, this is what catches it.
            assert_eq!(text, board.name());
            assert_eq!(ron::from_str::<GbCartType>(&text), Ok(board));
        }
    }

    #[test]
    fn an_unlisted_name_names_no_board() {
        assert!(ron::from_str::<GbCartType>("F8").is_err());
    }

    #[test]
    fn every_declared_board_round_trips_its_header_byte() {
        for row in BOARD_NAMES {
            let Some(byte) = row.declared else { continue };
            assert_eq!(GbCartType::from_header(byte), Ok(row.board));
        }
    }

    #[test]
    fn an_undeclared_board_answers_to_no_header_byte() {
        for byte in 0..=u8::MAX {
            let Ok(cart_type) = GbCartType::from_header(byte) else {
                continue;
            };
            assert!(!matches!(
                cart_type,
                GbCartType::Mbc1Multicart | GbCartType::Mbc30 | GbCartType::DbzTrans
            ));
        }
    }

    #[test]
    fn a_board_keeps_a_save_when_its_ram_sits_on_the_battery() {
        // $0F is MBC3 + timer + battery with no cartridge RAM: the battery
        // backs the clock alone, so there is no save to keep.
        const BATTERY_BYTES: &[u8] = &[0x03, 0x06, 0x09, 0x10, 0x13, 0x1b, 0x1e, 0x22, 0xfe, 0xff];
        for byte in 0..=u8::MAX {
            let Ok(cart_type) = GbCartType::from_header(byte) else {
                continue;
            };
            assert_eq!(
                cart_type.has_battery(),
                BATTERY_BYTES.contains(&byte),
                "${byte:02x}"
            );
        }
    }
}
