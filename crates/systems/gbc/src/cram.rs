use crate::screen::Color555;

/// One CGB colour-palette RAM (BG or OBJ): 8 palettes × 4 colours × 2 bytes,
/// addressed by a 6-bit index that auto-increments on data writes (BCPS/OCPS
/// bit 7). Data writes during mode 3 are dropped but still advance the index.
pub(crate) struct ColorRam {
    data: [u8; 64],
    index: u8,
    auto_increment: bool,
}

impl Default for ColorRam {
    fn default() -> Self {
        Self {
            data: [0; 64],
            index: 0,
            auto_increment: false,
        }
    }
}

impl ColorRam {
    pub(crate) fn read_index(&self) -> u8 {
        0x40 | ((self.auto_increment as u8) << 7) | self.index
    }

    pub(crate) fn write_index(&mut self, value: u8) {
        self.index = value & 0x3F;
        self.auto_increment = value & 0x80 != 0;
    }

    pub(crate) fn read_data(&self) -> u8 {
        self.data[self.index as usize]
    }

    pub(crate) fn write_data(&mut self, value: u8) {
        self.data[self.index as usize] = value;
        self.advance();
    }

    /// Mode-3 blocked write: the colour byte is dropped, but the index still advances.
    pub(crate) fn skip_data(&mut self) {
        self.advance();
    }

    /// The RGB555 colour for (palette 0-7, colour index 0-3): a little-endian
    /// 2-byte entry at `(palette*4 + index)*2`. Bit 15 is unused.
    pub(crate) fn color(&self, palette: u8, index: u8) -> Color555 {
        let base = (palette as usize * 4 + index as usize) * 2;
        let value = self.data[base] as u16 | ((self.data[base + 1] as u16) << 8);
        Color555(value & 0x7FFF)
    }

    /// Write a 4-colour RGB555 palette into one of the 8 slots (the boot ROM
    /// installs the DMG-compatibility palette this way).
    pub(crate) fn install(&mut self, palette: usize, colours: [u16; 4]) {
        for (index, &colour) in colours.iter().enumerate() {
            let base = (palette * 4 + index) * 2;
            self.data[base] = colour as u8;
            self.data[base + 1] = (colour >> 8) as u8;
        }
    }

    /// Seed every colour of all 8 palettes — the CGB boot ROM fades the BG
    /// palettes to white before handing off to the game.
    pub(crate) fn fill(&mut self, colour: Color555) {
        for palette in 0..8 {
            self.install(palette, [colour.0; 4]);
        }
    }

    fn advance(&mut self) {
        if self.auto_increment {
            self.index = (self.index + 1) & 0x3F;
        }
    }

    /// The raw 64 palette bytes, for a save-state capture.
    pub(crate) fn raw(&self) -> [u8; 64] {
        self.data
    }

    /// Reseat the palette bytes and the index register (BCPS/OCPS) from a save
    /// state — `index_register` carries both the auto-increment bit and index.
    pub(crate) fn restore(&mut self, data: [u8; 64], index_register: u8) {
        self.data = data;
        self.write_index(index_register);
    }
}

/// A CGB colour-palette RAM port. BCPS/BCPD ($FF68/9) address BG palettes;
/// OCPS/OCPD ($FF6A/B) address OBJ palettes. Index ports are always accessible;
/// data ports are blocked while the PPU renders (mode 3).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ColorRegister {
    BackgroundIndex,
    BackgroundData,
    ObjectIndex,
    ObjectData,
}
