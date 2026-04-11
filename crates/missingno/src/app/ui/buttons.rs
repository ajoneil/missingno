use iced::{
    Alignment::Center,
    Border, Color, Element, Theme,
    widget::{
        Button, button,
        button::{Status, Style},
        container,
    },
};

use super::icons;
use super::palette::{PURPLE, RED, TEXT};
use super::sizes::border_s;
use crate::app::Message;

const PURPLE_DIM: Color = Color::from_rgba(PURPLE.r, PURPLE.g, PURPLE.b, 0.2);
const PURPLE_HOVER: Color = Color::from_rgba(PURPLE.r, PURPLE.g, PURPLE.b, 0.3);
const RED_DIM: Color = Color::from_rgba(RED.r, RED.g, RED.b, 0.15);
const RED_HOVER: Color = Color::from_rgba(RED.r, RED.g, RED.b, 0.25);

/// Main action. Solid purple background, dark text.
/// One per context at most.
pub fn primary<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(min_height_content(content)).style(primary_style)
}

fn primary_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(PURPLE.scale_alpha(0.7).into()),
        text_color: Color::WHITE,
        border: Border::default().rounded(border_s()),
        ..Style::default()
    };

    match status {
        Status::Active | Status::Pressed | Status::Disabled => base,
        Status::Hovered => Style {
            background: Some(PURPLE.scale_alpha(0.85).into()),
            ..base
        },
    }
}

/// Normal button. Tinted purple background.
/// The default for most buttons.
pub fn standard<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(min_height_content(content)).style(standard_style)
}

fn standard_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(PURPLE_DIM.into()),
        text_color: TEXT,
        border: Border::default().rounded(border_s()),
        ..Style::default()
    };

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered => Style {
            background: Some(PURPLE_HOVER.into()),
            ..base
        },
        Status::Disabled => Style {
            background: base.background.map(|bg| bg.scale_alpha(0.5)),
            text_color: base.text_color.scale_alpha(0.5),
            ..base
        },
    }
}

/// Selected/active state. Same visual weight as standard but no dimming
/// when disabled. Use for active sidebar items, selected palette tiles, etc.
pub fn selected<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(min_height_content(content)).style(selected_style)
}

/// Selected style without min-height enforcement.
pub fn selected_raw<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(selected_style)
}

fn selected_style(_theme: &Theme, _status: Status) -> Style {
    Style {
        background: Some(PURPLE_DIM.into()),
        text_color: TEXT,
        border: Border::default().rounded(border_s()),
        ..Style::default()
    }
}

/// Text-only button. No background until hovered.
/// For links, toolbar items, inline actions.
pub fn subtle<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(min_height_content(content)).style(subtle_style)
}

fn subtle_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: None,
        text_color: TEXT,
        border: Border::default().rounded(border_s()),
        ..Style::default()
    };

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered => Style {
            background: Some(PURPLE_DIM.into()),
            ..base
        },
        Status::Disabled => Style {
            text_color: base.text_color.scale_alpha(0.5),
            ..base
        },
    }
}

/// Same size as subtle but invisible. Use for layout reservation.
pub fn invisible<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(min_height_content(content)).style(invisible_style)
}

fn invisible_style(_theme: &Theme, _status: Status) -> Style {
    Style {
        background: None,
        text_color: Color::TRANSPARENT,
        border: Border::default().rounded(border_s()),
        ..Style::default()
    }
}

/// Destructive action. Red-tinted background, same weight as standard.
pub fn danger<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(min_height_content(content)).style(danger_style)
}

/// Subtle button without min-height enforcement.
pub fn subtle_raw<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(subtle_style)
}

fn min_height_content<'a>(
    content: impl Into<Element<'a, Message>>,
) -> container::Container<'a, Message> {
    container(content).align_y(Center).height(icons::ICON_SIZE)
}

const DANGER_BORDER: Color = Color::from_rgba(RED.r, RED.g, RED.b, 0.2);

fn danger_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(RED_DIM.into()),
        text_color: RED,
        border: Border::default()
            .rounded(border_s())
            .width(1.0)
            .color(DANGER_BORDER),
        ..Style::default()
    };

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered => Style {
            background: Some(RED_HOVER.into()),
            ..base
        },
        Status::Disabled => Style {
            background: base.background.map(|bg| bg.scale_alpha(0.5)),
            text_color: base.text_color.scale_alpha(0.5),
            ..base
        },
    }
}
