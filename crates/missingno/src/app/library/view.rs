use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{column, container, image, mouse_area, row, scrollable, text, Column},
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

const COVER_HEIGHT: f32 = 100.0;
const COVER_WIDTH: f32 = 75.0;
const CARD_MIN_WIDTH: f32 = 350.0;

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
                let last_played = play_log.last_played.map(|ts| friendly_ago(ts));

                let save_manifest = library::saves::load_manifest(&game_dir);
                let save_count = save_manifest.saves.len();

                CachedGame {
                    entry,
                    cover,
                    play_time,
                    last_played,
                    save_count,
                }
            })
            .collect();
        Self { entries }
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
            let cards: Vec<Element<'_, app::Message>> = chunk
                .iter()
                .map(|game| game_card(game))
                .collect();
            rows_vec.push(
                row(cards).spacing(m()).into(),
            );
        }

        scrollable(
            container(Column::with_children(rows_vec).spacing(m()).padding(l()))
                .center_x(Fill),
        )
        .height(Fill)
        .into()
    })
    .into()
}

fn empty_view() -> Element<'static, app::Message> {
    container(
        column![
            iced::widget::svg(
                iced::advanced::svg::Handle::from_memory(
                    include_bytes!("../../app/core/icons/missingno.svg"),
                )
            )
            .width(120)
            .height(120)
            .style(|_, _| iced::widget::svg::Style { color: None }),
            app_text::xl("Welcome to Missingno"),
            text("Add some games to get started.").color(MUTED),
            row![
                buttons::primary("Add Game...")
                    .on_press(load::Message::Pick.into()),
                buttons::standard("Settings")
                    .on_press(app::Message::ShowSettings),
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
    let cover = if let Some(handle) = &game.cover {
        container(
            image(handle.clone())
                .width(COVER_WIDTH)
                .height(COVER_HEIGHT)
                .content_fit(iced::ContentFit::Contain),
        )
        .width(COVER_WIDTH)
        .height(COVER_HEIGHT)
        .center(iced::Length::Shrink)
    } else {
        container(
            iced::widget::Space::new().width(COVER_WIDTH).height(COVER_HEIGHT),
        )
        .width(COVER_WIDTH)
        .height(COVER_HEIGHT)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                ..Default::default()
            }
        })
    };

    // Info column
    let mut info = column![
        text(game.entry.display_title()).size(14),
    ]
    .spacing(3);

    // Publisher · Year
    let subtitle_parts: Vec<&str> = [
        game.entry.publisher.as_deref(),
        game.entry.year.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !subtitle_parts.is_empty() {
        info = info.push(text(subtitle_parts.join(" · ")).color(MUTED).size(11));
    }

    // Last played / play time
    if let Some(last) = &game.last_played {
        info = info.push(
            text(format!("Played {} · {}", last, game.play_time))
                .color(MUTED)
                .size(11),
        );
    }

    // Save count
    if game.save_count > 0 {
        let n = game.save_count;
        info = info.push(
            text(format!("{n} save{}", if n == 1 { "" } else { "s" }))
                .color(MUTED)
                .size(11),
        );
    }

    // Card layout: cover | info | quick-play
    let mut card_row = row![cover, info.width(Fill)]
        .spacing(m())
        .align_y(Center);

    if has_rom {
        card_row = card_row.push(
            buttons::primary(icons::m(Icon::Front))
                .on_press(Message::QuickPlay(game.entry.sha1.clone()).into()),
        );
    }

    let card = container(card_row.padding(s()))
        .width(Fill)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border::default().rounded(6),
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
