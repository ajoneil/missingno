//! The automation tools the app advertises and answers. Definitions only — the
//! bodies run on the UI thread in [`super::update`], since they read and drive
//! the live widget tree. The [`Tool`] vocabulary is the session's, so a client
//! sees the same tool shape it sees for a session.

use missingno_session::tools::Tool;
use serde_json::{Value, json};

fn empty() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn id_arg() -> Value {
    json!({ "type": "string", "description": "a stable ui node id, from ui_tree" })
}

/// The tools this app serves, in the order a `tools/list` lists them.
pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "status",
            description: "The app's current screen, window size and scale, and whether a \
                          game is loaded and running. UI coverage is partial."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "ui_tree",
            description: "The semantic UI tree of the current screen as JSON: the screen, the \
                          window size, and every on-screen node with its id, role, label, \
                          bounds, and whether it is enabled."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "activate",
            description: "Activate a node by id — press a button, toggle a toggle, select a \
                          game or a settings section."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": id_arg() },
                "required": ["id"],
            }),
        },
        Tool {
            name: "set_text",
            description: "Focus a text input by id and set its text (e.g. the library search \
                          field)."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": id_arg(),
                    "text": { "type": "string", "description": "the text to set" },
                },
                "required": ["id", "text"],
            }),
        },
        Tool {
            name: "scroll_to",
            description: "Scroll a scrollable node by id to an absolute offset (x, y in logical \
                          pixels, default 0)."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": id_arg(),
                    "x": { "type": "number", "minimum": 0.0 },
                    "y": { "type": "number", "minimum": 0.0 },
                },
                "required": ["id"],
            }),
        },
        Tool {
            name: "resize_window",
            description: "Resize the window to a logical width and height. Sizes below the app's \
                          minimum are allowed for the duration; the reply reports the actual size \
                          the window manager granted."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "width": { "type": "number", "minimum": 1.0, "description": "logical pixels" },
                    "height": { "type": "number", "minimum": 1.0, "description": "logical pixels" },
                },
                "required": ["width", "height"],
                "additionalProperties": false,
            }),
        },
        Tool {
            name: "screenshot",
            description: "Capture the window as a PNG. With no arguments, captures the whole \
                          window at its current size. Optionally resize first (width/height, \
                          logical px), and crop to either a region or a single element — not \
                          both. With a path, also writes the PNG to that file (its parent \
                          directory must exist)."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "width": { "type": "number", "minimum": 1.0, "description": "resize to this logical width first" },
                    "height": { "type": "number", "minimum": 1.0, "description": "resize to this logical height first" },
                    "element_id": {
                        "type": "string",
                        "description": "crop to this ui node's bounds (from ui_tree)",
                    },
                    "region": {
                        "type": "object",
                        "description": "crop to this logical-pixel rect within the window",
                        "properties": {
                            "x": { "type": "number", "minimum": 0.0 },
                            "y": { "type": "number", "minimum": 0.0 },
                            "width": { "type": "number", "minimum": 1.0 },
                            "height": { "type": "number", "minimum": 1.0 },
                        },
                        "required": ["width", "height"],
                    },
                    "path": {
                        "type": "string",
                        "description": "write the PNG here too (absolute or cwd-relative)",
                    },
                },
                "additionalProperties": false,
            }),
        },
    ]
}

/// [`tools`] as the `tools/list` result body.
pub fn tools_json() -> Value {
    json!({ "tools": tools().iter().map(Tool::to_json).collect::<Vec<_>>() })
}
