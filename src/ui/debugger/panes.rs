use crate::ui::Message;
use iced::{
    Border, Element, Font, Theme,
    widget::{checkbox, container, pane_grid, text},
};
pub enum Pane {
    Instructions,
    Cpu,
    Video,
    Audio,
}

pub fn pane<'a>(
    title: pane_grid::TitleBar<'a, Message>,
    content: Element<'a, Message>,
) -> pane_grid::Content<'a, Message> {
    pane_grid::Content::new(container(content).padding(10))
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
    pane_grid::TitleBar::new(text(label).font(Font {
        weight: iced::font::Weight::Bold,
        ..Default::default()
    }))
    .style(title_style)
    .padding(10)
}

pub fn checkbox_title_bar(label: &str, checked: bool) -> pane_grid::TitleBar<'_, Message> {
    pane_grid::TitleBar::new(checkbox(label, checked).font(Font {
        weight: iced::font::Weight::Bold,
        ..Default::default()
    }))
    .style(title_style)
    .padding(10)
}
