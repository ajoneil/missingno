//! The board a ROM is wired for, and the names it answers to.
//!
//! An SG-1000 dump is the ROM's contents and nothing else — no header, and no
//! length that tells a RAM-bearing board from a plain one — so nothing here is
//! inferred: a board is stated by a catalogue or an override, or the image
//! loads as a plain ROM.

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

/// One board's names: the code it goes by in interchange — game-db entries, the
/// CLI, a test's board override — and the name shown to a reader.
struct BoardNames {
    cart_type: CartType,
    code: &'static str,
    display: &'static str,
}

const fn row(cart_type: CartType, code: &'static str, display: &'static str) -> BoardNames {
    BoardNames {
        cart_type,
        code,
        display,
    }
}

/// The whole board vocabulary, one row per board. Every name a board answers to
/// derives from here.
const BOARD_NAMES: &[BoardNames] = &[
    row(CartType::Flat, "FLAT", "Plain ROM"),
    row(CartType::OthelloRam, "OTHELLO", "Sega 2 KB RAM (Othello)"),
    row(CartType::CastleRam, "CASTLE", "Sega 8 KB RAM (The Castle)"),
    row(CartType::DahjeeA, "DAHJEE-A", "DahJee expander Type A"),
    row(CartType::DahjeeB, "DAHJEE-B", "DahJee expander Type B"),
];

impl CartType {
    /// Every board the core knows, in the vocabulary's order.
    pub fn all() -> impl Iterator<Item = CartType> {
        BOARD_NAMES.iter().map(|board| board.cart_type)
    }

    /// The board a game-db board code names.
    pub fn from_code(code: &str) -> Option<CartType> {
        BOARD_NAMES
            .iter()
            .find(|board| board.code == code)
            .map(|board| board.cart_type)
    }

    /// The game-db board code for this board — the inverse of [`from_code`].
    ///
    /// [`from_code`]: CartType::from_code
    pub fn code(self) -> &'static str {
        self.names().code
    }

    /// The board's name for a reader.
    pub fn display_name(self) -> &'static str {
        self.names().display
    }

    fn names(self) -> &'static BoardNames {
        BOARD_NAMES
            .iter()
            .find(|board| board.cart_type == self)
            .expect("every board has a row in BOARD_NAMES")
    }

    /// How far the board's ROM reaches: `/EXM2` alone where the board's own RAM
    /// answers `/EXM1`, both windows where the image runs on into it.
    pub(super) fn rom_window(self) -> usize {
        match self {
            CartType::Flat | CartType::DahjeeB => CARTRIDGE_SPAN,
            CartType::OthelloRam | CartType::CastleRam | CartType::DahjeeA => EXM2_WINDOW,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_board_round_trips_its_code() {
        for board in BOARD_NAMES {
            assert_eq!(CartType::from_code(board.code), Some(board.cart_type));
            assert_eq!(board.cart_type.code(), board.code);
            assert!(!board.cart_type.display_name().is_empty());
        }
        assert_eq!(CartType::all().count(), BOARD_NAMES.len());
        assert_eq!(CartType::from_code("F8"), None);
    }
}
