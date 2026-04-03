use iced::widget::{Text, text, text::IntoFragment};

use crate::app::core::fonts;

/// Minimum text size — nothing in the app should go below this.
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn detail<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(MIN_SIZE)
}

// Keep the old names as aliases during migration so we don't have to
// update every call site at once. These will be removed once all
// callers are migrated.
#[allow(dead_code)]
pub fn m<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    label(content)
}

#[allow(dead_code)]
pub fn xl<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    heading(content)
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
