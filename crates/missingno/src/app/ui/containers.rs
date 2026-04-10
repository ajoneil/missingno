use iced::{Border, Theme, widget::container};

use super::sizes::border_m;

/// Card surface — weak background with rounded corners.
/// Use for game cards, activity entries, homebrew entries.
pub fn card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default().rounded(border_m()),
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
