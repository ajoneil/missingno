//! Saved debugger pane layout: which panes are open, how they're split, and
//! each instanceable pane's source selection.
//!
//! Panes are referred to by their registry label plus an optional source index
//! (an atlas, a map, or a memory region) and scroll offset, so a saved layout
//! survives new panes being added and restores each instance to what it showed.
//! A layout naming an unknown pane is discarded whole rather than restored
//! lopsided. A pre-instance layout — label-only, and naming the two separate
//! "Tile Map 0"/"Tile Map 1" panes — migrates onto the collapsed kind rather
//! than tripping the discard.

use std::fs;
use std::path::PathBuf;

use iced::widget::pane_grid;
use serde::{Deserialize, Serialize};

use super::panes::{Pane, all_descriptors};

#[derive(Serialize, Deserialize)]
pub struct SavedPanes(Option<SavedLayout>);

#[derive(Serialize, Deserialize)]
enum SavedLayout {
    Split {
        vertical: bool,
        ratio: f32,
        a: Box<SavedLayout>,
        b: Box<SavedLayout>,
    },
    Pane(SavedPane),
}

/// One persisted pane: its registry label plus, for an instanceable pane, the
/// source it showed and the memory viewer's scroll offset.
#[derive(Serialize, Deserialize)]
struct SavedPane {
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u32>,
}

/// The pre-instance layout schema: label-only panes. Parsed only as a fallback,
/// so a layout saved before instances migrates instead of being discarded.
#[derive(Deserialize)]
struct LegacySavedPanes(Option<LegacyLayout>);

#[derive(Deserialize)]
enum LegacyLayout {
    Split {
        vertical: bool,
        ratio: f32,
        a: Box<LegacyLayout>,
        b: Box<LegacyLayout>,
    },
    Pane(String),
}

impl From<LegacyLayout> for SavedLayout {
    fn from(legacy: LegacyLayout) -> Self {
        match legacy {
            LegacyLayout::Split {
                vertical,
                ratio,
                a,
                b,
            } => Self::Split {
                vertical,
                ratio,
                a: Box::new((*a).into()),
                b: Box::new((*b).into()),
            },
            LegacyLayout::Pane(label) => Self::Pane(SavedPane {
                label,
                source: None,
                offset: None,
            }),
        }
    }
}

impl From<LegacySavedPanes> for SavedPanes {
    fn from(legacy: LegacySavedPanes) -> Self {
        SavedPanes(legacy.0.map(Into::into))
    }
}

fn layout_path(key: &str) -> Option<PathBuf> {
    let file = if key.is_empty() {
        "debugger_layout.ron".to_string()
    } else {
        format!("debugger_layout_{key}.ron")
    };
    dirs::config_dir().map(|dir| dir.join("missingno").join(file))
}

/// Parse a saved layout, trying the current instance-aware schema first and
/// falling back to the label-only schema a pre-instance layout was saved under.
fn parse(data: &str) -> Option<SavedPanes> {
    ron::from_str::<SavedPanes>(data)
        .ok()
        .or_else(|| ron::from_str::<LegacySavedPanes>(data).ok().map(Into::into))
}

pub fn load(key: &str) -> Option<SavedPanes> {
    let path = layout_path(key)?;
    let data = fs::read_to_string(path).ok()?;
    parse(&data)
}

pub fn save(key: &str, state: Option<&pane_grid::State<Box<dyn Pane>>>) {
    let Some(path) = layout_path(key) else {
        return;
    };
    let saved = SavedPanes(state.and_then(|state| SavedLayout::capture(state.layout(), state)));
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(data) = ron::ser::to_string_pretty(&saved, ron::ser::PrettyConfig::default()) {
        let _ = fs::write(path, data);
    }
}

impl SavedPanes {
    pub fn into_state(self) -> Option<Option<pane_grid::State<Box<dyn Pane>>>> {
        match self.0 {
            None => Some(None),
            Some(layout) => {
                let config = layout.into_configuration()?;
                Some(Some(pane_grid::State::with_configuration(config)))
            }
        }
    }
}

impl SavedLayout {
    fn capture(node: &pane_grid::Node, state: &pane_grid::State<Box<dyn Pane>>) -> Option<Self> {
        match node {
            pane_grid::Node::Split {
                axis, ratio, a, b, ..
            } => Some(Self::Split {
                vertical: *axis == pane_grid::Axis::Vertical,
                ratio: *ratio,
                a: Box::new(Self::capture(a, state)?),
                b: Box::new(Self::capture(b, state)?),
            }),
            pane_grid::Node::Pane(pane) => {
                let instance = state.get(*pane)?;
                Some(Self::Pane(SavedPane {
                    label: instance.kind().descriptor().label.to_string(),
                    source: instance.source_index().map(|index| index as u32),
                    offset: instance.source_offset(),
                }))
            }
        }
    }

    fn into_configuration(self) -> Option<pane_grid::Configuration<Box<dyn Pane>>> {
        match self {
            Self::Split {
                vertical,
                ratio,
                a,
                b,
            } => Some(pane_grid::Configuration::Split {
                axis: if vertical {
                    pane_grid::Axis::Vertical
                } else {
                    pane_grid::Axis::Horizontal
                },
                ratio,
                a: Box::new(a.into_configuration()?),
                b: Box::new(b.into_configuration()?),
            }),
            Self::Pane(saved) => Some(pane_grid::Configuration::Pane(saved.build()?)),
        }
    }
}

impl SavedPane {
    /// Rebuild the pane, migrating the retired two-kind tile-map labels onto the
    /// collapsed kind and restoring its source and scroll offset.
    fn build(self) -> Option<Box<dyn Pane>> {
        // The pre-instance layout named a map per pane; the collapsed kind takes
        // that map as its source.
        let (label, source) = match self.label.as_str() {
            "Tile Map 0" => ("Tile Map", self.source.or(Some(0))),
            "Tile Map 1" => ("Tile Map", self.source.or(Some(1))),
            other => (other, self.source),
        };
        let descriptor = all_descriptors().find(|descriptor| descriptor.label == label)?;
        let mut pane = (descriptor.construct)();
        if let Some(index) = source {
            pane.set_source_index(index as usize);
        }
        if let Some(offset) = self.offset {
            pane.set_source_offset(offset);
        }
        Some(pane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::debugger::panes::DebuggerPane;

    fn construct(kind: DebuggerPane) -> Box<dyn Pane> {
        (kind.descriptor().construct)()
    }

    /// The kinds and sources of every pane in a captured layout, left to right.
    fn panes_of(state: &pane_grid::State<Box<dyn Pane>>) -> Vec<(DebuggerPane, Option<usize>)> {
        state
            .iter()
            .map(|(_, pane)| (pane.kind(), pane.source_index()))
            .collect()
    }

    #[test]
    fn layout_round_trips_with_instance_selections() {
        // Two tile maps on different maps plus a memory pane, exercising source
        // and offset persistence.
        let (mut state, first) = pane_grid::State::new(construct(DebuggerPane::Disassembly));
        let mut map1 = construct(DebuggerPane::TileMap);
        map1.set_source_index(1);
        let (map_handle, split) = state.split(pane_grid::Axis::Vertical, first, map1).unwrap();
        state.resize(split, 0.25);
        let mut memory = construct(DebuggerPane::Memory);
        memory.set_source_index(1);
        memory.set_source_offset(0x20);
        state
            .split(pane_grid::Axis::Horizontal, map_handle, memory)
            .unwrap();

        let saved = SavedLayout::capture(state.layout(), &state).unwrap();
        let serialized = ron::to_string(&saved).unwrap();
        let restored: SavedLayout = ron::from_str(&serialized).unwrap();
        let restored_state =
            pane_grid::State::with_configuration(restored.into_configuration().unwrap());

        // The tile map's map and the memory pane's region survive the trip.
        let panes = panes_of(&restored_state);
        assert!(panes.contains(&(DebuggerPane::TileMap, Some(1))));
        assert!(panes.contains(&(DebuggerPane::Memory, Some(1))));
        // And the serialized form is stable across a full round trip.
        let recaptured = SavedLayout::capture(restored_state.layout(), &restored_state).unwrap();
        assert_eq!(serialized, ron::to_string(&recaptured).unwrap());
    }

    #[test]
    fn legacy_two_kind_tile_maps_migrate_to_one_kind() {
        // A pre-instance layout naming both separate tile-map panes must load as
        // two instances of the collapsed kind — not trip the discard.
        let legacy =
            r#"(Some(Split(vertical:false,ratio:0.5,a:Pane("Tile Map 0"),b:Pane("Tile Map 1"))))"#;
        let parsed = parse(legacy).expect("legacy layout parses via the fallback schema");
        let state = parsed
            .into_state()
            .expect("known labels are not discarded")
            .expect("layout is non-empty");
        let mut panes = panes_of(&state);
        panes.sort_by_key(|(_, source)| *source);
        assert_eq!(
            panes,
            vec![
                (DebuggerPane::TileMap, Some(0)),
                (DebuggerPane::TileMap, Some(1)),
            ]
        );
    }

    #[test]
    fn legacy_single_panes_load_with_default_selection() {
        // A label-only "Tiles"/"Memory" loads as instance 0 of that kind.
        let legacy = r#"(Some(Split(vertical:false,ratio:0.5,a:Pane("Tiles"),b:Pane("Memory"))))"#;
        let state = parse(legacy).unwrap().into_state().unwrap().unwrap();
        let panes = panes_of(&state);
        assert!(panes.contains(&(DebuggerPane::Tiles, Some(0))));
        assert!(panes.contains(&(DebuggerPane::Memory, Some(0))));
    }

    #[test]
    fn unknown_pane_label_discards_layout() {
        let saved = SavedLayout::Pane(SavedPane {
            label: "Not A Pane".to_string(),
            source: None,
            offset: None,
        });
        assert!(saved.into_configuration().is_none());
        // And a whole file naming it discards rather than restoring lopsided.
        let data = r#"(Some(Pane(SavedPane(label:"Not A Pane"))))"#;
        assert!(parse(data).unwrap().into_state().is_none());
    }
}
