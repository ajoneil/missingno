//! The board a ROM is wired for, and the names it answers to.
//!
//! An SG-1000 dump is the ROM's contents and nothing else — no header, and no
//! length that tells a RAM-bearing board from a plain one — so nothing here is
//! inferred: a board is stated by a catalogue or an override, or the image
//! loads as a plain ROM.

use missingno_core::cartridge::{BoardNames, row};

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
                cart_type.code(),
                cart_type.rom_window()
            ),
        }
    }
}

impl std::error::Error for CartridgeError {}

/// The board a ROM is wired for. The two Sega boards answer `/EXM1` with work
/// RAM beside the ROM; the two Taiwanese expanders carry RAM over the console's
/// own work-RAM window and hold `/DSRAM` high to deselect it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CartType {
    /// ROM alone, repeating through the window its address lines can't decode.
    Flat,
    /// 2 KB of RAM behind `/EXM1` (Othello, board 171-5044).
    OthelloRam,
    /// 8 KB of RAM behind `/EXM1` (The Castle, board 171-5382).
    CastleRam,
    /// A pass-through expander: 8 KB inside `/EXM2` and 1 KB over the console's
    /// work RAM.
    DahjeeA,
    /// A pass-through expander: 8 KB over the console's work RAM.
    DahjeeB,
}

/// The whole board vocabulary, one row per board — the code a board goes by in
/// interchange (game-db entries, the CLI, a test's board override) and the name
/// shown to a reader. Every name a board answers to derives from here.
const BOARD_NAMES: &[BoardNames<CartType>] = &[
    row(CartType::Flat, "FLAT", "Plain ROM"),
    row(CartType::OthelloRam, "OTHELLO", "Sega 2 KB RAM (Othello)"),
    row(CartType::CastleRam, "CASTLE", "Sega 8 KB RAM (The Castle)"),
    row(CartType::DahjeeA, "DAHJEE-A", "DahJee expander Type A"),
    row(CartType::DahjeeB, "DAHJEE-B", "DahJee expander Type B"),
];

missingno_core::board_vocabulary!(CartType, BOARD_NAMES, "unknown SG-1000 board code");

impl CartType {
    /// How far the board's ROM reaches: `/EXM2` alone where the board's own RAM
    /// answers `/EXM1`, both windows where the image runs on into it.
    pub(super) fn rom_window(self) -> usize {
        match self {
            CartType::Flat | CartType::DahjeeA | CartType::DahjeeB => CARTRIDGE_SPAN,
            CartType::OthelloRam | CartType::CastleRam => EXM2_WINDOW,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_board_round_trips_its_code() {
        for row in BOARD_NAMES {
            assert_eq!(CartType::from_code(row.code), Some(row.board));
            assert_eq!(row.board.code(), row.code);
            assert!(!row.board.display_name().is_empty());
        }
        assert_eq!(CartType::all().count(), BOARD_NAMES.len());
        assert_eq!(CartType::from_code("F8"), None);
    }

    #[test]
    fn every_board_round_trips_through_ron() {
        for board in CartType::all() {
            let text = ron::to_string(&board).expect("a board serialises");
            assert_eq!(text, format!("{:?}", board.code()));
            assert_eq!(ron::from_str::<CartType>(&text), Ok(board));
        }
    }

    #[test]
    fn an_unlisted_code_names_no_board() {
        let error = ron::from_str::<CartType>("\"F8\"").expect_err("no such board");
        assert!(error.to_string().contains("\"F8\""), "{error}");
    }
}
