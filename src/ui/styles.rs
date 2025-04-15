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

pub mod spacing {
    pub fn xs() -> f32 {
        m() * 0.25
    }

    pub fn s() -> f32 {
        m() * 0.5
    }

    pub fn m() -> f32 {
        16.0
    }

    pub fn l() -> f32 {
        m() * 1.5
    }

    // pub fn xl() -> f32 {
    //     m() * 3.0
    // }
}
