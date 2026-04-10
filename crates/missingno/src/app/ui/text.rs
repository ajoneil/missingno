use std::borrow::Cow;

use iced::{
    Color, Element,
    widget::{Text, rich_text, text, text::IntoFragment, text::Span},
};

use crate::app;
use crate::app::ui::fonts;

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

/// A fragment of inline text, either plain or a clickable link.
pub enum TextPart<'a> {
    Plain(Cow<'a, str>),
    Link(Cow<'a, str>, &'static str),
}

impl<'a> TextPart<'a> {
    pub fn plain(text: impl Into<Cow<'a, str>>) -> Self {
        TextPart::Plain(text.into())
    }

    pub fn link(label: impl Into<Cow<'a, str>>, url: &'static str) -> Self {
        TextPart::Link(label.into(), url)
    }
}

/// Inline text with embedded clickable links that flow as a single paragraph.
///
/// Link spans show a pointer cursor and underline on hover (no permanent underline).
/// All text is rendered in the given color.
///
/// ```ignore
/// use app_text::TextPart;
/// app_text::link_text([
///     TextPart::plain("Read ROMs using a "),
///     TextPart::link("GBxCart RW", "https://www.gbxcart.com/"),
///     TextPart::plain(" device."),
/// ], MUTED)
/// ```
pub fn link_text<'a>(
    parts: impl IntoIterator<Item = TextPart<'a>>,
    color: Color,
) -> Element<'a, app::Message> {
    let spans: Vec<Span<'a, &'static str>> = parts
        .into_iter()
        .map(|part| match part {
            TextPart::Plain(t) => Span {
                text: t,
                color: Some(color),
                ..Default::default()
            },
            TextPart::Link(label, url) => Span {
                text: label,
                color: Some(color),
                link: Some(url),
                ..Default::default()
            },
        })
        .collect();

    rich_text(spans)
        .on_link_click(|url| app::Message::OpenUrl(url))
        .into()
}

/// Progress text with monospace numbers: "Label… 128 KB / 512 KB (25%)"
///
/// The label uses the default font, the numbers use monospace to prevent
/// text shifting as values change.
pub fn progress_text(
    label: &str,
    bytes_done: u32,
    bytes_total: u32,
    color: Color,
) -> Element<'_, app::Message> {
    use crate::cartridge_rw;

    let pct = if bytes_total > 0 {
        (bytes_done as f32 / bytes_total as f32) * 100.0
    } else {
        0.0
    };

    let mono = fonts::monospace();
    let spans: Vec<Span<'_, ()>> = vec![
        Span {
            text: format!("{label} ").into(),
            color: Some(color),
            ..Default::default()
        },
        Span {
            text: cartridge_rw::format_size(bytes_done).into(),
            color: Some(color),
            font: Some(mono),
            ..Default::default()
        },
        Span {
            text: " / ".into(),
            color: Some(color),
            ..Default::default()
        },
        Span {
            text: cartridge_rw::format_size(bytes_total).into(),
            color: Some(color),
            font: Some(mono),
            ..Default::default()
        },
        Span {
            text: format!(" ({pct:.0}%)").into(),
            color: Some(color),
            font: Some(mono),
            ..Default::default()
        },
    ];

    rich_text(spans).into()
}

/// Size constants for layout spacing (not text).
pub mod sizes {
    use crate::app::ui::sizes;

    pub fn m() -> f32 {
        sizes::m()
    }

    pub fn xl() -> f32 {
        m() * 2.0
    }
}
