#![recursion_limit = "256"]

//! missingno-curator — review, enrich, and confirm gamedb entries.
//!
//! v1: Backlog (uncurated entries) and Flags drain through one list+editor
//! screen; confirms stamp `curated` and accumulate into explicit git commits.

mod db;
mod play;
mod remote;
mod verify;

use std::path::PathBuf;

use clap::Parser;
use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input, toggler,
};
use iced::{Element, Length, Task, Theme};

use db::{Db, TextField, TreeId};
use remote::{Bridge, RemoteEndpoint, SharedSink, error_result, text_result};
use verify::RomIndex;
#[derive(Parser)]
struct Args {
    /// Path to the missingno-gamedb checkout.
    #[arg(default_value = "missingno-gamedb")]
    db_path: PathBuf,

    /// Local ROM collection to hash-match dump-only entries against.
    #[arg(long)]
    rom_dir: Option<PathBuf>,

    /// Don't publish the ui-<pid>.sock remote-control socket.
    #[arg(long)]
    no_remote: bool,

    /// Name recorded on curation stamps (default: git user.name).
    #[arg(long)]
    curator: Option<String>,
}

pub fn main() -> iced::Result {
    let args = Args::parse();
    let db_path = args.db_path.clone();
    let rom_dir = args.rom_dir.clone();
    let remote = !args.no_remote;
    let curator_name = args.curator.clone().unwrap_or_else(|| {
        std::process::Command::new("git")
            .args(["config", "user.name"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_owned())
    });
    iced::application(
        move || {
            Curator::new(
                db_path.clone(),
                rom_dir.clone(),
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
    search: String,
    selected: Option<usize>,
    status: String,
    rom_dir: Option<PathBuf>,
    rom_index: Option<std::sync::Arc<RomIndex>>,
    /// entry key → last fetch/verify status line.
    verify_status: std::collections::HashMap<String, String>,
    /// sha1 → fetched ROM bytes, kept for boot verification.
    rom_cache: std::collections::HashMap<String, std::sync::Arc<Vec<u8>>>,
    fetching: bool,
    scanning: bool,
    booting: bool,
    playing: Option<(String, play::PlaySession)>,
    play_frame: Option<iced::widget::image::Handle>,
    playing_sha1: Option<String>,
    /// Guards frame-loop messages from superseded play sessions.
    play_generation: u64,
    /// Agent-driven curation queue of entry keys; front = current.
    queue: std::collections::VecDeque<String>,
    /// entry key → the agent's explanation of its edits and sources.
    agent_notes: std::collections::HashMap<String, String>,
    /// Fetch in flight that should start a playtest when it lands.
    play_after_fetch: Option<String>,
    /// entry key → last fetched sha1 (bytes live in rom_cache).
    fetched_sha1: std::collections::HashMap<String, String>,
    /// entry key → boot note + screenshot.
    boot_shots: std::collections::HashMap<String, (String, iced::widget::image::Handle)>,
    /// Multiline editor state for the selected entry's description.
    description: iced::widget::text_editor::Content,
    /// cover url → fetched preview.
    cover_previews: std::collections::HashMap<String, iced::widget::image::Handle>,
    /// cover urls that failed to fetch (shown as an error instead of silence).
    cover_failed: std::collections::HashSet<String>,
    /// entries already auto-looked-up on Hasheous this session.
    enrich_attempted: std::collections::HashSet<String>,
    enriching: bool,
    remote_sink: SharedSink,
    _remote: Option<RemoteEndpoint>,
    /// Parked wait_for_action replies, answered when the human acts.
    action_waiters: Vec<std::sync::mpsc::Sender<serde_json::Value>>,
    /// Decisions made while no agent was waiting.
    action_events: std::collections::VecDeque<String>,
    curator_name: String,
    /// Stamp the next confirm as an editor's-choice recommendation.
    recommend_next: bool,
    /// The filter/list column; hidden automatically when an agent queues work.
    list_visible: bool,
    /// Open red-flag note input for the current entry (None = closed).
    flag_note: Option<String>,
    /// Open slug-rename input for the current entry (None = closed).
    slug_edit: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    Remote(Bridge),
    Play(BootSource),
    PlayFrame(u64),
    PlayEnded(u64),
    StopPlay,
    Pad(u8, bool),
    Boot(BootSource),
    Booted(String, Result<BootDone, String>),
    Fetch {
        play_after: bool,
    },
    Fetched(String, Result<(String, std::sync::Arc<Vec<u8>>), String>),
    ScanRoms,
    ScannedRoms(Result<std::sync::Arc<RomIndex>, String>),
    FilterTree(TreeChoice),
    OnlyBacklog(bool),
    OnlyFlagged(bool),
    Search(String),
    Select(usize),
    Edit(TextField, String),
    EditReleasePublisher(usize, String),
    SetArtifactLabel(String, String),
    RecordPlaytest(String),
    ArtifactsVerified {
        key: String,
        results: Vec<(String, verify::SigResult)>,
        reply: std::sync::mpsc::Sender<serde_json::Value>,
    },
    CurateMod {
        index: usize,
        recommend: bool,
    },
    DescriptionAction(iced::widget::text_editor::Action),
    Enrich,
    Enriched(String, Result<Option<verify::HasheousHit>, String>),
    CoverLoaded(String, Option<iced::widget::image::Handle>),
    ConfirmAndNext,
    Accept {
        recommend: bool,
    },
    FlagPrompt,
    FlagNote(String),
    SaveFlag,
    SlugPrompt,
    SlugInput(String),
    ApplySlug,
    ToggleList,
    CloseRequested,
    OpenLink(String),
    ResolveFlag(u32),
    Commit,
}

#[derive(Debug, Clone)]
enum BootSource {
    Cached(String),
    File(PathBuf),
}

#[derive(Debug, Clone)]
struct BootDone {
    note: String,
    shot: iced::widget::image::Handle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreeChoice(Option<TreeId>);

impl std::fmt::Display for TreeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.map(TreeId::label).unwrap_or("All platforms"))
    }
}

const TREE_CHOICES: [TreeChoice; 4] = [
    TreeChoice(None),
    TreeChoice(Some(TreeId::Gb)),
    TreeChoice(Some(TreeId::Gbc)),
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
            iced::Subscription::run(play::gamepad_worker).map(|(id, on)| Message::Pad(id, on)),
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(Message::CloseRequested)
                }
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
        remote: bool,
        curator_name: String,
    ) -> (Self, Task<Message>) {
        let has_rom_dir = rom_dir.is_some();
        let db = Db::load(db_path).map_err(|e| e.to_string());
        let remote_sink = SharedSink::default();
        let endpoint = remote
            .then(|| RemoteEndpoint::open(remote_sink.clone()).ok())
            .flatten();
        (
            Self {
                db,
                filter_tree: None,
                only_backlog: true,
                only_flagged: false,
                search: String::new(),
                selected: None,
                status: String::new(),
                rom_dir,
                rom_index: None,
                verify_status: std::collections::HashMap::new(),
                rom_cache: std::collections::HashMap::new(),
                fetching: false,
                scanning: false,
                booting: false,
                playing: None,
                play_frame: None,
                playing_sha1: None,
                play_generation: 0,
                queue: std::collections::VecDeque::new(),
                agent_notes: std::collections::HashMap::new(),
                play_after_fetch: None,
                fetched_sha1: std::collections::HashMap::new(),
                boot_shots: std::collections::HashMap::new(),
                description: iced::widget::text_editor::Content::new(),
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
                flag_note: None,
                slug_edit: None,
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
            .filter(|(_, e)| !self.only_backlog || e.game.curations().is_empty())
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
                        for (sha1, _) in db.entries[i].game.release_artifacts(r) {
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
                    self.stage_header_facts(i, &bytes);
                    match play::start(hint, &bytes, tv, cart) {
                        Ok(session) => {
                            let events = session.events.clone();
                            self.playing = Some((key, session));
                            self.play_frame = None;
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
                    if let Some(frame) = session.handle.latest_frame() {
                        let rgba = frame.resolve_rgba();
                        let (width, height, pixels) = verify::aspect_corrected(
                            rgba.width,
                            rgba.height,
                            session.pixel_aspect,
                            &rgba.pixels,
                        );
                        self.play_frame = Some(iced::widget::image::Handle::from_rgba(
                            width, height, pixels,
                        ));
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
                    self.play_frame = None;
                    self.playing_sha1 = None;
                }
            }
            Message::StopPlay => {
                self.playing = None;
                self.play_frame = None;
                self.playing_sha1 = None;
                self.play_generation += 1;
            }
            Message::Pad(id, pressed) => {
                if let Some((_, session)) = &self.playing {
                    session.set_control(id, pressed);
                }
            }
            Message::Boot(source) => {
                if let (Ok(db), Some(i)) = (&self.db, self.selected) {
                    let entry = &db.entries[i];
                    let key = entry.key();
                    let hint = match entry.tree {
                        TreeId::Gb => "verify.gb",
                        TreeId::Gbc => "verify.gbc",
                        TreeId::Vcs => "verify.a26",
                    };
                    let bytes = match &source {
                        BootSource::Cached(sha1) => self.rom_cache.get(sha1).cloned(),
                        BootSource::File(path) => std::fs::read(path).ok().map(std::sync::Arc::new),
                    };
                    let Some(bytes) = bytes else {
                        self.status = "no ROM bytes to boot".to_owned();
                        return Task::none();
                    };
                    let dump_sha1 = match &source {
                        BootSource::Cached(sha1) => sha1.clone(),
                        BootSource::File(_) => verify::sha1_hex(&bytes),
                    };
                    let (tv, cart) = entry.game.hints_for(&dump_sha1);
                    self.stage_header_facts(i, &bytes);
                    self.booting = true;
                    self.verify_status
                        .insert(key.clone(), "booting (300 frames)…".to_owned());
                    return Task::perform(
                        smol::unblock(move || {
                            verify::boot_check(hint, &bytes, tv, cart, 300).map(|shot| BootDone {
                                note: format!(
                                    "boots: {}/{} frames produced a screen ({}×{})",
                                    shot.frames_seen, shot.frames_run, shot.width, shot.height
                                ),
                                shot: iced::widget::image::Handle::from_rgba(
                                    shot.width,
                                    shot.height,
                                    shot.rgba,
                                ),
                            })
                        }),
                        move |result| Message::Booted(key.clone(), result),
                    );
                }
            }
            Message::Booted(key, result) => {
                self.booting = false;
                match result {
                    Ok(done) => {
                        self.verify_status.insert(key.clone(), done.note.clone());
                        self.boot_shots.insert(key, (done.note, done.shot));
                    }
                    Err(e) => {
                        self.verify_status.insert(key, format!("boot failed: {e}"));
                    }
                }
            }
            Message::Fetch { play_after } => {
                if let (Ok(db), Some(i)) = (&self.db, self.selected) {
                    let entry = &db.entries[i];
                    let Some(url) = entry.game.download_url() else {
                        return Task::none();
                    };
                    let key = entry.key();
                    self.play_after_fetch = play_after.then(|| key.clone());
                    self.fetching = true;
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
            Message::Fetched(key, result) => {
                self.fetching = false;
                match result {
                    Ok((url, bytes)) => {
                        let sha1 = verify::sha1_hex(&bytes);
                        let size = bytes.len() as u64;
                        self.rom_cache.insert(sha1.clone(), bytes);
                        self.fetched_sha1.insert(key.clone(), sha1.clone());
                        let sha1_for_play = sha1.clone();
                        if let Some(i) = self.find_entry(&key) {
                            let bytes = self.rom_cache.get(&sha1).cloned();
                            if let Some(bytes) = bytes {
                                self.stage_header_facts(i, &bytes);
                            }
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
                }
            }
            Message::ScanRoms => {
                if let Some(dir) = self.rom_dir.clone() {
                    self.scanning = true;
                    self.status = format!("scanning {}…", dir.display());
                    return Task::perform(
                        smol::unblock(move || {
                            RomIndex::scan(&dir)
                                .map(std::sync::Arc::new)
                                .map_err(|e| e.to_string())
                        }),
                        Message::ScannedRoms,
                    );
                }
            }
            Message::ScannedRoms(result) => {
                self.scanning = false;
                match result {
                    Ok(index) => {
                        self.status = format!("indexed {} ROM(s) from collection", index.scanned);
                        self.rom_index = Some(index);
                    }
                    Err(e) => self.status = format!("scan failed: {e}"),
                }
            }
            Message::FilterTree(TreeChoice(tree)) => {
                self.filter_tree = tree;
                self.selected = None;
            }
            Message::OnlyBacklog(v) => self.only_backlog = v,
            Message::OnlyFlagged(v) => self.only_flagged = v,
            Message::Search(s) => {
                self.search = s;
                self.selected = None;
            }
            Message::Select(index) => return self.select(index),
            Message::DescriptionAction(action) => {
                self.description.perform(action);
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    let text = self.description.text();
                    let text = text.trim_end();
                    if db.entries[i].game.text_field(TextField::Description) != text {
                        db.entries[i]
                            .game
                            .set_text_field(TextField::Description, text.to_owned());
                        db.entries[i].dirty = true;
                    }
                }
            }
            Message::Enrich => {
                if let (Ok(db), Some(i)) = (&self.db, self.selected) {
                    let entry = &db.entries[i];
                    let Some(sha1) = entry.game.artifact_sha1s().into_iter().next() else {
                        self.status = "no artifact hash to look up".to_owned();
                        return Task::none();
                    };
                    let key = entry.key();
                    self.enriching = true;
                    return Task::perform(
                        smol::unblock(move || verify::hasheous_lookup(&sha1)),
                        move |result| Message::Enriched(key.clone(), result),
                    );
                }
            }
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
            Message::CurateMod { index, recommend } => {
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    let by = self.curator_name.clone();
                    if db.entries[i].game.stamp_mod_curation(index, &by, recommend) {
                        db.entries[i].dirty = true;
                        match db.write_entry(i) {
                            Ok(()) => self.status = "mod endorsed".to_owned(),
                            Err(e) => self.status = format!("write failed: {e}"),
                        }
                    }
                }
            }
            Message::ArtifactsVerified {
                key,
                results,
                reply,
            } => {
                let mut lines = Vec::new();
                let mut recorded = 0;
                let found = self.find_entry(&key);
                if let (Ok(db), Some(i)) = (&mut self.db, found) {
                    for (sha1, outcome) in &results {
                        let short = &sha1[..12];
                        match outcome {
                            verify::SigResult::Found { signature, game } => {
                                let evidence = signature.clone().unwrap_or_else(|| game.clone());
                                match verify::classify_signature(&evidence) {
                                    Some(verify::SigFlag::Derived(reason)) => {
                                        lines.push(format!(
                                            "{short}… DERIVED ({reason}): {evidence} —                                              someone made this; judge and mark_hack"
                                        ));
                                    }
                                    Some(verify::SigFlag::Defective(reason)) => {
                                        // A dumper's mistake, not a work: the
                                        // evidence IS the record; nothing moves.
                                        if db.entries[i]
                                            .game
                                            .record_signature(sha1, "Hasheous", &evidence)
                                        {
                                            recorded += 1;
                                            db.entries[i].dirty = true;
                                        }
                                        lines.push(format!(
                                            "{short}… DEFECTIVE ({reason}): {evidence} —                                              evidence recorded; label_artifact it, and if it                                              fabricated a release (wrong board from an                                              overdump), move_artifact into the real one"
                                        ));
                                    }
                                    None => {
                                        if db.entries[i]
                                            .game
                                            .record_signature(sha1, "Hasheous", &evidence)
                                        {
                                            recorded += 1;
                                            db.entries[i].dirty = true;
                                        }
                                        let lower = evidence.to_lowercase();
                                        let suggest = if lower.contains("(prototype)")
                                            || lower.contains("(proto)")
                                        {
                                            " — a prototype build: consider split_release                                              (keep any working title it carries)"
                                        } else if lower.contains("(beta)") {
                                            " — a beta build: consider split_release"
                                        } else {
                                            ""
                                        };
                                        lines.push(format!(
                                            "{short}… confirmed: {evidence}{suggest}"
                                        ));
                                    }
                                }
                            }
                            verify::SigResult::Unknown => {
                                lines.push(format!("{short}… unknown to the signature database"))
                            }
                            verify::SigResult::Failed(e) => {
                                lines.push(format!("{short}… lookup failed: {e}"))
                            }
                        }
                    }
                    if recorded > 0
                        && let Err(e) = db.write_entry(i)
                    {
                        lines.push(format!("write failed: {e}"));
                    }
                    self.status = format!(
                        "verified {key}: {} recorded, {} dumps checked",
                        recorded,
                        results.len()
                    );
                } else {
                    lines.push(format!("entry {key} disappeared during verification"));
                }
                let _ = reply.send(text_result(lines.join("\n")));
            }
            Message::RecordPlaytest(sha1) => {
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    let by = self.curator_name.clone();
                    if db.entries[i].game.record_playtest(&sha1, &by) {
                        db.entries[i].dirty = true;
                        match db.write_entry(i) {
                            Ok(()) => {
                                self.status = format!("playtest recorded for {}…", &sha1[..12])
                            }
                            Err(e) => self.status = format!("write failed: {e}"),
                        }
                    }
                }
            }
            Message::SetArtifactLabel(sha1, label) => {
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected)
                    && db.entries[i].game.set_artifact_label(&sha1, &label)
                {
                    db.entries[i].dirty = true;
                }
            }
            Message::EditReleasePublisher(index, value) => {
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    db.entries[i].game.set_release_publisher(index, value);
                    db.entries[i].dirty = true;
                }
            }
            Message::Edit(field, value) => {
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    db.entries[i].game.set_text_field(field, value);
                    db.entries[i].dirty = true;
                }
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
            Message::FlagPrompt => {
                self.flag_note = match self.flag_note {
                    Some(_) => None,
                    None => Some(String::new()),
                };
            }
            Message::FlagNote(note) => self.flag_note = Some(note),
            Message::SaveFlag => {
                let Some(note) = self.flag_note.take() else {
                    return Task::none();
                };
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    let key = db.entries[i].key();
                    let flag = missingno_gamedb::Flag {
                        id: db.flags.next_id(),
                        kind: missingno_gamedb::FlagKind::Custom,
                        subject: vec![key.clone()],
                        note: format!("{}: {note}", self.curator_name),
                        resolved: None,
                    };
                    db.flags.flags.push(flag);
                    match db.save_flags() {
                        Ok(()) => self.status = format!("flagged {key}"),
                        Err(e) => self.status = format!("flag save failed: {e}"),
                    }
                    // A flag is "park it and move on": advance without a stamp.
                    if self.queue.front() == Some(&key) {
                        self.queue.pop_front();
                        self.playing = None;
                        self.play_frame = None;
                        while let Some(next_key) = self.queue.front().cloned() {
                            match self.find_entry(&next_key) {
                                Some(next) => {
                                    self.emit_action(format!(
                                        "flagged {key} ({note:?}); now playing {next_key}                                          ({} left in queue)",
                                        self.queue.len()
                                    ));
                                    return self.start_playtest_for(next);
                                }
                                None => {
                                    self.queue.pop_front();
                                }
                            }
                        }
                        self.emit_action(format!("flagged {key} ({note:?}); queue is empty"));
                    } else {
                        self.emit_action(format!("flagged {key} ({note:?})"));
                    }
                }
            }
            Message::SlugPrompt => {
                self.slug_edit = match self.slug_edit {
                    Some(_) => None,
                    None => self
                        .selected
                        .and_then(|i| self.db.as_ref().ok().map(|db| db.entries[i].slug.clone())),
                };
            }
            Message::SlugInput(slug) => self.slug_edit = Some(slug),
            Message::ApplySlug => {
                let Some(new_slug) = self.slug_edit.take() else {
                    return Task::none();
                };
                let (Ok(db), Some(i)) = (&mut self.db, self.selected) else {
                    return Task::none();
                };
                let old_key = db.entries[i].key();
                match db.rename_entry(i, new_slug.trim()) {
                    Ok(new_key) => {
                        self.rekey_entry(&old_key, &new_key);
                        self.status = format!("renamed {old_key} → {new_key}");
                        self.emit_action(format!("renamed {old_key} → {new_key}"));
                    }
                    Err(e) => {
                        self.status = format!("rename failed: {e}");
                        self.slug_edit = Some(new_slug);
                    }
                }
            }
            Message::CloseRequested => {
                // Tear down in order on this thread — session first (its worker
                // and the cpal stream go quietly), then the socket — so the
                // process exits 0 and an attached agent sees a clean end.
                self.playing = None;
                self.play_frame = None;
                self._remote = None;
                return iced::window::latest().and_then(iced::window::close);
            }
            Message::OpenLink(url) => {
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            }
            Message::ToggleList => self.list_visible = !self.list_visible,
            Message::ResolveFlag(id) => {
                if let Ok(db) = &mut self.db {
                    if let Some(flag) = db.flags.flags.iter_mut().find(|f| f.id == id) {
                        flag.resolved = Some(Db::today());
                    }
                    match db.save_flags() {
                        Ok(()) => self.status = format!("resolved flag #{id}"),
                        Err(e) => self.status = format!("flag save failed: {e}"),
                    }
                }
            }
            Message::Commit => {
                if let Ok(db) = &mut self.db {
                    let message = format!("Curate: {} file(s) updated", db.uncommitted);
                    match db.commit(&message) {
                        Ok(head) => self.status = format!("committed: {}", head.trim()),
                        Err(e) => self.status = format!("commit failed: {e}"),
                    }
                }
            }
        }
        Task::none()
    }

    /// Rebuild the description widget from the selected entry's data.
    fn sync_description(&mut self) {
        if let (Ok(db), Some(i)) = (&self.db, self.selected) {
            self.description = iced::widget::text_editor::Content::with_text(
                &db.entries[i].game.text_field(TextField::Description),
            );
        }
    }

    /// Select an entry: sync the description editor and kick a cover preview.
    fn select(&mut self, i: usize) -> Task<Message> {
        self.selected = Some(i);
        self.slug_edit = None;
        self.sync_description();
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
                    if let Some(path) = index.by_sha1.get(&sha1) {
                        break 'boot Task::done(Message::Play(BootSource::File(path.clone())));
                    }
                }
            }
            if let Some(sha1) = self.fetched_sha1.get(&key) {
                break 'boot Task::done(Message::Play(BootSource::Cached(sha1.clone())));
            }
            if entry.game.download_url().is_some() {
                break 'boot Task::done(Message::Fetch { play_after: true });
            }
            self.status = format!("{key}: no local dump and no download source");
            Task::none()
        };
        Task::batch([select_task, boot])
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
        self.status = format!("accepted {key}");
        if self.queue.front() == Some(&key) {
            self.queue.pop_front();
        }
        self.playing = None;
        self.play_frame = None;
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

    /// Read the GB-family header from ROM bytes and stage its facts (fills
    /// unknown enhancement flags and the mapper; conflicts go to the status).
    fn stage_header_facts(&mut self, i: usize, rom: &[u8]) {
        let Ok(db) = &mut self.db else { return };
        if matches!(db.entries[i].tree, TreeId::Vcs) {
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
        if let Some(v) = self.agent_notes.remove(old_key) {
            self.agent_notes.insert(new_key.to_owned(), v);
        }
        if let Some(v) = self.fetched_sha1.remove(old_key) {
            self.fetched_sha1.insert(new_key.to_owned(), v);
        }
        if let Some(v) = self.boot_shots.remove(old_key) {
            self.boot_shots.insert(new_key.to_owned(), v);
        }
        if self.enrich_attempted.remove(old_key) {
            self.enrich_attempted.insert(new_key.to_owned());
        }
    }

    /// Tell a waiting agent (or queue for the next wait) what the human did.
    fn emit_action(&mut self, event: String) {
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
        let str_arg = |k: &str| args.get(k).and_then(serde_json::Value::as_str);
        match name {
            "status" => {
                let Ok(db) = &self.db else {
                    return error_result("db not loaded");
                };
                text_result(format!(
                    "backlog: gb {}, gbc {}, vcs {} · open flags: {} · uncommitted files: {}",
                    db.backlog_count(TreeId::Gb),
                    db.backlog_count(TreeId::Gbc),
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
                    if backlog_only && !e.game.curations().is_empty() {
                        continue;
                    }
                    if !e.game.title().to_lowercase().contains(&query) && !e.slug.contains(&query) {
                        continue;
                    }
                    lines.push(format!(
                        "{} — {}{}",
                        e.key(),
                        e.game.title(),
                        if e.game.curations().is_empty() {
                            ""
                        } else {
                            " [curated]"
                        }
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
                        match db::parse_link_type(kind) {
                            Ok(link_type) => {
                                staged_links.push((name.to_owned(), url.to_owned(), link_type))
                            }
                            Err(e) => return error_result(e),
                        }
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
                for (name, url, link_type) in &staged_links {
                    entry.game.upsert_link(name, url, *link_type);
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
                    ("license", TextField::License),
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
                if let Some(publisher) = set.get("publisher").and_then(serde_json::Value::as_str) {
                    entry.game.set_release_publisher(0, publisher.to_owned());
                    applied.push("publisher");
                }
                if let Some(mapper) = set.get("mapper").and_then(serde_json::Value::as_str)
                    && entry.game.set_mapper(mapper)
                {
                    applied.push("mapper");
                }
                if let Some(cart) = set.get("cart_type").and_then(serde_json::Value::as_str)
                    && entry.game.set_cart_type(cart)
                {
                    applied.push("cart_type");
                }
                if applied.is_empty() {
                    return error_result("no recognized fields in set");
                }
                // Automation touching a curated entry re-opens it for review.
                entry.game.clear_curations();
                entry.dirty = true;
                self.selected = Some(i);
                // Staged text lives in the manifest, not just in memory: an
                // agent's research must survive the window closing.
                if let Ok(db) = &mut self.db
                    && let Err(e) = db.write_entry(i)
                {
                    return error_result(format!("staged, but writing {key} failed: {e}"));
                }
                self.sync_description();
                text_result(format!(
                    "staged {} on {key}; entry re-opened for review (uncommitted until the curator confirms)",
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
                        self.sync_description();
                        text_result(message)
                    }
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
                        text_result(format!(
                            "{old_key} renamed → {new_key}; use the new key from now on"
                        ))
                    }
                    Err(e) => error_result(e),
                }
            }
            "set_note" => {
                let (Some(key), Some(note)) = (str_arg("key"), str_arg("note")) else {
                    return error_result("missing key or note");
                };
                if self.find_entry(key).is_none() {
                    return error_result(format!("no entry {key}"));
                }
                self.agent_notes.insert(key.to_owned(), note.to_owned());
                text_result(format!("note shown on {key}"))
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
                    .filter(|e| !backlog_only || e.game.curations().is_empty())
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
            "mark_hack" => {
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
                let homepage = str_arg("url").map(str::to_owned);
                match db.mark_hack(i, sha1, title, category, base, homepage) {
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
                        if !db.entries[i].game.artifact_sha1s().contains(&base) {
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
                let url = set_str("url").map(str::to_owned);
                let applied = db.entries[i].game.edit_mod(mod_name, move |m| {
                    let mut applied = Vec::new();
                    if let Some(name) = rename {
                        m.name = name;
                        applied.push("name");
                    }
                    if let Some(category) = category {
                        m.category = category;
                        applied.push("category");
                    }
                    if let Some(author) = author {
                        m.author = (!author.is_empty()).then_some(author);
                        applied.push("author");
                    }
                    if let Some(url) = url {
                        match m.links.iter_mut().find(|l| l.name == "Homepage") {
                            Some(link) => link.url = url,
                            None => m.links.push(missingno_gamedb::Link {
                                name: "Homepage".to_owned(),
                                url,
                                link_type: missingno_gamedb::LinkType::Community,
                            }),
                        }
                        applied.push("url");
                    }
                    if let Some(release) = m.releases.get_mut(release_index) {
                        if let Some(base) = base {
                            release.base_sha1 = base;
                            applied.push("base_sha1");
                        }
                        if let Some(label) = label {
                            release.label = (!label.is_empty()).then_some(label);
                            applied.push("label");
                        }
                        if let Some(date) = date {
                            release.date = Some(date);
                            applied.push("date");
                        }
                    } else if base.is_some() || label.is_some() || date.is_some() {
                        applied.push("(release fields skipped: no such release_index)");
                    }
                    if !applied.is_empty() {
                        // Editing the mod un-vouches the mod (not the game).
                        m.curated.clear();
                    }
                    applied
                });
                match applied {
                    Some(applied) if applied.is_empty() => {
                        error_result("no recognized fields in set")
                    }
                    Some(applied) => {
                        db.entries[i].dirty = true;
                        text_result(format!("updated mod on {key}: {}", applied.join(", ")))
                    }
                    None => error_result(format!("mod {mod_name:?} vanished mid-edit")),
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
                        "{sha1} moved into its own {status:?} release of {key}; the entry                          is re-opened for review"
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
                    Some(d) => Some(match d.parse::<missingno_gamedb::ReleaseDate>() {
                        Ok(d) => d,
                        Err(e) => return error_result(e),
                    }),
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
                let tv_format = match set_str("tv_format") {
                    Some(f) => Some(match db::parse_tv_format(f) {
                        Ok(f) => f,
                        Err(e) => return error_result(e),
                    }),
                    None => None,
                };
                if status.is_none()
                    && title.is_none()
                    && label.is_none()
                    && date.is_none()
                    && publisher.is_none()
                    && regions.is_none()
                    && tv_format.is_none()
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
                };
                if db.entries[i].game.update_release(index as usize, edits) {
                    let tv = tv_format.is_some_and(|f| {
                        db.entries[i].game.set_release_tv_format(index as usize, f)
                    });
                    db.entries[i].game.clear_curations();
                    db.entries[i].dirty = true;
                    if tv_format.is_some() && !tv {
                        return error_result("tv_format applies to VCS releases only");
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
                        db.entries[i].game.clear_curations();
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
                match db.entries[i].game.remove_empty_release(index as usize) {
                    Ok(()) => {
                        db.entries[i].game.clear_curations();
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
                let (Some(key), Some(sha1), Some(label)) =
                    (str_arg("key"), str_arg("sha1"), str_arg("label"))
                else {
                    return error_result("missing key, sha1, or label");
                };
                let Some(i) = self.find_entry(key) else {
                    return error_result(format!("no entry {key}"));
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                if db.entries[i].game.set_artifact_label(sha1, label) {
                    db.entries[i].dirty = true;
                    text_result(format!("labelled {sha1} {label:?}"))
                } else {
                    error_result(format!("{sha1} is not an artifact of {key}"))
                }
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
                let mut needles = vec![missingno_gamedb::normalized_title(entry.game.title())];
                for release_title in entry.game.release_titles() {
                    needles.push(missingno_gamedb::normalized_title(&release_title));
                }
                needles.retain(|n| !n.is_empty());
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
                            if other.game.curations().is_empty() {
                                ""
                            } else {
                                " [curated]"
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
            "resolve_flag" => {
                let Some(id) = args.get("id").and_then(serde_json::Value::as_u64) else {
                    return error_result("missing id");
                };
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let Some(flag) = db.flags.flags.iter_mut().find(|f| f.id == id as u32) else {
                    return error_result(format!("no flag #{id}"));
                };
                if flag.resolved.is_some() {
                    return error_result(format!("flag #{id} already resolved"));
                }
                flag.resolved = Some(Db::today());
                match db.save_flags() {
                    Ok(()) => text_result(format!("flag #{id} resolved")),
                    Err(e) => error_result(format!("flag save failed: {e}")),
                }
            }
            other => error_result(format!("unknown tool: {other}")),
        }
    }

    /// One dump's row: playing marker, short hash, label input, local
    /// filename, and a Play button that switches sessions directly.
    fn artifact_row(&self, sha1: &str, label: &str) -> Element<'_, Message> {
        let short = format!("{}…", &sha1[..12]);
        let local = self
            .rom_index
            .as_ref()
            .and_then(|index| index.by_sha1.get(sha1));
        let playable = local.is_some() || self.rom_cache.contains_key(sha1);
        let playing_this = self.playing_sha1.as_deref() == Some(sha1);
        let mut line = row![].spacing(8).align_y(iced::Alignment::Center);
        line = line.push(
            text(if playing_this {
                format!("▶ {short}")
            } else {
                short
            })
            .size(12),
        );
        line = line.push({
            let sha1_for_label = sha1.to_owned();
            text_input("label…", label)
                .on_input(move |v| Message::SetArtifactLabel(sha1_for_label.clone(), v))
                .size(12)
                .width(Length::Fixed(140.0))
        });
        if let Some(path) = local {
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            line = line.push(text(file).size(11).width(Length::Fill));
        } else {
            line = line.push(Space::new().width(Length::Fill));
        }
        if let (Ok(db), Some(i)) = (&self.db, self.selected) {
            let marks = db.entries[i].game.verification_marks(sha1);
            if !marks.is_empty() {
                line = line.push(text(marks.join(" ")).size(11));
            }
        }
        if playing_this {
            line = line.push(
                button(text("✓ works").size(11)).on_press(Message::RecordPlaytest(sha1.to_owned())),
            );
        }
        if playable {
            let source = if let Some(path) = local {
                BootSource::File(path.clone())
            } else {
                BootSource::Cached(sha1.to_owned())
            };
            line = line.push(
                button(text(if playing_this { "playing" } else { "Play ▶" }).size(11))
                    .on_press_maybe((!playing_this).then_some(Message::Play(source))),
            );
        }
        line.into()
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
                top = top.push(text(entry.game.title().to_owned()).size(22));
                if !self.queue.is_empty() {
                    top = top.push(text(format!("· {} in queue", self.queue.len())).size(13));
                }
                top = top.push(Space::new().width(Length::Fill));
                match &self.flag_note {
                    Some(note) => {
                        top = top.push(
                            text_input("what's wrong with this entry?", note)
                                .on_input(Message::FlagNote)
                                .on_submit(Message::SaveFlag)
                                .width(Length::Fixed(360.0)),
                        );
                        top = top.push(
                            button(text("Save flag"))
                                .style(button::danger)
                                .on_press(Message::SaveFlag),
                        );
                        top = top.push(button(text("✕")).on_press(Message::FlagPrompt));
                    }
                    None => {
                        top = top.push(
                            button(text("Accept ✓")).on_press(Message::Accept { recommend: false }),
                        );
                        top = top.push(
                            button(text("Accept ★ recommend"))
                                .on_press(Message::Accept { recommend: true }),
                        );
                        top = top.push(
                            button(text("⚑ Flag"))
                                .style(button::danger)
                                .on_press(Message::FlagPrompt),
                        );
                    }
                }
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
            text(format!("{} match(es)", visible.len())).size(13),
        ]
        .spacing(8);

        let mut list = column![].spacing(2);
        for &i in visible.iter().take(LIST_LIMIT) {
            let entry = &db.entries[i];
            let marker = if entry.game.curations().is_empty() {
                ""
            } else if entry.game.curations().iter().any(|c| c.recommended) {
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
                let field = |label: &'static str, field: TextField| {
                    row![
                        text(label).width(Length::Fixed(90.0)).size(14),
                        text_input("", &entry.game.text_field(field))
                            .on_input(move |v| Message::Edit(field, v))
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                };
                let mut editor = column![
                    text(entry.game.title().to_owned()).size(22),
                    row![
                        text(format!(
                            "{} · {:?}{}{}",
                            entry.key(),
                            entry.game.kind(),
                            if entry.game.curations().is_empty() {
                                String::new()
                            } else {
                                format!(
                                    " · curated by {}",
                                    entry
                                        .game
                                        .curations()
                                        .iter()
                                        .map(|c| format!(
                                            "{}{} {}",
                                            c.by,
                                            if c.recommended { " ★" } else { "" },
                                            c.date
                                        ))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            },
                            if entry.dirty { " · edited" } else { "" },
                        ))
                        .size(13),
                        button(text("✎ rename").size(12))
                            .style(button::text)
                            .on_press(Message::SlugPrompt),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                ]
                .spacing(10);
                if let Some(slug) = &self.slug_edit {
                    editor = editor.push(
                        row![
                            text("Slug").width(Length::Fixed(90.0)).size(14),
                            text_input("new-slug", slug)
                                .on_input(Message::SlugInput)
                                .on_submit(Message::ApplySlug)
                                .size(13)
                                .width(Length::Fixed(280.0)),
                            button(text("Rename").size(13)).on_press(Message::ApplySlug),
                            button(text("✕").size(13))
                                .style(button::text)
                                .on_press(Message::SlugPrompt),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                    );
                }
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
                editor = editor
                    .push(field("Title", TextField::Title))
                    .push(field("Developer", TextField::Developer))
                    .push(
                        row![
                            text("Description").width(Length::Fixed(90.0)).size(14),
                            iced::widget::text_editor(&self.description)
                                .placeholder("multi-line description…")
                                .on_action(Message::DescriptionAction)
                                .height(Length::Shrink),
                        ]
                        .spacing(8),
                    )
                    .push(field("License", TextField::License));
                let tags = entry.game.tags();
                if !tags.is_empty() {
                    editor = editor.push(
                        row![
                            text("Tags").size(13).width(Length::Fixed(90.0)),
                            text(tags.join(", ")).size(13),
                        ]
                        .spacing(8),
                    );
                }
                let links = entry.game.links();
                if !links.is_empty() {
                    editor = editor.push(text("Links").size(13));
                    for (name, url) in links {
                        editor = editor.push(
                            row![
                                button(text(name).size(13))
                                    .style(button::text)
                                    .on_press(Message::OpenLink(url.clone())),
                                text(url).size(12).width(Length::Fill),
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                        );
                    }
                }
                editor = editor.push(text("Releases").size(16));
                for (r, line) in entry.game.release_lines().into_iter().enumerate() {
                    editor = editor.push(
                        container(
                            column![
                                text(line).size(13),
                                {
                                    let mut artifacts = column![].spacing(2);
                                    for (sha1, label) in entry.game.release_artifacts(r) {
                                        artifacts =
                                            artifacts.push(self.artifact_row(&sha1, &label));
                                    }
                                    artifacts
                                },
                                row![
                                    text("Publisher").size(13).width(Length::Fixed(90.0)),
                                    text_input("publisher…", &entry.game.release_publisher(r))
                                        .on_input(move |v| Message::EditReleasePublisher(r, v))
                                        .size(13)
                                        .width(Length::Fixed(220.0)),
                                ]
                                .spacing(8)
                                .align_y(iced::Alignment::Center),
                            ]
                            .spacing(6),
                        )
                        .padding([6, 10])
                        .style(container::rounded_box),
                    );
                }
                let entry_key = entry.key();

                let mods = entry.game.mod_lines();
                if !mods.is_empty() {
                    editor = editor.push(text("Mods").size(16));
                    for (index, (line, links)) in mods.into_iter().enumerate() {
                        editor = editor.push(
                            row![
                                text(format!("• {line}")).size(13).width(Length::Fill),
                                button(text("✓").size(12)).on_press(Message::CurateMod {
                                    index,
                                    recommend: false,
                                }),
                                button(text("★").size(12)).on_press(Message::CurateMod {
                                    index,
                                    recommend: true,
                                }),
                            ]
                            .spacing(6)
                            .align_y(iced::Alignment::Center),
                        );
                        for (name, url) in links {
                            editor = editor.push(
                                row![
                                    Space::new().width(Length::Fixed(16.0)),
                                    button(text(name).size(12))
                                        .style(button::text)
                                        .on_press(Message::OpenLink(url.clone())),
                                    text(url).size(11).width(Length::Fill),
                                ]
                                .spacing(8)
                                .align_y(iced::Alignment::Center),
                            );
                        }
                        for (sha1, label) in entry.game.mod_artifacts(index) {
                            editor = editor.push(
                                row![
                                    Space::new().width(Length::Fixed(16.0)),
                                    self.artifact_row(&sha1, &label),
                                ]
                                .align_y(iced::Alignment::Center),
                            );
                        }
                    }
                }
                editor = editor.push(text("Verify").size(16));
                if let Some(line) = self.verify_status.get(&entry_key) {
                    editor = editor.push(text(line.clone()).size(13));
                }
                if !entry.game.artifact_sha1s().is_empty() {
                    editor = editor.push(
                        button(text("Hasheous: cover & wiki").size(13))
                            .on_press_maybe((!self.enriching).then_some(Message::Enrich)),
                    );
                }
                if entry.game.download_url().is_some() {
                    editor = editor.push(button(text("Fetch & hash").size(13)).on_press_maybe(
                        (!self.fetching).then_some(Message::Fetch { play_after: false }),
                    ));
                }
                if let Some(sha1) = self.fetched_sha1.get(&entry_key) {
                    editor = editor.push(
                        button(text("Boot fetched ROM (300 frames)").size(13)).on_press_maybe(
                            (!self.booting)
                                .then(|| Message::Boot(BootSource::Cached(sha1.clone()))),
                        ),
                    );
                }
                match &self.rom_index {
                    Some(index) => {
                        for sha1 in entry.game.artifact_sha1s() {
                            if let Some(path) = index.by_sha1.get(&sha1) {
                                editor = editor.push(
                                    row![
                                        text(format!(
                                            "local dump {}…: {}",
                                            &sha1[..12],
                                            path.display()
                                        ))
                                        .size(13)
                                        .width(Length::Fill),
                                        button(text("Boot").size(13)).on_press_maybe(
                                            (!self.booting).then(|| Message::Boot(
                                                BootSource::File(path.clone())
                                            )),
                                        ),
                                    ]
                                    .spacing(8),
                                );
                            }
                        }
                    }
                    None if self.rom_dir.is_some() => {
                        editor =
                            editor.push(text("scan the ROM dir to match local dumps").size(13));
                    }
                    None => {}
                }
                if let Some((_, shot)) = self.boot_shots.get(&entry_key) {
                    editor =
                        editor.push(iced::widget::image(shot.clone()).width(Length::Fixed(320.0)));
                }

                let related: Vec<_> = db
                    .flags
                    .open()
                    .filter(|f| f.subject.contains(&entry_key))
                    .collect();
                if !related.is_empty() {
                    editor = editor.push(text("Flags").size(16));
                    for flag in related {
                        editor = editor.push(
                            row![
                                text(flag.note.clone()).size(13).width(Length::Fill),
                                button(text("Resolve").size(13))
                                    .on_press(Message::ResolveFlag(flag.id)),
                            ]
                            .spacing(8),
                        );
                    }
                }
                if self.playing.is_none()
                    && let Some(note) = self.agent_notes.get(&entry_key)
                {
                    editor = editor.push(text("Agent notes").size(15));
                    editor = editor.push(text(note.clone()).size(12));
                }
                scrollable(editor.padding(4)).into()
            }
            None => container(text("select an entry")).padding(20).into(),
        };

        let left: Option<Element<'_, Message>> = self.list_visible.then_some(left.into());
        let body: Element<'_, Message> = match &self.playing {
            Some((key, _)) => {
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
                if let Some(frame) = &self.play_frame {
                    pane = pane.push(
                        container(
                            iced::widget::image(frame.clone())
                                .filter_method(iced::widget::image::FilterMethod::Nearest)
                                .content_fit(iced::ContentFit::Contain)
                                .width(Length::Fill)
                                .height(Length::Fill),
                        )
                        .width(Length::Fill)
                        .height(Length::Fill),
                    );
                }
                if let Some(note) = self.agent_notes.get(key) {
                    pane = pane.push(text("Agent notes").size(15));
                    pane = pane
                        .push(scrollable(text(note.clone()).size(13)).height(Length::Fixed(160.0)));
                }
                let mut body = row![].spacing(16);
                if let Some(left) = left {
                    body = body.push(left);
                }
                body.push(container(right).width(Length::FillPortion(2)))
                    .push(container(pane).width(Length::FillPortion(3)))
                    .into()
            }
            None => {
                let mut body = row![].spacing(16);
                if let Some(left) = left {
                    body = body.push(left);
                }
                body.push(container(right).width(Length::Fill)).into()
            }
        };

        let mut bottom = row![].spacing(14).align_y(iced::Alignment::Center);
        bottom = bottom.push(text(&self.status).size(12).width(Length::Fill));
        for tree in TreeId::ALL {
            bottom =
                bottom.push(text(format!("{} {}", tree.label(), db.backlog_count(tree))).size(12));
        }
        bottom = bottom.push(text(format!("flags {}", db.flags.open().count())).size(12));
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
        if self.rom_dir.is_some() {
            let label = match &self.rom_index {
                Some(index) => format!("rescan ROMs ({})", index.scanned),
                None => "scan ROM dir".to_owned(),
            };
            bottom = bottom.push(
                button(text(label).size(12))
                    .style(button::text)
                    .on_press_maybe((!self.scanning).then_some(Message::ScanRoms)),
            );
        }
        bottom = bottom.push(
            button(text(format!("Commit ({})", db.uncommitted)).size(12))
                .on_press_maybe((db.uncommitted > 0).then_some(Message::Commit)),
        );

        column![top, body, bottom].spacing(12).padding(12).into()
    }
}
