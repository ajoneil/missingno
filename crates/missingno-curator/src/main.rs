//! missingno-curator — review, enrich, and confirm gamedb entries.
//!
//! v1: Backlog (uncurated entries) and Flags drain through one list+editor
//! screen; confirms stamp `curated` and accumulate into explicit git commits.

mod db;

use std::path::PathBuf;

use clap::Parser;
use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input, toggler,
};
use iced::{Element, Length, Task, Theme};

use db::{Db, TextField, TreeId};
#[derive(Parser)]
struct Args {
    /// Path to the missingno-gamedb checkout.
    #[arg(default_value = "missingno-gamedb")]
    db_path: PathBuf,
}

pub fn main() -> iced::Result {
    let args = Args::parse();
    let db_path = args.db_path.clone();
    iced::application(
        move || Curator::new(db_path.clone()),
        Curator::update,
        Curator::view,
    )
    .title(Curator::title)
    .theme(Curator::theme)
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
}

#[derive(Debug, Clone)]
enum Message {
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

    fn new(db_path: PathBuf) -> (Self, Task<Message>) {
        let db = Db::load(db_path).map_err(|e| e.to_string());
        (
            Self {
                db,
                filter_tree: None,
                only_backlog: true,
                only_flagged: false,
                search: String::new(),
                selected: None,
                status: String::new(),
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
