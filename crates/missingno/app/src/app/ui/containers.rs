use iced::{Border, Color, Theme, widget::container};

use super::sizes::border_m;

/// Subtle border color for containers on dark backgrounds.
const BORDER_COLOR: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.08);

/// Card surface — weak background with rounded corners.
/// Use for game cards, activity entries, homebrew entries.
pub fn card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(border_m())
            .width(1.0)
            .color(BORDER_COLOR),
        ..Default::default()
    }
}

/// Floating menu surface — elevated with stronger border.
/// Use for popover menus and dropdowns.
pub fn menu(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(border_m())
            .width(1.0)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.12)),
        ..Default::default()
    }
}

/// Cartridge card — weak background with teal accent border.
/// Use for physical cartridge identification tiles.
pub fn cartridge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(border_m())
            .width(1.0)
            .color(Color::from_rgba(
                super::palette::TEAL.r,
                super::palette::TEAL.g,
                super::palette::TEAL.b,
                0.3,
            )),
        ..Default::default()
    }
}

/// Sidebar surface — weak background, no border radius.
/// Use for settings sidebar, gallery sidebar.
pub fn sidebar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        ..Default::default()
    }
}
