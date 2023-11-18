use rgb::RGBA8;

use super::palette::Palette;

#[derive(Debug)]
pub struct Tile {
    data: [u8; 16],
}

impl Tile {
    pub fn new(data: [u8; 16]) -> Self {
        Self { data }
    }

    pub fn line(&self, line_num: u8, palette: &Palette) -> [RGBA8; 8] {
        [
            self.pixel_color(0, line_num, palette),
            self.pixel_color(1, line_num, palette),
            self.pixel_color(2, line_num, palette),
            self.pixel_color(3, line_num, palette),
            self.pixel_color(4, line_num, palette),
            self.pixel_color(5, line_num, palette),
            self.pixel_color(6, line_num, palette),
            self.pixel_color(7, line_num, palette),
        ]
    }

    pub fn pixel_color(&self, x: u8, y: u8, palette: &Palette) -> RGBA8 {
        palette.color(self.pixel_bits(x, y))
    }

    pub fn pixel_bits(&self, x: u8, y: u8) -> u8 {
        let line_start = y * 2;

        let low_byte = self.data[line_start as usize];
        let high_byte = self.data[(line_start + 1) as usize];

        let shift = 7 - x;

        let low_bit = (low_byte >> shift) & 0b1;
        let high_bit = (high_byte >> shift) & 0b1;

        let out = (high_bit << 1) + low_bit;

        out
    }
}
