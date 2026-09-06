//! What a core lets a frontend decide about a ROM before it boots.
//!
//! A core *states* the options it accepts — the broadcast standard to decode
//! for, the cartridge board a headerless dump sits on, the firmware to map —
//! and a frontend collects values for them however it likes: a command-line
//! flag, a catalogue entry, a dialog. Nothing here knows any console: the
//! options are named by the core that publishes them, and travel as a sparse
//! bag of the values a caller set.

use std::collections::{BTreeMap, BTreeSet};

use crate::cartridge::{BoardSpec, BoardValue};
use crate::firmware::FirmwareSlot;

/// One option a core accepts at launch.
#[derive(Clone)]
pub struct LaunchOptionDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: LaunchOptionKind,
}

/// What kind of value an option takes. Absence of a value always means
/// automatic: the core resolves it from the media, a header, or an inference.
#[derive(Clone)]
pub enum LaunchOptionKind {
    Choice {
        choices: Vec<LaunchChoice>,
    },
    /// Any number of `flags` at once, each independent of the rest.
    Flags {
        flags: Vec<LaunchChoice>,
    },
    Toggle,
    /// A file's contents, `label` naming what to pick.
    File {
        label: &'static str,
    },
    /// A board from `boards`, and the parts its silicon varies in.
    Board {
        boards: Vec<BoardSpec>,
    },
    /// An image for one of the board's firmware sockets. A caller states which
    /// by id (`LaunchValue::Choice`) and a frontend supplies the bytes; the
    /// core reads only `LaunchValue::File`.
    Firmware {
        slot: FirmwareSlot,
    },
}

/// One value a [`LaunchOptionKind::Choice`] accepts, or one flag of a
/// [`LaunchOptionKind::Flags`] set, and how to show it.
#[derive(Clone)]
pub struct LaunchChoice {
    pub value: &'static str,
    pub label: &'static str,
}

/// The cartridge-board option, as every core with a board vocabulary publishes
/// it. The caller supplies the boards, so it decides which of its catalogue a
/// frontend may state.
pub fn board_option(
    id: &'static str,
    boards: impl Iterator<Item = BoardSpec>,
) -> LaunchOptionDescriptor {
    LaunchOptionDescriptor {
        id,
        label: "Cartridge board",
        kind: LaunchOptionKind::Board {
            boards: boards.collect(),
        },
    }
}

/// A value a caller set for one option.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchValue {
    Choice(String),
    /// The flags of a set that are on; an empty set is all of them off.
    Flags(BTreeSet<String>),
    Toggle(bool),
    File(Vec<u8>),
    /// A whole board and the parts populated on it, as a catalogue states them.
    Board(BoardValue),
}

/// The options a caller explicitly set, keyed by descriptor id. Sparse: an
/// absent option is one the caller left to the core.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaunchValues(BTreeMap<String, LaunchValue>);

impl LaunchValues {
    /// The chosen value for `id`, or `None` where the caller set nothing.
    pub fn choice(&self, id: &str) -> Option<&str> {
        match self.0.get(id) {
            Some(LaunchValue::Choice(value)) => Some(value),
            _ => None,
        }
    }

    /// The flags set for `id`, or `None` where the caller set none — which is
    /// not the same as setting none of them.
    pub fn flags(&self, id: &str) -> Option<&BTreeSet<String>> {
        match self.0.get(id) {
            Some(LaunchValue::Flags(flags)) => Some(flags),
            _ => None,
        }
    }

    /// Whether the toggle is set; an absent toggle is off.
    pub fn toggle(&self, id: &str) -> bool {
        matches!(self.0.get(id), Some(LaunchValue::Toggle(true)))
    }

    /// The board stated for `id`, or `None` where the caller stated none.
    pub fn board(&self, id: &str) -> Option<&BoardValue> {
        match self.0.get(id) {
            Some(LaunchValue::Board(board)) => Some(board),
            _ => None,
        }
    }

    /// The file contents supplied for `id`, or `None` where the caller set none.
    pub fn file(&self, id: &str) -> Option<&[u8]> {
        match self.0.get(id) {
            Some(LaunchValue::File(bytes)) => Some(bytes),
            _ => None,
        }
    }

    /// Whatever the caller set for `id`, whichever kind it is.
    pub fn value(&self, id: &str) -> Option<&LaunchValue> {
        self.0.get(id)
    }

    pub fn set(&mut self, id: impl Into<String>, value: LaunchValue) {
        self.0.insert(id.into(), value);
    }

    /// Leave `id` to the core again.
    pub fn clear(&mut self, id: &str) {
        self.0.remove(id);
    }

    pub fn set_choice(&mut self, id: impl Into<String>, value: impl Into<String>) {
        self.0.insert(id.into(), LaunchValue::Choice(value.into()));
    }

    pub fn set_flags(&mut self, id: impl Into<String>, flags: BTreeSet<String>) {
        self.0.insert(id.into(), LaunchValue::Flags(flags));
    }

    pub fn set_toggle(&mut self, id: impl Into<String>, value: bool) {
        self.0.insert(id.into(), LaunchValue::Toggle(value));
    }

    pub fn set_file(&mut self, id: impl Into<String>, contents: Vec<u8>) {
        self.0.insert(id.into(), LaunchValue::File(contents));
    }

    pub fn set_board(&mut self, id: impl Into<String>, board: BoardValue) {
        self.0.insert(id.into(), LaunchValue::Board(board));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every option a caller set, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &LaunchValue)> {
        self.0.iter().map(|(id, value)| (id.as_str(), value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_option_reads_as_absent() {
        let values = LaunchValues::default();
        assert_eq!(values.choice("tv-standard"), None);
        assert_eq!(values.file("boot-rom"), None);
        assert!(!values.toggle("overdump"));
    }

    #[test]
    fn a_value_reads_back_only_as_the_kind_it_was_set_as() {
        let mut values = LaunchValues::default();
        values.set_choice("board", "F8");
        values.set_toggle("overdump", true);
        values.set_file("boot-rom", vec![0x31, 0xFE]);
        values.set_board("stated", BoardValue::new("Plain4K"));
        assert_eq!(values.choice("board"), Some("F8"));
        assert!(values.toggle("overdump"));
        assert_eq!(values.file("boot-rom"), Some([0x31, 0xFE].as_slice()));
        assert_eq!(
            values.board("stated").map(|b| b.board.as_str()),
            Some("Plain4K")
        );
        assert_eq!(values.choice("overdump"), None);
        assert_eq!(values.file("board"), None);
        assert_eq!(values.board("board"), None);
    }

    #[test]
    fn a_flag_set_reads_back_as_the_flags_it_was_set_with() {
        let mut values = LaunchValues::default();
        values.set_flags("enhancements", BTreeSet::new());
        assert_eq!(values.flags("enhancements"), Some(&BTreeSet::new()));
        assert_eq!(values.choice("enhancements"), None);
        assert!(!values.toggle("enhancements"));

        values.set_flags("enhancements", flags(["sgb", "cgb"]));
        assert_eq!(values.flags("enhancements"), Some(&flags(["cgb", "sgb"])));
        assert_eq!(values.flags("board"), None);
    }

    /// Overrides are kept in a game's `game.ron`, so a set has to survive the
    /// round trip in the order it will be written back in.
    #[test]
    fn a_flag_set_round_trips_through_ron() {
        let mut values = LaunchValues::default();
        values.set_flags("enhancements", flags(["sgb", "cgb"]));
        let written = ron::to_string(&values).expect("the values write");
        let read: LaunchValues = ron::from_str(&written).expect("the values read back");
        assert_eq!(read, values);
        assert!(written.contains("[\"cgb\",\"sgb\"]"), "{written}");
    }

    fn flags<const N: usize>(names: [&str; N]) -> BTreeSet<String> {
        names.into_iter().map(str::to_owned).collect()
    }
}
