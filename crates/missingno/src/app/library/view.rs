use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{Column, column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self,
    core::{
        buttons,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    load,
};

use crate::app::library;

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

use crate::app::core::fonts;

/// Deterministic accent colour from a title string, using Catppuccin Mocha accents
/// darkened to work as backgrounds with white text.
fn title_color(title: &str) -> Color {
    const ACCENTS: &[[f32; 3]] = &[
        [0.52, 0.24, 0.44], // mauve
        [0.44, 0.22, 0.50], // lavender-ish
        [0.20, 0.36, 0.52], // blue
        [0.16, 0.40, 0.44], // teal
        [0.24, 0.42, 0.28], // green
        [0.52, 0.40, 0.16], // yellow
        [0.52, 0.28, 0.16], // peach
        [0.52, 0.20, 0.24], // red
    ];
    let hash = title
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let [r, g, b] = ACCENTS[(hash as usize) % ACCENTS.len()];
    Color::from_rgb(r, g, b)
}

const COVER_HEIGHT: f32 = 160.0;
const COVER_WIDTH: f32 = 120.0;
const CARD_MIN_WIDTH: f32 = 400.0;

#[derive(Debug, Clone)]
pub enum Message {
    SelectGame(String),
    QuickPlay(String),
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Library(message)
    }
}

pub struct LibraryCache {
    pub entries: Vec<CachedGame>,
}

pub struct CachedGame {
    pub entry: library::GameEntry,
    pub cover: Option<image::Handle>,
    pub play_time: String,
    pub last_played: Option<String>,
    pub last_played_ts: Option<jiff::Timestamp>,
    pub save_count: usize,
}

impl LibraryCache {
    pub fn load() -> Self {
        let games = library::list_all();
        let entries = games
            .into_iter()
            .map(|(game_dir, entry)| {
                let cover = library::load_thumbnail(&game_dir)
                    .map(|bytes| image::Handle::from_bytes(bytes));

                let play_log = library::play_log::load(&game_dir);
                let play_time = play_log.format_play_time();
                let last_played_ts = play_log.last_played;
                let last_played = last_played_ts.map(|ts| friendly_ago(ts));

                let save_manifest = library::saves::load_manifest(&game_dir);
                let save_count = save_manifest.saves.len();

                CachedGame {
                    entry,
                    cover,
                    play_time,
                    last_played,
                    last_played_ts,
                    save_count,
                }
            })
            .collect();
        let mut cache = Self { entries };
        cache.sort();
        cache
    }

    /// Sort: recently played first, then alphabetically for never-played games.
    fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            match (&a.last_played_ts, &b.last_played_ts) {
                // Both played — most recent first
                (Some(a_ts), Some(b_ts)) => b_ts.cmp(a_ts),
                // Played beats never-played
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                // Neither played — alphabetical
                (None, None) => a.entry.display_title().to_lowercase()
                    .cmp(&b.entry.display_title().to_lowercase()),
            }
        });
    }

    /// Update a single entry in-place by SHA1, reloading only its data from disk.
    pub fn update_entry(&mut self, sha1: &str) {
        let Some((game_dir, entry)) = library::find_by_sha1(sha1) else {
            return;
        };

        let cover =
            library::load_thumbnail(&game_dir).map(|bytes| image::Handle::from_bytes(bytes));
        let play_log = library::play_log::load(&game_dir);
        let play_time = play_log.format_play_time();
        let last_played_ts = play_log.last_played;
        let last_played = last_played_ts.map(|ts| friendly_ago(ts));
        let save_manifest = library::saves::load_manifest(&game_dir);
        let save_count = save_manifest.saves.len();

        let cached = CachedGame {
            entry,
            cover,
            play_time,
            last_played,
            last_played_ts,
            save_count,
        };

        if let Some(existing) = self.entries.iter_mut().find(|e| e.entry.sha1 == sha1) {
            *existing = cached;
        } else {
            self.entries.push(cached);
        }
        self.sort();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[allow(private_interfaces)]
pub(crate) fn view(cache: &LibraryCache) -> Element<'_, app::Message> {
    if cache.is_empty() {
        return empty_view();
    }

    iced::widget::responsive(|size| {
        let usable = size.width - l() * 2.0;
        let cols = (usable / (CARD_MIN_WIDTH + m())).max(1.0) as usize;

        let mut rows_vec: Vec<Element<'_, app::Message>> = Vec::new();
        let chunks: Vec<&[CachedGame]> = cache.entries.chunks(cols).collect();

        for chunk in chunks {
            let mut cards: Vec<Element<'_, app::Message>> =
                chunk.iter().map(|game| game_card(game)).collect();
            // Pad incomplete rows with empty spacers so cards don't stretch
            while cards.len() < cols {
                cards.push(iced::widget::Space::new().width(Fill).into());
            }
            rows_vec.push(row(cards).spacing(m()).into());
        }

        scrollable(
            container(Column::with_children(rows_vec).spacing(m()).padding(l())).center_x(Fill),
        )
        .height(Fill)
        .into()
    })
    .into()
}

fn empty_view() -> Element<'static, app::Message> {
    container(
        column![
            iced::widget::svg(iced::advanced::svg::Handle::from_memory(include_bytes!(
                "../../app/core/icons/missingno.svg"
            ),))
            .width(120)
            .height(120)
            .style(|_, _| iced::widget::svg::Style { color: None }),
            app_text::heading("Welcome to Missingno"),
            column![
                text("Add a ROM file, or point Missingno at a folder").color(MUTED),
                text("of ROMs in Settings and they'll appear here.").color(MUTED),
            ]
            .align_x(Center),
            row![
                buttons::primary("Add Game...").on_press(load::Message::Pick.into()),
                buttons::standard("Settings").on_press(app::Message::ShowSettings),
            ]
            .spacing(s()),
        ]
        .spacing(l())
        .align_x(Center),
    )
    .center(Fill)
    .into()
}

fn game_card(game: &CachedGame) -> Element<'_, app::Message> {
    let has_rom = game.entry.rom_paths.first().is_some();

    // Cover art
    let cover: Element<'_, app::Message> = if let Some(handle) = &game.cover {
        image(handle.clone())
            .width(COVER_WIDTH)
            .height(COVER_HEIGHT)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(iced::border::Radius {
                top_left: 0.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 0.0,
            })
            .into()
    } else {
        let initial = game
            .entry
            .display_title()
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .next()
            .unwrap_or('?');
        let bg = title_color(&game.entry.display_title());

        container(
            text(initial)
                .size(COVER_HEIGHT * 0.35)
                .font(fonts::heading())
                .color(Color::WHITE),
        )
        .width(COVER_WIDTH)
        .height(COVER_HEIGHT)
        .align_x(Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(bg.into()),
            border: iced::Border {
                radius: iced::border::Radius {
                    top_left: 8.0,
                    top_right: 0.0,
                    bottom_right: 0.0,
                    bottom_left: 8.0,
                },
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };

    // Title — bold, readable size
    let mut info = column![text(game.entry.display_title()).font(fonts::bold()),].spacing(4);

    // Publisher · Year
    let subtitle_parts: Vec<&str> = [game.entry.publisher.as_deref(), game.entry.year.as_deref()]
        .into_iter()
        .flatten()
        .collect();

    if !subtitle_parts.is_empty() {
        info = info.push(app_text::detail(subtitle_parts.join(" · ")).color(MUTED));
    }

    // Last played / play time
    if let Some(last) = &game.last_played {
        info = info
            .push(app_text::detail(format!("Played {} · {}", last, game.play_time)).color(MUTED));
    } else if game.save_count > 0 {
        // Only show save count if we don't already have play info
        let n = game.save_count;
        info = info.push(
            app_text::detail(format!("{n} save{}", if n == 1 { "" } else { "s" })).color(MUTED),
        );
    }

    // Card layout: cover (flush left) | padded info | quick-play
    let mut info_row = row![info.width(Fill)].spacing(m()).align_y(Center);

    if has_rom {
        info_row = info_row.push(
            buttons::subtle(icons::m(Icon::Front))
                .on_press(Message::QuickPlay(game.entry.sha1.clone()).into()),
        );
    }

    let card_row = row![cover, container(info_row).padding(m()).width(Fill)].height(COVER_HEIGHT);

    let card = container(card_row)
        .width(Fill)
        .clip(true)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border::default().rounded(8),
                ..Default::default()
            }
        });

    mouse_area(card)
        .on_press(Message::SelectGame(game.entry.sha1.clone()).into())
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

fn friendly_ago(timestamp: jiff::Timestamp) -> String {
    let secs = jiff::Timestamp::now().duration_since(timestamp).as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{mins} minutes ago")
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "yesterday".to_string()
        } else {
            format!("{days} days ago")
        }
    }
}
