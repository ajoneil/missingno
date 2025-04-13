use rgb::RGB8;

pub struct Palette {
    colors: [RGB8; 4],
}

impl Palette {
    pub const MONOCHROME_GREEN: Self = Self {
        colors: [
            RGB8::new(0x2f, 0x41, 0x39),
            RGB8::new(0x39, 0x59, 0x4a),
            RGB8::new(0x5a, 0x79, 0x42),
            RGB8::new(0x7b, 0x82, 0x10),
        ],
    };

    pub fn color(&self, index: u8) -> RGB8 {
        self.colors[index as usize]
    }
}

pub struct PaletteMap(pub u8);

impl PaletteMap {
    pub fn get(&self, index: u8, palette: &Palette) -> RGB8 {
        palette.color(self.map(index))
    }

    pub fn map(&self, index: u8) -> u8 {
        (self.0 >> (index * 2)) & 0b11
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
