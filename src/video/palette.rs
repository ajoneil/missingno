use rgb::RGBA8;

pub struct Palette {
    colors: [RGBA8; 4],
}

impl Palette {
    pub const MONOCHROME_GREEN: Self = Self {
        colors: [
            RGBA8::new(0, 0x3f, 0, 0xff),
            RGBA8::new(0x2e, 0x73, 0x20, 0xff),
            RGBA8::new(0x8c, 0xbf, 0x0a, 0xff),
            RGBA8::new(0x8c, 0xbf, 0x0a, 0xff),
        ],
    };

    pub fn color(&self, index: u8) -> RGBA8 {
        self.colors[index as usize]
    }
}
