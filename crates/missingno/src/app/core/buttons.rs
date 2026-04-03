use iced::{
    Border, Color, Element, Theme,
    widget::{
        Button, button,
        button::{Status, Style},
    },
};

use crate::app::Message;

const PURPLE: Color = Color::from_rgb(
    0xcb as f32 / 255.0,
    0xa6 as f32 / 255.0,
    0xf7 as f32 / 255.0,
);
const PURPLE_DIM: Color = Color::from_rgba(
    0xcb as f32 / 255.0,
    0xa6 as f32 / 255.0,
    0xf7 as f32 / 255.0,
    0.2,
);
const PURPLE_HOVER: Color = Color::from_rgba(
    0xcb as f32 / 255.0,
    0xa6 as f32 / 255.0,
    0xf7 as f32 / 255.0,
    0.3,
);
const TEXT: Color = Color::from_rgb(
    0xcd as f32 / 255.0,
    0xd6 as f32 / 255.0,
    0xf4 as f32 / 255.0,
);
const RED: Color = Color::from_rgb(
    0xf3 as f32 / 255.0,
    0x8b as f32 / 255.0,
    0xa8 as f32 / 255.0,
);
const RED_DIM: Color = Color::from_rgba(
    0xf3 as f32 / 255.0,
    0x8b as f32 / 255.0,
    0xa8 as f32 / 255.0,
    0.15,
);
const RED_HOVER: Color = Color::from_rgba(
    0xf3 as f32 / 255.0,
    0x8b as f32 / 255.0,
    0xa8 as f32 / 255.0,
    0.25,
);

const BORDER_RADIUS: f32 = 4.0;

/// Main action. Solid purple background, dark text.
/// One per context at most.
pub fn primary<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(primary_style)
}

fn primary_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(PURPLE.scale_alpha(0.6).into()),
        text_color: Color::WHITE,
        border: Border::default().rounded(BORDER_RADIUS),
        ..Style::default()
    };

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered => Style {
            background: Some(PURPLE.scale_alpha(0.75).into()),
            ..base
        },
        Status::Disabled => Style {
            background: base.background.map(|bg| bg.scale_alpha(0.5)),
            text_color: base.text_color.scale_alpha(0.5),
            ..base
        },
    }
}

/// Normal button. Tinted purple background.
/// The default for most buttons.
pub fn standard<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(standard_style)
}

fn standard_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(PURPLE_DIM.into()),
        text_color: TEXT,
        border: Border::default().rounded(BORDER_RADIUS),
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

/// Text-only button. No background until hovered.
/// For links, toolbar items, inline actions.
pub fn subtle<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(subtle_style)
}

fn subtle_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: None,
        text_color: TEXT,
        border: Border::default().rounded(BORDER_RADIUS),
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
    button(content).style(invisible_style)
}

fn invisible_style(_theme: &Theme, _status: Status) -> Style {
    Style {
        background: None,
        text_color: Color::TRANSPARENT,
        border: Border::default().rounded(BORDER_RADIUS),
        ..Style::default()
    }
}

/// Destructive action. Red-tinted background, same weight as standard.
pub fn danger<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(danger_style)
}

fn danger_style(_theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(RED_DIM.into()),
        text_color: RED,
        border: Border::default().rounded(BORDER_RADIUS),
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
