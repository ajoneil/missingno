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
        sizes::{l, m, s},
        text as app_text,
    },
};

use crate::app::library;

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

#[derive(Debug, Clone)]
pub enum Message {
    PlayGame(String), // sha1
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
                let cover = library::load_cover(&game_dir)
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

    let games = Column::with_children(
        cache
            .entries
            .iter()
            .map(|game| game_row(game))
    )
    .spacing(s());

    column![
        app_text::xl("Library"),
        scrollable(games).height(Fill),
    ]
    .spacing(l())
    .padding(l())
    .into()
}

fn empty_view() -> Element<'static, app::Message> {
    container(
        column![
            app_text::xl("Library"),
            text("No games yet. Load a ROM or add a ROM folder in Settings.").color(MUTED),
        ]
        .spacing(l()),
    )
    .padding(l())
    .into()
}

fn game_row(game: &CachedGame) -> Element<'_, app::Message> {
    let rom_path = game.entry.rom_paths.first();

    let mut content = row![].spacing(m()).align_y(Center);

    if let Some(cover) = &game.cover {
        content = content.push(
            image(cover.clone())
                .height(48)
                .content_fit(iced::ContentFit::ScaleDown),
        );
    }

    let mut info = column![text(&game.entry.title)].spacing(2);

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

    let btn = buttons::subtle(content).width(Fill);

    if rom_path.is_some() {
        btn.on_press(Message::PlayGame(game.entry.sha1.clone()).into())
            .into()
    } else {
        btn.into()
    }
}
