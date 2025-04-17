use rgb::RGB8;

pub struct Palette {
    colors: [RGB8; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct PaletteIndex(pub u8);

impl Palette {
    pub const MONOCHROME_GREEN: Self = Self {
        colors: [
            RGB8::new(0x7b, 0x82, 0x10),
            RGB8::new(0x5a, 0x79, 0x42),
            RGB8::new(0x39, 0x59, 0x4a),
            RGB8::new(0x2f, 0x41, 0x39),
        ],
    };

    pub fn color(&self, index: PaletteIndex) -> RGB8 {
        self.colors[index.0 as usize]
    }
}

pub struct PaletteMap(pub u8);

impl PaletteMap {
    pub fn color(&self, index: PaletteIndex, palette: &Palette) -> RGB8 {
        palette.color(self.map(index))
    }

    pub fn map(&self, index: PaletteIndex) -> PaletteIndex {
        PaletteIndex((self.0 >> (index.0 * 2)) & 0b11)
    }
}

pub struct Palettes {
    pub background: PaletteMap,
    pub sprite0: PaletteMap,
    pub sprite1: PaletteMap,
}

impl Default for Palettes {
    fn default() -> Self {
        Self {
            background: PaletteMap(0xfc),
            sprite0: PaletteMap(0),
            sprite1: PaletteMap(0),
        }
    }
}
