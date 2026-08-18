#![recursion_limit = "256"]

//! missingno-curator — review, enrich, and confirm gamedb entries.
//!
//! v1: Backlog (uncurated entries) and Flags drain through one list+editor
//! screen; confirms stamp `curated` and accumulate into explicit git commits.

mod db;
mod play;
mod remote;
mod verify;
mod vocabulary;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input, toggler,
};
use iced::{Element, Length, Task, Theme};

use db::{Db, TextField, TreeId};
use missingno_core::system::{ControlId, ControlRole};
use remote::{Bridge, SharedSink, error_result, text_result};
use verify::RomIndex;
#[derive(Parser)]
struct Args {
    /// Path to the missingno-gamedb checkout.
    #[arg(default_value = "missingno-gamedb")]
    db_path: PathBuf,

    /// The inbox: a folder of to-be-curated ROMs to hash-match and queue.
    #[arg(long)]
    rom_dir: Option<PathBuf>,

    /// The curated collection: accepted games' ROMs move here, and inbox
    /// files already present here are set aside as duplicates at scan.
    #[arg(long)]
    collection_dir: Option<PathBuf>,

    /// Don't publish the ui-<pid>.sock remote-control socket.
    #[arg(long)]
    no_remote: bool,

    /// Curator identifier for recommendations (default: git user.name, first word lowercased).
    #[arg(long)]
    curator: Option<String>,
}

/// A mod's link, read from a tool call's `url` plus optional `link_name` and
/// `link_type`. A catalogue listing is not a homepage, so the name is only
/// "Homepage" by default.
fn mod_link(args: &serde_json::Value) -> Result<Option<missingno_gamedb::Link>, String> {
    let Some(url) = args.get("url").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    let name = args
        .get("link_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Homepage");
    let link_type = match args.get("link_type").and_then(serde_json::Value::as_str) {
        Some(kind) => db::parse_link_type(kind)?,
        None => missingno_gamedb::LinkType::Community,
    };
    Ok(Some(missingno_gamedb::Link {
        name: name.to_owned(),
        url: url.to_owned(),
        link_type,
        languages: Vec::new(),
    }))
}

/// This process's action-event log: `runtime_dir/curator-events-<pid>.log`.
/// A client tails it to see accepts/flags live without the long-poll.
fn event_log_path() -> std::path::PathBuf {
    missingno_session::attach::runtime_dir()
        .join(format!("curator-events-{}.log", std::process::id()))
}

fn append_event_log(event: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(event_log_path())
    {
        let _ = writeln!(f, "{event}");
    }
}

pub fn main() -> iced::Result {
    let args = Args::parse();
    let db_path = args.db_path.clone();
    let rom_dir = args.rom_dir.clone();
    let collection_dir = args.collection_dir.clone();
    let remote = !args.no_remote;
    // An identifier ("andy"), not a display name; git's name shrinks to one.
    let curator_name = args.curator.clone().unwrap_or_else(|| {
        std::process::Command::new("git")
            .args(["config", "user.name"])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .split_whitespace()
                    .next()
                    .map(str::to_lowercase)
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_owned())
    });
    iced::application(
        move || {
            Curator::new(
                db_path.clone(),
                rom_dir.clone(),
                collection_dir.clone(),
                remote,
                curator_name.clone(),
            )
        },
        Curator::update,
        Curator::view,
    )
    .title(Curator::title)
    .theme(Curator::theme)
    .subscription(Curator::subscription)
    .window_size(iced::Size::new(1280.0, 800.0))
    .exit_on_close_request(false)
    .run()
}

struct Curator {
    db: Result<Db, String>,
    filter_tree: Option<TreeId>,
    only_backlog: bool,
    only_flagged: bool,
    /// Show only ROMs discovered locally that match no manifest yet.
    only_new: bool,
    search: String,
    selected: Option<usize>,
    status: String,
    rom_dir: Option<PathBuf>,
    collection_dir: Option<PathBuf>,
    rom_index: Option<std::sync::Arc<RomIndex>>,
    /// entry key → last fetch/verify status line.
    verify_status: std::collections::HashMap<String, String>,
    /// sha1 → this session's Hasheous verdict (✓name / DERIVED / unknown).
    session_marks: std::collections::HashMap<String, String>,
    /// Every mutating tool call this session, so the end-of-session report is
    /// read off a record rather than reconstructed from memory.
    session_log: Vec<String>,
    /// sha1 → fetched ROM bytes, kept for boot verification.
    rom_cache: std::collections::HashMap<String, std::sync::Arc<Vec<u8>>>,
    playing: Option<(String, play::PlaySession)>,
    play_screen: Option<missingno_iced::ScreenView>,
    playing_sha1: Option<String>,
    /// Guards frame-loop messages from superseded play sessions.
    play_generation: u64,
    /// Agent-driven curation queue of entry keys; front = current.
    queue: std::collections::VecDeque<String>,
    /// Fetch in flight that should start a playtest when it lands.
    play_after_fetch: Option<String>,
    /// entry key → last fetched sha1 (bytes live in rom_cache).
    fetched_sha1: std::collections::HashMap<String, String>,
    /// cover url → fetched preview.
    cover_previews: std::collections::HashMap<String, iced::widget::image::Handle>,
    /// cover urls that failed to fetch (shown as an error instead of silence).
    cover_failed: std::collections::HashSet<String>,
    /// entries already auto-looked-up on Hasheous this session.
    enrich_attempted: std::collections::HashSet<String>,
    enriching: bool,
    remote_sink: SharedSink,
    _remote: Option<missingno_session::attach::SocketHost>,
    /// Parked wait_for_action replies, answered when the human acts.
    action_waiters: Vec<std::sync::mpsc::Sender<serde_json::Value>>,
    /// Decisions made while no agent was waiting.
    action_events: std::collections::VecDeque<String>,
    curator_name: String,
    /// Stamp the next confirm as an editor's-choice recommendation.
    recommend_next: bool,
    /// The filter/list column; hidden automatically when an agent queues work.
    list_visible: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Remote(Bridge),
    Play(BootSource),
    PlayFrame(u64),
    PlayEnded(u64),
    StopPlay,
    Pad(ControlId, bool),
    /// A host gamepad edge, landing in whichever jack the pad is patched into.
    Gamepad(ControlId, bool),
    /// Move the gamepad to the other controller jack.
    SwapPadJack,
    Paddle(f32),
    /// A keypad key edge from the host keyboard: key index, Shift held, pressed.
    PlayKey(u8, bool, bool),
    /// Flip a latching console switch (index into the family's switch list).
    ToggleSwitch(usize),
    /// Press a momentary console switch; release follows after a beat.
    TapSwitch(ControlId),
    Fetch,
    Fetched(String, Result<(String, std::sync::Arc<Vec<u8>>), String>),
    ScanRoms,
    ScannedRoms(Result<std::sync::Arc<RomIndex>, String>),
    FilterTree(TreeChoice),
    OnlyBacklog(bool),
    OnlyFlagged(bool),
    OnlyNew(bool),
    Search(String),
    Select(usize),
    ArtifactsVerified {
        key: String,
        results: Vec<(String, verify::SigResult)>,
        reply: std::sync::mpsc::Sender<serde_json::Value>,
    },
    Enriched(String, Result<Option<verify::HasheousHit>, String>),
    /// A tool whose answer needed the network, ready to send back verbatim.
    ToolText {
        text: String,
        reply: std::sync::mpsc::Sender<serde_json::Value>,
    },
    CoverLoaded(String, Option<iced::widget::image::Handle>),
    ConfirmAndNext,
    Accept {
        recommend: bool,
    },
    /// Advance past the current queued entry without accepting it — skip an
    /// uncurated entry to defer it for later.
    SkipNext,
    ToggleList,
    CloseRequested,
    OpenLink(String),
    ResolveFlag(u32),
}

#[derive(Debug, Clone)]
enum BootSource {
    Cached(String),
    File(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreeChoice(Option<TreeId>);

impl std::fmt::Display for TreeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.map(TreeId::label).unwrap_or("All platforms"))
    }
}

const TREE_CHOICES: [TreeChoice; 5] = [
    TreeChoice(None),
    TreeChoice(Some(TreeId::Gb)),
    TreeChoice(Some(TreeId::Gbc)),
    TreeChoice(Some(TreeId::Sg1000)),
    TreeChoice(Some(TreeId::Vcs)),
];

const LIST_LIMIT: usize = 250;

impl Curator {
    fn title(&self) -> String {
        match &self.db {
            Ok(db) => format!(
                "missingno curator — {} ({} uncommitted)",
                db.repo_root.display(),
                db.uncommitted
            ),
            Err(_) => "missingno curator".to_owned(),
        }
    }

    fn theme(&self) -> Theme {
        Theme::TokyoNight
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        let mut subscriptions = vec![
            iced::Subscription::run(play::gamepad_worker).map(|event| match event {
                play::PadEvent::Button(id, on) => Message::Gamepad(id, on),
                play::PadEvent::Paddle(position) => Message::Paddle(position),
            }),
            // Keyboard events only where no widget took them, so typing into a
            // field never reaches the keypad in the playtest's jack.
            iced::event::listen_with(|event, status, _| match (event, status) {
                (iced::Event::Window(iced::window::Event::CloseRequested), _) => {
                    Some(Message::CloseRequested)
                }
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key, modifiers, ..
                    }),
                    iced::event::Status::Ignored,
                ) => play::keypad_key(&key).map(|n| Message::PlayKey(n, modifiers.shift(), true)),
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                        key,
                        modifiers,
                        ..
                    }),
                    iced::event::Status::Ignored,
                ) => play::keypad_key(&key).map(|n| Message::PlayKey(n, modifiers.shift(), false)),
                _ => None,
            }),
        ];
        if self._remote.is_some() {
            subscriptions.push(iced::Subscription::run(remote::worker).map(Message::Remote));
        }
        iced::Subscription::batch(subscriptions)
    }

    fn new(
        db_path: PathBuf,
        rom_dir: Option<PathBuf>,
        collection_dir: Option<PathBuf>,
        remote: bool,
        curator_name: String,
    ) -> (Self, Task<Message>) {
        let has_rom_dir = rom_dir.is_some() || collection_dir.is_some();
        let db = Db::load(db_path).map_err(|e| e.to_string());
        let remote_sink = SharedSink::default();
        let endpoint = remote
            .then(|| remote::open(remote_sink.clone()).ok())
            .flatten();
        (
            Self {
                db,
                filter_tree: None,
                only_backlog: true,
                only_flagged: false,
                only_new: false,
                search: String::new(),
                selected: None,
                status: String::new(),
                rom_dir,
                collection_dir,
                rom_index: None,
                verify_status: std::collections::HashMap::new(),
                session_marks: std::collections::HashMap::new(),
                session_log: Vec::new(),
                rom_cache: std::collections::HashMap::new(),
                playing: None,
                play_screen: None,
                playing_sha1: None,
                play_generation: 0,
                queue: std::collections::VecDeque::new(),
                play_after_fetch: None,
                fetched_sha1: std::collections::HashMap::new(),
                cover_previews: std::collections::HashMap::new(),
                cover_failed: std::collections::HashSet::new(),
                enrich_attempted: std::collections::HashSet::new(),
                enriching: false,
                remote_sink,
                _remote: endpoint,
                action_waiters: Vec::new(),
                action_events: std::collections::VecDeque::new(),
                curator_name,
                recommend_next: false,
                list_visible: false,
            },
            // A --rom-dir given at launch (e.g. by an agent starting the
            // curator) scans immediately; nothing to click.
            if has_rom_dir {
                Task::done(Message::ScanRoms)
            } else {
                Task::none()
            },
        )
    }

    /// Indices of entries matching the current filters, in list order.
    fn visible(&self) -> Vec<usize> {
        let Ok(db) = &self.db else {
            return Vec::new();
        };
        let needle = self.search.to_lowercase();
        db.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.filter_tree.is_none_or(|t| e.tree == t))
            .filter(|(_, e)| !self.only_new || e.synthetic)
            .filter(|(_, e)| !self.only_backlog || !e.game.curated())
            .filter(|(_, e)| {
                !self.only_flagged || db.flags.open().any(|f| f.subject.contains(&e.key()))
            })
            .filter(|(_, e)| {
                needle.is_empty()
                    || e.game.title().to_lowercase().contains(&needle)
                    || e.slug.contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Remote(Bridge::Ready(sink)) => {
                self.remote_sink.set(sink);
                if let Some(endpoint) = &self._remote {
                    self.status = format!("remote socket: {}", endpoint.path().display());
                }
            }
            Message::Remote(Bridge::Call(call)) => {
                if call.name == "verify_artifacts" {
                    let Some(key) = call.args.get("key").and_then(serde_json::Value::as_str) else {
                        let _ = call.reply.send(error_result("missing key"));
                        return Task::none();
                    };
                    let Some(i) = self.find_entry(key) else {
                        let _ = call.reply.send(error_result(format!("no entry {key}")));
                        return Task::none();
                    };
                    let Ok(db) = &self.db else {
                        let _ = call.reply.send(error_result("db not loaded"));
                        return Task::none();
                    };
                    // Release dumps only: a hack hash being a hack is expected
                    // on a mod, and one entry is the polite unit of API load.
                    let mut sha1s = Vec::new();
                    for r in 0..db.entries[i].game.release_lines().len() {
                        for (sha1, _, _) in db.entries[i].game.release_artifacts(r) {
                            sha1s.push(sha1);
                        }
                    }
                    if sha1s.is_empty() {
                        let _ = call
                            .reply
                            .send(text_result(format!("{key} has no dumps to verify")));
                        return Task::none();
                    }
                    let key = key.to_owned();
                    let reply = call.reply.clone();
                    self.status = format!("verifying {} dump(s) of {key}…", sha1s.len());
                    return Task::perform(
                        smol::unblock(move || verify::lookup_signatures(sha1s)),
                        move |results| Message::ArtifactsVerified {
                            key: key.clone(),
                            results,
                            reply: reply.clone(),
                        },
                    );
                }
                if call.name == "identify_dump" {
                    let Some(sha1) = call.args.get("sha1").and_then(serde_json::Value::as_str)
                    else {
                        let _ = call.reply.send(error_result("missing sha1"));
                        return Task::none();
                    };
                    let sha1 = sha1.to_owned();
                    let reply = call.reply.clone();
                    let local = self
                        .db
                        .as_ref()
                        .ok()
                        .and_then(|db| db.find_dump(&sha1))
                        .map(|(key, title, what)| format!("in db: {key} ({title}) — {what}"))
                        .unwrap_or_else(|| "in db: not found".to_owned());
                    self.status = format!("identifying {sha1}…");
                    return Task::perform(
                        smol::unblock({
                            let sha1 = sha1.clone();
                            move || verify::hasheous_lookup(&sha1)
                        }),
                        move |hit| {
                            let mut lines = vec![local.clone()];
                            match hit {
                                Ok(Some(hit)) => {
                                    lines.push(format!("name: {}", hit.name));
                                    if let Some(s) = &hit.signature_name {
                                        lines.push(format!("signature: {s}"));
                                    }
                                    if let Some(n) = hit.rom_size {
                                        lines.push(format!("signature size: {n} bytes"));
                                    }
                                    for (what, v) in [
                                        ("publisher", &hit.signature_publisher),
                                        ("year", &hit.signature_year),
                                        ("country", &hit.signature_country),
                                    ] {
                                        if let Some(v) = v {
                                            lines.push(format!("signature {what}: {v}"));
                                        }
                                    }
                                    if let Some(u) = &hit.cover_url {
                                        lines.push(format!("cover: {u}"));
                                    }
                                    if let Some(u) = &hit.wikipedia_url {
                                        lines.push(format!("wikipedia (mapped): {u}"));
                                    }
                                }
                                Ok(None) => lines.push(
                                    "signature: unknown to the signature database — usually \
                                     homebrew, a prototype or a private dump"
                                        .to_owned(),
                                ),
                                Err(e) => lines.push(format!("lookup failed: {e}")),
                            }
                            Message::ToolText {
                                text: lines.join("\n"),
                                reply: reply.clone(),
                            }
                        },
                    );
                }
                if call.name == "cover_candidates" {
                    let Some(key) = call.args.get("key").and_then(serde_json::Value::as_str) else {
                        let _ = call.reply.send(error_result("missing key"));
                        return Task::none();
                    };
                    let Some(i) = self.find_entry(key) else {
                        let _ = call.reply.send(error_result(format!("no entry {key}")));
                        return Task::none();
                    };
                    let Ok(db) = &self.db else {
                        let _ = call.reply.send(error_result("db not loaded"));
                        return Task::none();
                    };
                    let staged: Vec<String> = db.entries[i].game.covers();
                    let system = match db.entries[i].tree {
                        db::TreeId::Vcs => "Atari - 2600",
                        db::TreeId::Sg1000 => "Sega - SG-1000",
                        db::TreeId::Gb => "Nintendo - Game Boy",
                        db::TreeId::Gbc => "Nintendo - Game Boy Color",
                    };
                    // The first dump of a release is regularly an unrecognised
                    // alt, so offer every release dump and take the first the
                    // signature database knows.
                    let dumps: Vec<String> = (0..db.entries[i].game.release_lines().len())
                        .flat_map(|r| db.entries[i].game.release_artifacts(r))
                        .map(|(sha1, _, _)| sha1)
                        .collect();
                    let title = db.entries[i].game.title().to_owned();
                    let reply = call.reply.clone();
                    self.status = format!("gathering covers for {key}…");
                    return Task::perform(
                        smol::unblock(move || {
                            let mut out: Vec<verify::CoverCandidate> = staged
                                .iter()
                                .map(|u| verify::measure_cover("staged", u.clone()))
                                .collect();
                            for sha1 in dumps.iter().take(12) {
                                let Ok(Some(hit)) = verify::hasheous_lookup(sha1) else {
                                    continue;
                                };
                                if let Some(u) = hit.cover_url
                                    && !out.iter().any(|c| c.url == u)
                                {
                                    out.push(verify::measure_cover("hasheous", u));
                                }
                                if let Some(name) = hit.signature_name
                                    && let Some(u) = verify::libretro_boxart_url(system, &name)
                                    && !out.iter().any(|c| c.url == u)
                                {
                                    out.push(verify::measure_cover("libretro", u));
                                }
                                break;
                            }
                            for url in verify::libretro_title_urls(system, &title) {
                                if !out.iter().any(|c| c.url == url) {
                                    out.push(verify::measure_cover("libretro", url));
                                }
                            }
                            // Keep the failures when nothing landed: "tried and
                            // missed" is a different answer from "never looked".
                            if out.iter().any(|c| c.error.is_none()) {
                                out.retain(|c| c.error.is_none() || c.source == "staged");
                            }
                            out
                        }),
                        move |candidates| {
                            let mut lines = Vec::new();
                            for c in &candidates {
                                let size = match c.dimensions {
                                    Some((w, h)) => format!("{w}x{h}"),
                                    None => "?".to_owned(),
                                };
                                match &c.error {
                                    Some(e) => {
                                        lines.push(format!("{}: {} — {e}", c.source, c.url));
                                    }
                                    None => lines.push(format!(
                                        "{}: {size}, {} bytes — {}",
                                        c.source, c.bytes, c.url
                                    )),
                                }
                            }
                            if lines.is_empty() {
                                lines.push("no cover candidates found".to_owned());
                            }
                            lines.push(String::new());
                            lines.push(
                                "Download and LOOK at the one you keep: same-size art may be \
                                 another platform's box, and a smaller image is often the same \
                                 scan cropped free of its platform banner."
                                    .to_owned(),
                            );
                            Message::ToolText {
                                text: lines.join("\n"),
                                reply: reply.clone(),
                            }
                        },
                    );
                }
                if call.name == "wait_for_action" {
                    match self.action_events.pop_front() {
                        Some(event) => {
                            let _ = call.reply.send(text_result(event));
                        }
                        None => self.action_waiters.push(call.reply),
                    }
                    return Task::none();
                }
                let (body, task) = self.run_tool_tasked(&call.name, &call.args);
                let _ = call.reply.send(body);
                return task;
            }
            Message::Play(source) => {
                if let (Ok(db), Some(i)) = (&self.db, self.selected) {
                    let entry = &db.entries[i];
                    let key = entry.key();
                    let hint = match entry.tree {
                        TreeId::Gb => "verify.gb",
                        TreeId::Gbc => "verify.gbc",
                        TreeId::Sg1000 => "verify.sg",
                        TreeId::Vcs => "verify.a26",
                    };
                    let bytes = match &source {
                        BootSource::Cached(sha1) => self.rom_cache.get(sha1).cloned(),
                        BootSource::File(path) => std::fs::read(path).ok().map(std::sync::Arc::new),
                    };
                    let Some(bytes) = bytes else {
                        self.status = "no ROM bytes to play".to_owned();
                        return Task::none();
                    };
                    let sha1 = match &source {
                        BootSource::Cached(sha1) => sha1.clone(),
                        BootSource::File(_) => verify::sha1_hex(&bytes),
                    };
                    let (tv, cart) = entry.game.hints_for(&sha1);
                    let overdump =
                        entry.game.defect_for(&sha1) == Some(missingno_gamedb::Defect::Overdump);
                    let controllers = entry.game.controllers_for(&sha1);
                    self.stage_header_facts(i, &bytes);
                    match play::start(hint, &bytes, tv, cart, overdump, &controllers) {
                        Ok(session) => {
                            let events = session.events.clone();
                            // Full device simulation, as the emulator's Device
                            // mode: persistence plus the technology's overlay
                            // (LCD grid or CRT scanlines — never both).
                            let mut screen = missingno_iced::ScreenView::new();
                            screen.set_technology(session.technology);
                            screen.set_pixel_grid(true);
                            screen.set_scanlines(true);
                            self.playing = Some((key, session));
                            self.play_screen = Some(screen);
                            self.playing_sha1 = Some(sha1);
                            self.play_generation += 1;
                            let generation = self.play_generation;
                            return Task::perform(
                                smol::unblock(move || play::await_frame(&events)),
                                move |_| Message::PlayFrame(generation),
                            );
                        }
                        Err(e) => self.status = format!("play failed: {e}"),
                    }
                }
            }
            Message::PlayFrame(generation) => {
                if generation != self.play_generation {
                    return Task::none(); // a superseded session's loop
                }
                if let Some((_, session)) = &self.playing {
                    if let (Some(screen), Some(frame)) =
                        (self.play_screen.as_mut(), session.handle.latest_frame())
                    {
                        screen.apply(&frame);
                    }
                    let events = session.events.clone();
                    return Task::perform(
                        smol::unblock(move || play::await_frame(&events)),
                        move |alive| {
                            if alive {
                                Message::PlayFrame(generation)
                            } else {
                                Message::PlayEnded(generation)
                            }
                        },
                    );
                }
            }
            Message::PlayEnded(generation) => {
                if generation == self.play_generation {
                    self.playing = None;
                    self.play_screen = None;
                    self.playing_sha1 = None;
                }
            }
            Message::StopPlay => {
                self.playing = None;
                self.play_screen = None;
                self.playing_sha1 = None;
                self.play_generation += 1;
            }
            Message::Pad(control, pressed) => {
                if let Some((_, session)) = &self.playing {
                    session.set_control(control, pressed);
                }
            }
            Message::Gamepad(control, pressed) => {
                if let Some((_, session)) = &self.playing {
                    session.set_pad_control(control, pressed);
                }
            }
            Message::SwapPadJack => {
                if let Some((_, session)) = &mut self.playing {
                    session.swap_pad_jack();
                }
            }
            Message::Paddle(position) => {
                if let Some((_, session)) = &self.playing {
                    session.set_paddle(position);
                }
            }
            Message::PlayKey(key, shift, pressed) => {
                if let Some((_, session)) = &self.playing {
                    session.set_key(key, shift, pressed);
                }
            }
            Message::ToggleSwitch(index) => {
                if let Some((_, session)) = &mut self.playing
                    && let (Some(switch), Some(level)) = (
                        session.switches.get(index),
                        session.switch_levels.get_mut(index),
                    )
                {
                    *level = !*level;
                    let (control, high) = (ControlId::panel(switch.role), *level);
                    session.set_control(control, high);
                }
            }
            Message::TapSwitch(control) => {
                if let Some((_, session)) = &self.playing {
                    session.set_control(control, true);
                    return Task::perform(
                        smol::Timer::after(Duration::from_millis(120)),
                        move |_| Message::Pad(control, false),
                    );
                }
            }
            Message::Fetch => {
                if let (Ok(db), Some(i)) = (&self.db, self.selected) {
                    let entry = &db.entries[i];
                    let Some(url) = entry.game.download_url() else {
                        return Task::none();
                    };
                    let key = entry.key();
                    self.play_after_fetch = Some(key.clone());
                    self.verify_status
                        .insert(key.clone(), format!("fetching {url}…"));
                    return Task::perform(
                        smol::unblock(move || {
                            verify::fetch(&url).map(|b| (url, std::sync::Arc::new(b)))
                        }),
                        move |result| Message::Fetched(key.clone(), result),
                    );
                }
            }
            Message::Fetched(key, result) => match result {
                Ok((url, bytes)) => {
                    let sha1 = verify::sha1_hex(&bytes);
                    let size = bytes.len() as u64;
                    self.rom_cache.insert(sha1.clone(), bytes.clone());
                    self.fetched_sha1.insert(key.clone(), sha1.clone());
                    let sha1_for_play = sha1.clone();
                    if let Some(i) = self.find_entry(&key) {
                        self.stage_header_facts(i, &bytes);
                    }
                    let mut line = format!("{} bytes from {url}\nsha1 {sha1}", size);
                    if let Ok(db) = &mut self.db
                        && let Some(i) = db.entries.iter().position(|e| e.key() == key)
                    {
                        if db.entries[i].game.stage_artifact(&sha1, size) {
                            db.entries[i].dirty = true;
                            line.push_str(" — NEW, staged onto sourced release");
                        } else {
                            line.push_str(" — matches a known artifact");
                        }
                    }
                    self.verify_status.insert(key.clone(), line);
                    if self.play_after_fetch.as_deref() == Some(key.as_str()) {
                        self.play_after_fetch = None;
                        return Task::done(Message::Play(BootSource::Cached(sha1_for_play)));
                    }
                }
                Err(e) => {
                    self.verify_status.insert(key, e);
                    self.play_after_fetch = None;
                }
            },
            Message::ScanRoms => {
                if self.rom_dir.is_some() || self.collection_dir.is_some() {
                    let inbox = self.rom_dir.clone();
                    let collection = self.collection_dir.clone();
                    self.status = "scanning ROM folders…".to_owned();
                    return Task::perform(
                        smol::unblock(move || {
                            RomIndex::scan(inbox.as_deref(), collection.as_deref())
                                .map(std::sync::Arc::new)
                                .map_err(|e| e.to_string())
                        }),
                        Message::ScannedRoms,
                    );
                }
            }
            Message::ScannedRoms(result) => match result {
                Ok(index) => {
                    let (collection, inbox) = (index.collection, index.inbox);
                    self.rom_index = Some(index.clone());
                    let added = match &mut self.db {
                        Ok(db) => db.add_unmatched_roms(&index),
                        Err(_) => 0,
                    };
                    let dupes = index.duplicates_moved;
                    let mut parts = vec![format!("{collection} in collection · {inbox} in inbox")];
                    if dupes > 0 {
                        parts.push(format!("{dupes} inbox duplicate(s) set aside"));
                    }
                    if added > 0 {
                        parts.push(format!("{added} matched no manifest → new records"));
                    }
                    self.status = parts.join(" · ");
                }
                Err(e) => self.status = format!("scan failed: {e}"),
            },
            Message::FilterTree(TreeChoice(tree)) => {
                self.filter_tree = tree;
                self.selected = None;
            }
            Message::OnlyBacklog(v) => self.only_backlog = v,
            Message::OnlyFlagged(v) => self.only_flagged = v,
            Message::OnlyNew(v) => {
                self.only_new = v;
                self.selected = None;
            }
            Message::Search(s) => {
                self.search = s;
                self.selected = None;
            }
            Message::Select(index) => return self.select(index),
            Message::Enriched(key, result) => {
                self.enriching = false;
                match result {
                    Ok(Some(hit)) => {
                        let mut changed = Vec::new();
                        let found = self.find_entry(&key);
                        if let (Ok(db), Some(i)) = (&mut self.db, found) {
                            if let Some(url) = &hit.cover_url
                                && db.entries[i].game.add_cover(url)
                            {
                                changed.push("cover");
                                db.entries[i].dirty = true;
                            }
                            if let Some(url) = &hit.wikipedia_url {
                                db.entries[i].game.set_wikipedia(url);
                                changed.push("wikipedia link");
                                db.entries[i].dirty = true;
                            }
                        }
                        self.status = if changed.is_empty() {
                            format!("Hasheous knows \"{}\" but adds nothing new", hit.name)
                        } else {
                            format!("Hasheous ({}): staged {}", hit.name, changed.join(" + "))
                        };
                        if let Some(i) = self.find_entry(&key) {
                            return self.load_cover_task(i);
                        }
                    }
                    Ok(None) => self.status = "Hasheous: no match for this hash".to_owned(),
                    Err(e) => self.status = format!("Hasheous: {e}"),
                }
            }
            Message::CoverLoaded(url, handle) => match handle {
                Some(handle) => {
                    self.cover_failed.remove(&url);
                    self.cover_previews.insert(url, handle);
                }
                None => {
                    self.cover_failed.insert(url);
                }
            },
            Message::ToolText { text, reply } => {
                let _ = reply.send(text_result(text));
            }
            Message::ArtifactsVerified {
                key,
                results,
                reply,
            } => {
                // The lookup informs the session; nothing is written to the
                // manifest — the report in chat is the record.
                let mut lines = Vec::new();
                for (sha1, outcome) in &results {
                    let short = &sha1[..12];
                    match outcome {
                        verify::SigResult::Found { signature, game } => {
                            let evidence = signature.clone().unwrap_or_else(|| game.clone());
                            match verify::classify_signature(&evidence) {
                                Some(verify::SigFlag::Derived(reason)) => {
                                    self.session_marks
                                        .insert(sha1.clone(), format!("⚠ derived ({reason})"));
                                    lines.push(format!(
                                        "{short}… DERIVED ({reason}): {evidence} —                                          someone made this; judge and mark_mod"
                                    ));
                                }
                                Some(verify::SigFlag::Defective(reason)) => {
                                    self.session_marks
                                        .insert(sha1.clone(), format!("⚠ defective ({reason})"));
                                    lines.push(format!(
                                        "{short}… DEFECTIVE ({reason}): {evidence} —                                          label_artifact it, and if it fabricated a release                                          (wrong board from an overdump), move_artifact                                          into the real one"
                                    ));
                                }
                                None => {
                                    self.session_marks
                                        .insert(sha1.clone(), "✓ Hasheous".to_owned());
                                    let lower = evidence.to_lowercase();
                                    let suggest = if lower.contains("(prototype)")
                                        || lower.contains("(proto)")
                                    {
                                        " — a prototype build: consider split_release                                          (keep any working title it carries)"
                                    } else if lower.contains("(beta)") {
                                        " — a beta build: consider split_release"
                                    } else {
                                        ""
                                    };
                                    lines.push(format!("{short}… confirmed: {evidence}{suggest}"));
                                }
                            }
                        }
                        verify::SigResult::Unknown => {
                            self.session_marks
                                .insert(sha1.clone(), "? unknown".to_owned());
                            lines.push(format!("{short}… unknown to the signature database"))
                        }
                        verify::SigResult::Failed(e) => {
                            lines.push(format!("{short}… lookup failed: {e}"))
                        }
                    }
                }
                self.status = format!("verified {key}: {} dumps checked", results.len());
                let _ = reply.send(text_result(lines.join("\n")));
            }
            Message::ConfirmAndNext => {
                let visible = self.visible();
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    let (by, recommended) = (self.curator_name.clone(), self.recommend_next);
                    db.entries[i].game.stamp_curation(&by, recommended);
                    let confirmed_key = db.entries[i].key();
                    let confirmed_star = if recommended { " ★ recommended" } else { "" };
                    self.recommend_next = false;
                    match db.write_entry(i) {
                        Ok(()) => {
                            self.status = format!("confirmed {}", db.entries[i].key());
                            self.emit_action(format!("accepted {confirmed_key}{confirmed_star}"));
                        }
                        Err(e) => {
                            self.status = format!("write failed: {e}");
                            return Task::none();
                        }
                    }
                    // Advance to the next visible entry (the confirmed one drops
                    // out of a backlog-filtered list, so "same position" is next).
                    let pos = visible.iter().position(|&v| v == i);
                    let next = pos
                        .and_then(|p| {
                            visible
                                .get(p + 1)
                                .or_else(|| visible.get(p.saturating_sub(1)))
                        })
                        .copied();
                    self.selected = if self.only_backlog {
                        next
                    } else {
                        self.selected
                    };
                }
            }
            Message::Accept { recommend } => {
                self.recommend_next = recommend;
                if self.queue.is_empty() {
                    return Task::done(Message::ConfirmAndNext);
                }
                return self.accept_and_next();
            }
            Message::SkipNext => {
                if !self.queue.is_empty() {
                    return self.skip_and_next();
                }
            }
            Message::CloseRequested => {
                // Tear down in order on this thread — session first (its worker
                // and the cpal stream go quietly), then the socket — so the
                // process exits 0 and an attached agent sees a clean end.
                self.playing = None;
                self.play_screen = None;
                self._remote = None;
                return iced::window::latest().and_then(iced::window::close);
            }
            Message::OpenLink(url) => {
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            }
            Message::ToggleList => self.list_visible = !self.list_visible,
            Message::ResolveFlag(id) => {
                if let Ok(db) = &mut self.db {
                    db.flags.flags.retain(|f| f.id != id);
                    match db.save_flags() {
                        Ok(()) => self.status = format!("cleared flag #{id}"),
                        Err(e) => self.status = format!("flag save failed: {e}"),
                    }
                }
            }
        }
        Task::none()
    }

    /// Select an entry and kick a cover preview.
    fn select(&mut self, i: usize) -> Task<Message> {
        self.selected = Some(i);
        Task::batch([self.load_cover_task(i), self.auto_enrich_task(i)])
    }

    /// Look a selected entry up on Hasheous unprompted — once per session,
    /// only when it has a hash to ask about and no cover yet.
    fn auto_enrich_task(&mut self, i: usize) -> Task<Message> {
        let Ok(db) = &self.db else {
            return Task::none();
        };
        let entry = &db.entries[i];
        let key = entry.key();
        if !entry.game.covers().is_empty() || self.enrich_attempted.contains(&key) || self.enriching
        {
            return Task::none();
        }
        let Some(sha1) = entry.game.artifact_sha1s().into_iter().next() else {
            return Task::none();
        };
        self.enrich_attempted.insert(key.clone());
        self.enriching = true;
        Task::perform(
            smol::unblock(move || verify::hasheous_lookup(&sha1)),
            move |result| Message::Enriched(key.clone(), result),
        )
    }

    fn load_cover_task(&self, i: usize) -> Task<Message> {
        let Ok(db) = &self.db else {
            return Task::none();
        };
        let Some(url) = db.entries[i].game.covers().first().cloned() else {
            return Task::none();
        };
        if self.cover_previews.contains_key(&url) || self.cover_failed.contains(&url) {
            return Task::none();
        }
        let fetch_url = url.clone();
        Task::perform(
            smol::unblock(move || verify::fetch(&fetch_url).ok()),
            move |bytes| {
                Message::CoverLoaded(
                    url.clone(),
                    bytes.map(iced::widget::image::Handle::from_bytes),
                )
            },
        )
    }

    fn start_playtest_for(&mut self, i: usize) -> Task<Message> {
        // Selecting fetches the cover preview; it has to ride along with the
        // boot, since a queued playtest never goes through a list click.
        let select_task = self.select(i);
        let Ok(db) = &self.db else {
            return select_task;
        };
        let entry = &db.entries[i];
        let key = entry.key();
        let boot = 'boot: {
            if let Some(index) = &self.rom_index {
                for sha1 in entry.game.artifact_sha1s() {
                    if let Some(rom) = index.by_sha1.get(&sha1) {
                        break 'boot Task::done(Message::Play(BootSource::File(rom.path.clone())));
                    }
                }
            }
            if let Some(sha1) = self.fetched_sha1.get(&key) {
                break 'boot Task::done(Message::Play(BootSource::Cached(sha1.clone())));
            }
            if entry.game.download_url().is_some() {
                break 'boot Task::done(Message::Fetch);
            }
            self.status = format!("{key}: no local dump and no download source");
            Task::none()
        };
        Task::batch([select_task, boot])
    }

    /// Move the accepted entry's inbox ROMs into the collection
    /// (`<collection>/<tree>/<slug>/`), so future scans match against them.
    fn archive_accepted(&mut self, i: usize) -> usize {
        let (Some(collection), Some(index), Ok(db)) =
            (&self.collection_dir, &mut self.rom_index, &self.db)
        else {
            return 0;
        };
        let entry = &db.entries[i];
        // A game dump lands in <slug>/, a mod's dump in <slug>/mods/.
        let mut sha1s: Vec<(String, bool)> = entry
            .game
            .artifact_sha1s()
            .into_iter()
            .map(|sha1| (sha1, false))
            .collect();
        for m in 0..entry.game.mod_lines().len() {
            sha1s.extend(
                entry
                    .game
                    .mod_artifacts(m)
                    .into_iter()
                    .map(|(sha1, _, _)| (sha1, true)),
            );
        }
        let slug_dir = collection.join(entry.tree.dir()).join(&entry.slug);
        let index = std::sync::Arc::make_mut(index);
        let mut moved = 0;
        for (sha1, is_mod) in sha1s {
            let Some(rom) = index.by_sha1.get_mut(&sha1) else {
                continue;
            };
            if rom.home != verify::RomHome::Inbox {
                continue;
            }
            let target_dir = if is_mod {
                slug_dir.join("mods")
            } else {
                slug_dir.clone()
            };
            if std::fs::create_dir_all(&target_dir).is_err() {
                break;
            }
            let target = target_dir.join(rom.path.file_name().unwrap_or(rom.path.as_os_str()));
            match verify::move_file(&rom.path, &target) {
                Ok(()) => {
                    rom.path = target;
                    rom.home = verify::RomHome::Collection;
                    moved += 1;
                }
                Err(error) => {
                    self.status =
                        format!("collection move FAILED for {}: {error}", rom.path.display());
                }
            }
        }
        moved
    }

    fn accept_and_next(&mut self) -> Task<Message> {
        let Some(i) = self.selected else {
            return Task::none();
        };
        let (key, recommended) = {
            let Ok(db) = &mut self.db else {
                return Task::none();
            };
            let (by, recommended) = (self.curator_name.clone(), self.recommend_next);
            db.entries[i].game.stamp_curation(&by, recommended);
            self.recommend_next = false;
            let key = db.entries[i].key();
            if let Err(e) = db.write_entry(i) {
                self.status = format!("write failed: {e}");
                return Task::none();
            }
            (key, recommended)
        };
        let moved = self.archive_accepted(i);
        self.status = if moved > 0 {
            format!("accepted {key} · {moved} ROM(s) moved to collection")
        } else {
            format!("accepted {key}")
        };
        if self.queue.front() == Some(&key) {
            self.queue.pop_front();
        }
        self.playing = None;
        self.play_screen = None;
        let star = if recommended { " ★ recommended" } else { "" };
        while let Some(next_key) = self.queue.front().cloned() {
            match self.find_entry(&next_key) {
                Some(next) => {
                    self.emit_action(format!(
                        "accepted {key}{star}; now playing {next_key} ({} left in queue)",
                        self.queue.len()
                    ));
                    return self.start_playtest_for(next);
                }
                None => {
                    self.status = format!("queued {next_key} not found — skipped");
                    self.queue.pop_front();
                }
            }
        }
        self.emit_action(format!("accepted {key}{star}; queue is empty"));
        Task::none()
    }

    /// Advance past the current queued entry without stamping a curation —
    /// the entry is already curated and needs no re-blessing. Mirrors
    /// accept_and_next's queue walk, minus the stamp and the write.
    fn skip_and_next(&mut self) -> Task<Message> {
        let Some(key) = self
            .selected
            .and_then(|i| self.db.as_ref().ok().map(|db| db.entries[i].key()))
        else {
            return Task::none();
        };
        self.status = format!("skipped {key}");
        if self.queue.front() == Some(&key) {
            self.queue.pop_front();
        }
        self.playing = None;
        self.play_screen = None;
        while let Some(next_key) = self.queue.front().cloned() {
            match self.find_entry(&next_key) {
                Some(next) => {
                    self.emit_action(format!(
                        "skipped {key}; now playing {next_key} ({} left in queue)",
                        self.queue.len()
                    ));
                    return self.start_playtest_for(next);
                }
                None => {
                    self.status = format!("queued {next_key} not found — skipped");
                    self.queue.pop_front();
                }
            }
        }
        self.emit_action(format!("skipped {key}; queue is empty"));
        Task::none()
    }

    /// Read the GB-family header from ROM bytes and stage its facts (fills
    /// unknown enhancement flags and the mapper; conflicts go to the status).
    fn stage_header_facts(&mut self, i: usize, rom: &[u8]) {
        let Ok(db) = &mut self.db else { return };
        if matches!(db.entries[i].tree, TreeId::Sg1000 | TreeId::Vcs) {
            return;
        }
        let Some(header) = verify::gb_header(rom) else {
            return;
        };
        let (staged, conflicts) = db.entries[i].game.stage_gb_header(&header);
        if !staged.is_empty() {
            db.entries[i].dirty = true;
        }
        let key = db.entries[i].key();
        let mut lines = Vec::new();
        if !staged.is_empty() {
            lines.push(format!("header: staged {}", staged.join(", ")));
        }
        if !conflicts.is_empty() {
            lines.push(format!("header CONFLICTS: {}", conflicts.join("; ")));
        }
        if !lines.is_empty() {
            let line = lines.join(" · ");
            self.verify_status
                .entry(key)
                .and_modify(|s| {
                    s.push('\n');
                    s.push_str(&line);
                })
                .or_insert(line);
        }
    }

    /// Migrate every key-indexed piece of session state after a slug rename.
    /// An absorbed entry's key stops existing: point what referenced it at the
    /// survivor, without queueing that survivor twice.
    fn merge_keys(&mut self, gone: &str, survivor: &str) {
        self.rekey_entry(gone, survivor);
        let mut seen = false;
        self.queue.retain(|key| {
            if key != survivor {
                return true;
            }
            let first = !seen;
            seen = true;
            first
        });
    }

    /// Keep the collection folder tracking the slug after a rename; the
    /// accepted entry's ROMs live under `<collection>/<tree>/<slug>/`.
    fn move_collection_dir(&self, old_key: &str, new_key: &str) -> String {
        let Some(root) = &self.collection_dir else {
            return String::new();
        };
        let from = root.join(old_key);
        if !from.is_dir() {
            return String::new();
        }
        let to = root.join(new_key);
        match std::fs::rename(&from, &to) {
            Ok(()) => format!(", collection folder moved to {}", to.display()),
            Err(e) => format!(", but moving {} failed: {e}", from.display()),
        }
    }

    fn rekey_entry(&mut self, old_key: &str, new_key: &str) {
        for key in self.queue.iter_mut().filter(|k| *k == old_key) {
            *key = new_key.to_owned();
        }
        if let Some((key, _)) = &mut self.playing
            && key == old_key
        {
            *key = new_key.to_owned();
        }
        if let Some(v) = self.verify_status.remove(old_key) {
            self.verify_status.insert(new_key.to_owned(), v);
        }
        if let Some(v) = self.fetched_sha1.remove(old_key) {
            self.fetched_sha1.insert(new_key.to_owned(), v);
        }
        if self.enrich_attempted.remove(old_key) {
            self.enrich_attempted.insert(new_key.to_owned());
        }
    }

    /// Tell a waiting agent (or queue for the next wait) what the human did.
    fn emit_action(&mut self, event: String) {
        // Also append to a per-process event log a client can `tail -f`, so a
        // chat agent picks up clicks without parking on the wait_for_action
        // long-poll.
        append_event_log(&event);
        if self.action_waiters.is_empty() {
            self.action_events.push_back(event);
            if self.action_events.len() > 20 {
                self.action_events.pop_front();
            }
        } else {
            for waiter in self.action_waiters.drain(..) {
                let _ = waiter.send(text_result(event.clone()));
            }
        }
    }

    fn find_entry(&self, key: &str) -> Option<usize> {
        let Ok(db) = &self.db else { return None };
        db.entries.iter().position(|e| e.key() == key)
    }

    /// Tools whose effects need follow-up work return it alongside the reply.
    fn run_tool_tasked(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> (serde_json::Value, Task<Message>) {
        let str_arg = |k: &str| args.get(k).and_then(serde_json::Value::as_str);
        match name {
            "extend_queue" => {
                let Some(keys) = args.get("keys").and_then(serde_json::Value::as_array) else {
                    return (error_result("missing keys array"), Task::none());
                };
                let mut added = Vec::new();
                let mut missing = Vec::new();
                for key in keys.iter().filter_map(serde_json::Value::as_str) {
                    if self.find_entry(key).is_none() {
                        missing.push(key.to_owned());
                    } else if self.queue.iter().any(|q| q == key) {
                        continue;
                    } else {
                        self.queue.push_back(key.to_owned());
                        added.push(key.to_owned());
                    }
                }
                let mut note = format!(
                    "appended {} game(s); {} now queued, playtest untouched",
                    added.len(),
                    self.queue.len()
                );
                if !missing.is_empty() {
                    note.push_str(&format!("; not found: {missing:?}"));
                }
                (text_result(note), Task::none())
            }
            "queue_games" => {
                let Some(keys) = args.get("keys").and_then(serde_json::Value::as_array) else {
                    return (error_result("missing keys array"), Task::none());
                };
                let mut queued = Vec::new();
                let mut missing = Vec::new();
                for key in keys.iter().filter_map(serde_json::Value::as_str) {
                    if self.find_entry(key).is_some() {
                        queued.push(key.to_owned());
                    } else {
                        missing.push(key.to_owned());
                    }
                }
                if queued.is_empty() {
                    return (
                        error_result(format!("no valid keys (missing: {missing:?})")),
                        Task::none(),
                    );
                }
                self.queue = queued.iter().cloned().collect();
                self.list_visible = false;
                let first = self.find_entry(&queued[0]).expect("validated above");
                let task = self.start_playtest_for(first);
                let mut note = format!("queued {} game(s); starting {}", queued.len(), queued[0]);
                if !missing.is_empty() {
                    note.push_str(&format!("; not found: {missing:?}"));
                }
                (text_result(note), task)
            }
            "play_game" => {
                let Some(key) = str_arg("key") else {
                    return (error_result("missing key"), Task::none());
                };
                match self.find_entry(key) {
                    Some(i) => {
                        let task = self.start_playtest_for(i);
                        (text_result(format!("starting playtest of {key}")), task)
                    }
                    None => (error_result(format!("no entry {key}")), Task::none()),
                }
            }
            "select_game" => {
                let body = self.run_tool(name, args);
                let task = str_arg("key")
                    .and_then(|key| self.find_entry(key))
                    .map(|i| self.select(i))
                    .unwrap_or_else(Task::none);
                (body, task)
            }
            _ => {
                // Anything that can change which entry is shown, or what cover
                // it carries, has to fetch it — the plain path returns no task.
                let body = self.run_tool(name, args);
                let task = if matches!(name, "update_game" | "merge_game" | "rename_game") {
                    self.selected
                        .map(|i| self.load_cover_task(i))
                        .unwrap_or_else(Task::none)
                } else {
                    Task::none()
                };
                (body, task)
            }
        }
    }

    fn run_tool(&mut self, name: &str, args: &serde_json::Value) -> serde_json::Value {
        let result = self.run_tool_inner(name, args);
        // Read-only tools would only pad the session record.
        const READ_ONLY: &[&str] = &[
            "status",
            "search_games",
            "get_game",
            "queue_status",
            "local_matches",
            "find_duplicates",
            "related_entries",
            "dump_info",
            "session_changes",
            "list_flags",
            "identify_dump",
            "cover_candidates",
        ];
        if !READ_ONLY.contains(&name) && result["isError"] != serde_json::Value::Bool(true) {
            let subject = args
                .get("key")
                .or_else(|| args.get("sha1"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-");
            let summary = result["content"][0]["text"].as_str().unwrap_or("");
            let summary = summary.lines().next().unwrap_or("");
            self.session_log
                .push(format!("{name} {subject}: {summary}"));
        }
        result
    }

    fn run_tool_inner(&mut self, name: &str, args: &serde_json::Value) -> serde_json::Value {
        let str_arg = |k: &str| args.get(k).and_then(serde_json::Value::as_str);
        // Names match list_flags' lowercased Debug spelling of the variant.
        fn flag_kind_from_str(s: &str) -> Option<missingno_gamedb::FlagKind> {
            use missingno_gamedb::FlagKind::*;
            Some(match s.to_lowercase().as_str() {
                "nearmisstitles" => NearMissTitles,
                "reviewcandidatefamilies" => ReviewCandidateFamilies,
                "leftover" => Leftover,
                "unknownqualifier" => UnknownQualifier,
                "conflictingfield" => ConflictingField,
                "retiredhash" => RetiredHash,
                "emulationincompatibility" => EmulationIncompatibility,
                "custom" => Custom,
                _ => return None,
            })
        }
        match name {
            "status" => {
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                text_result(format!(
                    "backlog: gb {}, gbc {}, sg1000 {}, vcs {} · open flags: {} · uncommitted files: {}",
                    db.backlog_count(TreeId::Gb),
                    db.backlog_count(TreeId::Gbc),
                    db.backlog_count(TreeId::Sg1000),
                    db.backlog_count(TreeId::Vcs),
                    db.flags.open().count(),
                    db.uncommitted,
                ))
            }
            "search_games" => {
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                let query = str_arg("query").unwrap_or("").to_lowercase();
                let tree = str_arg("tree");
                let backlog_only = args
                    .get("backlog_only")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let limit = args
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(25) as usize;
                let mut lines = Vec::new();
                for e in &db.entries {
                    if tree.is_some_and(|t| t != e.tree.dir()) {
                        continue;
                    }
                    if backlog_only && e.game.curated() {
                        continue;
                    }
                    if !e.game.title().to_lowercase().contains(&query) && !e.slug.contains(&query) {
                        continue;
                    }
                    lines.push(format!(
                        "{} — {}{}",
                        e.key(),
                        e.game.title(),
                        if !e.game.curated() { "" } else { " [curated]" }
                    ));
                    if lines.len() >= limit {
                        break;
                    }
                }
                text_result(if lines.is_empty() {
                    "no matches".to_owned()
                } else {
                    lines.join("\n")
                })
            }
            "get_game" => {
                let Some(key) = str_arg("key") else {
                    return error_result("missing key");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                let entry = &db.entries[i];
                let manifest = entry
                    .game
                    .to_ron_string()
                    .unwrap_or_else(|e| format!("<serialize error: {e}>"));
                let flags: Vec<String> = db
                    .flags
                    .open()
                    .filter(|f| f.subject.contains(&entry.key()))
                    .map(|f| format!("flag #{} [{:?}]: {}", f.id, f.kind, f.note))
                    .collect();
                text_result(format!("{manifest}\n{}", flags.join("\n")))
            }
            "update_game" => {
                let Some(key) = str_arg("key").map(str::to_owned) else {
                    return error_result("missing key");
                };
                let Some(i) = self.find_entry(&key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Some(set) = args.get("set").and_then(serde_json::Value::as_object) else {
                    return error_result("missing set object");
                };
                let mut staged_links = Vec::new();
                if let Some(links) = set.get("links").and_then(serde_json::Value::as_array) {
                    for link in links {
                        let (Some(name), Some(url)) = (
                            link.get("name").and_then(serde_json::Value::as_str),
                            link.get("url").and_then(serde_json::Value::as_str),
                        ) else {
                            return error_result("each link needs name and url");
                        };
                        let Some(kind) = link.get("link_type").and_then(serde_json::Value::as_str)
                        else {
                            return error_result(format!("link {name:?} needs a link_type"));
                        };
                        let link_type = match db::parse_link_type(kind) {
                            Ok(link_type) => link_type,
                            Err(e) => return error_result(e),
                        };
                        let mut languages = Vec::new();
                        if let Some(langs) =
                            link.get("languages").and_then(serde_json::Value::as_array)
                        {
                            for lang in langs {
                                let Some(s) = lang.as_str() else {
                                    return error_result(format!(
                                        "link {name:?} languages must be strings"
                                    ));
                                };
                                match db::parse_language(s) {
                                    Ok(language) => languages.push(language),
                                    Err(e) => return error_result(e),
                                }
                            }
                        }
                        staged_links.push((name.to_owned(), url.to_owned(), link_type, languages));
                    }
                }
                let remove_links: Vec<String> = set
                    .get("remove_links")
                    .and_then(serde_json::Value::as_array)
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let entry = &mut db.entries[i];
                let mut applied = Vec::new();
                for (name, url, link_type, languages) in &staged_links {
                    entry
                        .game
                        .upsert_link(name, url, *link_type, languages.clone());
                }
                if !staged_links.is_empty() {
                    applied.push("links");
                }
                if !remove_links.is_empty() && entry.game.remove_links(&remove_links) {
                    applied.push("removed links");
                }
                for (field_name, field) in [
                    ("title", TextField::Title),
                    ("developer", TextField::Developer),
                    ("description", TextField::Description),
                ] {
                    if let Some(value) = set.get(field_name).and_then(serde_json::Value::as_str) {
                        entry.game.set_text_field(field, value.to_owned());
                        applied.push(field_name);
                    }
                }
                if let Some(covers) = set.get("covers").and_then(serde_json::Value::as_array) {
                    entry.game.set_covers(
                        covers
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                    );
                    applied.push("covers");
                }
                if let Some(url) = set.get("wikipedia").and_then(serde_json::Value::as_str) {
                    entry.game.set_wikipedia(url);
                    applied.push("wikipedia");
                }
                if let Some(adult) = set.get("adult").and_then(serde_json::Value::as_bool) {
                    entry.game.set_adult(adult);
                    applied.push("adult");
                }
                if let Some(publisher) = set.get("publisher").and_then(serde_json::Value::as_str) {
                    entry.game.set_release_publisher(0, publisher.to_owned());
                    applied.push("publisher");
                }
                if let Some(mapper) = set.get("mapper").and_then(serde_json::Value::as_str) {
                    if let Err(error) = entry.game.set_mapper(mapper) {
                        return error_result(error);
                    }
                    applied.push("mapper");
                }
                if let Some(cart) = set.get("cart_type").and_then(serde_json::Value::as_str) {
                    if let Err(error) = entry.game.set_cart_type(cart) {
                        return error_result(error);
                    }
                    applied.push("cart_type");
                }
                if let Some(kind) = set.get("kind").and_then(serde_json::Value::as_str) {
                    let Some(kind) = vocabulary::GAME_KINDS.lookup_ignoring_case(kind) else {
                        return error_result(
                            vocabulary::GAME_KINDS.unknown(&kind.to_ascii_lowercase()),
                        );
                    };
                    entry.game.set_kind(kind);
                    applied.push("kind");
                }
                if applied.is_empty() {
                    return error_result("no recognized fields in set");
                }
                entry.dirty = true;
                self.selected = Some(i);
                // Staged text lives in the manifest, not just in memory: an
                // agent's research must survive the window closing.
                if let Ok(db) = &mut self.db
                    && let Err(e) = db.write_entry(i)
                {
                    return error_result(format!("staged, but writing {key} failed: {e}"));
                }
                text_result(format!(
                    "staged {} on {key} (uncommitted until the curator confirms)",
                    applied.join(", ")
                ))
            }
            "merge_game" => {
                let (Some(key), Some(from)) = (str_arg("key"), str_arg("from")) else {
                    return error_result("missing key or from");
                };
                let (Some(target), Some(source)) = (self.find_entry(key), self.find_entry(from))
                else {
                    return error_result(format!("no entry {key} or {from}"));
                };
                let (target_key, source_key) = (key.to_owned(), from.to_owned());
                let merged = {
                    let Ok(db) = &mut self.db else {
                        return error_result("db not loaded");
                    };
                    db.merge_entry(target, source)
                };
                match merged {
                    Ok(message) => {
                        // The absorbed key must stop naming a queue slot, and
                        // `selected` indexes a vec that just lost an element.
                        self.merge_keys(&source_key, &target_key);
                        // Tidying some other game must not take the window away
                        // from the one being playtested: follow the merge only
                        // when the human was already looking at what it touched.
                        let playing = self.playing.as_ref().map(|(key, _)| key.clone());
                        let follow = playing
                            .as_deref()
                            .is_none_or(|key| key == target_key || key == source_key);
                        self.selected = if follow {
                            self.find_entry(&target_key)
                        } else {
                            playing.as_deref().and_then(|key| self.find_entry(key))
                        };
                        text_result(message)
                    }
                    Err(e) => error_result(e),
                }
            }
            "split_game" => {
                let (Some(key), Some(title)) = (str_arg("key"), str_arg("title")) else {
                    return error_result("missing key or title");
                };
                let Some(release_index) = args
                    .get("release_index")
                    .and_then(serde_json::Value::as_u64)
                    .map(|i| i as usize)
                else {
                    return error_result("missing release_index");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let slug = str_arg("slug").map(str::to_owned);
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                match db.split_game(i, release_index, title, slug.as_deref()) {
                    // The split appends, so no index the window holds moves —
                    // the playtest stays on the entry the release left.
                    Ok(new_key) => text_result(format!(
                        "release {release_index} of {key} split out → {new_key}"
                    )),
                    Err(e) => error_result(e),
                }
            }
            "rename_game" => {
                let (Some(key), Some(new_slug)) = (str_arg("key"), str_arg("new_slug")) else {
                    return error_result("missing key or new_slug");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let old_key = key.to_owned();
                let renamed = {
                    let Ok(db) = &mut self.db else {
                        return error_result("db not loaded");
                    };
                    db.rename_entry(i, new_slug)
                };
                match renamed {
                    Ok(new_key) => {
                        self.rekey_entry(&old_key, &new_key);
                        // The collection folder is named for the slug, so it
                        // has to follow or the entry's ROMs are orphaned.
                        let moved = self.move_collection_dir(&old_key, &new_key);
                        text_result(format!(
                            "{old_key} renamed → {new_key}{moved}; use the new key from now on"
                        ))
                    }
                    Err(e) => error_result(e),
                }
            }
            "local_matches" => {
                let Some(index) = &self.rom_index else {
                    return error_result(
                        "no ROM dir scanned — the human must press Scan ROM dir first",
                    );
                };
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                let backlog_only = args
                    .get("backlog_only")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                let limit = args
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(50) as usize;
                let lines: Vec<String> = db
                    .entries
                    .iter()
                    .filter(|e| !backlog_only || !e.game.curated())
                    .filter(|e| {
                        e.game
                            .artifact_sha1s()
                            .iter()
                            .any(|sha1| index.by_sha1.contains_key(sha1))
                    })
                    .take(limit)
                    .map(|e| format!("{} — {}", e.key(), e.game.title()))
                    .collect();
                text_result(if lines.is_empty() {
                    "no local ROM matches".to_owned()
                } else {
                    lines.join("\n")
                })
            }
            "mark_mod" => {
                let (Some(key), Some(sha1)) = (str_arg("key"), str_arg("sha1")) else {
                    return error_result("missing key or sha1");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let category = match str_arg("category") {
                    Some(c) => match db::parse_mod_category(c) {
                        Ok(c) => c,
                        Err(e) => return error_result(e),
                    },
                    None => missingno_gamedb::ModCategory::ContentChange,
                };
                let title = str_arg("title").map(str::to_owned);
                let base = str_arg("base_sha1").map(str::to_owned);
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let homepage = match mod_link(args) {
                    Ok(link) => link,
                    Err(e) => return error_result(e),
                };
                match db.mark_mod(i, sha1, title, category, base, homepage) {
                    Ok(destination) => {
                        text_result(format!("{sha1} moved out of {key}'s dumps → {destination}"))
                    }
                    Err(e) => error_result(e),
                }
            }
            "update_mod" => {
                let (Some(key), Some(mod_name)) = (str_arg("key"), str_arg("mod")) else {
                    return error_result("missing key or mod (the mod's current name)");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Some(set) = args.get("set").and_then(serde_json::Value::as_object) else {
                    return error_result("missing set object");
                };
                let set_str = |k: &str| set.get(k).and_then(serde_json::Value::as_str);
                // Validate everything before touching anything.
                let category = match set_str("category") {
                    Some(c) => Some(match db::parse_mod_category(c) {
                        Ok(c) => c,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                let date = match set_str("date") {
                    Some(d) => Some(match d.parse::<missingno_gamedb::ReleaseDate>() {
                        Ok(d) => d,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let base = match set_str("base_sha1") {
                    Some("none") => Some(None),
                    Some(base) => {
                        let base = base.to_ascii_lowercase();
                        let game = &db.entries[i].game;
                        if !game.artifact_sha1s().contains(&base)
                            && !game.mod_artifact_sha1s().contains(&base)
                        {
                            return error_result(format!(
                                "base_sha1 {base} is not an artifact of {key}"
                            ));
                        }
                        match base.parse::<missingno_gamedb::Sha1>() {
                            Ok(sha1) => Some(Some(sha1)),
                            Err(e) => return error_result(e),
                        }
                    }
                    None => None,
                };
                if !db.entries[i].game.mod_names().iter().any(|n| n == mod_name) {
                    return error_result(format!(
                        "no mod named {mod_name:?} on {key}; mods: {:?}",
                        db.entries[i].game.mod_names()
                    ));
                }
                let release_index = set
                    .get("release_index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let rename = set_str("name").map(str::to_owned);
                let author = set_str("author").map(str::to_owned);
                let label = set_str("label").map(str::to_owned);
                let link = match mod_link(&serde_json::Value::Object(set.clone())) {
                    Ok(link) => link,
                    Err(e) => return error_result(e),
                };
                let tv_format = match set_str("tv_format") {
                    Some(f) => Some(match db::parse_tv_format(f) {
                        Ok(f) => f,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                let controllers = match set.get("controllers").and_then(serde_json::Value::as_array)
                {
                    Some(list) => {
                        let mut parsed = Vec::with_capacity(list.len());
                        for value in list {
                            let Some(name) = value.as_str() else {
                                return error_result("controllers must be strings");
                            };
                            match db::parse_controller(name) {
                                Ok(controller) => parsed.push(controller),
                                Err(e) => return error_result(e),
                            }
                        }
                        Some(parsed)
                    }
                    None => None,
                };
                let mut applied = match db.entries[i].game.update_mod(
                    mod_name,
                    crate::db::ModEdits {
                        name: rename,
                        category,
                        author,
                        link,
                        release_index,
                        base_sha1: base,
                        label,
                        date,
                    },
                ) {
                    Some(applied) => applied,
                    None => return error_result(format!("mod {mod_name:?} vanished mid-edit")),
                };
                // A conversion often exists precisely to change these.
                if let Some(format) = tv_format {
                    if db.entries[i]
                        .game
                        .set_mod_tv_format(mod_name, release_index, format)
                    {
                        applied.push("tv_format");
                    } else {
                        return error_result("tv_format applies to VCS mods only");
                    }
                }
                if let Some(wanted) = controllers {
                    if db.entries[i]
                        .game
                        .set_mod_controllers(mod_name, release_index, wanted)
                    {
                        applied.push("controllers");
                    } else {
                        return error_result("controllers apply to VCS mods only");
                    }
                }
                if applied.is_empty() {
                    error_result("no recognized fields in set")
                } else {
                    db.entries[i].dirty = true;
                    if let Err(e) = db.write_entry(i) {
                        return error_result(format!("staged, but writing {key} failed: {e}"));
                    }
                    text_result(format!("updated mod on {key}: {}", applied.join(", ")))
                }
            }
            "split_release" => {
                let (Some(key), Some(sha1), Some(status)) =
                    (str_arg("key"), str_arg("sha1"), str_arg("status"))
                else {
                    return error_result("missing key, sha1, or status");
                };
                let status = match db::parse_release_status(status) {
                    Ok(status) => status,
                    Err(e) => return error_result(e),
                };
                let date = match str_arg("date") {
                    Some(d) => Some(match d.parse::<missingno_gamedb::ReleaseDate>() {
                        Ok(d) => d,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let title = str_arg("title").map(str::to_owned);
                let label = str_arg("label").map(str::to_owned);
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                match db.split_release(i, sha1, status, title, label, date) {
                    Ok(()) => text_result(format!(
                        "{sha1} moved into its own {status:?} release of {key}"
                    )),
                    Err(e) => error_result(e),
                }
            }
            "update_release" => {
                let (Some(key), Some(index)) = (
                    str_arg("key"),
                    args.get("release_index")
                        .and_then(serde_json::Value::as_u64),
                ) else {
                    return error_result("missing key or release_index");
                };
                let Some(set) = args.get("set").and_then(serde_json::Value::as_object) else {
                    return error_result("missing set object");
                };
                let set_str = |k: &str| set.get(k).and_then(serde_json::Value::as_str);
                let status = match set_str("status") {
                    Some(s) => Some(match db::parse_release_status(s) {
                        Ok(s) => s,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                let date = match set_str("date") {
                    Some("") => Some(None),
                    Some(d) => Some(Some(match d.parse::<missingno_gamedb::ReleaseDate>() {
                        Ok(d) => d,
                        Err(e) => return error_result(e),
                    })),
                    None => None,
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let title = set_str("title").map(str::to_owned);
                let label = set_str("label").map(str::to_owned);
                let publisher = set_str("publisher").map(str::to_owned);
                let regions = match set.get("regions").and_then(serde_json::Value::as_array) {
                    Some(list) => {
                        let mut parsed = Vec::with_capacity(list.len());
                        for value in list {
                            let Some(name) = value.as_str() else {
                                return error_result("regions must be strings");
                            };
                            match db::parse_region(name) {
                                Ok(region) => parsed.push(region),
                                Err(e) => return error_result(e),
                            }
                        }
                        Some(parsed)
                    }
                    None => None,
                };
                let languages = match set.get("languages").and_then(serde_json::Value::as_array) {
                    Some(list) => {
                        let mut parsed = Vec::with_capacity(list.len());
                        for value in list {
                            let Some(name) = value.as_str() else {
                                return error_result("languages must be strings");
                            };
                            match db::parse_language(name) {
                                Ok(language) => parsed.push(language),
                                Err(e) => return error_result(e),
                            }
                        }
                        Some(parsed)
                    }
                    None => None,
                };
                let tv_format = match set_str("tv_format") {
                    Some(f) => Some(match db::parse_tv_format(f) {
                        Ok(f) => f,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                let controllers = match set.get("controllers").and_then(serde_json::Value::as_array)
                {
                    Some(list) => {
                        let mut parsed = Vec::with_capacity(list.len());
                        for value in list {
                            let Some(name) = value.as_str() else {
                                return error_result("controllers must be strings");
                            };
                            match db::parse_controller(name) {
                                Ok(controller) => parsed.push(controller),
                                Err(e) => return error_result(e),
                            }
                        }
                        Some(parsed)
                    }
                    None => None,
                };
                let cart_type = set_str("cart_type").map(str::to_owned);
                if status.is_none()
                    && title.is_none()
                    && label.is_none()
                    && date.is_none()
                    && publisher.is_none()
                    && regions.is_none()
                    && languages.is_none()
                    && tv_format.is_none()
                    && controllers.is_none()
                    && cart_type.is_none()
                {
                    return error_result("no recognized fields in set");
                }
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let edits = db::ReleaseEdits {
                    status,
                    title,
                    label,
                    date,
                    publisher,
                    regions,
                    languages,
                };
                if db.entries[i].game.update_release(index as usize, edits) {
                    let tv = tv_format.is_some_and(|f| {
                        db.entries[i].game.set_release_tv_format(index as usize, f)
                    });
                    let ctrl = controllers.clone().is_some_and(|c| {
                        db.entries[i]
                            .game
                            .set_release_controllers(index as usize, c)
                    });
                    db.entries[i].dirty = true;
                    if let Some(code) = cart_type.as_deref()
                        && let Err(error) = db.entries[i]
                            .game
                            .set_release_cart_type(index as usize, code)
                    {
                        return error_result(error);
                    }
                    if tv_format.is_some() && !tv {
                        return error_result("tv_format applies to VCS releases only");
                    }
                    if controllers.is_some() && !ctrl {
                        return error_result("controllers apply to VCS releases only");
                    }
                    if let Err(e) = db.write_entry(i) {
                        return error_result(format!("staged, but writing {key} failed: {e}"));
                    }
                    text_result(format!("release {index} of {key} updated"))
                } else {
                    error_result(format!("{key} has no release {index}"))
                }
            }
            "attach_dump_to_mod" => {
                let (Some(key), Some(mod_name), Some(sha1)) =
                    (str_arg("key"), str_arg("mod"), str_arg("sha1"))
                else {
                    return error_result("missing key, mod, or sha1");
                };
                let as_version = args
                    .get("as_version")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let label = str_arg("label").map(str::to_owned);
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                match db.entries[i].game.attach_dump_to_mod(
                    mod_name,
                    &sha1.to_ascii_lowercase(),
                    as_version,
                    label,
                ) {
                    Ok(message) => {
                        db.entries[i].dirty = true;
                        if let Err(e) = db.write_entry(i) {
                            return error_result(format!(
                                "attached, but writing {key} failed: {e}"
                            ));
                        }
                        text_result(message)
                    }
                    Err(e) => error_result(e),
                }
            }
            "add_release" => {
                let Some(key) = str_arg("key") else {
                    return error_result("missing key");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let index = db.entries[i].game.add_release();
                db.entries[i].dirty = true;
                if let Err(e) = db.write_entry(i) {
                    return error_result(format!("added, but writing {key} failed: {e}"));
                }
                text_result(format!(
                    "release {index} added to {key}; set its fields with update_release"
                ))
            }
            "remove_release" => {
                let (Some(key), Some(index)) = (
                    str_arg("key"),
                    args.get("release_index")
                        .and_then(serde_json::Value::as_u64),
                ) else {
                    return error_result("missing key or release_index");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let discard = args
                    .get("discard_dumps")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                match db.entries[i].game.remove_release(index as usize, discard) {
                    Ok(()) => {
                        db.entries[i].dirty = true;
                        if let Err(e) = db.write_entry(i) {
                            return error_result(format!("removed, but writing {key} failed: {e}"));
                        }
                        text_result(format!("release {index} removed from {key}"))
                    }
                    Err(e) => error_result(e),
                }
            }
            "move_artifact" => {
                let (Some(key), Some(sha1), Some(to)) = (
                    str_arg("key"),
                    str_arg("sha1"),
                    args.get("to_release_index")
                        .and_then(serde_json::Value::as_u64),
                ) else {
                    return error_result("missing key, sha1, or to_release_index");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                match db.move_artifact(i, sha1, to as usize) {
                    Ok(true) => text_result(format!(
                        "{sha1} moved; its old release had nothing else and was removed                          (it only existed because of this dump)"
                    )),
                    Ok(false) => text_result(format!("{sha1} moved")),
                    Err(e) => error_result(e),
                }
            }
            "label_artifact" => {
                let (Some(key), Some(sha1)) = (str_arg("key"), str_arg("sha1")) else {
                    return error_result("missing key or sha1");
                };
                let label = str_arg("label");
                let defect = match str_arg("defect").map(db::parse_defect) {
                    Some(Ok(d)) => Some(d),
                    Some(Err(e)) => return error_result(e),
                    None => None,
                };
                if label.is_none() && defect.is_none() {
                    return error_result("provide a label, a defect, or both");
                }
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let mut applied = Vec::new();
                if let Some(label) = label {
                    if !db.entries[i].game.set_artifact_label(sha1, label) {
                        return error_result(format!("{sha1} is not an artifact of {key}"));
                    }
                    applied.push(format!("label {label:?}"));
                }
                if let Some(defect) = defect {
                    if !db.entries[i].game.set_artifact_defect(sha1, defect) {
                        return error_result(format!("{sha1} is not an artifact of {key}"));
                    }
                    applied.push(match defect {
                        Some(d) => format!("defect {}", d.label()),
                        None => "defect cleared".to_owned(),
                    });
                }
                db.entries[i].dirty = true;
                if let Err(e) = db.write_entry(i) {
                    return error_result(format!("staged, but writing {key} failed: {e}"));
                }
                text_result(format!("{sha1}: {}", applied.join(", ")))
            }
            "find_duplicates" => {
                let Some(key) = str_arg("key") else {
                    return error_result("missing key");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                let entry = &db.entries[i];
                let needles = entry.title_needles();
                let lines: Vec<String> = db
                    .entries
                    .iter()
                    .filter(|other| other.key() != entry.key())
                    .filter(|other| {
                        let other_norm = missingno_gamedb::normalized_title(other.game.title());
                        needles.contains(&other_norm)
                            || other.game.release_titles().iter().any(|rt| {
                                let rt = missingno_gamedb::normalized_title(rt);
                                needles.contains(&rt)
                            })
                    })
                    .map(|other| {
                        format!(
                            "{} — {}{}",
                            other.key(),
                            other.game.title(),
                            if other.game.curated() {
                                " [curated]"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect();
                text_result(if lines.is_empty() {
                    "no duplicate candidates".to_owned()
                } else {
                    format!("possible duplicates of {key}:\n{}", lines.join("\n"))
                })
            }
            "queue_status" => {
                let preview: Vec<&str> = self.queue.iter().take(10).map(String::as_str).collect();
                text_result(format!(
                    "current: {} · {} queued: {}",
                    self.queue.front().map(String::as_str).unwrap_or("(none)"),
                    self.queue.len(),
                    preview.join(", ")
                ))
            }
            "select_game" => {
                // Handled in run_tool_tasked: navigating has to fetch the
                // cover, which needs the task the plain path throws away.
                let Some(key) = str_arg("key") else {
                    return error_result("missing key");
                };
                match self.find_entry(key) {
                    Some(_) => text_result(format!("showing {key}")),
                    None => error_result(format!("no entry {key}")),
                }
            }
            "list_flags" => {
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                let kind = str_arg("kind").map(str::to_lowercase);
                let key = str_arg("key");
                let limit = args
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(25) as usize;
                let lines: Vec<String> = db
                    .flags
                    .open()
                    .filter(|f| {
                        kind.as_deref()
                            .is_none_or(|k| format!("{:?}", f.kind).to_lowercase() == k)
                    })
                    .filter(|f| key.is_none_or(|k| f.subject.iter().any(|s| s == k)))
                    .take(limit)
                    .map(|f| format!("#{} [{:?}] {}", f.id, f.kind, f.note))
                    .collect();
                text_result(if lines.is_empty() {
                    "no open flags match".to_owned()
                } else {
                    lines.join("\n")
                })
            }
            "retitle" => {
                let (Some(key), Some(title)) = (str_arg("key"), str_arg("title")) else {
                    return error_result("missing key or title");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let old_key = key.to_owned();
                {
                    let Ok(db) = &mut self.db else {
                        return error_result("db not loaded");
                    };
                    db.entries[i]
                        .game
                        .set_text_field(db::TextField::Title, title.to_owned());
                    db.entries[i].dirty = true;
                    if let Err(e) = db.write_entry(i) {
                        return error_result(format!("writing {old_key} failed: {e}"));
                    }
                }
                let slug_now = old_key.rsplit('/').next().unwrap_or_default();
                let Some(new_slug) = str_arg("slug").filter(|s| *s != slug_now) else {
                    return text_result(format!("{old_key} retitled to {title:?}"));
                };
                let renamed = {
                    let Ok(db) = &mut self.db else {
                        return error_result("db not loaded");
                    };
                    db.rename_entry(i, new_slug)
                };
                match renamed {
                    Ok(new_key) => {
                        self.rekey_entry(&old_key, &new_key);
                        let moved = self.move_collection_dir(&old_key, &new_key);
                        text_result(format!(
                            "{old_key} retitled to {title:?} and renamed → {new_key}{moved}; \
                             use the new key from now on"
                        ))
                    }
                    Err(e) => error_result(format!("retitled, but rename failed: {e}")),
                }
            }
            "related_entries" => {
                let Some(key) = str_arg("key") else {
                    return error_result("missing key");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                let found = db.related_entries(i);
                if found.is_empty() {
                    return text_result(format!("no related entries for {key}"));
                }
                let lines: Vec<String> = found
                    .iter()
                    .map(|(k, title, reason, curated)| {
                        format!(
                            "{k} — {title} [{reason}]{}",
                            if *curated { " (curated)" } else { "" }
                        )
                    })
                    .collect();
                text_result(format!(
                    "{}\n\nRead each title before folding it in: a hack names itself, not its \
                     base, so an adjacent slug may belong to a different game.",
                    lines.join("\n")
                ))
            }
            "dump_info" => {
                let Some(sha1) = str_arg("sha1") else {
                    return error_result("missing sha1");
                };
                let mut out = Vec::new();
                match self.db.as_ref().ok().and_then(|db| db.find_dump(sha1)) {
                    Some((key, title, what)) => {
                        out.push(format!("in db: {key} ({title}) — {what}"));
                    }
                    None => out.push("in db: not found".to_owned()),
                }
                match self
                    .rom_index
                    .as_ref()
                    .and_then(|index| index.by_sha1.get(sha1))
                {
                    Some(rom) => {
                        let size = std::fs::metadata(&rom.path).map(|m| m.len()).unwrap_or(0);
                        out.push(format!(
                            "local: {} ({size} bytes, {})",
                            rom.path.display(),
                            match rom.home {
                                verify::RomHome::Inbox => "inbox",
                                verify::RomHome::Collection => "collection",
                            }
                        ));
                    }
                    None => out.push(
                        "local: not in the scanned ROM dirs — size cannot be checked, so an \
                         overdump cannot be told from a variant by size alone"
                            .to_owned(),
                    ),
                }
                text_result(out.join("\n"))
            }
            "session_changes" => {
                if self.session_log.is_empty() {
                    return text_result("nothing staged this session");
                }
                text_result(self.session_log.join("\n"))
            }
            "resolve_flag" => {
                let Some(id) = args.get("id").and_then(serde_json::Value::as_u64) else {
                    return error_result("missing id");
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let before = db.flags.flags.len();
                db.flags.flags.retain(|f| f.id != id as u32);
                if db.flags.flags.len() == before {
                    return error_result(format!("no flag #{id}"));
                }
                match db.save_flags() {
                    Ok(()) => text_result(format!("flag #{id} cleared")),
                    Err(e) => error_result(format!("flag save failed: {e}")),
                }
            }
            "update_flag" => {
                let Some(id) = args.get("id").and_then(serde_json::Value::as_u64) else {
                    return error_result("missing id");
                };
                let kind = match str_arg("kind") {
                    None => None,
                    Some(k) => match flag_kind_from_str(k) {
                        Some(kind) => Some(kind),
                        None => return error_result(format!("unknown kind {k:?}")),
                    },
                };
                let note = str_arg("note").map(str::to_owned);
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let Some(flag) = db.flags.flags.iter_mut().find(|f| f.id == id as u32) else {
                    return error_result(format!("no flag #{id}"));
                };
                if let Some(kind) = kind {
                    flag.kind = kind;
                }
                if let Some(note) = note {
                    flag.note = note;
                }
                let kind = flag.kind;
                match db.save_flags() {
                    Ok(()) => text_result(format!("flag #{id} [{kind:?}] updated")),
                    Err(e) => error_result(format!("flag save failed: {e}")),
                }
            }
            "raise_flag" => {
                let (Some(key), Some(note)) = (str_arg("key"), str_arg("note")) else {
                    return error_result("missing key or note");
                };
                let kind = match str_arg("kind") {
                    None => missingno_gamedb::FlagKind::EmulationIncompatibility,
                    Some(k) => match flag_kind_from_str(k) {
                        Some(kind) => kind,
                        None => return error_result(format!("unknown kind {k:?}")),
                    },
                };
                if self.find_entry(key).is_none() {
                    return error_result(format!("no entry {key}"));
                }
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let id = db.flags.next_id();
                db.flags.flags.push(missingno_gamedb::Flag {
                    id,
                    kind,
                    subject: vec![key.to_owned()],
                    note: note.to_owned(),
                });
                match db.save_flags() {
                    Ok(()) => text_result(format!("raised flag #{id} [{kind:?}] on {key}")),
                    Err(e) => error_result(format!("flag save failed: {e}")),
                }
            }
            other => error_result(format!("unknown tool: {other}")),
        }
    }

    /// One dump's row: playing marker, short hash, label, defect badge, local
    /// filename, and a Play button that switches sessions directly.
    fn artifact_row(
        &self,
        sha1: &str,
        label: &str,
        defect: Option<missingno_gamedb::Defect>,
    ) -> Element<'_, Message> {
        let short = format!("{}…", &sha1[..12]);
        let local = self
            .rom_index
            .as_ref()
            .and_then(|index| index.by_sha1.get(sha1));
        let playable = local.is_some() || self.rom_cache.contains_key(sha1);
        let playing_this = self.playing_sha1.as_deref() == Some(sha1);
        let is_new = local.is_some_and(|rom| rom.home == verify::RomHome::Inbox);
        let mut line = row![].spacing(10).align_y(iced::Alignment::Center);
        line = line.push(
            text(if playing_this {
                format!("▶ {short}")
            } else {
                short
            })
            .size(12),
        );
        if is_new {
            line = line.push(container(text("NEW").size(13)).padding([2, 8]).style(
                |theme: &Theme| {
                    let palette = theme.extended_palette();
                    container::Style {
                        background: Some(palette.danger.strong.color.into()),
                        text_color: Some(palette.danger.strong.text),
                        border: iced::border::rounded(4),
                        ..Default::default()
                    }
                },
            ));
        }
        line = line.push(Space::new().width(Length::Fill));
        if playable {
            let (source, play_label) = if let Some(rom) = local {
                let label = match (playing_this, is_new) {
                    (true, _) => "playing",
                    (false, true) => "Play new ▶",
                    (false, false) => "Play ▶",
                };
                (BootSource::File(rom.path.clone()), label)
            } else {
                (
                    BootSource::Cached(sha1.to_owned()),
                    if playing_this { "playing" } else { "Play ▶" },
                )
            };
            let play = button(text(play_label).size(11))
                .on_press_maybe((!playing_this).then_some(Message::Play(source)));
            line = line.push(if is_new {
                play.style(button::primary)
            } else {
                play
            });
        }
        let mut rows = column![line].spacing(2);
        // Annotations ride their own line so a label plus a defect badge never
        // crowd the Play button off the row. A persisted defect already conveys
        // a "defective" verify result, so the session mark is dropped there.
        let session_mark = self
            .session_marks
            .get(sha1)
            .filter(|m| !(defect.is_some() && m.contains("defective")));
        if !label.is_empty() || defect.is_some() || session_mark.is_some() {
            let mut ann = row![Space::new().width(Length::Fixed(16.0))]
                .spacing(8)
                .align_y(iced::Alignment::Center);
            if !label.is_empty() {
                let shown: String = if label.chars().count() > 40 {
                    label.chars().take(38).collect::<String>() + "…"
                } else {
                    label.to_owned()
                };
                ann = ann.push(text(shown).size(12).style(text::secondary));
            }
            if let Some(defect) = defect {
                let bad = matches!(defect, missingno_gamedb::Defect::BadDump);
                ann = ann.push(
                    container(text(format!("⚠ {}", defect.label())).size(11))
                        .padding([2, 8])
                        .style(move |theme: &Theme| {
                            let palette = theme.extended_palette();
                            let pair = if bad {
                                palette.danger.strong
                            } else {
                                palette.secondary.strong
                            };
                            container::Style {
                                background: Some(pair.color.into()),
                                text_color: Some(pair.text),
                                border: iced::border::rounded(4),
                                ..Default::default()
                            }
                        }),
                );
            }
            if let Some(mark) = session_mark {
                let compact = if mark.starts_with('✓') {
                    "✓"
                } else {
                    mark.as_str()
                };
                ann = ann.push(text(compact.to_owned()).size(11).style(text::secondary));
            }
            rows = rows.push(ann);
        }
        if let Some(rom) = local {
            let file = rom
                .path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            rows = rows.push(text(format!("    {file}")).size(11).style(text::secondary));
        }
        rows.into()
    }

    fn view(&self) -> Element<'_, Message> {
        let db = match &self.db {
            Ok(db) => db,
            Err(e) => {
                return container(text(format!("failed to open gamedb: {e}")))
                    .padding(20)
                    .into();
            }
        };

        // ── Workflow bar: the current game and what to do with it ─────
        let mut top = row![].spacing(12).align_y(iced::Alignment::Center);
        match self.selected.and_then(|i| db.entries.get(i)) {
            Some(entry) => {
                let mut sub = format!("{} · {:?}", entry.key(), entry.game.kind());
                if entry.game.adult() {
                    sub.push_str(" · 🔞 adult");
                }
                if entry.dirty {
                    sub.push_str(" · uncommitted");
                }
                top = top.push(
                    column![
                        text(entry.game.title().to_owned()).size(22),
                        text(sub).size(12),
                    ]
                    .spacing(2),
                );
                if !self.queue.is_empty() {
                    top = top.push(text(format!("· {} in queue", self.queue.len())).size(13));
                }
                top = top.push(Space::new().width(Length::Fill));
                if entry.game.curated() {
                    let stars = entry.game.recommended_by().join(", ");
                    top = top.push(
                        text(if stars.is_empty() {
                            "✓ curated".to_owned()
                        } else {
                            format!("✓ curated · ★ {stars}")
                        })
                        .size(13),
                    );
                } else if !self.queue.is_empty() {
                    top = top.push(button(text("Skip ▶")).on_press(Message::SkipNext));
                }
                top = top
                    .push(button(text("Accept ✓")).on_press(Message::Accept { recommend: false }));
                top = top.push(
                    button(text("Accept ★ recommend"))
                        .on_press(Message::Accept { recommend: true }),
                );
            }
            None => {
                top = top.push(text("missingno curator").size(22));
                top = top.push(Space::new().width(Length::Fill));
            }
        }

        // ── Left: filters + list ──────────────────────────────────────
        let visible = self.visible();
        let filters = column![
            pick_list(
                TREE_CHOICES,
                Some(TreeChoice(self.filter_tree)),
                Message::FilterTree
            )
            .width(Length::Fill),
            text_input("search…", &self.search)
                .on_input(Message::Search)
                .width(Length::Fill),
            toggler(self.only_backlog)
                .label("backlog only")
                .on_toggle(Message::OnlyBacklog),
            toggler(self.only_flagged)
                .label("flagged only")
                .on_toggle(Message::OnlyFlagged),
            toggler(self.only_new)
                .label("new (unmatched ROMs)")
                .on_toggle(Message::OnlyNew),
            text(format!("{} match(es)", visible.len())).size(13),
        ]
        .spacing(8);

        let mut list = column![].spacing(2);
        for &i in visible.iter().take(LIST_LIMIT) {
            let entry = &db.entries[i];
            let marker = if entry.synthetic {
                "◆ "
            } else if !entry.game.curated() {
                ""
            } else if !entry.game.recommended_by().is_empty() {
                "★ "
            } else {
                "✓ "
            };
            let label = format!("{marker}{}  ({})", entry.game.title(), entry.key());
            let mut item = button(text(label).size(14)).width(Length::Fill).style(
                if self.selected == Some(i) {
                    button::primary
                } else {
                    button::text
                },
            );
            item = item.on_press(Message::Select(i));
            list = list.push(item);
        }
        if visible.len() > LIST_LIMIT {
            list = list.push(text(format!("… {} more", visible.len() - LIST_LIMIT)).size(13));
        }
        let left = column![filters, scrollable(list).height(Length::Fill)]
            .spacing(12)
            .width(Length::Fixed(320.0));

        // ── Right: editor ─────────────────────────────────────────────
        let right: Element<'_, Message> = match self.selected {
            Some(i) => {
                let entry = &db.entries[i];
                // Read-only labelled value row.
                let field = |label: &'static str, value: String| -> Element<'_, Message> {
                    row![
                        text(label).width(Length::Fixed(90.0)).size(13),
                        text(value).size(14).width(Length::Fill),
                    ]
                    .spacing(8)
                    .into()
                };
                let mut editor = column![].spacing(12);

                if let Some(url) = entry.game.covers().first() {
                    if let Some(preview) = self.cover_previews.get(url) {
                        editor = editor.push(
                            iced::widget::image(preview.clone())
                                .content_fit(iced::ContentFit::Contain)
                                .height(Length::Fixed(240.0)),
                        );
                    } else if self.cover_failed.contains(url) {
                        editor = editor.push(text(format!("cover failed to load: {url}")).size(12));
                    } else {
                        editor = editor.push(text("cover loading…").size(12));
                    }
                }

                let developer = entry.game.text_field(TextField::Developer);
                if !developer.is_empty() {
                    editor = editor.push(field("Developer", developer));
                }
                let description = entry.game.text_field(TextField::Description);
                if !description.is_empty() {
                    editor = editor.push(
                        row![
                            text("Description").width(Length::Fixed(90.0)).size(13),
                            text(description).size(14).width(Length::Fill),
                        ]
                        .spacing(8),
                    );
                }
                let tags = entry.game.tags();
                if !tags.is_empty() {
                    editor = editor.push(field("Tags", tags.join(", ")));
                }

                let links = entry.game.links();
                if !links.is_empty() {
                    editor = editor.push(text("Links").size(15));
                    for (name, url, languages) in links {
                        let mut link_row = row![
                            button(text(name).size(13))
                                .style(button::secondary)
                                .on_press(Message::OpenLink(url.clone())),
                            text(url.clone()).size(11).style(text::secondary),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center);
                        if !languages.is_empty() {
                            link_row =
                                link_row.push(text(languages).size(11).style(text::secondary));
                        }
                        editor = editor.push(link_row);
                    }
                }

                editor = editor.push(text("Releases").size(16));
                for (r, line) in entry.game.release_lines().into_iter().enumerate() {
                    let mut header = row![].spacing(6).align_y(iced::Alignment::Center);
                    if let Some(title) = line.title {
                        header = header.push(text(title).size(13).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..iced::Font::DEFAULT
                        }));
                    }
                    if let Some(label) = line.label {
                        header =
                            header.push(text(format!("({label})")).size(13).style(text::secondary));
                    }
                    if !line.detail.is_empty() {
                        header = header.push(text(line.detail).size(13));
                    }
                    let mut rel = column![header].spacing(4);
                    let publisher = entry.game.release_publisher(r);
                    if !publisher.is_empty() {
                        rel = rel.push(text(format!("Publisher: {publisher}")).size(12));
                    }
                    for (sha1, label, defect) in entry.game.release_artifacts(r) {
                        rel = rel.push(self.artifact_row(&sha1, &label, defect));
                    }
                    editor = editor.push(
                        container(rel)
                            .padding([6, 10])
                            .style(container::rounded_box),
                    );
                }

                let mods = entry.game.mod_lines();
                if !mods.is_empty() {
                    editor = editor.push(text("Mods").size(16));
                    for (index, (line, links)) in mods.into_iter().enumerate() {
                        editor =
                            editor.push(text(format!("• {line}")).size(13).width(Length::Fill));
                        for (name, url) in links {
                            editor = editor.push(
                                row![
                                    Space::new().width(Length::Fixed(16.0)),
                                    button(text(name).size(12))
                                        .style(button::secondary)
                                        .on_press(Message::OpenLink(url.clone())),
                                ]
                                .spacing(8)
                                .align_y(iced::Alignment::Center),
                            );
                        }
                        for version in entry.game.mod_version_lines(index) {
                            editor = editor.push(
                                row![
                                    Space::new().width(Length::Fixed(16.0)),
                                    text(version).size(12).style(text::secondary),
                                ]
                                .align_y(iced::Alignment::Center),
                            );
                        }
                        for (sha1, label, defect) in entry.game.mod_artifacts(index) {
                            editor = editor.push(
                                row![
                                    Space::new().width(Length::Fixed(16.0)),
                                    self.artifact_row(&sha1, &label, defect),
                                ]
                                .align_y(iced::Alignment::Center),
                            );
                        }
                    }
                }

                let entry_flags: Vec<_> = db
                    .flags
                    .open()
                    .filter(|f| f.subject.iter().any(|s| *s == entry.key()))
                    .collect();
                if !entry_flags.is_empty() {
                    editor = editor.push(text("Flags").size(16));
                    for flag in entry_flags {
                        editor = editor.push(
                            row![
                                text(format!("#{} [{:?}] {}", flag.id, flag.kind, flag.note))
                                    .size(12)
                                    .width(Length::Fill),
                                button(text("Resolve").size(12))
                                    .on_press(Message::ResolveFlag(flag.id)),
                            ]
                            .spacing(8),
                        );
                    }
                }

                if let Some(line) = self.verify_status.get(&entry.key()) {
                    editor = editor.push(text("Verify").size(16));
                    editor = editor.push(text(line.clone()).size(13));
                }

                scrollable(editor.padding(4)).into()
            }
            None => container(text("select an entry")).padding(20).into(),
        };

        let left: Option<Element<'_, Message>> = self.list_visible.then_some(left.into());
        let play_region: Element<'_, Message> = match &self.playing {
            Some((key, session)) => {
                let mut pane = column![
                    row![
                        text(format!(
                            "Playing {key} — pad: dpad/stick · south=A/Fire · east=B · start · select"
                        ))
                        .size(13)
                        .width(Length::Fill),
                        button(text("Stop ■").size(13)).on_press(Message::StopPlay),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                ]
                .spacing(8);
                let mut switches = row![
                    button(text("Reset").size(12))
                        .on_press(Message::TapSwitch(ControlId::panel(ControlRole::Reset))),
                    button(text("Select").size(12))
                        .on_press(Message::TapSwitch(ControlId::panel(ControlRole::Select))),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center);
                let jack = if session.pad_jack == missingno_vcs::debug::RIGHT_PORT {
                    "right"
                } else {
                    "left"
                };
                switches = switches.push(
                    button(text(format!("Pad: {jack} jack")).size(12))
                        .on_press(Message::SwapPadJack),
                );
                for (i, switch) in session.switches.iter().enumerate() {
                    let level = session.switch_levels.get(i).copied().unwrap_or(false);
                    let Some((positions, _)) = switch.toggle() else {
                        continue;
                    };
                    let position = positions[usize::from(level)];
                    switches = switches.push(
                        button(text(format!("{}: {position}", switch.label)).size(12))
                            .on_press(Message::ToggleSwitch(i)),
                    );
                }
                pane = pane.push(switches);
                if let Some(screen) = &self.play_screen {
                    let paddles = session.paddles;
                    pane = pane.push(iced::widget::responsive(move |size| {
                        let (width, height) = screen.fitted_size(size);
                        // Horizontal position over the screen drives the
                        // paddle, as in the emulator; the triggers wind it.
                        let mut area = iced::widget::mouse_area(
                            iced::widget::shader(screen)
                                .width(Length::Fixed(width))
                                .height(Length::Fixed(height)),
                        )
                        .on_move(move |point| Message::Paddle((point.x / width).clamp(0.0, 1.0)));
                        if paddles {
                            // Click = the paddle's trigger while aiming by pointer.
                            let trigger = ControlId::port(play::PLAY_PORT, ControlRole::Action(0));
                            area = area
                                .on_press(Message::Pad(trigger, true))
                                .on_release(Message::Pad(trigger, false));
                        }
                        container(area).center(Length::Fill).into()
                    }));
                }
                pane.into()
            }
            // Reserve the play region even when idle so starting or stopping
            // emulation never relayouts the editor column.
            None => container(text("Boot a dump to play").size(13))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into(),
        };

        let mut body = row![].spacing(16);
        if let Some(left) = left {
            body = body.push(left);
        }
        let body: Element<'_, Message> = body
            .push(container(right).width(Length::FillPortion(2)))
            .push(container(play_region).width(Length::FillPortion(3)))
            .into();

        let mut bottom = row![].spacing(14).align_y(iced::Alignment::Center);
        bottom = bottom.push(text(&self.status).size(12).width(Length::Fill));
        bottom = bottom.push(
            button(
                text(if self.list_visible {
                    "☰ hide list"
                } else {
                    "☰ list"
                })
                .size(12),
            )
            .style(button::text)
            .on_press(Message::ToggleList),
        );

        column![top, body, bottom].spacing(12).padding(12).into()
    }
}
