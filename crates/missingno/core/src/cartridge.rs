//! The names a cartridge board answers to, shared by every core that has a
//! board vocabulary.
//!
//! A board is one row of names: the value the media declares it by where the
//! media names one at all, the variant's own name — what serde writes, and what
//! a game-db entry, the CLI and a launch value all say — and the name shown to a
//! reader. A core states its own rows; the lookups over them are the same
//! everywhere.

/// One board's names. `Declared` carries what the media itself says names this
/// board, and is `()` on a console whose dumps carry no header. `name` is the
/// variant's own name, so it is the same string serde reads and writes; a test
/// per vocabulary holds the two together.
pub struct BoardNames<Board, Declared = ()> {
    pub board: Board,
    pub declared: Declared,
    pub name: &'static str,
    pub display: &'static str,
}

/// One row of a headerless console's vocabulary.
pub const fn row<Board>(
    board: Board,
    name: &'static str,
    display: &'static str,
) -> BoardNames<Board> {
    BoardNames {
        board,
        declared: (),
        name,
        display,
    }
}

/// Every board in the vocabulary, in its order.
pub fn boards<Board: Copy, Declared>(
    rows: &'static [BoardNames<Board, Declared>],
) -> impl Iterator<Item = Board> {
    rows.iter().map(|row| row.board)
}

/// The board a variant name names.
pub fn board_from_name<Board: Copy, Declared>(
    rows: &'static [BoardNames<Board, Declared>],
    name: &str,
) -> Option<Board> {
    rows.iter()
        .find(|row| row.name == name)
        .map(|row| row.board)
}

/// The row a board's names live in.
pub fn names<Board: Copy + PartialEq, Declared>(
    rows: &'static [BoardNames<Board, Declared>],
    board: Board,
) -> &'static BoardNames<Board, Declared> {
    rows.iter()
        .find(|row| row.board == board)
        .expect("every board has a row in the vocabulary")
}

/// What every board enum answers, so a consumer can be written once over any
/// core's vocabulary rather than once per system.
pub trait BoardVocabulary: Copy + Sized {
    /// This board's variant name — the string serde reads and writes.
    fn name(self) -> &'static str;
    /// The board a variant name names.
    fn from_name(name: &str) -> Option<Self>;
    /// Every name in the vocabulary, in its order.
    fn names() -> Vec<&'static str>;
    /// The message an unlisted name is refused with.
    fn unknown_name() -> &'static str;
}

/// Bind a core's board enum to its vocabulary: `all`, `from_name`, `name` and
/// `display_name`. The enum derives its own serialised form, so the variant is
/// what a manifest holds; the third argument opens the message an unlisted name
/// is refused with.
#[macro_export]
macro_rules! board_vocabulary {
    ($board:ty, $rows:expr, $unknown_name:expr) => {
        impl $board {
            /// Every board the core knows, in the vocabulary's order.
            pub fn all() -> impl Iterator<Item = $board> {
                $crate::cartridge::boards($rows)
            }

            /// The board a variant name names — the inverse of `name`.
            pub fn from_name(name: &str) -> Option<$board> {
                $crate::cartridge::board_from_name($rows, name)
            }

            /// This board's variant name, the string every channel says it by.
            pub fn name(self) -> &'static str {
                $crate::cartridge::names($rows, self).name
            }

            /// The board's name for a reader.
            pub fn display_name(self) -> &'static str {
                $crate::cartridge::names($rows, self).display
            }

            /// The message an unlisted name is refused with.
            pub const fn unknown_name_message() -> &'static str {
                $unknown_name
            }
        }

        impl $crate::cartridge::BoardVocabulary for $board {
            fn name(self) -> &'static str {
                <$board>::name(self)
            }

            fn from_name(name: &str) -> Option<Self> {
                <$board>::from_name(name)
            }

            fn names() -> Vec<&'static str> {
                <$board>::all().map(<$board>::name).collect()
            }

            fn unknown_name() -> &'static str {
                <$board>::unknown_name_message()
            }
        }
    };
}
