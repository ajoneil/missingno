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
    library::homebrew_hub,
};

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

// ── State ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct BrowserState {
    /// Current search text.
    pub search_text: String,
    /// The query that produced the current results.
    pub active_query: Option<homebrew_hub::SearchQuery>,
    /// Current search results (None = not yet loaded).
    pub results: Option<homebrew_hub::SearchResults>,
    /// Cover images keyed by slug.
    pub covers: std::collections::HashMap<String, image::Handle>,
    /// Whether a search is in progress.
    pub loading: bool,
    /// Error message from the last failed request.
    pub error: Option<String>,
}

impl BrowserState {
    pub fn new() -> Self {
        Self {
            search_text: String::new(),
            active_query: None,
            results: None,
            covers: std::collections::HashMap::new(),
            loading: true,
            error: None,
        }
    }

    /// Build a search query from the current state. Always filters to GB platform.
    pub fn query(&self) -> homebrew_hub::SearchQuery {
        homebrew_hub::SearchQuery {
            platform: Some("GB".to_string()),
            title: if self.search_text.is_empty() {
                None
            } else {
                Some(self.search_text.clone())
            },
            ..Default::default()
        }
    }
}

// ── Messages ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    SearchTextChanged(String),
    SubmitSearch,
    SearchCompleted(Result<homebrew_hub::SearchResults, String>),
    CoverLoaded(String, Vec<u8>), // (slug, image bytes)
    NextPage,
    PrevPage,
    PageLoaded(Result<homebrew_hub::SearchResults, String>),
    Download(homebrew_hub::Entry),
    Back,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::HomebrewBrowser(message)
    }
}

// ── View ──────────────────────────────────────────────────────────────

#[allow(private_interfaces)]
pub(crate) fn view(state: &BrowserState) -> Element<'_, app::Message> {
    let search_bar = text_input("Search homebrew games...", &state.search_text)
        .on_input(|s| Message::SearchTextChanged(s).into())
        .on_submit(Message::SubmitSearch.into())
        .width(Fill);

    let content: Element<'_, app::Message> = if state.loading {
        container(app_text::detail("Loading…").color(MUTED))
            .center(Fill)
            .into()
    } else if let Some(error) = &state.error {
        container(app_text::detail(format!("Error: {error}")).color(MUTED))
            .center(Fill)
            .into()
    } else if let Some(results) = &state.results {
        if results.entries.is_empty() {
            container(app_text::detail("No games found").color(MUTED))
                .center(Fill)
                .into()
        } else {
            results_view(results, &state.covers)
        }
    } else {
        container(app_text::detail("Search for homebrew games above").color(MUTED))
            .center(Fill)
            .into()
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
    results: &'a homebrew_hub::SearchResults,
    covers: &'a std::collections::HashMap<String, image::Handle>,
) -> Element<'a, app::Message> {
    let mut entries_col = column![].spacing(m());

    for entry in &results.entries {
        entries_col = entries_col.push(entry_card(entry, covers.get(&entry.slug)));
    }

    // Pagination
    let mut pagination = row![].spacing(m()).align_y(Center);

    if results.page_current > 1 {
        pagination = pagination
            .push(buttons::standard("← Previous").on_press(Message::PrevPage.into()));
    }

    pagination = pagination.push(
        app_text::detail(format!(
            "Page {} of {} ({} games)",
            results.page_current, results.page_total, results.results
        ))
        .color(MUTED),
    );

    if results.page_current < results.page_total {
        pagination =
            pagination.push(buttons::standard("Next →").on_press(Message::NextPage.into()));
    }

    entries_col = entries_col.push(container(pagination).center_x(Fill));

    scrollable(container(entries_col.max_width(900)).padding(l()).center_x(Fill))
        .height(Fill)
        .into()
}

fn entry_card<'a>(
    entry: &'a homebrew_hub::Entry,
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
    let mut info = column![text(&entry.title).font(fonts::bold())].spacing(2);

    if let Some(dev) = &entry.developer {
        info = info.push(app_text::detail(dev.clone()).color(MUTED));
    }

    if let Some(desc) = &entry.description {
        let short = if desc.len() > 120 {
            format!("{}…", &desc[..120])
        } else {
            desc.clone()
        };
        info = info.push(app_text::detail(short).color(MUTED));
    }

    let mut meta_parts = Vec::new();
    if let Some(platform) = &entry.platform {
        meta_parts.push(platform.clone());
    }
    if !entry.tags.is_empty() {
        meta_parts.push(entry.tags.join(", "));
    }
    if !meta_parts.is_empty() {
        info = info.push(app_text::detail(meta_parts.join(" · ")).color(MUTED));
    }

    // Download button
    let download = if entry.playable_file().is_some() {
        buttons::primary(
            row![icons::m(Icon::Download), "Add to Library"]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(Message::Download(entry.clone()).into())
    } else {
        buttons::standard("No ROM available")
    };

    let card = row![cover_el, info.width(Fill), download]
        .spacing(m())
        .align_y(Center);

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
        .padding(m())
        .into()
}
