//! An app-owned automation surface: a semantic view of the live UI, and the
//! ability to drive it, published to another process over a Unix socket so an
//! external agent can enumerate controls, activate them, type, and scroll.
//!
//! The pieces mirror the session's attach surface, inverted: [`endpoint`] hosts
//! the socket and speaks newline-delimited JSON-RPC, [`bridge`] carries each
//! call from a socket thread into the app's `update` loop, [`registry`] maps a
//! stable id to a UI role and the message that drives it, and [`tools`] names
//! what a client may ask for.

use std::collections::HashMap;

use iced::widget::Id;
use iced::{Element, Rectangle};

use crate::app::Message;

pub mod bridge;
pub mod capture;
#[cfg(unix)]
pub mod endpoint;
pub mod ids;
pub mod registry;
pub mod tools;
pub mod update;

/// The role a tagged element plays, so a client picks the right verb for it.
/// Some roles are reserved for surfaces not yet tagged (sliders, scrollables).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKind {
    Button,
    Toggle,
    TextInput,
    Slider,
    Scrollable,
    Region,
}

impl UiKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UiKind::Button => "button",
            UiKind::Toggle => "toggle",
            UiKind::TextInput => "text_input",
            UiKind::Slider => "slider",
            UiKind::Scrollable => "scrollable",
            UiKind::Region => "region",
        }
    }
}

/// One node of the semantic UI tree: a stable id, its role, a human label, its
/// on-screen bounds, and whether it currently accepts input.
#[derive(Debug, Clone)]
pub struct UiNode {
    pub id: String,
    pub kind: UiKind,
    pub label: String,
    pub bounds: Rectangle,
    pub enabled: bool,
}

impl UiNode {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "kind": self.kind.as_str(),
            "label": self.label,
            "bounds": {
                "x": self.bounds.x,
                "y": self.bounds.y,
                "width": self.bounds.width,
                "height": self.bounds.height,
            },
            "enabled": self.enabled,
        })
    }
}

/// Wrap `content` in a bounds-reporting container carrying `id`. Upstream iced
/// buttons report as an anonymous container in an [`Operation`], so a button's
/// bounds come from this id'd wrapper; text inputs and scrollables carry their
/// own id instead and need no wrapping.
///
/// [`Operation`]: iced::advanced::widget::Operation
pub(in crate::app) fn tag<'a>(
    id: &str,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    iced::widget::container(content)
        .id(Id::from(id.to_string()))
        .into()
}

/// Walks the widget tree recording the on-screen bounds of every tagged element
/// whose id it is looking for. Missing ids are simply not returned — a control
/// off the current screen has no bounds.
pub struct CollectBounds {
    targets: HashMap<Id, String>,
    found: Vec<(String, Rectangle)>,
}

impl CollectBounds {
    pub fn new(ids: &[String]) -> Self {
        let targets = ids
            .iter()
            .map(|id| (Id::from(id.clone()), id.clone()))
            .collect();
        Self {
            targets,
            found: Vec::new(),
        }
    }

    fn record(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if let Some(id) = id
            && let Some(name) = self.targets.get(id)
        {
            self.found.push((name.clone(), bounds));
        }
    }
}

impl iced::advanced::widget::Operation<Vec<(String, Rectangle)>> for CollectBounds {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(
            &mut dyn iced::advanced::widget::Operation<Vec<(String, Rectangle)>>,
        ),
    ) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.record(id, bounds);
    }

    fn text_input(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _state: &mut dyn iced::advanced::widget::operation::TextInput,
    ) {
        self.record(id, bounds);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        _translation: iced::Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        self.record(id, bounds);
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<Vec<(String, Rectangle)>> {
        iced::advanced::widget::operation::Outcome::Some(self.found.clone())
    }
}

/// The app-side messages the automation surface raises through the main
/// `update` loop.
#[derive(Debug, Clone)]
pub enum Msg {
    /// An item from the app-lifetime bridge subscription: the call sink handed
    /// over once at startup, then each call forwarded through it.
    Bridge(bridge::AutomationBridge),
    /// A `ui_tree` bounds walk finished; join the bounds with the parked
    /// request's described nodes and answer it.
    TreeCollected {
        request: u64,
        pairs: Vec<(String, Rectangle)>,
    },
    /// A resize leg's fallback timer fired: settle the parked request with the
    /// window's actual size, in case the WM clamped it and no `Resized` came.
    ResizeTimeout { request: u64 },
    /// The post-resize redraw settle elapsed for a screenshot; capture now.
    ShotSettled { request: u64 },
    /// An `element_id` screenshot's bounds walk finished; crop to it and capture.
    ShotBounds {
        request: u64,
        pairs: Vec<(String, Rectangle)>,
    },
    /// The window screenshot for a parked request arrived; crop, encode, answer.
    ShotCaptured {
        request: u64,
        shot: iced::window::Screenshot,
    },
    /// The window's current scale factor, cached for `status`/`ui_tree`.
    ScaleCached(f32),
}
