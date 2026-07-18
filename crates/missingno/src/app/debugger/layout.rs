//! Saved debugger pane layout: which panes are open and how they're split.
//!
//! Panes are referred to by their registry label, so a saved layout survives
//! new panes being added; a layout naming an unknown pane is discarded whole
//! rather than restored lopsided.

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
    Pane(String),
}

fn layout_path(key: &str) -> Option<PathBuf> {
    let file = if key.is_empty() {
        "debugger_layout.ron".to_string()
    } else {
        format!("debugger_layout_{key}.ron")
    };
    dirs::config_dir().map(|dir| dir.join("missingno").join(file))
}

pub fn load(key: &str) -> Option<SavedPanes> {
    let path = layout_path(key)?;
    let data = fs::read_to_string(path).ok()?;
    ron::from_str(&data).ok()
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
                Some(Self::Pane(instance.kind().descriptor().label.to_string()))
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
            Self::Pane(label) => {
                let descriptor = all_descriptors().find(|descriptor| descriptor.label == label)?;
                Some(pane_grid::Configuration::Pane((descriptor.construct)()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::debugger::panes::DebuggerPane;
    use missingno_gb::ppu::types::tiles::TileMapId;

    fn construct(kind: DebuggerPane) -> Box<dyn Pane> {
        (kind.descriptor().construct)()
    }

    #[test]
    fn layout_round_trips_through_ron() {
        let (mut state, first) = pane_grid::State::new(construct(DebuggerPane::Disassembly));
        let (screen, split) = state
            .split(
                pane_grid::Axis::Vertical,
                first,
                construct(DebuggerPane::Screen),
            )
            .unwrap();
        state.resize(split, 0.25);
        state
            .split(
                pane_grid::Axis::Horizontal,
                screen,
                construct(DebuggerPane::TileMap(TileMapId(1))),
            )
            .unwrap();

        let saved = SavedLayout::capture(state.layout(), &state).unwrap();
        let serialized = ron::to_string(&saved).unwrap();

        let restored: SavedLayout = ron::from_str(&serialized).unwrap();
        let restored_state =
            pane_grid::State::with_configuration(restored.into_configuration().unwrap());
        let recaptured = SavedLayout::capture(restored_state.layout(), &restored_state).unwrap();

        assert_eq!(serialized, ron::to_string(&recaptured).unwrap());
    }

    #[test]
    fn unknown_pane_label_discards_layout() {
        let saved = SavedLayout::Pane("Not A Pane".to_string());
        assert!(saved.into_configuration().is_none());
    }
}
