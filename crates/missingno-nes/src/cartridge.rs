//! iNES cartridge: header parse and the NROM board. Mapper breadth is
//! deliberately deferred; anything but mapper 0 is rejected loudly.

pub struct Cartridge {
    prg: Vec<u8>,
    chr: Vec<u8>,
    /// CHR count 0 means the board carries RAM instead of ROM.
    chr_writable: bool,
    pub mirroring: Mirroring,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mirroring {
    Horizontal,
    Vertical,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    NotInes,
    UnsupportedMapper(u16),
    Truncated,
}

impl Cartridge {
    pub fn load(rom: &[u8]) -> Result<Cartridge, CartridgeError> {
        if rom.len() < 16 || &rom[0..4] != b"NES\x1A" {
            return Err(CartridgeError::NotInes);
        }
        let prg_pages = rom[4] as usize;
        let chr_pages = rom[5] as usize;
        let mapper = ((rom[6] >> 4) | (rom[7] & 0xF0)) as u16;
        if mapper != 0 {
            return Err(CartridgeError::UnsupportedMapper(mapper));
        }
        let mirroring = if rom[6] & 0x01 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
        let trainer = if rom[6] & 0x04 != 0 { 512 } else { 0 };

        let prg_start = 16 + trainer;
        let prg_len = prg_pages * 0x4000;
        let chr_len = chr_pages * 0x2000;
        if rom.len() < prg_start + prg_len + chr_len {
            return Err(CartridgeError::Truncated);
        }
        let prg = rom[prg_start..prg_start + prg_len].to_vec();
        let chr = if chr_pages == 0 {
            vec![0; 0x2000]
        } else {
            rom[prg_start + prg_len..prg_start + prg_len + chr_len].to_vec()
        };
        Ok(Cartridge {
            prg,
            chr,
            chr_writable: chr_pages == 0,
            mirroring,
        })
    }

    /// CPU $8000-$FFFF; 16 KB boards mirror into both halves.
    pub fn read_prg(&self, address: u16) -> u8 {
        self.prg[(address as usize - 0x8000) % self.prg.len()]
    }

    /// PPU $0000-$1FFF.
    pub fn read_chr(&self, address: u16) -> u8 {
        self.chr[address as usize & 0x1FFF]
    }

    pub fn write_chr(&mut self, address: u16, data: u8) {
        if self.chr_writable {
            self.chr[address as usize & 0x1FFF] = data;
        }
    }
}
