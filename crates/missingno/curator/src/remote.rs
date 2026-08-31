//! `ui-<pid>.sock` endpoint speaking the same newline JSON-RPC protocol as the
//! emulator's UI-automation surface, so `missingno-remote` discovers the
//! curator and forwards its tools over MCP without any server changes. The
//! socket is the session crate's shared host, in the shared runtime directory,
//! named by pid, created mode 0600 under a 0700 directory, removed on drop.

use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use iced::futures::{StreamExt, channel::mpsc::UnboundedSender, stream};
use missingno_session::attach::{
    HostSpec, PartialFrames, Request, Serving, SocketHost, error_frame, success_frame,
};
use missingno_session::tools::{outcome_json, text};
use serde_json::{Value, json};

use missingno_gamedb::FactKind;

use crate::db::{self, fact_description};
use crate::vocabulary::{
    CONTROLLERS, DEFECTS, FEATURES, GAME_KINDS, LANGUAGES, LINK_TYPES, MOD_CATEGORIES, REGIONS,
    RELEASE_STATUSES, TV_FORMATS,
};

/// How long a parked reply waits between checks that the endpoint is still open.
const REPLY_POLL: Duration = Duration::from_millis(100);

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
    let (sink, calls) = iced::futures::channel::mpsc::unbounded::<ToolCall>();
    stream::once(async move { Bridge::Ready(sink) }).chain(calls.map(Bridge::Call))
}

const CURATOR_HOST: HostSpec = HostSpec {
    socket_prefix: "ui",
    occupant: "a curator",
    accept_thread: "curator-remote",
    client_thread: "curator-remote-client",
    partial_frames: PartialFrames::Resumed,
};

/// Publish the curator's tool surface in the default runtime directory.
pub fn open(sink: SharedSink) -> std::io::Result<SocketHost> {
    open_in(&missingno_session::attach::runtime_dir(), sink)
}

/// Publish in `dir`.
pub fn open_in(dir: &Path, sink: SharedSink) -> std::io::Result<SocketHost> {
    SocketHost::open_in(dir, CURATOR_HOST, move |line, serving| {
        Some(answer(line, &sink, serving))
    })
}

/// Dispatch one request frame.
fn answer(line: &str, sink: &SharedSink, serving: &Serving) -> Value {
    let Request { id, method, params } = match Request::parse(line) {
        Ok(request) => request,
        Err(error) => return error_frame(Value::Null, &format!("bad json: {error}")),
    };
    match method.as_str() {
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
            match dispatch(sink, name, args, serving) {
                Ok(body) => success_frame(id, body),
                Err(message) => error_frame(id, &message),
            }
        }
        other => error_frame(id, &format!("method not found: {other}")),
    }
}

fn dispatch(
    sink: &SharedSink,
    name: &str,
    args: Value,
    serving: &Serving,
) -> Result<Value, String> {
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
    // Anything that reaches the network needs longer than a UI round trip.
    let timeout = if matches!(
        name,
        "wait_for_action" | "verify_artifacts" | "identify_dump" | "cover_candidates"
    ) {
        Duration::from_secs(55)
    } else {
        REPLY_TIMEOUT
    };
    let unanswered = || {
        if name == "wait_for_action" {
            "no action yet — call wait_for_action again".to_owned()
        } else {
            "curator UI did not answer in time".to_owned()
        }
    };
    // Waiting in slices keeps a parked call from outlasting the endpoint.
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || !serving.is_serving() {
            return Err(unanswered());
        }
        match answer.recv_timeout(remaining.min(REPLY_POLL)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(unanswered()),
        }
    }
}

pub fn text_result(body: impl Into<String>) -> Value {
    outcome_json(text(body))
}

pub fn error_result(body: impl Into<String>) -> Value {
    outcome_json(Err(body.into()))
}

/// The `set` property one hardware fact takes, in the shape its kind states —
/// so a platform declaring a new key gets a property without a change here.
fn fact_property(key: &'static str, kind: &'static FactKind, lead: &str) -> Value {
    let doc = format!("{lead}{}", fact_description(key));
    match kind {
        FactKind::TvStandard => json!({
            "type": "string", "enum": TV_FORMATS.schema(), "description": doc,
        }),
        FactKind::Controllers { catalogue } => json!({
            "type": "array",
            "items": { "type": "string", "enum": CONTROLLERS.schema_of(catalogue) },
            "description": format!("{doc} Replaces the list; omit or empty for the platform default."),
        }),
        FactKind::Features { catalogue } => json!({
            "type": "array",
            "items": { "type": "string", "enum": FEATURES.schema_of(catalogue) },
            "description": format!("{doc} Replaces the list; empty = the release has none of them."),
        }),
        FactKind::Board { .. } => board_property(key, &doc),
    }
}

/// A board statement: the board's own name plus one property per part any
/// board under this key carries, each enumerated from the platforms' own
/// catalogues.
fn board_property(key: &'static str, doc: &str) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "board".to_owned(),
        json!({
            "type": "string",
            "description": "the board's own name, as its platform's vocabulary lists it",
        }),
    );
    for part in db::board_attributes(key) {
        let mut description = part.label.to_owned();
        if !part.choices.is_empty() {
            description.push_str(&format!(" — one of {}", part.choices.join(", ")));
        }
        let types = match part.types.as_slice() {
            [one] => json!(one),
            many => json!(many),
        };
        properties.insert(
            part.key.to_owned(),
            json!({ "type": types, "description": description }),
        );
    }
    json!({
        "type": "object",
        "properties": properties,
        "description": format!(
            "{doc} State the board and the parts populated on it, e.g. \
             {{\"board\": \"Mbc5\", \"rom\": \"1M\", \"ram\": \"32K\", \"battery\": true}}. \
             An empty object, or an empty board name, clears it back to unstated. \
             Boards, with the parts each carries (\"?\" = the board may go without it) — {}",
            db::board_vocabulary_doc(key)
        ),
    })
}

/// Add every platform-declared hardware fact to a `set` object's properties,
/// `lead` saying whose hardware the payload's facts describe.
fn with_fact_properties(mut properties: Value, lead: &str) -> Value {
    if let Some(map) = properties.as_object_mut() {
        for (key, kind) in db::fact_kinds() {
            map.insert(key.to_owned(), fact_property(key, kind, lead));
        }
    }
    properties
}

fn tool_definitions() -> Value {
    let object = |properties: Value, required: &[&str]| json!({ "type": "object", "properties": properties, "required": required });
    let board_of_first_release = db::fact_kinds()
        .into_iter()
        .find(|(key, _)| *key == "cart_type")
        .map(|(key, kind)| fact_property(key, kind, "The first release's board. "))
        .unwrap_or(Value::Null);
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
                "tree": { "type": "string", "enum": missingno_gamedb::platform_dirs() },
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
            "description": "Stage edits to a game. Text fields plus cover image URLs (remote links only — Hasheous, the project's own repo/pouet page, libretro-thumbnails, or Wikimedia; never store CDNs) and a Wikipedia article link. Setting `wikipedia` creates the game's \"Wikipedia\" link by itself — do not also pass one in `links`, or the article ends up listed twice. `remove_links` drops links by name. Edits appear live in the curator UI as uncommitted changes until committed; a curated stamp stands (edits happen at the curator's request).",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "set": { "type": "object", "properties": {
                    "title": { "type": "string" },
                    "developer": { "type": "string" },
                    "description": { "type": "string" },
                    "kind": { "type": "string", "enum": GAME_KINDS.schema(),
                              "description": "what kind of work the entry is; Test = diagnostic/calibration utility, Tool = authoring/programming software, Software = anything else nobody played (reference databases, exam drills, organisers) where Test and Tool do not fit" },
                    "adult": { "type": "boolean",
                               "description": "the work is adult material — set it on the pornographic carts (Mystique/PlayAround and friends), judged on content, not on a suggestive title" },
                    "publisher": { "type": "string",
                                   "description": "publisher of the first release (release-level; UI edits others)" },
                    "covers": { "type": "array", "items": { "type": "string" },
                                "description": "remote image URLs, preference order" },
                    "wikipedia": { "type": "string", "description": "article URL" },
                    "links": { "type": "array",
                               "description": "durable source receipts: record the page that backed each staged fact (AtariAge thread, author's site, pouet prod, MobyGames…). Upserts by name — re-staging the same source never duplicates. This is where sources survive: chat dies with the session, links live in the manifest.",
                               "items": { "type": "object", "properties": {
                                   "name": { "type": "string" },
                                   "url": { "type": "string" },
                                   "link_type": { "type": "string",
                                                  "enum": LINK_TYPES.schema(),
                                                  "description": "Store = a page to buy the game (a store product page for a paid aftermarket/homebrew cart); DownloadPage = a page to obtain the ROM from (forum thread / download page, human-followed); Download = a direct fetchable ROM file URL (.zip/.bin/.a26). Use DownloadPage/Download for freeware." },
                                   "languages": { "type": "array",
                                                  "items": { "type": "string", "enum": LANGUAGES.schema() },
                                                  "description": "Languages this link's text is in. Tag a manual explicitly as you curate it, English included; list every language present when it is multilingual (a trilingual manual = [English, German, French]). Empty still reads as English." },
                               }, "required": ["name", "url", "link_type"] } },
                    "remove_links": { "type": "array", "items": { "type": "string" },
                                      "description": "link names to drop, for clearing duplicates or a link that turned out to be wrong" },
                    "cart_type": board_of_first_release,
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
            "name": "mark_mod",
            "description": "A hash in an entry turned out to be a modified dump — a mod. Everything except a total conversion becomes a mod ATTACHED to the same game — its own name, homepage link, versions and independent curation; a fan translation is still the same game, exactly as official localizations are releases of it. A Compatibility mod (an NTSC/PAL conversion, a bankswitch re-encoding) is the same game made to run elsewhere. Only TotalConversion splits into its own derived-work entry. Supply title (the mod's real name) and url (a page for it) whenever known — name that page with link_name, since only the author's own page or release thread is a Homepage.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "title": { "type": "string" },
                "category": { "type": "string",
                              "enum": MOD_CATEGORIES.schema() },
                "base_sha1": { "type": "string", "description": "the dump this mod was made from — a release's, or another mod's when this derives from that hack (a Supercharger conversion of a hack); 'none' when the base is genuinely unknown — never guess one" },
                "url": { "type": "string", "description": "a page for the mod: its homepage, the author's release thread, or a catalogue listing" },
                "link_name": { "type": "string", "description": "what that page is, named as it will read in the manifest — \"Homepage\" (default) only for the author's own page or release thread; a catalogue listing takes the site's name, e.g. \"AtariAge\"" },
                "link_type": { "type": "string", "enum": LINK_TYPES.schema(),
                               "description": "defaults to Community" },
            }), &["key", "sha1"]),
        },
        {
            "name": "update_mod",
            "description": "Correct or enrich an attached mod's recorded fields: rename it, fix its category, author, a link (url plus link_name/link_type), or a release's base_sha1 ('none' to clear), label, or date. Identify the mod by its current name.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "mod": { "type": "string", "description": "the mod's current name" },
                "set": { "type": "object", "properties": with_fact_properties(json!({
                    "name": { "type": "string" },
                    "category": { "type": "string",
                                  "enum": MOD_CATEGORIES.schema() },
                    "author": { "type": "string" },
                    "url": { "type": "string", "description": "a page for the mod; empty string drops the link named by link_name" },
                    "link_name": { "type": "string", "description": "what that page is — \"Homepage\" (default) only for the author's own page or release thread; a catalogue listing takes the site's name, e.g. \"AtariAge\". Upserts by name." },
                    "link_type": { "type": "string", "enum": LINK_TYPES.schema(),
                                   "description": "defaults to Community" },
                    "release_index": { "type": "integer" },
                    "base_sha1": { "type": "string", "description": "a release's dump, or another mod's when this derives from that hack" },
                    "label": { "type": "string" },
                    "date": { "type": "string" },
                }), "What THIS build runs on where a conversion moved it off the game's own hardware — an NTSC build of a PAL game, a joystick build of a keypad game. ")},
            }), &["key", "mod", "set"]),
        },
        {
            "name": "split_release",
            "description": "An artifact that is really its own release — a (Prototype) or (Beta) build sitting in the retail release: move it into a new release with the given status, inheriting hardware and publisher but not the retail date. Keep a working title (e.g. 'Jungle Runner') via title.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "status": { "type": "string",
                            "enum": RELEASE_STATUSES.schema() },
                "title": { "type": "string" },
                "label": { "type": "string" },
                "date": { "type": "string" },
            }), &["key", "sha1", "status"]),
        },
        {
            "name": "update_release",
            "description": "Set fields on an existing release: status, title, label, date, publisher, regions, controllers (VCS). `title` is the name this release shipped under when it differs from the game's canonical title (a localized or retitled reissue). An empty string clears title, label, publisher, or date (a carried-over date that is wrong for this release beats leaving a false one). `regions` replaces the release's region list; the vocabulary is closed, so a region the list lacks is a schema question, not a free-text value.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "release_index": { "type": "integer" },
                "set": { "type": "object", "properties": with_fact_properties(json!({
                    "status": { "type": "string",
                                "enum": RELEASE_STATUSES.schema() },
                    "title": { "type": "string" },
                    "label": { "type": "string" },
                    "date": { "type": "string" },
                    "publisher": { "type": "string" },
                    "regions": { "type": "array", "items": { "type": "string",
                        "enum": REGIONS.schema() } },
                    "languages": { "type": "array", "items": { "type": "string",
                        "enum": LANGUAGES.schema() },
                        "description": "Languages this release presents to the player; replaces the list. Record them wherever a release carries a substantial amount of text — English included, even though it is the default, so a later fan translation reads as a change. Omit only where there is too little text to matter, which covers most Atari carts." },
                }), "")},
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
            "name": "add_release",
            "description": "Record a release the catalogue knows shipped but holds no dump of — a rare cart whose ROM has never surfaced. Adds an empty release and returns its index; set its publisher, date, title and hardware with update_release. Not for a release you have a dump for: those arrive with the dump.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
            }), &["key"]),
        },
        {
            "name": "remove_release",
            "description": "Drop a release that holds nothing — a phantom left behind when its only dump was re-filed as a mod or moved elsewhere. Refuses while the release still carries dumps or sources unless discard_dumps is true — pass that ONLY on the developer's explicit instruction, because it permanently drops the hashes with the release.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "release_index": { "type": "integer" },
                "discard_dumps": { "type": "boolean" },
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
            "description": "Annotate a dump: give it a benign distinguishing label ('alt', 'PAL conversion') so multiple hashes in one release are tellable apart, and/or record a quality `defect`. `Overdump` ([o]) is padded but still plays; `BadDump` ([b]) is corrupt or truncated and does not. Pass defect \"None\" to clear it. Provide a label, a defect, or both.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "sha1": { "type": "string" },
                "label": { "type": "string" },
                "defect": { "type": "string", "enum": DEFECTS.schema() },
            }), &["key", "sha1"]),
        },
        {
            "name": "merge_game",
            "description": "Fold a duplicate entry into the one that should survive: `from`'s releases and mods become the target's, its directory is deleted and open flags follow the surviving key. Use when two entries catalogue the same game (an unlicensed reissue, a localized retitling) — not for a genuinely different product that merely shares a title, like a multicart. Dumps the target already holds are dropped rather than duplicated, and curated stamps from both sides stand on the survivor. Pick the target by identity, not effort: the original release is the entry that survives (a localized reissue or retitled skin folds into the original, never the reverse).",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "from": { "type": "string", "description": "the entry being absorbed; it ceases to exist" },
            }), &["key", "from"]),
        },
        {
            "name": "reject_game",
            "description": "Turn an entry away for good: it is not software this database catalogues. Its manifest is deleted, its dumps are recorded as out of scope so a rescan never offers them again, and the inbox copies are removed — the archive beside them is left, so re-extracting undoes it. Use for firmware belonging to a hardware accessory rather than a title someone played; a duplicate entry is merge_game, and a game you simply have no dump for stays in the backlog. Rejecting the game being playtested moves the queue on.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "reason": { "type": "string", "description": "why it is out of scope; it is recorded beside the dumps so nobody re-curates them" },
            }), &["key", "reason"]),
        },
        {
            "name": "split_game",
            "description": "The inverse of merge_game: an import lumped two different games into one entry, so a release moves out whole and becomes an entry of its own, keeping its publisher, date, regions and hardware. Mods whose base dump leaves travel with it. Use when a release turns out to be an unrelated game sharing a title — not for a pre-retail build of the same game, which is split_release. Returns the new tree/slug key.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "release_index": { "type": "integer", "description": "the release to move out" },
                "title": { "type": "string", "description": "the new entry's title — the game's real name, not the one the lumped entry wore" },
                "slug": { "type": "string", "description": "optional slug; derived from the title when omitted" },
            }), &["key", "release_index", "title"]),
        },
        {
            "name": "move_game",
            "description": "Re-file an entry under another platform: an import that guessed the wrong tree files a Game Boy Color game in gb, or a dump the extension mislabelled in the wrong console's tree entirely. The manifest is rebuilt under the target platform and its directory moves; every hardware fact the target also states carries over, and one it does not — a Game Boy Color entry has no sgb/cgb of its own — is dropped and named in the reply. Refuses when the slug is already taken in the target tree; rename first. Flags follow, and the returned key is the one to use afterwards.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "tree": { "type": "string", "enum": missingno_gamedb::platform_dirs(),
                          "description": "the tree the entry belongs in" },
            }), &["key", "tree"]),
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
            "description": "Check one entry's release dumps against the Hasheous signature database (sequential, rate-limited; can take ~10s). Confirmed originals get Signature evidence recorded on the artifact; DERIVED results (TOSEC [h]/[t]/[tr]/[cr]/[b]/[o] flags) are reported for you to judge and mark_mod — the bracket note is a cataloguer's shorthand, not the mod's real name. 'Unknown' is a result, not an error: homebrew, prototypes and private dumps are usually unsigned. Curations are never touched — verification is evidence about an immutable hash.",
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
            "description": "Clear a flag once its work is done — this deletes it. A flag is future work, not a record: git carries the history.",
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
            "name": "related_entries",
            "description": "Entries that may be the same game — run this for every game, before find_duplicates. Catches what title matching cannot: slug-suffixed splits (-ntsc, -pal, -f0), dump-flag entries (-a), and hacks filed as games, plus same-title and title-contains matches. Each result says why it matched. Read the titles before folding anything in: a hack names itself, not its base, so an adjacent slug can belong to a different game.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "identify_dump",
            "description": "Everything known about one hash in a single call: where it sits in the database, the signature database's per-dump name and its publisher/year/country/size, the cover image Hasheous holds for it, and any mapped Wikipedia article. The signature name carries the TOSEC bracket flags that identify a hack, a bad dump or an overdump; the size is what tells a padded overdump from a genuine variant.",
            "inputSchema": object(json!({ "sha1": { "type": "string" } }), &["sha1"]),
        },
        {
            "name": "dump_info",
            "description": "Where a hash lives: which entry and release (or mod) holds it, and the local file and byte size if the ROM dirs have been scanned. Offline — use identify_dump for the signature database.",
            "inputSchema": object(json!({ "sha1": { "type": "string" } }), &["sha1"]),
        },
        {
            "name": "cover_candidates",
            "description": "Fetch every candidate cover for a game — what is already staged, what Hasheous holds for its lead dump, and the libretro-thumbnails boxart — and report each one's pixel size and byte count so they can be compared before staging. Hasheous groups variants under one record, so its image is regularly another platform's box or the same art cropped free of its platform banner; a size mismatch is the tell. Still download and look at the one you keep.",
            "inputSchema": object(json!({ "key": { "type": "string" } }), &["key"]),
        },
        {
            "name": "session_changes",
            "description": "Every mutating tool call made this session, in order, with what each returned. Use it to write the end-of-session summary off a record rather than from memory.",
            "inputSchema": object(json!({}), &[]),
        },
        {
            "name": "extend_queue",
            "description": "Append games to the end of the queue without touching the current playtest. Use this to top up mid-session — queue_games replaces the whole queue and restarts its first key, which yanks the game the developer is playing.",
            "inputSchema": object(json!({ "keys": { "type": "array", "items": { "type": "string" } } }), &["keys"]),
        },
        {
            "name": "retitle",
            "description": "Set a game's title and, when the slug should follow it, rename the entry and move its collection folder in one call. Reply names the new tree/slug key — use it for every later call. Take the title from the box or manual cover: the import titles entries from a No-Intro/TOSEC filename, which carries taglines, ad copy, dump flags and publisher qualifiers that are not the name.",
            "inputSchema": object(json!({
                "key": { "type": "string" },
                "title": { "type": "string" },
                "slug": { "type": "string", "description": "new slug, when the title change should move the entry too" },
            }), &["key", "title"]),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;

    /// Open an endpoint in a temp dir behind a stub UI, exchange the three
    /// methods, then drop it — the drop must stop the accept thread and clear
    /// the socket rather than hang or leave the file behind.
    #[test]
    fn framing_round_trip_and_shutdown() {
        let dir = tempfile::tempdir().expect("temp dir");
        let shared = SharedSink::default();

        let (sink, mut calls) = iced::futures::channel::mpsc::unbounded::<ToolCall>();
        shared.set(sink);
        let _stub = std::thread::spawn(move || {
            while let Some(call) = iced::futures::executor::block_on(calls.next()) {
                let _ = call.reply.send(text_result(format!("ran {}", call.name)));
            }
        });

        let endpoint = open_in(dir.path(), shared).expect("open endpoint");
        let path = endpoint.path().to_path_buf();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let stream = UnixStream::connect(&path).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = |frame: Value| {
            writeln!(writer, "{frame}").unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        };

        let info = request(json!({ "jsonrpc": "2.0", "id": 1, "method": "ui/info" }));
        assert_eq!(info["result"]["app"], "net.andyofniall.missingno-curator");

        let list = request(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"status"));
        assert!(names.contains(&"queue_games"));

        let called = request(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                                     "params": { "name": "status", "arguments": {} } }));
        assert_eq!(called["result"]["isError"], false);
        assert_eq!(called["result"]["content"][0]["text"], "ran status");

        // A second host on the same socket is refused, not a silent takeover.
        match open_in(dir.path(), SharedSink::default()) {
            Ok(_) => panic!("a live host keeps its socket"),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse),
        }

        drop(endpoint);
        assert!(!path.exists(), "drop clears the socket file");
    }

    fn set_properties(tools: &Value, name: &str) -> Value {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} is published"))["inputSchema"]["properties"]["set"]
            ["properties"]
            .clone()
    }

    /// The editing tools offer exactly the fact keys the platforms declare, so
    /// a key the schema shows is one the parser reads and a payload can be
    /// asked for — and a key no platform states any more is gone from both.
    #[test]
    fn the_editing_tools_offer_every_declared_fact() {
        let tools = tool_definitions();
        for tool in ["update_release", "update_mod"] {
            let set = set_properties(&tools, tool);
            for (key, _) in db::fact_kinds() {
                assert!(!set[key].is_null(), "{tool} offers no {key}");
            }
            assert!(set["rom_size"].is_null(), "{tool} still states a ROM size");
        }
        assert!(
            set_properties(&tools, "update_game")["mapper"].is_null(),
            "the board's key is cart_type everywhere"
        );

        let board = set_properties(&tools, "update_release")["cart_type"].clone();
        assert_eq!(board["type"], "object");
        assert_eq!(board["properties"]["board"]["type"], "string");
        assert_eq!(board["properties"]["battery"]["type"], "boolean");
        // A part one platform names and another measures takes either shape.
        assert_eq!(
            board["properties"]["rom"]["type"],
            json!(["string", "integer"])
        );
        let described = board["description"].as_str().unwrap();
        assert!(
            described.contains("Mbc5(rom, ram?, battery?, rumble?)"),
            "{described}"
        );
        assert!(described.contains("Tigervision(rom?)"), "{described}");
    }
}
