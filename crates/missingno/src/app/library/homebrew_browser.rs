use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{column, container, image, row, scrollable, text, text_input},
};

use crate::app::{
    self,
    ui::{
        buttons, fonts,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    library::catalogue::{Catalogue, CatalogueEntry},
};

// Catppuccin Mocha subtext0
use crate::app::library::activity;

const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

pub const PAGE_SIZE: usize = 20;

// ── State ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct BrowserState {
    pub search_text: String,
    /// How many entries to show (grows as user scrolls / clicks "Show more").
    pub visible_count: usize,
    /// Index of the selected entry for detail view.
    pub selected_slug: Option<String>,
    /// Cover images keyed by slug.
    pub covers: std::collections::HashMap<String, image::Handle>,
    /// Raw cover bytes keyed by slug (for saving to library on download).
    pub cover_bytes: std::collections::HashMap<String, Vec<u8>>,
    /// Error message to show (e.g. download failure).
    pub error: Option<String>,
}

impl BrowserState {
    pub fn new() -> Self {
        Self {
            search_text: String::new(),
            visible_count: PAGE_SIZE,
            selected_slug: None,
            covers: std::collections::HashMap::new(),
            cover_bytes: std::collections::HashMap::new(),
            error: None,
        }
    }
}

// ── Messages ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    SearchTextChanged(String),
    SelectEntry(String),          // slug
    CoverLoaded(String, Vec<u8>), // (slug, image bytes)
    Download(String),             // slug
    DownloadFailed(String),       // error message
    ShowMore,
    DismissError,
    Back,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::HomebrewBrowser(message)
    }
}

// ── View ──────────────────────────────────────────────────────────────

#[allow(private_interfaces)]
pub(crate) fn view<'a>(
    state: &'a BrowserState,
    catalogue: &'a Catalogue,
) -> Element<'a, app::Message> {
    // If an entry is selected, show the detail view
    if let Some(slug) = &state.selected_slug {
        if let Some(entry) = catalogue.lookup_slug(slug) {
            return entry_detail(entry, state.covers.get(slug), state.error.as_deref());
        }
    }

    let search_bar = text_input("Search homebrew games...", &state.search_text)
        .on_input(|s| Message::SearchTextChanged(s).into())
        .size(18.0);

    let results = if state.search_text.is_empty() {
        catalogue.homebrew()
    } else {
        catalogue.search_homebrew(&state.search_text)
    };

    let content = if results.is_empty() {
        container(app_text::detail("No games found").color(MUTED))
            .center(Fill)
            .into()
    } else {
        results_view(&results, &state.covers, state.visible_count)
    };

    let mut page = column![container(search_bar).padding(l()), content,].height(Fill);

    if let Some(bar) = error_bar(state.error.as_deref()) {
        page = page.push(bar);
    }

    page.into()
}

fn results_view<'a>(
    results: &[&'a CatalogueEntry],
    covers: &'a std::collections::HashMap<String, image::Handle>,
    visible_count: usize,
) -> Element<'a, app::Message> {
    let mut entries_col = column![].spacing(m());

    let total = results.len();
    let showing = visible_count.min(total);

    entries_col = entries_col.push(app_text::detail(format!("{total} games")).color(MUTED));

    for entry in results.iter().take(showing) {
        entries_col = entries_col.push(entry_card(entry, covers.get(&entry.slug)));
    }

    // "Show more" button if there are more results
    if showing < total {
        entries_col = entries_col.push(
            container(
                buttons::standard(text(format!("Show more ({} remaining)", total - showing)))
                    .on_press(Message::ShowMore.into()),
            )
            .center_x(Fill),
        );
    }

    // Bottom padding
    entries_col = entries_col.push(iced::widget::Space::new().height(l()));

    scrollable(
        container(entries_col.max_width(900))
            .padding([0.0, l()])
            .center_x(Fill),
    )
    .height(Fill)
    .into()
}

const CARD_COVER_WIDTH: f32 = 160.0;
const CARD_HEIGHT: f32 = 160.0;

fn entry_card<'a>(
    entry: &'a CatalogueEntry,
    cover: Option<&'a image::Handle>,
) -> Element<'a, app::Message> {
    // Cover image or placeholder — flush left, full card height, GB aspect ratio
    let cover_el: Element<'_, app::Message> = if let Some(handle) = cover {
        image(handle.clone())
            .width(CARD_COVER_WIDTH)
            .height(CARD_HEIGHT)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(iced::border::Radius {
                top_left: 0.0,
                top_right: 6.0,
                bottom_right: 6.0,
                bottom_left: 0.0,
            })
            .into()
    } else {
        container(
            text(
                entry
                    .manifest
                    .title
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .next()
                    .unwrap_or('?'),
            )
            .size(24.0)
            .font(fonts::heading())
            .color(Color::WHITE),
        )
        .width(CARD_COVER_WIDTH)
        .height(CARD_HEIGHT)
        .align_x(Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(|_: &iced::Theme| container::Style {
            background: Some(Color::from_rgb(0.3, 0.2, 0.4).into()),
            border: iced::Border {
                radius: iced::border::Radius {
                    top_left: 0.0,
                    top_right: 6.0,
                    bottom_right: 6.0,
                    bottom_left: 0.0,
                },
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };

    // Info — padded separately
    let mut info = column![text(&entry.manifest.title).font(fonts::bold())].spacing(2);

    let mut subtitle_parts = Vec::new();
    if let Some(dev) = &entry.manifest.developer {
        subtitle_parts.push(dev.clone());
    }
    if let Some(date) = &entry.manifest.date {
        subtitle_parts.push(activity::format_date_string(date));
    }
    if !subtitle_parts.is_empty() {
        info = info.push(app_text::detail(subtitle_parts.join(" · ")).color(MUTED));
    }

    if let Some(desc) = &entry.manifest.description {
        let short = if desc.len() > 120 {
            format!("{}…", &desc[..120])
        } else {
            desc.clone()
        };
        info = info.push(app_text::detail(short).color(MUTED));
    }

    if !entry.manifest.tags.is_empty() {
        info = info.push(app_text::detail(entry.manifest.tags.join(", ")).color(MUTED));
    }

    let slug = entry.slug.clone();
    let card = row![cover_el, container(info.width(Fill)).padding(m()),].height(CARD_HEIGHT);

    iced::widget::mouse_area(container(card).width(Fill).style(|theme: &iced::Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(palette.background.weak.color.into()),
            border: iced::Border::default().rounded(6),
            ..Default::default()
        }
    }))
    .on_press(Message::SelectEntry(slug).into())
    .interaction(iced::mouse::Interaction::Pointer)
    .into()
}

fn error_bar(error: Option<&str>) -> Option<Element<'static, app::Message>> {
    let error = error?;
    Some(
        iced::widget::mouse_area(
            container(
                row![
                    text(error.to_string()).color(Color::WHITE),
                    iced::widget::Space::new().width(Fill),
                    text("Dismiss").color(MUTED),
                ]
                .spacing(m())
                .align_y(Center),
            )
            .padding(m())
            .width(Fill)
            .style(|_: &iced::Theme| container::Style {
                background: Some(Color::from_rgb(0.5, 0.15, 0.15).into()),
                border: iced::Border::default().rounded(6),
                ..Default::default()
            }),
        )
        .on_press(Message::DismissError.into())
        .into(),
    )
}

fn entry_detail<'a>(
    entry: &'a CatalogueEntry,
    cover: Option<&'a image::Handle>,
    error: Option<&str>,
) -> Element<'a, app::Message> {
    let mut content = column![].spacing(m());

    // Header: cover + title + metadata
    let cover_el: Element<'_, app::Message> = if let Some(handle) = cover {
        image(handle.clone())
            .width(160)
            .height(160)
            .content_fit(iced::ContentFit::ScaleDown)
            .border_radius(6)
            .into()
    } else {
        iced::widget::Space::new().width(160).height(160).into()
    };

    let mut info =
        column![text(&entry.manifest.title).size(24.0).font(fonts::bold()),].spacing(s());

    let mut subtitle_parts = Vec::new();
    if let Some(dev) = &entry.manifest.developer {
        subtitle_parts.push(format!("by {dev}"));
    }
    if let Some(date) = &entry.manifest.date {
        subtitle_parts.push(activity::format_date_string(date));
    }
    if !subtitle_parts.is_empty() {
        info = info.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    if !entry.manifest.tags.is_empty() {
        info = info.push(app_text::detail(entry.manifest.tags.join(", ")).color(MUTED));
    }

    if let Some(license) = &entry.manifest.license {
        info = info.push(app_text::detail(format!("License: {license}")).color(MUTED));
    }

    // Links
    let mut links = row![].spacing(m());
    for link in &entry.manifest.links {
        links = links.push(
            iced::widget::mouse_area(
                row![icons::m(Icon::Globe), text(&link.name).color(MUTED)]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(app::Message::OpenUrl(leak_str(&link.url)))
            .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    if !entry.manifest.links.is_empty() {
        info = info.push(links);
    }

    let header = row![cover_el, info.width(Fill)].spacing(m());
    content = content.push(header);

    // Description
    if let Some(desc) = &entry.manifest.description {
        content = content.push(text(desc.clone()));
    }

    // Actions
    let slug = entry.slug.clone();
    let mut actions = row![iced::widget::Space::new().width(Fill),]
        .spacing(s())
        .align_y(Center);

    if entry.download_url().is_some() {
        actions = actions.push(
            buttons::primary(
                row![icons::m(Icon::Download), "Add to Library"]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(Message::Download(slug).into()),
        );
    }

    content = content.push(actions);

    let mut page = column![
        scrollable(
            container(content.max_width(900))
                .padding(l())
                .center_x(Fill)
        )
        .height(Fill),
    ]
    .height(Fill);

    if let Some(bar) = error_bar(error) {
        page = page.push(bar);
    }

    page.into()
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
