use super::{
    sprites::{Sprite, SpriteId},
    tile_maps::{TileMap, TileMapId},
    tiles::{TileBlock, TileBlockId},
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
                self.tile_maps[map.0 as usize].data[offset as usize]
            }
            MappedAddress::Sprite(SpriteAddress { sprite, offset }) => {
                self.sprites[sprite.0 as usize].0[offset as usize]
            }
        }
    }

    pub fn write(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Tile(TileAddress { block, offset }) => {
                self.tiles[block.0 as usize].data[offset as usize] = value;
            }
            MappedAddress::TileMap(TileMapAddress { map, offset }) => {
                self.tile_maps[map.0 as usize].data[offset as usize] = value;
            }
            MappedAddress::Sprite(SpriteAddress { sprite, offset }) => {
                self.sprites[sprite.0 as usize].0[offset as usize] = value;
            }
        }
    }

    pub fn tile_block(&self, block: TileBlockId) -> &TileBlock {
        &self.tiles[block.0 as usize]
    }
}

pub enum MappedAddress {
    Tile(TileAddress),
    TileMap(TileMapAddress),
    Sprite(SpriteAddress),
}

pub struct TileAddress {
    block: TileBlockId,
    offset: u16,
}

pub struct TileMapAddress {
    map: TileMapId,
    offset: u16,
}

pub struct SpriteAddress {
    sprite: SpriteId,
    offset: u8,
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
                offset: ((address - 0xfe00) % 4) as u8,
            }),
            _ => unreachable!(),
        }
    }
}
