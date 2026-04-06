use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{column, container, image, row, scrollable, text, text_input},
};

use crate::app::{
    self,
    core::{
        buttons, fonts,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    library::catalogue::{Catalogue, CatalogueEntry},
};

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

pub const MAX_RESULTS: usize = 50;

// ── State ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct BrowserState {
    pub search_text: String,
    /// Index of the selected entry for detail view.
    pub selected_slug: Option<String>,
    /// Cover images keyed by slug.
    pub covers: std::collections::HashMap<String, image::Handle>,
}

impl BrowserState {
    pub fn new() -> Self {
        Self {
            search_text: String::new(),
            selected_slug: None,
            covers: std::collections::HashMap::new(),
        }
    }
}

// ── Messages ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    SearchTextChanged(String),
    SelectEntry(String), // slug
    CoverLoaded(String, Vec<u8>), // (slug, image bytes)
    Download(String), // slug
    BackToResults,
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
            return entry_detail(entry, state.covers.get(slug));
        }
    }

    let search_bar = text_input("Search homebrew games...", &state.search_text)
        .on_input(|s| Message::SearchTextChanged(s).into())
        .width(Fill);

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
        results_view(&results, &state.covers)
    };

    column![
        container(search_bar).padding([0.0, l()]),
        content,
    ]
    .spacing(m())
    .height(Fill)
    .into()
}

fn results_view<'a>(
    results: &[&'a CatalogueEntry],
    covers: &'a std::collections::HashMap<String, image::Handle>,
) -> Element<'a, app::Message> {
    let mut entries_col = column![].spacing(m());

    let showing = results.len().min(MAX_RESULTS);
    let total = results.len();

    entries_col = entries_col.push(
        app_text::detail(if showing < total {
            format!("Showing {showing} of {total} games")
        } else {
            format!("{total} games")
        })
        .color(MUTED),
    );

    for entry in results.iter().take(MAX_RESULTS) {
        entries_col = entries_col.push(entry_card(entry, covers.get(&entry.slug)));
    }

    scrollable(container(entries_col.max_width(900)).padding(l()).center_x(Fill))
        .height(Fill)
        .into()
}

fn entry_card<'a>(
    entry: &'a CatalogueEntry,
    cover: Option<&'a image::Handle>,
) -> Element<'a, app::Message> {
    // Cover image or placeholder
    let cover_el: Element<'_, app::Message> = if let Some(handle) = cover {
        image(handle.clone())
            .width(80)
            .height(80)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(4)
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
        .width(80)
        .height(80)
        .align_x(Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(|_: &iced::Theme| container::Style {
            background: Some(Color::from_rgb(0.3, 0.2, 0.4).into()),
            border: iced::Border::default().rounded(4),
            ..Default::default()
        })
        .into()
    };

    // Info
    let mut info = column![text(&entry.manifest.title).font(fonts::bold())].spacing(2);

    if let Some(dev) = &entry.manifest.developer {
        info = info.push(app_text::detail(dev.clone()).color(MUTED));
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
    let card = row![cover_el, info.width(Fill)]
        .spacing(m())
        .align_y(Center);

    iced::widget::mouse_area(
        container(card)
            .width(Fill)
            .style(|theme: &iced::Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.background.weak.color.into()),
                    border: iced::Border::default().rounded(6),
                    ..Default::default()
                }
            })
            .padding(m()),
    )
    .on_press(Message::SelectEntry(slug).into())
    .interaction(iced::mouse::Interaction::Pointer)
    .into()
}

fn entry_detail<'a>(
    entry: &'a CatalogueEntry,
    cover: Option<&'a image::Handle>,
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

    let mut info = column![
        text(&entry.manifest.title).size(24.0).font(fonts::bold()),
    ]
    .spacing(s());

    if let Some(dev) = &entry.manifest.developer {
        info = info.push(text(format!("by {dev}")).color(MUTED));
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
    let mut actions = row![
        buttons::subtle("← Back to results").on_press(Message::BackToResults.into()),
        iced::widget::Space::new().width(Fill),
    ]
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

    scrollable(container(content.max_width(900)).padding(l()).center_x(Fill))
        .height(Fill)
        .into()
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
