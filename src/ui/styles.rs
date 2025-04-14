pub mod fonts {
    use iced::{Font, font};

    pub const DEFAULT: Font = Font::with_name("Noto Sans");
    pub const TITLE: Font = Font {
        weight: font::Weight::Bold,
        ..DEFAULT
    };
    pub const MONOSPACE: Font = Font::with_name("Noto Sans Mono");
    pub const EMOJI: Font = Font::with_name("Noto Color Emoji");
}
