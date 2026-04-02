use iced::{Font, font::Weight};

const SPACE_GROTESK_REGULAR: &[u8] = include_bytes!("../../../fonts/SpaceGrotesk-Regular.ttf");
const SPACE_GROTESK_BOLD: &[u8] = include_bytes!("../../../fonts/SpaceGrotesk-Bold.ttf");
const CHAKRA_PETCH_REGULAR: &[u8] = include_bytes!("../../../fonts/ChakraPetch-Regular.ttf");
const CHAKRA_PETCH_BOLD: &[u8] = include_bytes!("../../../fonts/ChakraPetch-Bold.ttf");

pub fn load() -> Vec<&'static [u8]> {
    vec![
        SPACE_GROTESK_REGULAR,
        SPACE_GROTESK_BOLD,
        CHAKRA_PETCH_REGULAR,
        CHAKRA_PETCH_BOLD,
    ]
}

pub fn default() -> Font {
    Font::with_name("Space Grotesk")
}

pub fn heading() -> Font {
    Font::with_name("Chakra Petch")
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
