//! The names a cartridge board answers to, and the parts populated on it,
//! shared by every core that has a board vocabulary.
//!
//! A board is a chip plus whatever silicon sits beside it: a ROM of some size,
//! a RAM chip or none, a battery, a clock. The chip is named — the variant's own
//! name, what serde writes and what a game-db entry, the CLI and a launch value
//! all say — and the parts are its attributes. A core states its own catalogue;
//! a consumer that renders or edits a board reads the catalogue rather than
//! knowing any console.

use std::collections::BTreeMap;

/// One board in a vocabulary: what it is called, and what its silicon can vary.
#[derive(Clone, Debug)]
pub struct BoardSpec {
    /// The variant's own name, so it is the same string serde reads and writes.
    pub name: &'static str,
    pub display: &'static str,
    pub attributes: &'static [AttributeSpec],
}

/// One part a board can carry, and how a caller states it.
#[derive(Clone, Debug)]
pub struct AttributeSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: AttributeKind,
    /// An optional attribute may be left unstated; what that means is the
    /// board's own — a chip that isn't populated, or silicon nobody measured.
    pub optional: bool,
}

/// What kind of value an attribute takes.
#[derive(Clone, Debug)]
pub enum AttributeKind {
    /// One of the sizes or settings this board's silicon comes in.
    Choice {
        names: &'static [&'static str],
    },
    Toggle,
    /// A measured byte count, for boards any sum of chips can make.
    Bytes,
}

impl AttributeKind {
    fn noun(&self) -> &'static str {
        match self {
            AttributeKind::Choice { .. } => "choice",
            AttributeKind::Toggle => "toggle",
            AttributeKind::Bytes => "byte count",
        }
    }
}

/// A board and its parts in the untyped form that crosses generic seams —
/// catalogue facts, agent tools, launch values.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BoardValue {
    pub board: String,
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// One attribute's value, in the shape its [`AttributeKind`] takes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AttributeValue {
    Choice(String),
    Toggle(bool),
    Bytes(u32),
}

impl AttributeValue {
    fn noun(&self) -> &'static str {
        match self {
            AttributeValue::Choice(_) => "choice",
            AttributeValue::Toggle(_) => "toggle",
            AttributeValue::Bytes(_) => "byte count",
        }
    }

    fn matches(&self, kind: &AttributeKind) -> bool {
        matches!(
            (self, kind),
            (AttributeValue::Choice(_), AttributeKind::Choice { .. })
                | (AttributeValue::Toggle(_), AttributeKind::Toggle)
                | (AttributeValue::Bytes(_), AttributeKind::Bytes)
        )
    }
}

impl BoardValue {
    /// A board with nothing stated beside it.
    pub fn new(board: &str) -> BoardValue {
        BoardValue {
            board: board.to_owned(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: &str, value: AttributeValue) -> BoardValue {
        self.attributes.insert(key.to_owned(), value);
        self
    }

    /// State `key` only where the board carries that part.
    pub fn with_optional(self, key: &str, value: Option<AttributeValue>) -> BoardValue {
        match value {
            Some(value) => self.with(key, value),
            None => self,
        }
    }

    pub fn with_choice(self, key: &str, name: &str) -> BoardValue {
        self.with(key, AttributeValue::Choice(name.to_owned()))
    }

    pub fn with_toggle(self, key: &str, on: bool) -> BoardValue {
        self.with(key, AttributeValue::Toggle(on))
    }

    /// Check the whole statement against `catalogue`, refusing in the
    /// vocabulary's own words: an unknown board, an attribute the board has
    /// none of, a value of the wrong kind, a choice outside the board's list,
    /// or a part the board always carries left unstated.
    pub fn read(
        &self,
        vocabulary: &'static str,
        catalogue: &'static [BoardSpec],
    ) -> Result<BoardReader<'_>, String> {
        let spec = catalogue
            .iter()
            .find(|spec| spec.name == self.board)
            .ok_or_else(|| format!("unknown {vocabulary} board \"{}\"", self.board))?;
        let board = &self.board;

        for (key, value) in &self.attributes {
            let attribute = spec
                .attributes
                .iter()
                .find(|attribute| attribute.key == key)
                .ok_or_else(|| {
                    format!("the {vocabulary} {board} board carries no \"{key}\" attribute")
                })?;
            if !value.matches(&attribute.kind) {
                return Err(format!(
                    "the {vocabulary} {board} board's \"{key}\" is a {}, not a {}",
                    attribute.kind.noun(),
                    value.noun()
                ));
            }
            if let (AttributeKind::Choice { names }, AttributeValue::Choice(chosen)) =
                (&attribute.kind, value)
                && !names.contains(&chosen.as_str())
            {
                return Err(format!(
                    "the {vocabulary} {board} board has no \"{chosen}\" {key}; it takes {}",
                    names.join(", ")
                ));
            }
        }

        for attribute in spec.attributes {
            if !attribute.optional && !self.attributes.contains_key(attribute.key) {
                return Err(format!(
                    "the {vocabulary} {board} board needs a \"{}\" attribute",
                    attribute.key
                ));
            }
        }

        Ok(BoardReader { value: self, spec })
    }
}

/// A checked [`BoardValue`], so a vocabulary can read its own attributes back
/// without re-stating the refusals.
pub struct BoardReader<'a> {
    value: &'a BoardValue,
    spec: &'static BoardSpec,
}

impl BoardReader<'_> {
    /// The board's name, which the catalogue lists.
    pub fn board(&self) -> &'static str {
        self.spec.name
    }

    fn get(&self, key: &str) -> Option<&AttributeValue> {
        debug_assert!(
            self.spec.attributes.iter().any(|a| a.key == key),
            "a vocabulary reads only the attributes its catalogue declares"
        );
        self.value.attributes.get(key)
    }

    pub fn optional_choice(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(AttributeValue::Choice(name)) => Some(name),
            _ => None,
        }
    }

    /// A choice the board always carries, so [`BoardValue::read`] has already
    /// refused a statement without one.
    pub fn choice(&self, key: &str) -> &str {
        self.optional_choice(key)
            .expect("a required attribute is stated")
    }

    /// An unstated toggle is off.
    pub fn toggle(&self, key: &str) -> bool {
        matches!(self.get(key), Some(AttributeValue::Toggle(true)))
    }

    pub fn optional_bytes(&self, key: &str) -> Option<u32> {
        match self.get(key) {
            Some(AttributeValue::Bytes(bytes)) => Some(*bytes),
            _ => None,
        }
    }

    /// A byte count the board always carries.
    pub fn bytes(&self, key: &str) -> u32 {
        self.optional_bytes(key)
            .expect("a required attribute is stated")
    }
}

/// One row of a vocabulary, tying a board's variant to its catalogue entry.
pub struct BoardNames<Board> {
    pub board: Board,
    pub name: &'static str,
    pub display: &'static str,
    pub attributes: &'static [AttributeSpec],
}

/// One row of a board whose silicon varies in nothing.
pub const fn row<Board>(
    board: Board,
    name: &'static str,
    display: &'static str,
) -> BoardNames<Board> {
    BoardNames {
        board,
        name,
        display,
        attributes: &[],
    }
}

/// One row of a board with parts a caller can state, `board` carrying the form
/// with none of them stated.
pub const fn attributed_row<Board>(
    board: Board,
    name: &'static str,
    display: &'static str,
    attributes: &'static [AttributeSpec],
) -> BoardNames<Board> {
    BoardNames {
        board,
        name,
        display,
        attributes,
    }
}

/// Every board in the vocabulary, in its order, with nothing stated beside it.
pub fn boards<Board: Copy>(rows: &'static [BoardNames<Board>]) -> impl Iterator<Item = Board> {
    rows.iter().map(|row| row.board)
}

/// The board a variant name names, with nothing stated beside it.
pub fn board_from_name<Board: Copy>(
    rows: &'static [BoardNames<Board>],
    name: &str,
) -> Option<Board> {
    rows.iter()
        .find(|row| row.name == name)
        .map(|row| row.board)
}

/// The row a board's names live in. Attributes ride the variant's fields, so a
/// board is matched to its row by which variant it is, not by what it carries.
pub fn names<Board>(
    rows: &'static [BoardNames<Board>],
    board: &Board,
) -> &'static BoardNames<Board> {
    let variant = std::mem::discriminant(board);
    rows.iter()
        .find(|row| std::mem::discriminant(&row.board) == variant)
        .expect("every board has a row in the vocabulary")
}

/// The type-erased catalogue a rows table describes.
pub fn catalogue_of<Board>(rows: &'static [BoardNames<Board>]) -> Vec<BoardSpec> {
    rows.iter()
        .map(|row| BoardSpec {
            name: row.name,
            display: row.display,
            attributes: row.attributes,
        })
        .collect()
}

/// What every board enum answers, so a consumer can be written once over any
/// core's vocabulary rather than once per system.
pub trait BoardVocabulary: Sized + Clone {
    /// Every board the core knows, and what each can carry.
    fn catalogue() -> &'static [BoardSpec];
    /// This board and its parts, in the form that crosses generic seams.
    fn to_value(&self) -> BoardValue;
    /// The board a statement names; `Err` carries the vocabulary's refusal.
    fn from_value(value: &BoardValue) -> Result<Self, String>;
    /// The board and its parts, for a reader.
    fn display_name(&self) -> String;
}

/// Bind a core's board enum to its vocabulary rows: `all`, `from_name`, `name`,
/// `catalogue` and the row's own display. The enum derives its own serialised
/// form, so the variant is what a manifest holds; `to_value`, `from_value` and
/// `display_name` read the variant's fields, so a vocabulary states those
/// itself.
#[macro_export]
macro_rules! board_vocabulary {
    ($board:ty, $rows:expr) => {
        impl $board {
            /// Every board the core knows, in the vocabulary's order, with
            /// nothing stated beside it.
            pub fn all() -> impl Iterator<Item = $board> {
                $crate::cartridge::boards($rows)
            }

            /// The board a variant name names, with nothing stated beside it.
            pub fn from_name(name: &str) -> Option<$board> {
                $crate::cartridge::board_from_name($rows, name)
            }

            /// This board's variant name, the string every channel says it by.
            pub fn name(&self) -> &'static str {
                $crate::cartridge::names($rows, self).name
            }

            /// The board's own name for a reader, without its parts.
            pub fn board_display(&self) -> &'static str {
                $crate::cartridge::names($rows, self).display
            }

            /// Every board the core knows, and what each can carry.
            pub fn catalogue() -> &'static [$crate::cartridge::BoardSpec] {
                static CATALOGUE: std::sync::OnceLock<Vec<$crate::cartridge::BoardSpec>> =
                    std::sync::OnceLock::new();
                CATALOGUE.get_or_init(|| $crate::cartridge::catalogue_of($rows))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: &[&str] = &["8K", "32K"];
    const PARTS: &[AttributeSpec] = &[
        AttributeSpec {
            key: "ram",
            label: "RAM",
            kind: AttributeKind::Choice { names: SIZES },
            optional: true,
        },
        AttributeSpec {
            key: "rom",
            label: "ROM",
            kind: AttributeKind::Bytes,
            optional: false,
        },
    ];
    const CATALOGUE: &[BoardSpec] = &[
        BoardSpec {
            name: "Flat",
            display: "Plain ROM",
            attributes: &[],
        },
        BoardSpec {
            name: "Expander",
            display: "Expander",
            attributes: PARTS,
        },
    ];

    fn expander() -> BoardValue {
        BoardValue::new("Expander").with("rom", AttributeValue::Bytes(0x8000))
    }

    fn refusal(value: &BoardValue) -> String {
        value
            .read("Test", CATALOGUE)
            .err()
            .expect("the statement is refused")
    }

    #[test]
    fn a_statement_reads_back_what_it_stated() {
        let value = expander().with_choice("ram", "32K");
        let reader = value.read("Test", CATALOGUE).unwrap();
        assert_eq!(reader.board(), "Expander");
        assert_eq!(reader.bytes("rom"), 0x8000);
        assert_eq!(reader.optional_choice("ram"), Some("32K"));
    }

    #[test]
    fn an_unstated_optional_attribute_is_absent() {
        let value = expander();
        let reader = value.read("Test", CATALOGUE).unwrap();
        assert_eq!(reader.optional_choice("ram"), None);
    }

    #[test]
    fn every_refusal_names_the_vocabulary() {
        let unknown_board = BoardValue::new("Nothing");
        assert!(refusal(&unknown_board).contains("unknown Test board"));

        let unknown_attribute = expander().with_toggle("battery", true);
        assert!(refusal(&unknown_attribute).contains("Test Expander"));

        let wrong_kind = expander().with_toggle("ram", true);
        assert!(refusal(&wrong_kind).contains("is a choice, not a toggle"));

        let outside_the_list = expander().with_choice("ram", "1M");
        assert!(refusal(&outside_the_list).contains("8K, 32K"));

        let missing = BoardValue::new("Expander");
        assert!(refusal(&missing).contains("needs a \"rom\" attribute"));
    }

    #[test]
    fn a_board_with_no_parts_takes_none() {
        assert!(BoardValue::new("Flat").read("Test", CATALOGUE).is_ok());
        let stated = BoardValue::new("Flat").with("rom", AttributeValue::Bytes(8));
        assert!(refusal(&stated).contains("carries no \"rom\" attribute"));
    }
}
