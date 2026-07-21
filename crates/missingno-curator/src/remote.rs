//! `ui-<pid>.sock` endpoint speaking the same newline JSON-RPC protocol as the
//! emulator's UI-automation surface, so `missingno-remote` discovers the
//! curator and forwards its tools over MCP without any server changes.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use iced::futures::{SinkExt, StreamExt, channel::mpsc::UnboundedSender};
use serde_json::{Value, json};

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// One tools/call in flight from a socket client to the UI thread.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
    pub reply: mpsc::Sender<Value>,
}

/// The socket threads' handle on the UI sink; the sink arrives after startup.
#[derive(Clone, Default)]
pub struct SharedSink(Arc<Mutex<Option<UnboundedSender<ToolCall>>>>);

impl SharedSink {
    pub fn set(&self, sink: UnboundedSender<ToolCall>) {
        *self.0.lock().unwrap() = Some(sink);
    }

    fn get(&self) -> Option<UnboundedSender<ToolCall>> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone)]
pub enum Bridge {
    Ready(UnboundedSender<ToolCall>),
    Call(ToolCall),
}

/// Subscription worker: hands the UI a sink, then streams tool calls.
pub fn worker() -> impl iced::futures::Stream<Item = Bridge> {
    iced::stream::channel(16, async move |mut output| {
        let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
        let _ = output.send(Bridge::Ready(tx)).await;
        while let Some(call) = rx.next().await {
            let _ = output.send(Bridge::Call(call)).await;
        }
    })
}

pub struct RemoteEndpoint {
    path: PathBuf,
}

impl RemoteEndpoint {
    pub fn open(sink: SharedSink) -> std::io::Result<Self> {
        let dir = missingno_session::attach::runtime_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("ui-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::thread::Builder::new()
            .name("curator-remote".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let sink = sink.clone();
                    std::thread::spawn(move || serve(stream, sink));
                }
            })?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for RemoteEndpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(stream: UnixStream, sink: SharedSink) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut writer = write_half;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = respond(
                    &mut writer,
                    error_frame(Value::Null, &format!("bad json: {e}")),
                );
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let frame = match method {
            "ui/info" => success_frame(
                id,
                json!({
                    "app": "net.andyofniall.missingno-curator",
                    "pid": std::process::id(),
                    "version": env!("CARGO_PKG_VERSION"),
                }),
            ),
            "tools/list" => success_frame(id, json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match dispatch(&sink, name, args) {
                    Ok(body) => success_frame(id, body),
                    Err(message) => error_frame(id, &message),
                }
            }
            other => error_frame(id, &format!("method not found: {other}")),
        };
        if respond(&mut writer, frame).is_err() {
            break;
        }
    }
}

fn dispatch(sink: &SharedSink, name: &str, args: Value) -> Result<Value, String> {
    let sink = sink.get().ok_or("curator UI not ready")?;
    let (reply, answer) = mpsc::channel();
    sink.unbounded_send(ToolCall {
        name: name.to_owned(),
        args,
        reply,
    })
    .map_err(|_| "curator UI gone")?;
    answer
        .recv_timeout(REPLY_TIMEOUT)
        .map_err(|_| "curator UI did not answer in time".to_owned())
}

fn respond(writer: &mut UnixStream, frame: Value) -> std::io::Result<()> {
    writeln!(writer, "{frame}")
}

fn success_frame(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_frame(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })
}

pub fn text_result(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": false })
}

pub fn error_result(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": true })
}

fn tool_definitions() -> Value {
    let object = |properties: Value, required: &[&str]| json!({ "type": "object", "properties": properties, "required": required });
    json!([
        {
            "name": "status",
            "description": "Curation queue counts: per-platform backlog, open flags, uncommitted files.",
            "inputSchema": object(json!({}), &[]),
        },
        {
            "name": "search_games",
            "description": "Search the game database by title or slug. Returns tree/slug keys.",
            "inputSchema": object(json!({
                "query": { "type": "string" },
                "tree": { "type": "string", "enum": ["gb", "gbc", "vcs"] },
                "backlog_only": { "type": "boolean" },
                "limit": { "type": "integer" },
            }), &["query"]),
        },
        {
            "name": "get_game",
            "description": "Full manifest (RON) and open flags for one game, by tree/slug key.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "update_game",
            "description": "Stage edits to a game. Text fields plus cover image URLs (remote links only — Hasheous, the project's own repo/pouet page, libretro-thumbnails, or Wikimedia; never store CDNs) and a Wikipedia article link. Edits appear live in the curator UI as uncommitted changes; a curated stamp is cleared.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "set": { "type": "object", "properties": {
                    "title": { "type": "string" },
                    "developer": { "type": "string" },
                    "description": { "type": "string" },
                    "license": { "type": "string" },
                    "covers": { "type": "array", "items": { "type": "string" },
                                "description": "remote image URLs, preference order" },
                    "wikipedia": { "type": "string", "description": "article URL" },
                    "mapper": { "type": "string",
                                "description": "GB/GBC cartridge mapper override (first release), e.g. \"MBC5+RUMBLE\" — when the header lies" },
                    "cart_type": { "type": "string",
                                   "description": "VCS board override (first release), e.g. \"F6SC\" — playtests boot with it; change it if the game boots wrong" },
                }},
            }), &["key", "set"]),
        },
        {
            "name": "queue_games",
            "description": "Set the curation queue (ordered tree/slug keys). The first game auto-downloads (or uses a local dump) and starts playing for the human to playtest; enrich it while they play. When they Accept, the next queued game starts — poll queue_status to follow along.",
            "inputSchema": object(json!({
                "keys": { "type": "array", "items": { "type": "string" } },
            }), &["keys"]),
        },
        {
            "name": "play_game",
            "description": "Fetch (if needed) and start a live playtest of one game in the curator.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "local_matches",
            "description": "Games whose ROM dumps hash-match the human's scanned local collection — ideal input for queue_games.",
            "inputSchema": object(json!({
                "backlog_only": { "type": "boolean" },
                "limit": { "type": "integer" },
            }), &[]),
        },
        {
            "name": "find_duplicates",
            "description": "Entries whose normalized title (or any localized release title) collides with this game's — merge candidates. Run this for every game you curate; duplicates hide under punctuation, articles, and localized names.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "queue_status",
            "description": "Current playtest game and remaining queue.",
            "inputSchema": object(json!({}), &[]),
        },
        {
            "name": "set_note",
            "description": "Show the human your reasoning: what you changed, which sources you used, and anything they should double-check. Displayed in the editor next to the Accept button.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "note": { "type": "string" },
            }), &["key", "note"]),
        },
        {
            "name": "select_game",
            "description": "Navigate the curator UI to a game so the human sees what you're working on.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "list_flags",
            "description": "Open curation flags (importer questions awaiting a decision).",
            "inputSchema": object(json!({
                "kind": { "type": "string" },
                "key": { "type": "string", "description": "only flags about this tree/slug" },
                "limit": { "type": "integer" },
            }), &[]),
        },
        {
            "name": "resolve_flag",
            "description": "Mark a curation flag resolved (the data now reflects the decision).",
            "inputSchema": object(json!({ "id": { "type": "integer" } }), &["id"]),
        },
    ])
}
