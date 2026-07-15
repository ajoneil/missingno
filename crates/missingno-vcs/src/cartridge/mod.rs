//! The cartridge: whatever board sits in the slot.
//!
//! The 6507 hands the slot a 4 KB window — every access with A12 high — and the
//! board decides what answers there. A plain ROM mirrors up into it; the rest
//! page it, shadow it with RAM, or both. One module per board.

pub mod atari;
pub mod cv;
pub mod e0;
pub mod e7;
pub mod fa;
pub mod plain;
pub mod three_f;
pub mod ua;

use atari::Atari;
use cv::Cv;
use e0::E0;
use e7::E7;
use fa::Fa;
use plain::Plain;
use three_f::ThreeF;
use ua::Ua;

/// The board in the slot. Bank state and cart RAM live inline, so one board
/// exists per console and survives a power cycle exactly as the silicon does.
pub enum Board {
    /// Nothing in the slot: no silicon drives the window, so it floats.
    Empty,
    Plain(Plain),
    Atari(Atari),
    Fa(Fa),
    E0(E0),
    E7(E7),
    Cv(Cv),
    Ua(Ua),
    ThreeF(ThreeF),
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
    /// 12 KB across three banks, with 256 bytes of cart RAM and a
    /// data-bus-gated switch (CBS RAM Plus).
    Fa,
    /// 8 KB as eight 1 KB slices in three pageable windows (Parker Bros).
    E0,
    /// 16 KB as eight 2 KB banks, with 1 KB and 4×256 B cart RAMs (M-Network).
    E7,
    /// 2 KB of ROM over 1 KB of read-low cart RAM (CommaVid).
    Cv,
    /// 8 KB across two banks, selected from loosely decoded hotspots below the
    /// window (UA Ltd).
    Ua,
    /// A 2 KB fixed half over a bus-latched paged half (Tigervision).
    ThreeF,
}

impl CartType {
    /// The image size the board is wired for.
    fn image_size(self) -> usize {
        match self {
            CartType::Plain2K => 0x800,
            CartType::Plain4K => 0x1000,
            CartType::F8 | CartType::F8Sc | CartType::E0 | CartType::Ua | CartType::ThreeF => {
                0x2000
            }
            CartType::F6 | CartType::F6Sc | CartType::E7 => 0x4000,
            CartType::F4 | CartType::F4Sc => 0x8000,
            CartType::Fa => 0x3000,
            CartType::Cv => 0x800,
        }
    }
}

/// A12 hands the bus to the cart. The port has no chip select, so this is the
/// board's own decode, not the console's — which is why a board is free to
/// watch the address lines below it too.
pub(crate) fn selects_window(address: u16) -> bool {
    address & 0x1000 != 0
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
            CartType::Fa => Board::Fa(Fa::new(rom)),
            CartType::E0 => Board::E0(E0::new(rom)),
            CartType::E7 => Board::E7(E7::new(rom)),
            CartType::Cv => Board::Cv(Cv::new(rom)),
            CartType::Ua => Board::Ua(Ua::new(rom)),
            CartType::ThreeF => Board::ThreeF(ThreeF::new(rom)),
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

    /// A read cycle at the cart edge. `residue` is the byte the bus still
    /// carries entering the cycle: a board with a write port latches it, and
    /// the 3F latch samples it at an A12 rise. Returns the byte the board
    /// drives, or `None` where it leaves the bus to the console — an empty
    /// slot always does, and a board does outside its own window.
    pub fn read(&mut self, address: u16, residue: u8) -> Option<u8> {
        let window = selects_window(address);
        match &mut self.board {
            Board::Plain(board) if window => Some(board.read(address)),
            Board::Atari(board) if window => Some(board.read(address, residue)),
            Board::Fa(board) if window => Some(board.read(address, residue)),
            Board::E0(board) if window => Some(board.read(address)),
            Board::E7(board) if window => Some(board.read(address, residue)),
            Board::Cv(board) if window => Some(board.read(address, residue)),
            // Boards whose hotspots live below the window watch every cycle,
            // and answer only inside it.
            Board::Ua(board) => board.read(address),
            Board::ThreeF(board) => board.read(address, residue),
            _ => None,
        }
    }

    /// A write cycle at the cart edge: no data lands in ROM, but the address
    /// still drives the hotspot decode and any write port.
    pub fn write_access(&mut self, address: u16, data: u8, residue: u8) {
        let window = selects_window(address);
        match &mut self.board {
            Board::Atari(board) if window => board.write_access(address, data),
            Board::Fa(board) if window => board.write_access(address, data),
            Board::E0(board) if window => board.write_access(address),
            Board::E7(board) if window => board.write_access(address, data),
            Board::Cv(board) if window => board.write_access(address, data),
            Board::Ua(board) => board.write_access(address),
            Board::ThreeF(board) => board.write_access(address, residue),
            _ => {}
        }
    }

    /// Side-effect-free read for inspection: never trips a hotspot.
    pub fn peek(&self, address: u16) -> u8 {
        match &self.board {
            Board::Empty => 0,
            Board::Plain(board) => board.read(address),
            Board::Atari(board) => board.peek(address),
            Board::Fa(board) => board.peek(address),
            Board::E0(board) => board.peek(address),
            Board::E7(board) => board.peek(address),
            Board::Cv(board) => board.peek(address),
            Board::Ua(board) => board.peek(address),
            Board::ThreeF(board) => board.peek(address),
        }
    }
}
