use iced::{
    Color, Element,
    widget::{Text, rich_text, span, text, text::IntoFragment},
};

use crate::app;
use crate::app::core::fonts;

/// Minimum text size — nothing in the app should go below this.
const MIN_SIZE: f32 = 14.0;

/// Large section heading. Chakra Petch, 32px.
pub fn heading<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(32.0).font(fonts::heading())
}

/// Bold label for game titles, section names, etc. 16px bold.
pub fn label<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).font(fonts::bold())
}

/// Secondary/muted detail text. 14px (the minimum size).
pub fn detail<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(MIN_SIZE)
}

/// Inline text with a clickable link that opens a URL.
///
/// Renders as underlined text that shows a pointer cursor on hover.
/// Flows inline with surrounding `rich_text` spans.
pub fn web_link<'a>(label: &'a str, url: &'static str) -> Element<'a, app::Message> {
    web_link_colored(label, url, None)
}

/// Inline text link with a specific color.
pub fn web_link_colored<'a>(
    label: &'a str,
    url: &'static str,
    color: Option<Color>,
) -> Element<'a, app::Message> {
    let mut s = span(label).underline(true).link(url);
    if let Some(c) = color {
        s = s.color(c);
    }
    rich_text![s]
        .on_link_click(|url| app::Message::OpenUrl(url))
        .into()
}

/// Size constants for layout spacing (not text).
pub mod sizes {
    use crate::app::core::sizes;

    pub fn m() -> f32 {
        sizes::m()
    }

    pub fn xl() -> f32 {
        m() * 2.0
    }
}
