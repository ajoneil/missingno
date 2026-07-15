//! The cartridge: whatever board sits in the slot.
//!
//! The 6507 hands the slot a 4 KB window — every access with A12 high — and the
//! board decides what answers there. A plain ROM mirrors up into it; the rest
//! page it, shadow it with RAM, or both. One module per board.

pub mod ar;
pub mod atari;
pub mod cv;
pub mod dpc;
pub mod e0;
pub mod e7;
pub mod f0;
pub mod fa;
pub mod fc;
pub mod fe;
pub mod jane;
pub mod mdm;
pub mod plain;
pub mod sb;
pub mod three_e;
pub mod three_e_plus;
pub mod three_f;
pub mod ua;
pub mod wd;
pub mod wf8;
pub mod x07;
pub mod zero_3e0;
pub mod zero_840;
pub mod zero_fa0;

use ar::Ar;
use atari::Atari;
use cv::Cv;
use dpc::Dpc;
use e0::E0;
use e7::E7;
use f0::F0;
use fa::Fa;
use fc::Fc;
use fe::Fe;
use jane::Jane;
use mdm::Mdm;
use plain::Plain;
use sb::Sb;
use three_e::ThreeE;
use three_e_plus::ThreeEPlus;
use three_f::ThreeF;
use ua::Ua;
use wd::Wd;
use wf8::Wf8;
use x07::X07;
use zero_3e0::Zero3E0;
use zero_840::Zero840;
use zero_fa0::ZeroFa0;

/// The board in the slot. Bank state and cart RAM live inline, so one board
/// exists per console and survives a power cycle exactly as the silicon does.
/// The Supercharger's 6 KB dwarfs a plain board's state; one exists per
/// console, so the spread costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum Board {
    /// Nothing in the slot: no silicon drives the window, so it floats.
    Empty,
    Ar(Ar),
    Plain(Plain),
    Atari(Atari),
    Fa(Fa),
    E0(E0),
    E7(E7),
    Cv(Cv),
    Dpc(Dpc),
    F0(F0),
    Jane(Jane),
    Wf8(Wf8),
    Wd(Wd),
    Fc(Fc),
    ZeroFa0(ZeroFa0),
    Zero3E0(Zero3E0),
    Fe(Fe),
    Ua(Ua),
    ThreeF(ThreeF),
    ThreeE(ThreeE),
    ThreeEPlus(ThreeEPlus),
    Sb(Sb),
    Zero840(Zero840),
    X07(X07),
    Mdm(Mdm),
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
    /// 8 KB across two banks, picked by an address comparator on $01FE and
    /// data line D5 (Activision).
    Fe,
    /// An F8-banked 8 KB program ROM beside the Display Processor Chip and its
    /// own 2 KB display ROM (Pitfall II).
    Dpc,
    /// 6 KB of tape-loaded RAM and a BIOS, driven entirely by reads (Starpath
    /// Supercharger). The image is a fastload container, not a ROM.
    Ar,
    /// 64 KB across sixteen banks, stepped one at a time (Dynacom Megaboy).
    F0,
    /// 16 KB across four banks on scattered hotspots (Tarzan prototype).
    Jane,
    /// 8 KB across two banks, chosen by the written data (Coleco white label).
    Wf8,
    /// 8 KB as eight 1 KB banks filling four segments at once, on a delayed
    /// switch (Wickstead Design prototype).
    Wd,
    /// 32 KB across eight banks, latched in two halves and committed
    /// separately (Amiga Power Play Arcade).
    Fc,
    /// 8 KB across two banks, selected from loosely decoded low-memory
    /// hotspots (Fotomania).
    ZeroFa0,
    /// 8 KB as eight 1 KB slices in three segments, selected active-low from
    /// low memory (Brazilian Parker Bros).
    Zero3E0,
    /// 3F with a cart-RAM path on its own hotspot (homebrew).
    ThreeE,
    /// Four independently banked 1 KB segments, each ROM or RAM (homebrew).
    ThreeEPlus,
    /// 64 KB across sixteen banks, on the hotspot family's wider run (homebrew).
    Ef,
    /// 128 KB across thirty-two banks (homebrew).
    Df,
    /// 256 KB across sixty-four banks (homebrew).
    Bf,
    /// 128 KB across thirty-two banks selected from low memory, waking in the
    /// last of them (SuperBanking).
    Sb,
    /// 8 KB across two banks, selected by one address line (EconoBanking).
    Zero840,
    /// 64 KB across sixteen banks, with a second switch riding on TIA writes
    /// (Stocking).
    X07,
    /// 32 KB across eight banks named by an address's low byte, with a one-way
    /// lock (Menu Driven Megacart).
    Mdm,
}

impl CartType {
    /// The image size the board is wired for.
    fn image_size(self) -> usize {
        match self {
            CartType::Plain2K => 0x800,
            CartType::Plain4K => 0x1000,
            CartType::F8
            | CartType::F8Sc
            | CartType::E0
            | CartType::Ua
            | CartType::ThreeF
            | CartType::Fe => 0x2000,
            CartType::F6 | CartType::F6Sc | CartType::E7 => 0x4000,
            CartType::F4 | CartType::F4Sc => 0x8000,
            CartType::Ef | CartType::X07 => 0x10000,
            CartType::Zero840 => 0x2000,
            CartType::Mdm => 0x8000,
            CartType::Df | CartType::Sb => 0x20000,
            CartType::Bf => 0x40000,
            CartType::Fa => 0x3000,
            CartType::Cv => 0x800,
            // 8 KB program + 2 KB display, and the dumper's trailing RNG stream.
            CartType::Dpc => 0x2900,
            CartType::Ar => ar::IMAGE_SIZE,
            CartType::F0 => 0x10000,
            CartType::Jane => 0x4000,
            CartType::Wf8
            | CartType::Wd
            | CartType::ZeroFa0
            | CartType::Zero3E0
            | CartType::ThreeE
            | CartType::ThreeEPlus => 0x2000,
            CartType::Fc => 0x8000,
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
            CartType::Ef => atari(atari::EF_HOTSPOT_BASE, false),
            CartType::Df => Board::Atari(
                Atari::new(rom, atari::DF_HOTSPOT_BASE, false).waking_in(atari::DF_START_BANK),
            ),
            CartType::Bf => atari(atari::BF_HOTSPOT_BASE, false),
            CartType::Fa => Board::Fa(Fa::new(rom)),
            CartType::E0 => Board::E0(E0::new(rom)),
            CartType::E7 => Board::E7(E7::new(rom)),
            CartType::Cv => Board::Cv(Cv::new(rom)),
            CartType::Ua => Board::Ua(Ua::new(rom)),
            CartType::ThreeF => Board::ThreeF(ThreeF::new(rom)),
            CartType::Fe => Board::Fe(Fe::new(rom)),
            CartType::Dpc => Board::Dpc(Dpc::new(rom)),
            CartType::Ar => Board::Ar(Ar::new(rom)),
            CartType::F0 => Board::F0(F0::new(rom)),
            CartType::Jane => Board::Jane(Jane::new(rom)),
            CartType::Wf8 => Board::Wf8(Wf8::new(rom)),
            CartType::Wd => Board::Wd(Wd::new(rom)),
            CartType::Fc => Board::Fc(Fc::new(rom)),
            CartType::ZeroFa0 => Board::ZeroFa0(ZeroFa0::new(rom)),
            CartType::Zero3E0 => Board::Zero3E0(Zero3E0::new(rom)),
            CartType::ThreeE => Board::ThreeE(ThreeE::new(rom)),
            CartType::ThreeEPlus => Board::ThreeEPlus(ThreeEPlus::new(rom)),
            CartType::Sb => Board::Sb(Sb::new(rom)),
            CartType::Zero840 => Board::Zero840(Zero840::new(rom)),
            CartType::X07 => Board::X07(X07::new(rom)),
            CartType::Mdm => Board::Mdm(Mdm::new(rom)),
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
            Board::Dpc(board) if window => Some(board.read(address, residue)),
            Board::F0(board) if window => Some(board.read(address)),
            Board::Jane(board) if window => Some(board.read(address)),
            Board::Wf8(board) if window => Some(board.peek(address)),
            Board::Fc(board) if window => Some(board.read(address)),
            // Boards that watch the whole address bus — for hotspots below the
            // window, or to count its transitions — and answer only inside it.
            Board::Ar(board) => board.read(address, residue),
            Board::Wd(board) => board.read(address, residue),
            Board::ZeroFa0(board) => board.read(address),
            Board::Zero3E0(board) => board.read(address),
            Board::Sb(board) => board.read(address),
            Board::Zero840(board) => board.read(address),
            Board::X07(board) => board.read(address),
            Board::Mdm(board) => board.read(address),
            Board::Ua(board) => board.read(address),
            Board::ThreeF(board) => board.read(address, residue),
            Board::ThreeE(board) => board.read(address, residue),
            Board::ThreeEPlus(board) => board.read(address, residue),
            Board::Fe(board) => board.read(address, residue),
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
            Board::Dpc(board) if window => board.write_access(address, data),
            Board::F0(board) if window => board.write_access(address),
            Board::Jane(board) if window => board.write_access(address),
            Board::Wf8(board) if window => board.write_access(address, data),
            Board::Fc(board) if window => board.write_access(address, data),
            Board::Ar(board) => board.write_access(address),
            Board::Wd(board) => board.write_access(address, data),
            Board::ZeroFa0(board) => board.write_access(address),
            Board::Zero3E0(board) => board.write_access(address),
            Board::Sb(board) => board.write_access(address),
            Board::Zero840(board) => board.write_access(address),
            Board::X07(board) => board.write_access(address),
            Board::Mdm(board) => board.write_access(address),
            Board::Ua(board) => board.write_access(address),
            Board::ThreeF(board) => board.write_access(address, residue),
            Board::ThreeE(board) => board.write_access(address, residue, data),
            Board::ThreeEPlus(board) => board.write_access(address, residue, data),
            Board::Fe(board) => board.write_access(address, residue),
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
            Board::Dpc(board) => board.peek(address),
            Board::F0(board) => board.peek(address),
            Board::Jane(board) => board.peek(address),
            Board::Wf8(board) => board.peek(address),
            Board::Wd(board) => board.peek(address),
            Board::Fc(board) => board.peek(address),
            Board::ZeroFa0(board) => board.peek(address),
            Board::Zero3E0(board) => board.peek(address),
            Board::Sb(board) => board.peek(address),
            Board::Zero840(board) => board.peek(address),
            Board::X07(board) => board.peek(address),
            Board::Mdm(board) => board.peek(address),
            Board::Ar(board) => board.peek(address).unwrap_or(0),
            Board::Ua(board) => board.peek(address),
            Board::ThreeF(board) => board.peek(address),
            Board::ThreeE(board) => board.peek(address),
            Board::ThreeEPlus(board) => board.peek(address),
            Board::Fe(board) => board.peek(address),
        }
    }
}
