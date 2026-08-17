//! The board a cartridge is built on, as its header declares it.
//!
//! A Game Boy cartridge names its own board in header byte `$0147`, so the
//! header is the normal path. A few boards no byte names — a multicart, the
//! MBC30 chip, an unlicensed mapper hiding behind a borrowed byte — so a caller
//! that knows better can state the board instead.

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// One board's names: the byte its header declares it by, the code it goes by in
/// interchange — game-db entries, the CLI, a launch value — and the name shown
/// to a reader.
struct BoardNames {
    cart_type: GbCartType,
    /// The `$0147` value declaring this board; `None` where no byte names it.
    header: Option<u8>,
    code: &'static str,
    display: &'static str,
}

const fn row(
    cart_type: GbCartType,
    header: Option<u8>,
    code: &'static str,
    display: &'static str,
) -> BoardNames {
    BoardNames {
        cart_type,
        header,
        code,
        display,
    }
}

/// The whole board vocabulary, one row per board. Every name a board answers to
/// derives from here.
const BOARD_NAMES: &[BoardNames] = &[
    row(GbCartType::Rom, Some(0x00), "ROM", "ROM only"),
    row(GbCartType::RomRam, Some(0x08), "ROM+RAM", "ROM + RAM"),
    row(
        GbCartType::RomRamBattery,
        Some(0x09),
        "ROM+RAM+BATTERY",
        "ROM + RAM + battery",
    ),
    row(GbCartType::Mbc1, Some(0x01), "MBC1", "MBC1"),
    row(GbCartType::Mbc1Ram, Some(0x02), "MBC1+RAM", "MBC1 + RAM"),
    row(
        GbCartType::Mbc1RamBattery,
        Some(0x03),
        "MBC1+RAM+BATTERY",
        "MBC1 + RAM + battery",
    ),
    row(GbCartType::Mbc1Multicart, None, "MBC1M", "MBC1 multicart"),
    row(GbCartType::Mbc2, Some(0x05), "MBC2", "MBC2"),
    row(
        GbCartType::Mbc2Battery,
        Some(0x06),
        "MBC2+BATTERY",
        "MBC2 + battery",
    ),
    row(
        GbCartType::Mbc3TimerBattery,
        Some(0x0f),
        "MBC3+TIMER+BATTERY",
        "MBC3 + timer + battery",
    ),
    row(
        GbCartType::Mbc3TimerRamBattery,
        Some(0x10),
        "MBC3+TIMER+RAM+BATTERY",
        "MBC3 + timer + RAM + battery",
    ),
    row(GbCartType::Mbc3, Some(0x11), "MBC3", "MBC3"),
    row(GbCartType::Mbc3Ram, Some(0x12), "MBC3+RAM", "MBC3 + RAM"),
    row(
        GbCartType::Mbc3RamBattery,
        Some(0x13),
        "MBC3+RAM+BATTERY",
        "MBC3 + RAM + battery",
    ),
    row(GbCartType::Mbc30, None, "MBC30", "MBC30"),
    row(GbCartType::Mbc5, Some(0x19), "MBC5", "MBC5"),
    row(GbCartType::Mbc5Ram, Some(0x1a), "MBC5+RAM", "MBC5 + RAM"),
    row(
        GbCartType::Mbc5RamBattery,
        Some(0x1b),
        "MBC5+RAM+BATTERY",
        "MBC5 + RAM + battery",
    ),
    row(
        GbCartType::Mbc5Rumble,
        Some(0x1c),
        "MBC5+RUMBLE",
        "MBC5 + rumble",
    ),
    row(
        GbCartType::Mbc5RumbleRam,
        Some(0x1d),
        "MBC5+RUMBLE+RAM",
        "MBC5 + rumble + RAM",
    ),
    row(
        GbCartType::Mbc5RumbleRamBattery,
        Some(0x1e),
        "MBC5+RUMBLE+RAM+BATTERY",
        "MBC5 + rumble + RAM + battery",
    ),
    row(GbCartType::Mbc6, Some(0x20), "MBC6", "MBC6"),
    row(GbCartType::Mbc7, Some(0x22), "MBC7", "MBC7"),
    row(GbCartType::Huc3, Some(0xfe), "HUC3", "HuC-3"),
    row(GbCartType::Huc1, Some(0xff), "HUC1", "HuC-1"),
    row(
        GbCartType::DbzTrans,
        None,
        "DBZTRANS",
        "DBZ Trans (unlicensed)",
    ),
];

impl GbCartType {
    /// The board header byte `$0147` declares; `Err` carries a byte naming none.
    pub fn from_header(byte: u8) -> Result<GbCartType, u8> {
        BOARD_NAMES
            .iter()
            .find(|board| board.header == Some(byte))
            .map(|board| board.cart_type)
            .ok_or(byte)
    }

    /// Every board the core knows, in the vocabulary's order.
    pub fn all() -> impl Iterator<Item = GbCartType> {
        BOARD_NAMES.iter().map(|board| board.cart_type)
    }

    /// The board a board code names.
    pub fn from_code(code: &str) -> Option<GbCartType> {
        BOARD_NAMES
            .iter()
            .find(|board| board.code == code)
            .map(|board| board.cart_type)
    }

    /// The board code for this board — the inverse of [`from_code`].
    ///
    /// [`from_code`]: GbCartType::from_code
    pub fn code(self) -> &'static str {
        self.names().code
    }

    /// The board's name for a reader: the mapper chip and the parts beside it.
    pub fn display_name(self) -> &'static str {
        self.names().display
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

    fn names(self) -> &'static BoardNames {
        BOARD_NAMES
            .iter()
            .find(|board| board.cart_type == self)
            .expect("every board has a row in BOARD_NAMES")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_board_round_trips_its_code() {
        for board in BOARD_NAMES {
            assert_eq!(GbCartType::from_code(board.code), Some(board.cart_type));
            assert_eq!(board.cart_type.code(), board.code);
        }
    }

    #[test]
    fn every_declared_board_round_trips_its_header_byte() {
        for board in BOARD_NAMES {
            let Some(byte) = board.header else { continue };
            assert_eq!(GbCartType::from_header(byte), Ok(board.cart_type));
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
