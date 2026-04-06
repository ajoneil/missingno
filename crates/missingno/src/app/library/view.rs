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
const CARD_MIN_WIDTH: f32 = 340.0;

#[derive(Debug, Clone)]
pub enum Message {
    SelectGame(String),
    QuickPlay(String),
    HoverGame(String),
    UnhoverGame,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Library(message)
    }
}

use super::store::{GameStore, GameSummary};

#[allow(private_interfaces)]
pub(crate) fn view<'a>(
    store: &'a GameStore,
    hovered_sha1: Option<&'a str>,
) -> Element<'a, app::Message> {
    if store.is_empty() {
        return empty_view();
    }

    let games = store.all_summaries();
    let hovered_sha1 = hovered_sha1.map(|s| s.to_string());
    iced::widget::responsive(move |size| {
        let usable = size.width - l() * 2.0;
        let cols = (usable / (CARD_MIN_WIDTH + m())).max(1.0) as usize;

        let mut rows_vec: Vec<Element<'_, app::Message>> = Vec::new();

        for chunk in games.chunks(cols) {
            let mut cards: Vec<Element<'_, app::Message>> = chunk
                .iter()
                .map(|game| {
                    let hovered = hovered_sha1.as_deref() == Some(game.entry.sha1.as_str());
                    game_card(game, hovered)
                })
                .collect();
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

fn game_card(game: &GameSummary, hovered: bool) -> Element<'_, app::Message> {
    use iced::widget::stack;

    let has_rom = game.entry.rom_paths.first().is_some();
    let sha1 = &game.entry.sha1;

    // Cover art
    let cover_image: Element<'_, app::Message> = if let Some(handle) = &game.thumbnail {
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

    // Overlay play button on cover when hovered
    let cover: Element<'_, app::Message> = if hovered && has_rom {
        use iced::widget::button;

        stack![
            cover_image,
            container(iced::widget::Space::new())
                .width(COVER_WIDTH)
                .height(COVER_HEIGHT)
                .style(|_: &iced::Theme| container::Style {
                    background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4).into()),
                    ..Default::default()
                }),
            container(
                button(
                    icons::xl(Icon::Play).style(|_, _| iced::widget::svg::Style {
                        color: Some(Color::WHITE),
                    }),
                )
                .on_press(Message::QuickPlay(sha1.clone()).into())
                .style(|_: &iced::Theme, status| {
                    let bg_alpha = match status {
                        button::Status::Hovered => 0.8,
                        _ => 0.5,
                    };
                    button::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, bg_alpha).into()),
                        text_color: Color::WHITE,
                        border: iced::Border::default().rounded(24),
                        ..Default::default()
                    }
                }),
            )
            .width(COVER_WIDTH)
            .height(COVER_HEIGHT)
            .align_x(Center)
            .align_y(iced::alignment::Vertical::Center)
        ]
        .into()
    } else {
        cover_image
    };

    // Title — bold, readable size
    let mut info = column![text(game.entry.display_title()).font(fonts::bold()),].spacing(4);

    // Publisher · Date
    let subtitle_parts: Vec<String> = [
        game.entry.publisher.clone(),
        game.entry
            .year
            .as_ref()
            .map(|y| library::activity::format_date_string(y)),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !subtitle_parts.is_empty() {
        info = info.push(app_text::detail(subtitle_parts.join(" · ")).color(MUTED));
    }

    // Last played / play time
    if let Some(last_ts) = game.last_played {
        let last = friendly_ago(last_ts);
        let play_time = library::activity::format_play_time(game.play_time_secs);
        info = info.push(app_text::detail(format!("Played {last} · {play_time}")).color(MUTED));
    } else if game.save_count > 0 {
        let n = game.save_count;
        info = info.push(
            app_text::detail(format!("{n} save{}", if n == 1 { "" } else { "s" })).color(MUTED),
        );
    }

    let card_row =
        row![cover, container(info.width(Fill)).padding(m()).width(Fill)].height(COVER_HEIGHT);

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
        .on_press(Message::SelectGame(sha1.clone()).into())
        .on_enter(Message::HoverGame(sha1.clone()).into())
        .on_exit(Message::UnhoverGame.into())
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
