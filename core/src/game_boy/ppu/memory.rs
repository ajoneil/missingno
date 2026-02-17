use super::{
    sprites::{self, Sprite, SpriteId},
    tile_maps::{TileMap, TileMapId},
    tiles::{TileBlock, TileBlockId, TileIndex},
};

/// VRAM: tile data and tile maps. Physically on the VRAM data bus
/// (0x8000–0x9FFF), separate from OAM.
pub struct Vram {
    tiles: [TileBlock; 3],
    tile_maps: [TileMap; 2],
}

impl Vram {
    pub fn new() -> Self {
        Self {
            tiles: [TileBlock::new(); 3],
            tile_maps: [TileMap::new(); 2],
        }
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
}

/// OAM: sprite attribute memory. SoC-internal, not on either data bus.
/// 40 sprites × 4 bytes = 160 bytes (0xFE00–0xFE9F).
pub struct Oam {
    sprites: [Sprite; 40],
}

impl Oam {
    pub fn new() -> Self {
        Self {
            sprites: [Sprite::new(); 40],
        }
    }

    pub fn read(&self, address: OamAddress) -> u8 {
        let sprite = &self.sprites[address.sprite.0 as usize];
        match address.byte {
            SpriteByte::PositionY => sprite.position.y_plus_16,
            SpriteByte::PositionX => sprite.position.x_plus_8,
            SpriteByte::Tile => sprite.tile.0,
            SpriteByte::Attributes => sprite.attributes.0,
        }
    }

    pub fn write(&mut self, address: OamAddress, value: u8) {
        match address.byte {
            SpriteByte::PositionY => {
                self.sprites[address.sprite.0 as usize].position.y_plus_16 = value
            }
            SpriteByte::PositionX => {
                self.sprites[address.sprite.0 as usize].position.x_plus_8 = value
            }
            SpriteByte::Tile => self.sprites[address.sprite.0 as usize].tile = TileIndex(value),
            SpriteByte::Attributes => {
                self.sprites[address.sprite.0 as usize].attributes = sprites::Attributes(value)
            }
        }
    }

    pub fn sprites(&self) -> &[Sprite] {
        &self.sprites
    }

    pub fn sprite(&self, id: SpriteId) -> &Sprite {
        &self.sprites[id.0 as usize]
    }

    /// Read the Y and X position bytes for an OAM entry.
    ///
    /// On hardware, the OAM bus is 16 bits wide. During Mode 2 scanning,
    /// the scan counter drives `OAM_A[7:2]` with `A[1:0] = 0`, so both
    /// byte 0 (Y) and byte 1 (X) are read in a single access. Bytes 2–3
    /// (tile index, attributes) are not on the bus during scanning — they
    /// are only read during Mode 3's sprite tile fetch.
    pub fn sprite_position(&self, id: SpriteId) -> (u8, u8) {
        let sprite = &self.sprites[id.0 as usize];
        (sprite.position.y_plus_16, sprite.position.x_plus_8)
    }

    /// Read a raw byte from OAM at the given byte offset (0–159).
    pub(crate) fn oam_byte(&self, offset: u8) -> u8 {
        let sprite = &self.sprites[(offset / 4) as usize];
        match offset % 4 {
            0 => sprite.position.y_plus_16,
            1 => sprite.position.x_plus_8,
            2 => sprite.tile.0,
            3 => sprite.attributes.0,
            _ => unreachable!(),
        }
    }

    /// Write a raw byte to OAM at the given byte offset (0–159).
    pub(crate) fn set_oam_byte(&mut self, offset: u8, value: u8) {
        let sprite = &mut self.sprites[(offset / 4) as usize];
        match offset % 4 {
            0 => sprite.position.y_plus_16 = value,
            1 => sprite.position.x_plus_8 = value,
            2 => sprite.tile = TileIndex(value),
            3 => sprite.attributes = sprites::Attributes(value),
            _ => unreachable!(),
        }
    }

    /// Read a little-endian 16-bit word from OAM at the given byte offset.
    pub(crate) fn oam_word(&self, offset: u8) -> u16 {
        let lo = self.oam_byte(offset) as u16;
        let hi = self.oam_byte(offset + 1) as u16;
        lo | (hi << 8)
    }

    /// Write a little-endian 16-bit word to OAM at the given byte offset.
    pub(crate) fn set_oam_word(&mut self, offset: u8, value: u16) {
        self.set_oam_byte(offset, value as u8);
        self.set_oam_byte(offset + 1, (value >> 8) as u8);
    }
}

/// Address on the VRAM bus (0x8000–0x9FFF).
#[derive(Debug, Clone)]
pub enum VramAddress {
    Tile(TileAddress),
    TileMap(TileMapAddress),
}

/// Address in OAM (0xFE00–0xFE9F).
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
