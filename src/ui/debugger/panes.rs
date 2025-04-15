use super::breakpoints;
use crate::ui::{
    Message,
    styles::{fonts, spacing},
};
use iced::{
    Border, Element, Theme,
    widget::{checkbox, container, pane_grid, text},
};

pub enum PaneState {
    Instructions,
    Breakpoints(breakpoints::State),
    Cpu,
    Video,
    Audio,
}

pub fn pane<'a>(
    title: pane_grid::TitleBar<'a, Message>,
    content: Element<'a, Message>,
) -> pane_grid::Content<'a, Message> {
    pane_grid::Content::new(container(content).padding(spacing::m()))
        .title_bar(title)
        .style(pane_style)
}

pub fn pane_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        border: Border {
            width: 2.0,
            color: palette.primary.strong.color,
            ..Border::default()
        },
        background: Some(palette.background.base.color.into()),
        ..Default::default()
    }
}

pub fn title_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        text_color: Some(palette.primary.strong.text),
        background: Some(palette.primary.strong.color.into()),
        ..Default::default()
    }
}

pub fn title_bar(label: &str) -> pane_grid::TitleBar<'_, Message> {
    pane_grid::TitleBar::new(text(label).font(fonts::TITLE))
        .style(title_style)
        .padding(spacing::s())
}

pub fn checkbox_title_bar(label: &str, checked: bool) -> pane_grid::TitleBar<'_, Message> {
    pane_grid::TitleBar::new(checkbox(label, checked).font(fonts::TITLE))
        .style(title_style)
        .padding(spacing::s())
}
