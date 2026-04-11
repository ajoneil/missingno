use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{Column, column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self,
    settings::view as settings_view,
    ui::{
        buttons, containers, fonts,
        icons::{self, Icon},
        palette::MUTED,
        sizes::{border_l, border_m, l, m, s},
        text as app_text,
    },
    load,
};

use crate::app::library;

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
    DumpCartridge,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Library(message)
    }
}

use super::store::{GameStore, GameSummary};
use crate::cartridge_rw;

#[allow(private_interfaces)]
pub(crate) fn view<'a>(
    store: &'a GameStore,
    hovered_sha1: Option<&'a str>,
    inserted_cartridge: Option<&'a cartridge_rw::CartridgeHeader>,
    dump_progress: Option<&'a cartridge_rw::DumpProgress>,
    homebrew_enabled: bool,
) -> Element<'a, app::Message> {
    if store.is_empty() && inserted_cartridge.is_none() {
        return empty_view(homebrew_enabled);
    }

    let games = store.all_summaries();

    // Match inserted cartridge against library by raw header title
    let matched_sha1 = inserted_cartridge.and_then(|cart| {
        games
            .iter()
            .find(|g| {
                g.entry
                    .header_title
                    .as_ref()
                    .is_some_and(|ht| ht == &cart.title)
            })
            .map(|g| g.entry.sha1.clone())
    });

    let hovered_sha1 = hovered_sha1.map(|s| s.to_string());
    iced::widget::responsive(move |size| {
        let usable = size.width - l() * 2.0;
        let cols = (usable / (CARD_MIN_WIDTH + m())).max(1.0) as usize;

        let mut content: Vec<Element<'_, app::Message>> = Vec::new();

        // Inserted cartridge section
        if let Some(cart) = inserted_cartridge {
            content.push(app_text::label("Inserted Cartridge").into());

            let matched_game = matched_sha1
                .as_deref()
                .and_then(|sha1| games.iter().find(|g| g.entry.sha1 == sha1));

            let card: Element<'_, app::Message> = if let Some(game) = matched_game {
                let hovered = hovered_sha1.as_deref() == Some(game.entry.sha1.as_str());
                cartridge_game_card(game, hovered)
            } else {
                unmatched_cartridge_card(cart, dump_progress)
            };

            // Pad with spacers to match grid column width
            let mut card_row = vec![card];
            while card_row.len() < cols {
                card_row.push(iced::widget::Space::new().width(Fill).into());
            }
            content.push(row(card_row).spacing(m()).into());

            content.push(iced::widget::Space::new().height(s()).into());
        }

        // Library grid — exclude the matched cartridge game to avoid duplication
        let grid_games: Vec<&&GameSummary> = games
            .iter()
            .filter(|g| matched_sha1.as_deref() != Some(g.entry.sha1.as_str()))
            .collect();

        if !grid_games.is_empty() {
            if inserted_cartridge.is_some() {
                content.push(app_text::label("Library").into());
            }

            for chunk in grid_games.chunks(cols) {
                let mut cards: Vec<Element<'_, app::Message>> = chunk
                    .iter()
                    .map(|game| {
                        let hovered =
                            hovered_sha1.as_deref() == Some(game.entry.sha1.as_str());
                        game_card(game, hovered)
                    })
                    .collect();
                while cards.len() < cols {
                    cards.push(iced::widget::Space::new().width(Fill).into());
                }
                content.push(row(cards).spacing(m()).into());
            }
        }

        scrollable(
            container(Column::with_children(content).spacing(m()).padding(l())).center_x(Fill),
        )
        .height(Fill)
        .into()
    })
    .into()
}

fn empty_view(homebrew_enabled: bool) -> Element<'static, app::Message> {
    let mut actions = column![
        buttons::primary(
            row![icons::m(Icon::FolderOpen), "Add ROM folder..."]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(settings_view::Message::PickRomDirectory.into()),
    ]
    .spacing(s())
    .align_x(Center);

    if homebrew_enabled {
        actions = actions.push(
            buttons::standard(
                row![icons::m(Icon::Globe), "Browse Homebrew"]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(app::Message::OpenHomebrewBrowser),
        );
    }

    actions = actions.push(
        buttons::subtle("Open a ROM file...").on_press(load::Message::Pick.into()),
    );

    container(
        column![
            iced::widget::svg(iced::advanced::svg::Handle::from_memory(include_bytes!(
                "../../app/ui/icons/missingno.svg"
            ),))
            .width(120)
            .height(120)
            .style(|_, _| iced::widget::svg::Style { color: None }),
            app_text::heading("Welcome to Missingno"),
            text("Add a folder of ROMs and Missingno will keep your library in sync.").color(MUTED),
            actions,
        ]
        .spacing(l())
        .align_x(Center)
        .max_width(420),
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
                        border: iced::Border::default().rounded(border_l()),
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
        .style(containers::card);

    mouse_area(card)
        .on_press(Message::SelectGame(sha1.clone()).into())
        .on_enter(Message::HoverGame(sha1.clone()).into())
        .on_exit(Message::UnhoverGame.into())
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// A library game card with a cartridge indicator overlay.
fn cartridge_game_card(game: &GameSummary, hovered: bool) -> Element<'_, app::Message> {
    use iced::widget::stack;

    stack![
        game_card(game, hovered),
        container(
            container(
                row![
                    icons::m(Icon::CircuitBoard).style(|_, _| iced::widget::svg::Style {
                        color: Some(Color::WHITE),
                    }),
                ]
                .spacing(s()),
            )
            .padding(s())
            .style(|_: &iced::Theme| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                border: iced::Border::default().rounded(border_m()),
                ..Default::default()
            }),
        )
        .padding(s())
        .align_right(Fill)
        .align_bottom(Fill),
    ]
    .into()
}

/// A card for an inserted cartridge that doesn't match any library game.
fn unmatched_cartridge_card<'a>(
    cart: &'a cartridge_rw::CartridgeHeader,
    dump_progress: Option<&'a cartridge_rw::DumpProgress>,
) -> Element<'a, app::Message> {
    let display_title = if cart.title.is_empty() {
        if cart.flashable() { "Empty Flash Cart" } else { "Unknown Cartridge" }
    } else {
        &cart.title
    };
    let bg = title_color(display_title);
    let initial = display_title
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .next()
        .unwrap_or('?');

    let cover: Element<'_, app::Message> = container(
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
    .into();

    let mut info = column![text(display_title).font(fonts::bold()),].spacing(4);

    info = info.push(
        app_text::detail(format!(
            "{} · ROM {} · RAM {}",
            cart.mapper_name,
            cart.rom_size_display(),
            cart.ram_size_display(),
        ))
        .color(MUTED),
    );

    if let Some(progress) = dump_progress {
        let pct = if progress.bytes_total > 0 {
            progress.bytes_done as f32 / progress.bytes_total as f32
        } else {
            0.0
        };
        info = info.push(app_text::progress_text(
            "Reading…",
            progress.bytes_done as u32,
            progress.bytes_total as u32,
            MUTED,
        ));
        info = info.push(
            iced::widget::progress_bar(0.0..=1.0, pct)
                .girth(6),
        );
    } else if cart.rom_size > 0 {
        info = info.push(
            buttons::primary("Add to Library").on_press(Message::DumpCartridge.into()),
        );
    }

    let card_row =
        row![cover, container(info.width(Fill)).padding(m()).width(Fill)].height(COVER_HEIGHT);

    container(card_row)
        .width(Fill)
        .clip(true)
        .style(containers::card)
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
