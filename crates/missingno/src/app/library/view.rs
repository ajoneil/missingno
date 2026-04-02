use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{column, container, image, row, scrollable, text, Column},
};

use crate::app::{
    self,
    core::{
        buttons,
        icons::{self, Icon},
        sizes::{l, m, s},
    },
};

use crate::app::library;

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

const COVER_HEIGHT: f32 = 120.0;
const COVER_WIDTH: f32 = 90.0;

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
}

impl LibraryCache {
    pub fn load() -> Self {
        let games = library::list_all();
        let entries = games
            .into_iter()
            .map(|(game_dir, entry)| {
                let cover = library::load_thumbnail(&game_dir)
                    .map(|bytes| image::Handle::from_bytes(bytes));
                CachedGame { entry, cover }
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

    let games = Column::with_children(cache.entries.iter().map(|game| game_row(game))).spacing(s());

    scrollable(
        container(games)
            .padding(l())
            .center_x(Fill),
    )
    .height(Fill)
    .into()
}

fn empty_view() -> Element<'static, app::Message> {
    container(
        text("No games yet. Load a ROM or add a ROM folder in Settings.").color(MUTED),
    )
    .padding(l())
    .into()
}

fn game_row(game: &CachedGame) -> Element<'_, app::Message> {
    let rom_path = game.entry.rom_paths.first();

    let mut content = row![].spacing(m()).align_y(Center);

    let cover_slot = if let Some(cover) = &game.cover {
        container(
            image(cover.clone())
                .width(COVER_WIDTH)
                .height(COVER_HEIGHT)
                .content_fit(iced::ContentFit::Contain),
        )
        .width(COVER_WIDTH)
        .height(COVER_HEIGHT)
        .center(iced::Length::Shrink)
    } else {
        container(text("").width(COVER_WIDTH).height(COVER_HEIGHT))
            .width(COVER_WIDTH)
            .height(COVER_HEIGHT)
    };

    content = content.push(cover_slot);

    let mut info = column![text(game.entry.display_title())].spacing(2);

    let subtitle_parts: Vec<&str> = [
        game.entry.publisher.as_deref(),
        game.entry.year.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !subtitle_parts.is_empty() {
        info = info.push(text(subtitle_parts.join(" · ")).color(MUTED).size(12));
    }

    content = content.push(info);

    let detail_btn = buttons::subtle(content)
        .width(Fill)
        .on_press(Message::SelectGame(game.entry.sha1.clone()).into());

    if rom_path.is_some() {
        row![
            detail_btn,
            buttons::primary(icons::m(Icon::Front))
                .on_press(Message::QuickPlay(game.entry.sha1.clone()).into()),
        ]
        .spacing(s())
        .align_y(Center)
        .into()
    } else {
        detail_btn.into()
    }
}
