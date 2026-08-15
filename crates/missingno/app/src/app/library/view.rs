use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{Column, column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self, load,
    settings::view as settings_view,
    ui::{
        buttons, containers, fonts,
        icons::{self, Icon},
        palette::MUTED,
        sizes::{border_l, l, m, s, xs},
        text as app_text,
    },
};

use crate::app::library;
use crate::app::system::Platform;
use crate::app::views::friendly_ago;

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

pub(crate) const COVER_HEIGHT: f32 = 160.0;
const COVER_WIDTH: f32 = 120.0;
const CARD_MIN_WIDTH: f32 = 340.0;

/// Whether the library body renders as a cover grid or a compact list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum LibraryLayout {
    #[default]
    Grid,
    List,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectGame(String),
    QuickPlay(String),
    HoverGame(String),
    UnhoverGame,
    DumpCartridge,
    SearchChanged(String),
    SortSelected(super::store::SortKey),
    SystemFilterSelected(super::store::SystemFilter),
    LayoutSelected(LibraryLayout),
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Library(message)
    }
}

use super::store::{GameStore, GameSummary, SortKey, SystemFilter};
use crate::cartridge_rw;

/// Everything the library page needs to render its toolbar and body.
pub(crate) struct LibraryView<'a> {
    pub store: &'a GameStore,
    pub hovered_sha1: Option<&'a str>,
    pub inserted_cartridge: Option<&'a cartridge_rw::CartridgeHeader>,
    pub dump_progress: Option<&'a cartridge_rw::DumpProgress>,
    pub homebrew_enabled: bool,
    pub sort: SortKey,
    pub layout: LibraryLayout,
    pub search: &'a str,
    pub system_filter: SystemFilter,
}

#[allow(private_interfaces)]
pub(crate) fn view(data: LibraryView<'_>) -> Element<'_, app::Message> {
    let LibraryView {
        store,
        hovered_sha1,
        inserted_cartridge,
        dump_progress,
        homebrew_enabled,
        sort,
        layout,
        search,
        system_filter,
    } = data;

    if store.is_empty() && inserted_cartridge.is_none() {
        return empty_view(homebrew_enabled);
    }

    // Resolve the inserted cartridge against the whole library by raw header
    // title — a search filter shouldn't turn a known game into an "unmatched"
    // cartridge card.
    let matched_game = inserted_cartridge.and_then(|cart| {
        store.all_summaries().into_iter().find(|g| {
            g.entry
                .header_title
                .as_ref()
                .is_some_and(|ht| ht == &cart.title)
        })
    });

    let games = store.summaries_sorted(sort, search, system_filter);
    let hovered_sha1 = hovered_sha1.map(|s| s.to_string());

    let body: Element<'_, app::Message> = if games.is_empty() && inserted_cartridge.is_none() {
        no_results_view(search)
    } else {
        match layout {
            LibraryLayout::Grid => grid_body(
                games,
                inserted_cartridge,
                dump_progress,
                matched_game,
                hovered_sha1,
            ),
            LibraryLayout::List => list_body(
                games,
                inserted_cartridge,
                dump_progress,
                matched_game,
                hovered_sha1,
            ),
        }
    };

    column![toolbar(sort, layout, search, system_filter), body]
        .spacing(m())
        .height(Fill)
        .into()
}

/// Search field, filters, sort picker, and grid/list toggle above the body.
fn toolbar<'a>(
    sort: SortKey,
    layout: LibraryLayout,
    search: &'a str,
    system_filter: SystemFilter,
) -> Element<'a, app::Message> {
    use iced::widget::{pick_list, stack, text_input};

    let search_field = text_input("Search library...", search)
        .id(crate::app::automation::ids::LIBRARY_SEARCH)
        .on_input(|value| Message::SearchChanged(value).into())
        .width(Fill);

    // A clear affordance appears once there's something to clear, overlaid at
    // the trailing edge of the input. The stack is always present so the
    // input keeps its widget state (focus) when the button appears.
    let clear: Element<'a, app::Message> = if search.is_empty() {
        iced::widget::Space::new().into()
    } else {
        buttons::subtle_raw(icons::m_muted(Icon::Close))
            .on_press(Message::SearchChanged(String::new()).into())
            .into()
    };
    let search_area: Element<'a, app::Message> = stack![
        search_field,
        container(clear)
            .width(Fill)
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Center)
    ]
    .into();

    let system_picker = crate::app::automation::tag(
        crate::app::automation::ids::LIBRARY_FILTER,
        pick_list(SystemFilter::all_options(), Some(system_filter), |filter| {
            Message::SystemFilterSelected(filter).into()
        }),
    );

    let sort_picker = crate::app::automation::tag(
        crate::app::automation::ids::LIBRARY_SORT,
        pick_list(SortKey::ALL, Some(sort), |key| {
            Message::SortSelected(key).into()
        }),
    );

    let layout_toggle = row![
        crate::app::automation::tag(
            crate::app::automation::ids::LIBRARY_VIEW_GRID,
            layout_button(
                Icon::Grid,
                layout == LibraryLayout::Grid,
                LibraryLayout::Grid
            ),
        ),
        crate::app::automation::tag(
            crate::app::automation::ids::LIBRARY_VIEW_LIST,
            layout_button(
                Icon::List,
                layout == LibraryLayout::List,
                LibraryLayout::List
            ),
        ),
    ]
    .spacing(xs());

    container(
        row![search_area, system_picker, sort_picker, layout_toggle]
            .spacing(m())
            .align_y(Center),
    )
    .padding([s(), l()])
    .into()
}

fn layout_button(
    icon: Icon,
    active: bool,
    target: LibraryLayout,
) -> Element<'static, app::Message> {
    let content = icons::m(icon);
    let button = if active {
        buttons::selected(content)
    } else {
        buttons::subtle(content)
    };
    button
        .on_press(Message::LayoutSelected(target).into())
        .into()
}

/// Cartridge-first cover grid, wrapped by column count to the viewport width.
fn grid_body<'a>(
    games: Vec<&'a GameSummary>,
    inserted_cartridge: Option<&'a cartridge_rw::CartridgeHeader>,
    dump_progress: Option<&'a cartridge_rw::DumpProgress>,
    matched_game: Option<&'a GameSummary>,
    hovered_sha1: Option<String>,
) -> Element<'a, app::Message> {
    let matched_sha1 = matched_game.map(|g| g.entry.sha1.clone());
    iced::widget::responsive(move |size| {
        let usable = size.width - l() * 2.0;
        let cols = (usable / (CARD_MIN_WIDTH + m())).max(1.0) as usize;

        let mut all_cards: Vec<Element<'_, app::Message>> = Vec::new();

        if let Some(cart) = inserted_cartridge {
            if let Some(game) = matched_game {
                all_cards.push(cartridge_game_card(game, cart));
            } else {
                all_cards.push(unmatched_cartridge_card(cart, dump_progress));
            }
        }

        for game in &games {
            // Skip the matched cartridge game — it's already first
            if matched_sha1.as_deref() == Some(game.entry.sha1.as_str()) {
                continue;
            }
            let hovered = hovered_sha1.as_deref() == Some(game.entry.sha1.as_str());
            all_cards.push(crate::app::automation::tag(
                &crate::app::automation::ids::game(&game.entry.sha1),
                game_card(game, hovered),
            ));
        }

        let mut content: Vec<Element<'_, app::Message>> = Vec::new();
        let mut cards_iter = all_cards.into_iter();
        loop {
            let mut row_cards: Vec<Element<'_, app::Message>> =
                (&mut cards_iter).take(cols).collect();
            if row_cards.is_empty() {
                break;
            }
            while row_cards.len() < cols {
                row_cards.push(iced::widget::Space::new().width(Fill).into());
            }
            content.push(row(row_cards).spacing(m()).into());
        }

        scrollable(
            container(
                Column::with_children(content)
                    .spacing(m())
                    .padding(body_padding()),
            )
            .center_x(Fill),
        )
        .height(Fill)
        .into()
    })
    .into()
}

/// Grid/list body padding — the toolbar already supplies the top gap.
fn body_padding() -> iced::Padding {
    iced::Padding {
        top: 0.0,
        right: l(),
        bottom: l(),
        left: l(),
    }
}

/// Compact one-line-per-game list — the scannable option for large libraries.
fn list_body<'a>(
    games: Vec<&'a GameSummary>,
    inserted_cartridge: Option<&'a cartridge_rw::CartridgeHeader>,
    dump_progress: Option<&'a cartridge_rw::DumpProgress>,
    matched_game: Option<&'a GameSummary>,
    hovered_sha1: Option<String>,
) -> Element<'a, app::Message> {
    let mut rows: Vec<Element<'_, app::Message>> = Vec::new();

    // An unmatched cartridge gets its own row on top; a matched one just
    // appears in the list below like any other game.
    if let (Some(cart), None) = (inserted_cartridge, matched_game) {
        rows.push(unmatched_cartridge_card(cart, dump_progress));
    }

    for game in &games {
        let hovered = hovered_sha1.as_deref() == Some(game.entry.sha1.as_str());
        rows.push(crate::app::automation::tag(
            &crate::app::automation::ids::game(&game.entry.sha1),
            list_row(game, hovered),
        ));
    }

    scrollable(
        container(
            Column::with_children(rows)
                .spacing(s())
                .padding(body_padding())
                .max_width(LIST_MAX_WIDTH),
        )
        .center_x(Fill),
    )
    .height(Fill)
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

    actions =
        actions.push(buttons::subtle("Open a ROM file...").on_press(load::Message::Pick.into()));

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

/// Shown when a search filter (or an as-yet-empty library) leaves no games.
fn no_results_view(search: &str) -> Element<'static, app::Message> {
    let message = if search.trim().is_empty() {
        "No games in your library yet.".to_string()
    } else {
        format!("No games match “{}”.", search.trim())
    };
    container(text(message).color(MUTED)).center(Fill).into()
}

const LIST_COVER_HEIGHT: f32 = 48.0;
const LIST_COVER_WIDTH: f32 = 36.0;
/// Rows stay readable rather than stretching edge-to-edge on wide windows.
const LIST_MAX_WIDTH: f32 = 900.0;

/// One compact library row: thumbnail, title, metadata, play stats.
fn list_row(game: &GameSummary, hovered: bool) -> Element<'_, app::Message> {
    let has_rom = !game.entry.rom_paths.is_empty();
    let sha1 = &game.entry.sha1;

    let subtitle_parts: Vec<String> = [
        game.entry.platform.map(|p| p.name().to_string()),
        game.entry.publisher.clone(),
        game.entry
            .year
            .as_ref()
            .map(|y| library::activity::release_year(y)),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut info = column![text(game.entry.display_title()).font(fonts::bold())].spacing(2);
    if !subtitle_parts.is_empty() {
        info = info.push(app_text::detail(subtitle_parts.join(" · ")).color(MUTED));
    }

    let stats: Element<'_, app::Message> = if let Some(last_ts) = game.last_played {
        let last = friendly_ago(last_ts);
        let play_time = library::activity::format_play_time(game.play_time_secs);
        column![
            app_text::detail(format!("Played {last}")).color(MUTED),
            app_text::detail(play_time).color(MUTED),
        ]
        .spacing(2)
        .align_x(iced::Alignment::End)
        .into()
    } else if game.save_count > 0 {
        let n = game.save_count;
        app_text::detail(format!("{n} save{}", if n == 1 { "" } else { "s" }))
            .color(MUTED)
            .into()
    } else {
        iced::widget::Space::new().into()
    };

    let mut card_row = row![
        list_cover(game),
        container(info.width(Fill)).width(Fill),
        stats,
    ]
    .spacing(m())
    .align_y(Center)
    .height(LIST_COVER_HEIGHT);

    if hovered && has_rom {
        card_row = card_row.push(
            buttons::subtle(icons::m(Icon::Play)).on_press(Message::QuickPlay(sha1.clone()).into()),
        );
    }

    let card = container(card_row)
        .width(Fill)
        .padding([xs(), s()])
        .clip(true)
        .style(containers::card);

    mouse_area(card)
        .on_press(Message::SelectGame(sha1.clone()).into())
        .on_enter(Message::HoverGame(sha1.clone()).into())
        .on_exit(Message::UnhoverGame.into())
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// Small thumbnail (or initial-letter placeholder) for a list row.
fn list_cover(game: &GameSummary) -> Element<'_, app::Message> {
    let radius = iced::border::Radius::from(4.0);
    if let Some(handle) = &game.thumbnail {
        image(handle.clone())
            .width(LIST_COVER_WIDTH)
            .height(LIST_COVER_HEIGHT)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(radius)
            .into()
    } else {
        cartridge_placeholder(
            &game.entry.display_title(),
            game.entry.platform,
            LIST_COVER_WIDTH,
            LIST_COVER_HEIGHT,
            radius,
        )
    }
}

fn game_card(game: &GameSummary, hovered: bool) -> Element<'_, app::Message> {
    use iced::widget::stack;

    let has_rom = !game.entry.rom_paths.is_empty();
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
        cartridge_placeholder(
            &game.entry.display_title(),
            game.entry.platform,
            COVER_WIDTH,
            COVER_HEIGHT,
            iced::border::Radius {
                top_left: 8.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 8.0,
            },
        )
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

    // Publisher · Date · Platform
    let subtitle_parts: Vec<String> = [
        game.entry.publisher.clone(),
        game.entry
            .year
            .as_ref()
            .map(|y| library::activity::release_year(y)),
        game.entry.platform.map(|p| p.name().to_string()),
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

/// A library game card styled as a cartridge tile.
fn cartridge_game_card<'a>(
    game: &'a GameSummary,
    cart: &'a cartridge_rw::CartridgeHeader,
) -> Element<'a, app::Message> {
    let mut parts: Vec<String> = Vec::new();

    // Publisher · Year (same as game_card)
    let meta: Vec<String> = [
        game.entry.publisher.clone(),
        game.entry
            .year
            .as_ref()
            .map(|y| library::activity::release_year(y)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !meta.is_empty() {
        parts.push(meta.join(" · "));
    }

    // Play time
    if let Some(last_ts) = game.last_played {
        let last = friendly_ago(last_ts);
        let play_time = library::activity::format_play_time(game.play_time_secs);
        parts.push(format!("Played {last} · {play_time}"));
    }

    // Hardware info — flash cart shows chip info, regular cart shows mapper/ROM
    if let Some(flash) = &cart.flash {
        let mut hw = format!("Flash {}", cartridge_rw::format_size(flash.size));
        if cart.ram_size > 0 {
            hw.push_str(&format!(
                " · RAM {}",
                cartridge_rw::format_size(cart.ram_size)
            ));
        }
        parts.push(hw);
    } else {
        parts.push(format!(
            "{} · {}",
            cart.mapper_name,
            cartridge_rw::format_size(cart.rom_size)
        ));
    }

    let subtitle = parts.join("\n");
    let cover = game.thumbnail.as_ref();

    let tile = cartridge_tile(&game.entry.display_title(), &subtitle, cover);

    mouse_area(tile)
        .on_press(Message::SelectGame(game.entry.sha1.clone()).into())
        .on_enter(Message::HoverGame(game.entry.sha1.clone()).into())
        .on_exit(Message::UnhoverGame.into())
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// A card for an inserted cartridge that doesn't match any library game.
fn unmatched_cartridge_card<'a>(
    cart: &'a cartridge_rw::CartridgeHeader,
    dump_progress: Option<&'a cartridge_rw::DumpProgress>,
) -> Element<'a, app::Message> {
    let display_title = if cart.title.is_empty() {
        if cart.flashable() {
            "Empty Flash Cart"
        } else {
            "Unknown Cartridge"
        }
    } else {
        &cart.title
    };
    let cover: Element<'_, app::Message> = cartridge_placeholder(
        display_title,
        None,
        COVER_WIDTH,
        COVER_HEIGHT,
        iced::border::Radius {
            top_left: 8.0,
            top_right: 0.0,
            bottom_right: 0.0,
            bottom_left: 8.0,
        },
    );

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
        info = info.push(iced::widget::progress_bar(0.0..=1.0, pct).girth(6));
    } else if cart.rom_size > 0 {
        info =
            info.push(buttons::primary("Add to Library").on_press(Message::DumpCartridge.into()));
    }

    let card_row =
        row![cover, container(info.width(Fill)).padding(m()).width(Fill)].height(COVER_HEIGHT);

    container(card_row)
        .width(Fill)
        .clip(true)
        .style(containers::card)
        .into()
}

/// Cartridge silhouette for a platform (generic when the system is unknown).
/// The SVGs carry their own white relief so they read as artwork over the
/// deterministic placeholder background.
fn cartridge_for(platform: Option<Platform>, size: f32) -> Element<'static, app::Message> {
    let bytes: &'static [u8] = match platform {
        Some(Platform::GameBoy | Platform::GameBoyColor) => {
            include_bytes!("../../app/ui/icons/cartridges/gb.svg")
        }
        Some(Platform::Nes) => include_bytes!("../../app/ui/icons/cartridges/nes.svg"),
        Some(Platform::MasterSystem) => include_bytes!("../../app/ui/icons/cartridges/sms.svg"),
        Some(Platform::Sg1000) => include_bytes!("../../app/ui/icons/cartridges/sg1000.svg"),
        Some(Platform::AtariVcs) => include_bytes!("../../app/ui/icons/cartridges/vcs.svg"),
        None => include_bytes!("../../app/ui/icons/cartridges/generic.svg"),
    };
    iced::widget::svg(iced::advanced::svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .into()
}

/// Placeholder used wherever a game has no cover: the per-system cartridge
/// silhouette over the title's deterministic accent background, with the title
/// beneath it as a title card on tiles tall enough to fit one.
pub(super) fn cartridge_placeholder(
    title: &str,
    platform: Option<Platform>,
    width: f32,
    height: f32,
    radius: iced::border::Radius,
) -> Element<'static, app::Message> {
    const LABEL: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.92);
    let bg = title_color(title);

    // Small list thumbnails stay art-only; taller tiles read as a title card.
    let show_title = height >= 100.0;
    let (content, padding): (Element<'static, app::Message>, iced::Padding) = if show_title {
        // Underscores aren't line-break opportunities like hyphens are; a
        // zero-width space after each lets snake_case titles wrap.
        let wrappable_title = title.replace('_', "_\u{200B}");
        let card = column![
            cartridge_for(platform, height * 0.4),
            text(wrappable_title)
                .size(13)
                .color(LABEL)
                .width(Fill)
                .align_x(Center),
        ]
        .spacing(s())
        .align_x(Center);
        (card.into(), [m(), s()].into())
    } else {
        (cartridge_for(platform, height * 0.5), xs().into())
    };

    container(content)
        .width(width)
        .height(height)
        .padding(padding)
        .clip(true)
        .align_x(Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(bg.into()),
            border: iced::Border {
                radius,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Cover art or cartridge placeholder for tile cards.
fn cover_element<'a>(title: &str, cover: Option<&'a image::Handle>) -> Element<'a, app::Message> {
    if let Some(handle) = cover {
        image(handle.clone())
            .width(COVER_WIDTH)
            .height(COVER_HEIGHT)
            .content_fit(iced::ContentFit::Cover)
            .into()
    } else {
        cartridge_placeholder(
            title,
            None,
            COVER_WIDTH,
            COVER_HEIGHT,
            iced::border::Radius::from(0.0),
        )
    }
}

/// Non-interactive game tile for display purposes (e.g. flash confirmation).
#[allow(private_interfaces)]
pub(crate) fn game_tile<'a>(
    title: &str,
    subtitle: &str,
    cover: Option<&'a image::Handle>,
) -> Element<'a, app::Message> {
    let info = column![
        text(title.to_string()).font(fonts::bold()),
        app_text::detail(subtitle.to_string()).color(MUTED),
    ]
    .spacing(4);

    let card_row = row![
        cover_element(title, cover),
        container(info.width(Fill)).padding(m()).width(Fill),
    ]
    .height(COVER_HEIGHT);

    container(card_row)
        .width(Fill)
        .clip(true)
        .style(containers::card)
        .into()
}

/// Reusable cartridge identification tile.
/// Shows a game card with a teal-accented border and circuit board icon.
/// Used in the library view and the cartridge actions screen.
#[allow(private_interfaces)]
pub(crate) fn cartridge_tile<'a>(
    title: &str,
    subtitle: &str,
    cover: Option<&'a image::Handle>,
) -> Element<'a, app::Message> {
    use crate::app::ui::palette::TEAL;

    let info = column![
        row![
            icons::m(Icon::CircuitBoard)
                .style(move |_, _| iced::widget::svg::Style { color: Some(TEAL) }),
            text(title.to_string()).font(fonts::bold()),
        ]
        .spacing(s())
        .align_y(Center),
        app_text::detail(subtitle.to_string()).color(MUTED),
    ]
    .spacing(4);

    let card_row = row![
        cover_element(title, cover),
        container(info.width(Fill)).padding(m()).width(Fill),
    ]
    .height(COVER_HEIGHT);

    container(card_row)
        .width(Fill)
        .clip(true)
        .style(containers::cartridge)
        .into()
}
