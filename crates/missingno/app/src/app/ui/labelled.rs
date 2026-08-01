use iced::{
    Border, Element, Theme,
    widget::{container, text, tooltip},
};

use super::fonts;
use super::sizes::s;
use crate::app::Message;

/// An icon-only control paired with a hover tooltip naming what it does, so the
/// button reads to a screen reader and to a person who does not recognise the
/// glyph. The label is verb-first ("Open menu", "Back to library").
pub fn labelled<'a>(
    control: impl Into<Element<'a, Message>>,
    label: &'a str,
) -> Element<'a, Message> {
    tooltip(
        control,
        container(text(label).font(fonts::monospace()).size(12.0)).padding([2.0, s()]),
        tooltip::Position::Bottom,
    )
    .style(tooltip_style)
    .into()
}

fn tooltip_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(palette.background.strong.color),
        ..Default::default()
    }
}
