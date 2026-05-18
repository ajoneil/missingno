use super::types::{
    sprites::{self, Sprite, SpriteId},
    tiles::{TileBlock, TileBlockId, TileIndex, TileMap, TileMapId},
};

/// VRAM data bus (0x8000–0x9FFF): tile data and tile maps.
#[derive(Default)]
pub struct Vram {
    pub(crate) tiles: [TileBlock; 3],
    pub(crate) tile_maps: [TileMap; 2],
}

impl Vram {
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut vram = Vram::default();
        let len = data.len().min(0x2000);
        for i in 0..len {
            if i < 0x1800 {
                let block = i / 0x800;
                let within = i % 0x800;
                vram.tiles[block].data[within] = data[i];
            } else {
                let map_offset = i - 0x1800;
                let map = map_offset / 0x400;
                let within = map_offset % 0x400;
                vram.tile_maps[map].data[within] = TileIndex(data[i]);
            }
        }
        vram
    }

    pub fn read(&self, address: VramAddress) -> u8 {
        match address {
            VramAddress::Tile(TileAddress { block, offset }) => {
                self.tiles[block.0 as usize].data[offset as usize]
            }
            VramAddress::TileMap(TileMapAddress { map, offset }) => {
                self.tile_maps[map.0 as usize].data[offset as usize].0
            }
        }
    }

    pub fn write(&mut self, address: VramAddress, value: u8) {
        match address {
            VramAddress::Tile(TileAddress { block, offset }) => {
                self.tiles[block.0 as usize].data[offset as usize] = value;
            }
            VramAddress::TileMap(TileMapAddress { map, offset }) => {
                self.tile_maps[map.0 as usize].data[offset as usize] = TileIndex(value);
            }
        }
    }

    pub fn tile_block(&self, block: TileBlockId) -> &TileBlock {
        &self.tiles[block.0 as usize]
    }

    pub fn tile_map(&self, id: TileMapId) -> &TileMap {
        &self.tile_maps[id.0 as usize]
    }

    /// Read by flat offset (0x0000–0x1FFF; the 0x8000–0x9FFF range).
    pub fn read_byte(&self, offset: u16) -> u8 {
        let offset = offset as usize & 0x1FFF;
        if offset < 0x1800 {
            let block = offset / 0x800;
            let within = offset % 0x800;
            self.tiles[block].data[within]
        } else {
            let map_offset = offset - 0x1800;
            let map = map_offset / 0x400;
            let within = map_offset % 0x400;
            self.tile_maps[map].data[within].0
        }
    }

    /// Populate VRAM with the state the DMG boot ROM leaves behind: decompressed
    /// Nintendo logo tiles (1-24), ® (tile 25), and tile-map entries. `logo` is
    /// the 48-byte logo region of the cartridge header (0x0104-0x0133).
    pub fn init_post_boot(&mut self, logo: &[u8; 0x30]) {
        // Each nibble's bits are doubled horizontally (1 bit → 2 pixels) and the row is doubled vertically.
        let mut vram_offset: usize = 0x10;
        for &logo_byte in logo {
            for &nibble in &[logo_byte >> 4, logo_byte & 0x0F] {
                let expanded = (((nibble >> 3) & 1) * 0xC0)
                    | (((nibble >> 2) & 1) * 0x30)
                    | (((nibble >> 1) & 1) * 0x0C)
                    | ((nibble & 1) * 0x03);
                self.tiles[0].data[vram_offset] = expanded;
                self.tiles[0].data[vram_offset + 2] = expanded;
                vram_offset += 4;
            }
        }

        const REGISTERED_SYMBOL: [u8; 8] = [0x3C, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x3C];
        let tile_25_offset: usize = 25 * 16;
        for (i, &byte) in REGISTERED_SYMBOL.iter().enumerate() {
            self.tiles[0].data[tile_25_offset + i * 2] = byte;
        }

        for col in 0u16..12 {
            let map_offset = (8 * 32 + 4 + col) as usize;
            self.tile_maps[0].data[map_offset] = TileIndex((col + 1) as u8);
        }
        self.tile_maps[0].data[8 * 32 + 16] = TileIndex(25);
        for col in 0u16..12 {
            let map_offset = (9 * 32 + 4 + col) as usize;
            self.tile_maps[0].data[map_offset] = TileIndex((col + 13) as u8);
        }
    }
}

/// Sprite attribute memory (0xFE00–0xFE9F): 40 sprites × 4 bytes. SoC-internal.
pub struct Oam {
    sprites: [Sprite; 40],
}

impl Default for Oam {
    fn default() -> Self {
        Self {
            sprites: [Sprite::default(); 40],
        }
    }
}

impl Oam {
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut oam = Oam::default();
        for i in 0..40 {
            let base = i * 4;
            if base + 3 < data.len() {
                oam.sprites[i].position.y = data[base];
                oam.sprites[i].position.x = data[base + 1];
                oam.sprites[i].tile = TileIndex(data[base + 2]);
                oam.sprites[i].attributes = sprites::Attributes(data[base + 3]);
            }
        }
        oam
    }

    pub fn read(&self, address: OamAddress) -> u8 {
        let sprite = &self.sprites[address.sprite.0 as usize];
        match address.byte {
            SpriteByte::PositionY => sprite.position.y,
            SpriteByte::PositionX => sprite.position.x,
            SpriteByte::Tile => sprite.tile.0,
            SpriteByte::Attributes => sprite.attributes.0,
        }
    }

    pub fn write(&mut self, address: OamAddress, value: u8) {
        match address.byte {
            SpriteByte::PositionY => {
                self.sprites[address.sprite.0 as usize].position.y = value
            }
            SpriteByte::PositionX => {
                self.sprites[address.sprite.0 as usize].position.x = value
            }
            SpriteByte::Tile => self.sprites[address.sprite.0 as usize].tile = TileIndex(value),
            SpriteByte::Attributes => {
                self.sprites[address.sprite.0 as usize].attributes = sprites::Attributes(value)
            }
        }
    }

    pub fn sprite(&self, id: SpriteId) -> &Sprite {
        &self.sprites[id.0 as usize]
    }

    /// Read the Y/X byte-pair driven on the 16-bit OAM bus during Mode 2 scanning.
    pub(in crate::ppu) fn sprite_position(&self, id: SpriteId) -> (u8, u8) {
        let sprite = &self.sprites[id.0 as usize];
        (sprite.position.y, sprite.position.x)
    }

    pub(in crate::ppu) fn oam_byte(&self, offset: u8) -> u8 {
        let sprite = &self.sprites[(offset / 4) as usize];
        match offset % 4 {
            0 => sprite.position.y,
            1 => sprite.position.x,
            2 => sprite.tile.0,
            3 => sprite.attributes.0,
            _ => unreachable!(),
        }
    }

    pub(in crate::ppu) fn set_oam_byte(&mut self, offset: u8, value: u8) {
        let sprite = &mut self.sprites[(offset / 4) as usize];
        match offset % 4 {
            0 => sprite.position.y = value,
            1 => sprite.position.x = value,
            2 => sprite.tile = TileIndex(value),
            3 => sprite.attributes = sprites::Attributes(value),
            _ => unreachable!(),
        }
    }

    pub(in crate::ppu) fn oam_word(&self, offset: u8) -> u16 {
        let lo = self.oam_byte(offset) as u16;
        let hi = self.oam_byte(offset + 1) as u16;
        lo | (hi << 8)
    }

    pub(in crate::ppu) fn set_oam_word(&mut self, offset: u8, value: u16) {
        self.set_oam_byte(offset, value as u8);
        self.set_oam_byte(offset + 1, (value >> 8) as u8);
    }
}

#[derive(Debug, Clone)]
pub enum VramAddress {
    Tile(TileAddress),
    TileMap(TileMapAddress),
}

#[derive(Debug, Clone)]
pub struct OamAddress {
    pub sprite: SpriteId,
    pub byte: SpriteByte,
}

#[derive(Debug, Clone)]
pub struct TileAddress {
    block: TileBlockId,
    offset: u16,
}

#[derive(Debug, Clone)]
pub struct TileMapAddress {
    map: TileMapId,
    offset: u16,
}

#[derive(Debug, Clone)]
pub enum SpriteByte {
    PositionX,
    PositionY,
    Tile,
    Attributes,
}

#[derive(Debug, Clone)]
pub enum MappedAddress {
    Vram(VramAddress),
    Oam(OamAddress),
}

impl MappedAddress {
    pub fn map(address: u16) -> Self {
        match address {
            0x8000..=0x87ff => Self::Vram(VramAddress::Tile(TileAddress {
                block: TileBlockId(0),
                offset: address - 0x8000,
            })),
            0x8800..=0x8fff => Self::Vram(VramAddress::Tile(TileAddress {
                block: TileBlockId(1),
                offset: address - 0x8800,
            })),
            0x9000..=0x97ff => Self::Vram(VramAddress::Tile(TileAddress {
                block: TileBlockId(2),
                offset: address - 0x9000,
            })),
            0x9800..=0x9bff => Self::Vram(VramAddress::TileMap(TileMapAddress {
                map: TileMapId(0),
                offset: address - 0x9800,
            })),
            0x9c00..=0x9fff => Self::Vram(VramAddress::TileMap(TileMapAddress {
                map: TileMapId(1),
                offset: address - 0x9c00,
            })),
            0xfe00..=0xfe9f => Self::Oam(OamAddress {
                sprite: SpriteId(((address - 0xfe00) / 4) as u8),
                byte: match (address - 0xfe00) % 4 {
                    0 => SpriteByte::PositionY,
                    1 => SpriteByte::PositionX,
                    2 => SpriteByte::Tile,
                    3 => SpriteByte::Attributes,
                    _ => unreachable!(),
                },
            }),
            _ => unreachable!(),
        }
    }
}
