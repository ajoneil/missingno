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
    // The long-poll parks its reply until the human acts; everything else
    // answers promptly or is stuck.
    let timeout = if matches!(name, "wait_for_action" | "verify_artifacts") {
        Duration::from_secs(55)
    } else {
        REPLY_TIMEOUT
    };
    answer.recv_timeout(timeout).map_err(|_| {
        if name == "wait_for_action" {
            "no action yet — call wait_for_action again".to_owned()
        } else {
            "curator UI did not answer in time".to_owned()
        }
    })
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
            "description": "Stage edits to a game. Text fields plus cover image URLs (remote links only — Hasheous, the project's own repo/pouet page, libretro-thumbnails, or Wikimedia; never store CDNs) and a Wikipedia article link. Setting `wikipedia` creates the game's \"Wikipedia\" link by itself — do not also pass one in `links`, or the article ends up listed twice. `remove_links` drops links by name. Edits appear live in the curator UI as uncommitted changes; a curated stamp is cleared.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "set": { "type": "object", "properties": {
                    "title": { "type": "string" },
                    "developer": { "type": "string" },
                    "description": { "type": "string" },
                    "license": { "type": "string" },
                    "publisher": { "type": "string",
                                   "description": "publisher of the first release (release-level; UI edits others)" },
                    "covers": { "type": "array", "items": { "type": "string" },
                                "description": "remote image URLs, preference order" },
                    "wikipedia": { "type": "string", "description": "article URL" },
                    "links": { "type": "array",
                               "description": "durable source receipts: record the page that backed each staged fact (AtariAge thread, author's site, pouet prod, MobyGames…). Upserts by name — re-staging the same source never duplicates. This, not set_note, is where sources survive: notes die with the session, links live in the manifest.",
                               "items": { "type": "object", "properties": {
                                   "name": { "type": "string" },
                                   "url": { "type": "string" },
                                   "link_type": { "type": "string",
                                                  "enum": ["Wiki", "Manual", "Source", "Speedrun", "UnusedContent", "TechnicalReference", "Guide", "Community"] },
                               }, "required": ["name", "url", "link_type"] } },
                    "remove_links": { "type": "array", "items": { "type": "string" },
                                      "description": "link names to drop, for clearing duplicates or a link that turned out to be wrong" },
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
            "name": "mark_hack",
            "description": "A hash in an entry turned out to be a hacked/modified dump. Everything except a total conversion becomes a mod ATTACHED to the same game — its own name, homepage link, versions and independent curation; a fan translation is still the same game, exactly as official localizations are releases of it. Only TotalConversion splits into its own derived-work entry. Supply title (the mod's real name) and url (its homepage) whenever known.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "title": { "type": "string" },
                "category": { "type": "string",
                              "enum": ["Translation", "QualityOfLife", "ContentChange", "TotalConversion"] },
                "base_sha1": { "type": "string" },
                "url": { "type": "string", "description": "the mod's homepage" },
            }), &["key", "sha1"]),
        },
        {
            "name": "update_mod",
            "description": "Correct or enrich an attached mod's recorded fields: rename it, fix its category, author, homepage url, or a release's base_sha1 ('none' to clear), label, or date. Identify the mod by its current name.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "mod": { "type": "string", "description": "the mod's current name" },
                "set": { "type": "object", "properties": {
                    "name": { "type": "string" },
                    "category": { "type": "string",
                                  "enum": ["Translation", "QualityOfLife", "ContentChange", "TotalConversion"] },
                    "author": { "type": "string" },
                    "url": { "type": "string" },
                    "release_index": { "type": "integer" },
                    "base_sha1": { "type": "string" },
                    "label": { "type": "string" },
                    "date": { "type": "string" },
                }},
            }), &["key", "mod", "set"]),
        },
        {
            "name": "split_release",
            "description": "An artifact that is really its own release — a (Prototype) or (Beta) build sitting in the retail release: move it into a new release with the given status, inheriting hardware and publisher but not the retail date. Keep a working title (e.g. 'Jungle Runner') via title.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "status": { "type": "string",
                            "enum": ["Released", "WorkInProgress", "Beta", "Prototype"] },
                "title": { "type": "string" },
                "label": { "type": "string" },
                "date": { "type": "string" },
            }), &["key", "sha1", "status"]),
        },
        {
            "name": "update_release",
            "description": "Set fields on an existing release: status, title, label, date, publisher, regions, controllers (VCS). `title` is the name this release shipped under when it differs from the game's canonical title (a localized or retitled reissue). `regions` replaces the release's region list; the vocabulary is closed, so a region the list lacks is a schema question, not a free-text value.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "release_index": { "type": "integer" },
                "set": { "type": "object", "properties": {
                    "status": { "type": "string",
                                "enum": ["Released", "WorkInProgress", "Beta", "Prototype"] },
                    "title": { "type": "string" },
                    "label": { "type": "string" },
                    "date": { "type": "string" },
                    "publisher": { "type": "string" },
                    "tv_format": { "type": "string", "enum": ["Ntsc", "Pal", "PalM", "Secam"],
                        "description": "VCS only. PalM is Brazil's PAL-M: PAL colour on System M's 525-line/59.94 Hz raster, so it runs at NTSC timing, not PAL's — never file a Brazilian release as Pal" },
                    "controllers": { "type": "array", "items": { "type": "string",
                        "enum": ["Joystick", "Paddle", "Driving", "Keypad", "Trackball", "BoosterGrip"] },
                        "description": "VCS only. Controllers this release supports; replaces the list. Omit/empty for the default joystick, which most games use; list several when a game supports more than one." },
                    "regions": { "type": "array", "items": { "type": "string",
                        "enum": ["Japan", "Usa", "Europe", "World", "Taiwan", "Germany",
                                 "France", "China", "Spain", "Italy", "Australia",
                                 "UnitedKingdom", "Korea", "HongKong", "Sweden",
                                 "Netherlands", "Canada", "Brazil"] } },
                }},
            }), &["key", "release_index", "set"]),
        },
        {
            "name": "attach_dump_to_mod",
            "description": "Re-file a release dump onto a mod already attached to this game, instead of inventing a second mod for it. Use for a hack's later build (`as_version: true`, label it \"8K\" or \"v2\"), and for an alternate or defective dump of a hack (`as_version: false` — it joins the mod's latest version, labelled \"alt [a]\" or \"overdump\"). A bad dump of a hack is that hack's, not a work of its own.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "mod": { "type": "string", "description": "name of the mod already attached to this game" },
                "sha1": { "type": "string" },
                "as_version": { "type": "boolean", "description": "true = a distinct build of the mod; false (default) = another dump of the build it already has" },
                "label": { "type": "string" },
            }), &["key", "mod", "sha1"]),
        },
        {
            "name": "remove_release",
            "description": "Drop a release that holds nothing — a phantom left behind when its only dump was re-filed as a mod or moved elsewhere. Refuses while the release still carries dumps or sources, so it can never discard evidence.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "release_index": { "type": "integer" },
            }), &["key", "release_index"]),
        },
        {
            "name": "move_artifact",
            "description": "Move a dump into another release (by index). Use when a defective dump fabricated a release — an 8K overdump of a 4K game fingerprints as the wrong board and invents a product that never shipped; moving the dump out prunes a release left with nothing.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "to_release_index": { "type": "integer" },
            }), &["key", "sha1", "to_release_index"]),
        },
        {
            "name": "label_artifact",
            "description": "Give a dump a short distinguishing label ('alt', 'overdump', 'PAL conversion') so multiple hashes in one release are tellable apart.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "label": { "type": "string" },
            }), &["key", "sha1", "label"]),
        },
        {
            "name": "merge_game",
            "description": "Fold a duplicate entry into the one that should survive: `from`'s releases and mods become the target's, its directory is deleted and open flags follow the surviving key. Use when two entries catalogue the same game (an unlicensed reissue, a localized retitling) — not for a genuinely different product that merely shares a title, like a multicart. Dumps the target already holds are dropped rather than duplicated, and the target's curated stamp clears.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "from": { "type": "string", "description": "the entry being absorbed; it ceases to exist" },
            }), &["key", "from"]),
        },
        {
            "name": "rename_game",
            "description": "Change an entry's slug (its directory name and tree/slug key). Moves the manifest on disk and re-points open flags and the play queue; curations stand, since the game's content is unchanged. Use the returned new key afterwards. Slugs are lowercase alphanumerics, '-' or '_'.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "new_slug": { "type": "string" },
            }), &["key", "new_slug"]),
        },
        {
            "name": "find_duplicates",
            "description": "Entries whose normalized title (or any localized release title) collides with this game's — merge candidates. Run this for every game you curate; duplicates hide under punctuation, articles, and localized names.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "verify_artifacts",
            "description": "Check one entry's release dumps against the Hasheous signature database (sequential, rate-limited; can take ~10s). Confirmed originals get Signature evidence recorded on the artifact; DERIVED results (TOSEC [h]/[t]/[tr]/[cr]/[b]/[o] flags) are reported for you to judge and mark_hack — the bracket note is a cataloguer's shorthand, not the mod's real name. 'Unknown' is a result, not an error: homebrew, prototypes and private dumps are usually unsigned. Curations are never touched — verification is evidence about an immutable hash.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "wait_for_action",
            "description": "Long-poll for the developer's next decision: blocks up to ~50s and returns when they Accept (with or without a recommendation) or Flag the current entry, including which game is now up. Call it after you finish enriching; on timeout just call again. Events queue while you're not waiting, so nothing is missed.",
            "inputSchema": object(json!({}), &[]),
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
        {
            "name": "raise_flag",
            "description": "Raise a curation flag on a game — an issue to address later, with context. Defaults to kind EmulationIncompatibility (the emulator diverges from the hardware for this game); find these later with list_flags kind=EmulationIncompatibility. Put the full explanation in note; it survives the session where a chat message does not.",
            "inputSchema": object(json!({
                "key": { "type": "string", "description": "tree/slug the flag concerns" },
                "note": { "type": "string", "description": "the issue and the context needed to act on it later" },
                "kind": { "type": "string", "description": "flag kind; defaults to EmulationIncompatibility" },
            }), &["key", "note"]),
        },
        {
            "name": "update_flag",
            "description": "Amend an open flag by id: replace its note and/or change its kind. Use to reword a flag without resolving and re-raising it.",
            "inputSchema": object(json!({
                "id": { "type": "integer" },
                "note": { "type": "string", "description": "replacement note" },
                "kind": { "type": "string", "description": "new flag kind" },
            }), &["id"]),
        },
    ])
}
