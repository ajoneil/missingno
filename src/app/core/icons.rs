use iced::Theme;
use iced_font_awesome::{FaIcon, fa_icon_solid};

pub fn close<'a>() -> FaIcon<'a, Theme> {
    fa_icon_solid("xmark")
}
