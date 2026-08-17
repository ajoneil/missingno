//! What a core lets a frontend decide about a ROM before it boots.
//!
//! A core *states* the options it accepts — the broadcast standard to decode
//! for, the cartridge board a headerless dump sits on, the boot ROM to map —
//! and a frontend collects values for them however it likes: a command-line
//! flag, a catalogue entry, a dialog. Nothing here knows any console: the
//! options are named by the core that publishes them, and travel as a sparse
//! bag of the values a caller set.

use std::collections::BTreeMap;

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
    Toggle,
    /// A file's contents, `label` naming what to pick.
    File {
        label: &'static str,
    },
}

/// One value a [`LaunchOptionKind::Choice`] accepts, and how to show it.
#[derive(Clone)]
pub struct LaunchChoice {
    pub value: &'static str,
    pub label: &'static str,
}

/// The cartridge-board option, as every core with a board vocabulary publishes
/// it. The caller supplies the choices, so it decides which of its boards a
/// frontend may state.
pub fn board_option(
    id: &'static str,
    choices: impl Iterator<Item = LaunchChoice>,
) -> LaunchOptionDescriptor {
    LaunchOptionDescriptor {
        id,
        label: "Cartridge board",
        kind: LaunchOptionKind::Choice {
            choices: choices.collect(),
        },
    }
}

/// A value a caller set for one option.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchValue {
    Choice(String),
    Toggle(bool),
    File(Vec<u8>),
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

    /// Whether the toggle is set; an absent toggle is off.
    pub fn toggle(&self, id: &str) -> bool {
        matches!(self.0.get(id), Some(LaunchValue::Toggle(true)))
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

    pub fn set_toggle(&mut self, id: impl Into<String>, value: bool) {
        self.0.insert(id.into(), LaunchValue::Toggle(value));
    }

    pub fn set_file(&mut self, id: impl Into<String>, contents: Vec<u8>) {
        self.0.insert(id.into(), LaunchValue::File(contents));
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
        assert_eq!(values.choice("board"), Some("F8"));
        assert!(values.toggle("overdump"));
        assert_eq!(values.file("boot-rom"), Some([0x31, 0xFE].as_slice()));
        assert_eq!(values.choice("overdump"), None);
        assert_eq!(values.file("board"), None);
    }
}
