use super::{
    sprites::{self, Sprite, SpriteId},
    tile_maps::{TileMap, TileMapId},
    tiles::{TileBlock, TileBlockId, TileIndex},
};

pub struct VideoMemory {
    tiles: [TileBlock; 3],
    tile_maps: [TileMap; 2],
    sprites: [Sprite; 40],
}

impl VideoMemory {
    pub fn new() -> Self {
        Self {
            tiles: [TileBlock::new(); 3],
            tile_maps: [TileMap::new(); 2],
            sprites: [Sprite::new(); 40],
        }
    }

    pub fn read(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::Tile(TileAddress { block, offset }) => {
                self.tiles[block.0 as usize].data[offset as usize]
            }
            MappedAddress::TileMap(TileMapAddress { map, offset }) => {
                self.tile_maps[map.0 as usize].data[offset as usize].0
            }
            MappedAddress::Sprite(SpriteAddress { sprite, byte }) => {
                let sprite = &self.sprites[sprite.0 as usize];
                match byte {
                    SpriteByte::PositionY => sprite.position.y_plus_16,
                    SpriteByte::PositionX => sprite.position.x_plus_8,
                    SpriteByte::Tile => sprite.tile.0,
                    SpriteByte::Attributes => sprite.attributes.0,
                }
            }
        }
    }

    pub fn write(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Tile(TileAddress { block, offset }) => {
                self.tiles[block.0 as usize].data[offset as usize] = value;
            }
            MappedAddress::TileMap(TileMapAddress { map, offset }) => {
                self.tile_maps[map.0 as usize].data[offset as usize] = TileIndex(value);
            }
            MappedAddress::Sprite(SpriteAddress { sprite, byte }) => match byte {
                SpriteByte::PositionY => self.sprites[sprite.0 as usize].position.y_plus_16 = value,
                SpriteByte::PositionX => self.sprites[sprite.0 as usize].position.x_plus_8 = value,
                SpriteByte::Tile => self.sprites[sprite.0 as usize].tile = TileIndex(value),
                SpriteByte::Attributes => {
                    self.sprites[sprite.0 as usize].attributes = sprites::Attributes(value)
                }
            },
        }
    }

    pub fn tile_block(&self, block: TileBlockId) -> &TileBlock {
        &self.tiles[block.0 as usize]
    }

    pub fn tile_map(&self, id: TileMapId) -> &TileMap {
        &self.tile_maps[id.0 as usize]
    }

    pub fn sprites(&self) -> &[Sprite] {
        &self.sprites
    }

    pub fn sprite(&self, id: SpriteId) -> &Sprite {
        &self.sprites[id.0 as usize]
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

#[derive(Debug, Clone)]
pub enum MappedAddress {
    Tile(TileAddress),
    TileMap(TileMapAddress),
    Sprite(SpriteAddress),
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
enum SpriteByte {
    PositionX,
    PositionY,
    Tile,
    Attributes,
}

#[derive(Debug, Clone)]
pub struct SpriteAddress {
    sprite: SpriteId,
    byte: SpriteByte,
}

impl MappedAddress {
    pub fn map(address: u16) -> Self {
        match address {
            0x8000..=0x87ff => Self::Tile(TileAddress {
                block: TileBlockId(0),
                offset: address - 0x8000,
            }),
            0x8800..=0x8fff => Self::Tile(TileAddress {
                block: TileBlockId(1),
                offset: address - 0x8800,
            }),
            0x9000..=0x97ff => Self::Tile(TileAddress {
                block: TileBlockId(2),
                offset: address - 0x9000,
            }),
            0x9800..=0x9bff => Self::TileMap(TileMapAddress {
                map: TileMapId(0),
                offset: address - 0x9800,
            }),
            0x9c00..=0x9fff => Self::TileMap(TileMapAddress {
                map: TileMapId(1),
                offset: address - 0x9c00,
            }),
            0xfe00..=0xfe9f => Self::Sprite(SpriteAddress {
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
