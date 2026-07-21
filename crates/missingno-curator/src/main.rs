//! missingno-curator — review, enrich, and confirm gamedb entries.
//!
//! v1: Backlog (uncurated entries) and Flags drain through one list+editor
//! screen; confirms stamp `curated` and accumulate into explicit git commits.

mod db;
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
}

pub fn main() -> iced::Result {
    let args = Args::parse();
    let db_path = args.db_path.clone();
    let rom_dir = args.rom_dir.clone();
    let remote = !args.no_remote;
    iced::application(
        move || Curator::new(db_path.clone(), rom_dir.clone(), remote),
        Curator::update,
        Curator::view,
    )
    .title(Curator::title)
    .theme(Curator::theme)
    .subscription(Curator::subscription)
    .window_size(iced::Size::new(1280.0, 800.0))
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
    /// entry key → last fetched sha1 (bytes live in rom_cache).
    fetched_sha1: std::collections::HashMap<String, String>,
    /// entry key → boot note + screenshot.
    boot_shots: std::collections::HashMap<String, (String, iced::widget::image::Handle)>,
    remote_sink: SharedSink,
    _remote: Option<RemoteEndpoint>,
}

#[derive(Debug, Clone)]
enum Message {
    Remote(Bridge),
    Boot(BootSource),
    Booted(String, Result<BootDone, String>),
    Fetch,
    Fetched(String, Result<(String, std::sync::Arc<Vec<u8>>), String>),
    ScanRoms,
    ScannedRoms(Result<std::sync::Arc<RomIndex>, String>),
    FilterTree(TreeChoice),
    OnlyBacklog(bool),
    OnlyFlagged(bool),
    Search(String),
    Select(usize),
    Edit(TextField, String),
    ConfirmAndNext,
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
        if self._remote.is_some() {
            iced::Subscription::run(remote::worker).map(Message::Remote)
        } else {
            iced::Subscription::none()
        }
    }

    fn new(db_path: PathBuf, rom_dir: Option<PathBuf>, remote: bool) -> (Self, Task<Message>) {
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
                fetched_sha1: std::collections::HashMap::new(),
                boot_shots: std::collections::HashMap::new(),
                remote_sink,
                _remote: endpoint,
            },
            Task::none(),
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
            .filter(|(_, e)| !self.only_backlog || e.game.curated().is_none())
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
                let body = self.run_tool(&call.name, &call.args);
                let _ = call.reply.send(body);
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
                    let tv = entry.game.tv_hint();
                    let bytes = match &source {
                        BootSource::Cached(sha1) => self.rom_cache.get(sha1).cloned(),
                        BootSource::File(path) => std::fs::read(path).ok().map(std::sync::Arc::new),
                    };
                    let Some(bytes) = bytes else {
                        self.status = "no ROM bytes to boot".to_owned();
                        return Task::none();
                    };
                    self.booting = true;
                    self.verify_status
                        .insert(key.clone(), "booting (300 frames)…".to_owned());
                    return Task::perform(
                        smol::unblock(move || {
                            verify::boot_check(hint, &bytes, tv, 300).map(|shot| BootDone {
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
            Message::Fetch => {
                if let (Ok(db), Some(i)) = (&self.db, self.selected) {
                    let entry = &db.entries[i];
                    let Some(url) = entry.game.download_url() else {
                        return Task::none();
                    };
                    let key = entry.key();
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
                        self.verify_status.insert(key, line);
                    }
                    Err(e) => {
                        self.verify_status.insert(key, e);
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
            Message::Select(index) => self.selected = Some(index),
            Message::Edit(field, value) => {
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    db.entries[i].game.set_text_field(field, value);
                    db.entries[i].dirty = true;
                }
            }
            Message::ConfirmAndNext => {
                let visible = self.visible();
                if let (Ok(db), Some(i)) = (&mut self.db, self.selected) {
                    db.entries[i].game.set_curated(Some(Db::today()));
                    match db.write_entry(i) {
                        Ok(()) => self.status = format!("confirmed {}", db.entries[i].key()),
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

    fn find_entry(&self, key: &str) -> Option<usize> {
        let Ok(db) = &self.db else { return None };
        db.entries.iter().position(|e| e.key() == key)
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
                    if backlog_only && e.game.curated().is_some() {
                        continue;
                    }
                    if !e.game.title().to_lowercase().contains(&query) && !e.slug.contains(&query) {
                        continue;
                    }
                    lines.push(format!(
                        "{} — {}{}",
                        e.key(),
                        e.game.title(),
                        if e.game.curated().is_some() {
                            " [curated]"
                        } else {
                            ""
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
                let Ok(db) = &mut self.db else {
                    return error_result("db not loaded");
                };
                let entry = &mut db.entries[i];
                let mut applied = Vec::new();
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
                if applied.is_empty() {
                    return error_result("no recognized fields in set");
                }
                // Automation touching a curated entry re-opens it for review.
                entry.game.set_curated(None);
                entry.dirty = true;
                self.selected = Some(i);
                text_result(format!(
                    "staged {} on {key}; entry re-opened for review (uncommitted until the curator confirms)",
                    applied.join(", ")
                ))
            }
            "select_game" => {
                let Some(key) = str_arg("key") else {
                    return error_result("missing key");
                };
                match self.find_entry(key) {
                    Some(i) => {
                        self.selected = Some(i);
                        text_result(format!("showing {key}"))
                    }
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

    fn view(&self) -> Element<'_, Message> {
        let db = match &self.db {
            Ok(db) => db,
            Err(e) => {
                return container(text(format!("failed to open gamedb: {e}")))
                    .padding(20)
                    .into();
            }
        };

        // ── Top bar: queue counts + commit ────────────────────────────
        let mut top = row![].spacing(16).align_y(iced::Alignment::Center);
        for tree in TreeId::ALL {
            top = top.push(text(format!(
                "{}: {} to review",
                tree.label(),
                db.backlog_count(tree)
            )));
        }
        top = top.push(text(format!("flags: {}", db.flags.open().count())));
        top = top.push(Space::new().width(Length::Fill));
        top = top.push(text(&self.status).size(13));
        if self.rom_dir.is_some() {
            let label = match &self.rom_index {
                Some(index) => format!("Rescan ROMs ({})", index.scanned),
                None => "Scan ROM dir".to_owned(),
            };
            top = top.push(
                button(text(label).size(14))
                    .on_press_maybe((!self.scanning).then_some(Message::ScanRoms)),
            );
        }
        top = top.push(
            button(text(format!("Commit ({})", db.uncommitted)))
                .on_press_maybe((db.uncommitted > 0).then_some(Message::Commit)),
        );

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
            let marker = if entry.game.curated().is_some() {
                "✓ "
            } else {
                ""
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
            .width(Length::Fixed(420.0));

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
                    text(format!(
                        "{} · {:?}{}{}",
                        entry.key(),
                        entry.game.kind(),
                        entry
                            .game
                            .curated()
                            .map(|d| format!(" · curated {d}"))
                            .unwrap_or_default(),
                        if entry.dirty { " · edited" } else { "" },
                    ))
                    .size(13),
                    field("Title", TextField::Title),
                    field("Developer", TextField::Developer),
                    field("Description", TextField::Description),
                    field("License", TextField::License),
                    text("Releases").size(16),
                ]
                .spacing(10);
                for line in entry.game.release_lines() {
                    editor = editor.push(text(format!("• {line}")).size(13));
                }
                let entry_key = entry.key();

                editor = editor.push(text("Verify").size(16));
                if let Some(line) = self.verify_status.get(&entry_key) {
                    editor = editor.push(text(line.clone()).size(13));
                }
                if entry.game.download_url().is_some() {
                    editor = editor.push(
                        button(text("Fetch & hash").size(13))
                            .on_press_maybe((!self.fetching).then_some(Message::Fetch)),
                    );
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
                editor = editor.push(
                    button(text("Confirm ✓ (stamp curated & next)"))
                        .on_press(Message::ConfirmAndNext),
                );
                scrollable(editor.padding(4)).into()
            }
            None => container(text("select an entry")).padding(20).into(),
        };

        column![
            top,
            row![left, container(right).width(Length::Fill)].spacing(16)
        ]
        .spacing(12)
        .padding(12)
        .into()
    }
}
