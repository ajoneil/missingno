use iced::{Font, font::Weight};

pub fn default() -> Font {
    Font::with_name("Noto Sans")
}

pub fn title() -> Font {
    Font {
        weight: Weight::Bold,
        ..default()
    }
}

pub fn monospace() -> Font {
    Font::with_name("Noto Sans Mono")
}
