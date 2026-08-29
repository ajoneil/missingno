//! The board a ROM is wired for, and the names it answers to.
//!
//! An SG-1000 dump is the ROM's contents and nothing else — no header, and no
//! length that tells a RAM-bearing board from a plain one — so nothing here is
//! inferred: a board is stated by a catalogue or an override, or the image
//! loads as a plain ROM.

use missingno_core::cartridge::{
    AttributeKind, AttributeSpec, AttributeValue, BoardNames, BoardSpec, BoardValue,
    BoardVocabulary, attributed_row,
};

use super::{CARTRIDGE_SPAN, EXM2_WINDOW};

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    /// A flat image spans at most the two cartridge windows.
    UnsupportedSize(usize),
    /// The image runs past the ROM window of the board it was declared as.
    WrongSizeForBoard { cart_type: CartType, size: usize },
}

impl std::fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CartridgeError::UnsupportedSize(size) => write!(f, "unsupported image size {size}"),
            CartridgeError::WrongSizeForBoard { cart_type, size } => write!(
                f,
                "image is {size} bytes but a {} board holds at most {}",
                cart_type.name(),
                cart_type.rom_window()
            ),
        }
    }
}

impl std::error::Error for CartridgeError {}

/// The board a ROM is wired for. The two Sega boards answer `/EXM1` with work
/// RAM beside the ROM; the two Taiwanese expanders carry RAM over the console's
/// own work-RAM window and hold `/DSRAM` high to deselect it.
///
/// A chip smaller than the window it answers repeats through the rest, so a
/// dump can be longer than the silicon it came off: the ROM's size is a
/// measurement, not the image's length.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CartType {
    /// ROM alone, repeating through the window its address lines can't decode.
    Flat { rom: Option<u32> },
    /// 2 KB of RAM behind `/EXM1` (Othello, board 171-5044).
    OthelloRam { rom: Option<u32> },
    /// 8 KB of RAM behind `/EXM1` (The Castle, board 171-5382).
    CastleRam { rom: Option<u32> },
    /// A pass-through expander: 8 KB inside `/EXM2` and 1 KB over the console's
    /// work RAM.
    DahjeeA { rom: Option<u32> },
    /// A pass-through expander: 8 KB over the console's work RAM.
    DahjeeB { rom: Option<u32> },
}

/// The name refusals say this vocabulary by.
const VOCABULARY: &str = "SG-1000";

/// The ROM chip's measured size, unstated where nobody has measured it.
const ROM: &[AttributeSpec] = &[AttributeSpec {
    key: "rom",
    label: "ROM",
    kind: AttributeKind::Bytes,
    optional: true,
}];

/// The whole board vocabulary, one row per board — the code a board goes by in
/// interchange (game-db entries, the CLI, a test's board override), the name
/// shown to a reader, and the silicon a caller can state beside it. Every name
/// a board answers to derives from here.
const BOARD_NAMES: &[BoardNames<CartType>] = &[
    attributed_row(CartType::Flat { rom: None }, "Flat", "Plain ROM", ROM),
    attributed_row(
        CartType::OthelloRam { rom: None },
        "OthelloRam",
        "Sega 2 KB RAM (Othello)",
        ROM,
    ),
    attributed_row(
        CartType::CastleRam { rom: None },
        "CastleRam",
        "Sega 8 KB RAM (The Castle)",
        ROM,
    ),
    attributed_row(
        CartType::DahjeeA { rom: None },
        "DahjeeA",
        "DahJee expander Type A",
        ROM,
    ),
    attributed_row(
        CartType::DahjeeB { rom: None },
        "DahjeeB",
        "DahJee expander Type B",
        ROM,
    ),
];

missingno_core::board_vocabulary!(CartType, BOARD_NAMES);

impl BoardVocabulary for CartType {
    fn catalogue() -> &'static [BoardSpec] {
        CartType::catalogue()
    }

    fn to_value(&self) -> BoardValue {
        BoardValue::new(self.name()).with_optional("rom", self.rom().map(AttributeValue::Bytes))
    }

    fn from_value(value: &BoardValue) -> Result<CartType, String> {
        let reader = value.read(VOCABULARY, CartType::catalogue())?;
        let rom = reader.optional_bytes("rom");
        Ok(match reader.board() {
            "Flat" => CartType::Flat { rom },
            "OthelloRam" => CartType::OthelloRam { rom },
            "CastleRam" => CartType::CastleRam { rom },
            "DahjeeA" => CartType::DahjeeA { rom },
            "DahjeeB" => CartType::DahjeeB { rom },
            board => unreachable!("the catalogue names no {board} board"),
        })
    }

    fn display_name(&self) -> String {
        match self.rom() {
            Some(rom) => format!("{} ({rom} bytes)", self.board_display()),
            None => self.board_display().to_owned(),
        }
    }
}

impl CartType {
    /// The ROM chip's measured size, where anyone has measured it.
    pub fn rom(self) -> Option<u32> {
        match self {
            CartType::Flat { rom }
            | CartType::OthelloRam { rom }
            | CartType::CastleRam { rom }
            | CartType::DahjeeA { rom }
            | CartType::DahjeeB { rom } => rom,
        }
    }

    /// How far the board's ROM reaches: `/EXM2` alone where the board's own RAM
    /// answers `/EXM1`, both windows where the image runs on into it.
    pub(super) fn rom_window(self) -> usize {
        match self {
            CartType::Flat { .. } | CartType::DahjeeA { .. } | CartType::DahjeeB { .. } => {
                CARTRIDGE_SPAN
            }
            CartType::OthelloRam { .. } | CartType::CastleRam { .. } => EXM2_WINDOW,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_board_round_trips_its_name() {
        for row in BOARD_NAMES {
            assert_eq!(CartType::from_name(row.name), Some(row.board));
            assert_eq!(row.board.name(), row.name);
            assert!(!row.board.display_name().is_empty());
        }
        assert_eq!(CartType::all().count(), BOARD_NAMES.len());
        assert_eq!(CartType::from_name("F8"), None);
    }

    #[test]
    fn every_board_round_trips_through_its_value() {
        for board in CartType::all() {
            assert_eq!(CartType::from_value(&board.to_value()), Ok(board));
            let measured =
                CartType::from_value(&board.to_value().with("rom", AttributeValue::Bytes(0x4000)))
                    .unwrap();
            assert_eq!(measured.rom(), Some(0x4000));
            assert_eq!(measured.name(), board.name());
        }
    }

    #[test]
    fn every_board_round_trips_through_ron() {
        for board in CartType::all() {
            let text = ron::to_string(&board).expect("a board serialises");
            // The vocabulary's name and the serialised variant are one string:
            // if a row drifts from its variant, this is what catches it.
            assert!(text.starts_with(board.name()), "{text}");
            assert_eq!(ron::from_str::<CartType>(&text), Ok(board));
        }
    }

    #[test]
    fn an_unlisted_name_names_no_board() {
        assert!(ron::from_str::<CartType>("F8").is_err());
        assert!(
            CartType::from_value(&BoardValue::new("F8"))
                .unwrap_err()
                .contains("unknown SG-1000 board")
        );
    }
}
