use iced::{Font, font::Weight};

const INTER_REGULAR: &[u8] = include_bytes!("../../../fonts/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../../fonts/Inter-Bold.ttf");
const SPACE_GROTESK_REGULAR: &[u8] = include_bytes!("../../../fonts/SpaceGrotesk-Regular.ttf");
const SPACE_GROTESK_BOLD: &[u8] = include_bytes!("../../../fonts/SpaceGrotesk-Bold.ttf");

pub fn load() -> Vec<&'static [u8]> {
    vec![
        INTER_REGULAR,
        INTER_BOLD,
        SPACE_GROTESK_REGULAR,
        SPACE_GROTESK_BOLD,
    ]
}

pub fn default() -> Font {
    Font::with_name("Inter")
}

pub fn heading() -> Font {
    Font::with_name("Space Grotesk")
}

pub fn title() -> Font {
    Font {
        weight: Weight::Bold,
        ..heading()
    }
}

pub fn monospace() -> Font {
    Font::with_name("Noto Sans Mono")
}
