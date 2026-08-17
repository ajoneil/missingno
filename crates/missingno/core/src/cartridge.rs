//! The names a cartridge board answers to, shared by every core that has a
//! board vocabulary.
//!
//! A board is one row of names: the value the media declares it by where the
//! media names one at all, the code it goes by in interchange — game-db
//! entries, the CLI, a launch value — and the name shown to a reader. A core
//! states its own rows; the lookups over them, and the serialised form the code
//! *is*, are the same everywhere.

/// One board's names. `Declared` carries what the media itself says names this
/// board, and is `()` on a console whose dumps carry no header.
pub struct BoardNames<Board, Declared = ()> {
    pub board: Board,
    pub declared: Declared,
    pub code: &'static str,
    pub display: &'static str,
}

/// One row of a headerless console's vocabulary.
pub const fn row<Board>(
    board: Board,
    code: &'static str,
    display: &'static str,
) -> BoardNames<Board> {
    BoardNames {
        board,
        declared: (),
        code,
        display,
    }
}

/// Every board in the vocabulary, in its order.
pub fn boards<Board: Copy, Declared>(
    rows: &'static [BoardNames<Board, Declared>],
) -> impl Iterator<Item = Board> {
    rows.iter().map(|row| row.board)
}

/// The board a code names.
pub fn board_from_code<Board: Copy, Declared>(
    rows: &'static [BoardNames<Board, Declared>],
    code: &str,
) -> Option<Board> {
    rows.iter()
        .find(|row| row.code == code)
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

/// Bind a core's board enum to its vocabulary: `all`, `from_code`, `code` and
/// `display_name`, and the interchange code as the whole serialised form. The
/// third argument opens the message an unlisted code is refused with.
#[macro_export]
macro_rules! board_vocabulary {
    ($board:ty, $rows:expr, $unknown_code:expr) => {
        /// A board crosses a catalogue as its interchange code, so the
        /// vocabulary is the whole serialised form: an unlisted code names no
        /// board this core builds.
        impl ::serde::Serialize for $board {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.code())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $board {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let code = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                <$board>::from_code(&code).ok_or_else(|| {
                    ::serde::de::Error::custom(format!("{} {code:?}", $unknown_code))
                })
            }
        }

        impl $board {
            /// Every board the core knows, in the vocabulary's order.
            pub fn all() -> impl Iterator<Item = $board> {
                $crate::cartridge::boards($rows)
            }

            /// The board a board code names.
            pub fn from_code(code: &str) -> Option<$board> {
                $crate::cartridge::board_from_code($rows, code)
            }

            /// The board code for this board — the inverse of `from_code`.
            pub fn code(self) -> &'static str {
                $crate::cartridge::names($rows, self).code
            }

            /// The board's name for a reader.
            pub fn display_name(self) -> &'static str {
                $crate::cartridge::names($rows, self).display
            }
        }
    };
}
