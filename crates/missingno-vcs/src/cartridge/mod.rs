//! The cartridge: whatever board sits in the slot.
//!
//! The 6507 hands the slot a 4 KB window — every access with A12 high — and the
//! board decides what answers there. A plain ROM mirrors up into it; the rest
//! page it, shadow it with RAM, or both. One module per board.

pub mod atari;
pub mod plain;

use atari::Atari;
use plain::Plain;

/// The board in the slot. Bank state and cart RAM live inline, so one board
/// exists per console and survives a power cycle exactly as the silicon does.
pub enum Board {
    /// Nothing in the slot: no silicon drives the window, so it floats.
    Empty,
    Plain(Plain),
    Atari(Atari),
}

pub struct Cartridge {
    board: Board,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    UnsupportedSize(usize),
    /// The image is the wrong size for the board it was declared as.
    WrongSizeForBoard {
        cart_type: CartType,
        size: usize,
    },
}

/// The board a ROM is wired for. The Atari codes (F8/F6/F4) name the hotspot
/// ranges that page the 4 KB window; `*Sc` variants add Superchip (SARA) cart
/// RAM, which a raw dump can't be told from a plain board by size alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CartType {
    /// 2 KB, mirrored into the window.
    Plain2K,
    /// 4 KB, fills the window.
    Plain4K,
    /// 8 KB across two banks.
    F8,
    F8Sc,
    /// 16 KB across four banks.
    F6,
    F6Sc,
    /// 32 KB across eight banks.
    F4,
    F4Sc,
}

impl CartType {
    /// The image size the board is wired for.
    fn image_size(self) -> usize {
        match self {
            CartType::Plain2K => 0x800,
            CartType::Plain4K => 0x1000,
            CartType::F8 | CartType::F8Sc => 0x2000,
            CartType::F6 | CartType::F6Sc => 0x4000,
            CartType::F4 | CartType::F4Sc => 0x8000,
        }
    }
}

/// The RAM ports shadow the bottom 256 bytes of every bank, so a Superchip
/// dump repeats each bank's first 128 bytes of filler into the next 128.
fn has_superchip_signature(rom: &[u8]) -> bool {
    use atari::SUPERCHIP_RAM_SIZE;
    rom.chunks_exact(0x1000)
        .all(|bank| bank[..SUPERCHIP_RAM_SIZE] == bank[SUPERCHIP_RAM_SIZE..2 * SUPERCHIP_RAM_SIZE])
}

impl Cartridge {
    /// Build a cartridge, honouring an explicit board type when the caller has
    /// one and inferring a best-effort board from ROM size otherwise.
    pub fn load(rom: &[u8], cart_type: Option<CartType>) -> Result<Cartridge, CartridgeError> {
        let board = match cart_type {
            Some(cart_type) => Cartridge::build(rom, cart_type)?,
            None => Cartridge::infer(rom)?,
        };
        Ok(Cartridge { board })
    }

    fn build(rom: &[u8], cart_type: CartType) -> Result<Board, CartridgeError> {
        if rom.len() != cart_type.image_size() {
            return Err(CartridgeError::WrongSizeForBoard {
                cart_type,
                size: rom.len(),
            });
        }
        let atari =
            |hotspot_base, superchip| Board::Atari(Atari::new(rom, hotspot_base, superchip));
        Ok(match cart_type {
            CartType::Plain2K | CartType::Plain4K => Board::Plain(Plain::new(rom)),
            CartType::F8 => atari(atari::F8_HOTSPOT_BASE, false),
            CartType::F8Sc => atari(atari::F8_HOTSPOT_BASE, true),
            CartType::F6 => atari(atari::F6_HOTSPOT_BASE, false),
            CartType::F6Sc => atari(atari::F6_HOTSPOT_BASE, true),
            CartType::F4 => atari(atari::F4_HOTSPOT_BASE, false),
            CartType::F4Sc => atari(atari::F4_HOTSPOT_BASE, true),
        })
    }

    fn infer(rom: &[u8]) -> Result<Board, CartridgeError> {
        let atari = |hotspot_base| {
            Board::Atari(Atari::new(rom, hotspot_base, has_superchip_signature(rom)))
        };
        match rom.len() {
            0x800 | 0x1000 => Ok(Board::Plain(Plain::new(rom))),
            0x2000 => Ok(atari(atari::F8_HOTSPOT_BASE)),
            0x4000 => Ok(atari(atari::F6_HOTSPOT_BASE)),
            0x8000 => Ok(atari(atari::F4_HOTSPOT_BASE)),
            size => Err(CartridgeError::UnsupportedSize(size)),
        }
    }

    /// An empty slot. Nothing drives the window, so it reads as the floating
    /// bus.
    pub fn unplugged() -> Cartridge {
        Cartridge {
            board: Board::Empty,
        }
    }

    /// A read cycle on the cart bus. `bus` is the byte the data bus still
    /// carries, which a board with a write port latches and an empty slot
    /// leaves standing.
    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        match &mut self.board {
            Board::Empty => bus,
            Board::Plain(board) => board.read(address),
            Board::Atari(board) => board.read(address, bus),
        }
    }

    /// A write cycle on the cart bus: no data lands in ROM, but the address
    /// still drives the hotspot decode and any write port.
    pub fn write_access(&mut self, address: u16, data: u8) {
        match &mut self.board {
            Board::Empty | Board::Plain(_) => {}
            Board::Atari(board) => board.write_access(address, data),
        }
    }

    /// Side-effect-free read for inspection: never trips a hotspot.
    pub fn peek(&self, address: u16) -> u8 {
        match &self.board {
            Board::Empty => 0,
            Board::Plain(board) => board.read(address),
            Board::Atari(board) => board.peek(address),
        }
    }
}
