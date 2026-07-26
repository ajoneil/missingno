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

mod cart_type;
mod stores;

pub use cart_type::{CartType, CartridgeError, DumpFit};

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

/// A read-only view of the board and, on a DPC cart, its custom chip, for the
/// debugger's Cartridge sidebar section.
#[derive(Clone, Debug, Default)]
pub struct CartridgeInspect {
    pub board: &'static str,
    /// The 4 KB bank in the window, on boards that page a single one.
    pub bank: Option<usize>,
    pub dpc: Option<dpc::DpcView>,
}

impl Board {
    /// A short display name for the debugger.
    fn name(&self) -> &'static str {
        match self {
            Board::Empty => "empty",
            Board::Ar(_) => "Supercharger",
            Board::Plain(_) => "plain",
            Board::Atari(_) => "Atari bankswitch",
            Board::Fa(_) => "CBS RAM Plus",
            Board::E0(_) => "Parker Bros E0",
            Board::E7(_) => "M-Network E7",
            Board::Cv(_) => "CommaVid",
            Board::Dpc(_) => "DPC (Pitfall II)",
            Board::F0(_) => "Megaboy F0",
            Board::Jane(_) => "Tarzan (Jane)",
            Board::Wf8(_) => "Coleco WF8",
            Board::Wd(_) => "Wickstead WD",
            Board::Fc(_) => "Amiga FC",
            Board::ZeroFa0(_) => "Fotomania",
            Board::Zero3E0(_) => "Parker Bros 3E0",
            Board::Fe(_) => "Activision FE",
            Board::Ua(_) => "UA Ltd",
            Board::ThreeF(_) => "Tigervision 3F",
            Board::ThreeE(_) => "3E",
            Board::ThreeEPlus(_) => "3E+",
            Board::Sb(_) => "SuperBanking",
            Board::Zero840(_) => "EconoBanking",
            Board::X07(_) => "X07",
            Board::Mdm(_) => "Megacart MDM",
        }
    }

    /// The single 4 KB bank paged into the window, on boards that keep one.
    fn selected_bank(&self) -> Option<usize> {
        match self {
            Board::Atari(board) => Some(board.selected_bank()),
            Board::F0(board) => Some(board.selected_bank()),
            Board::Jane(board) => Some(board.selected_bank()),
            Board::Wf8(board) => Some(board.selected_bank()),
            Board::Fc(board) => Some(board.selected_bank()),
            Board::Sb(board) => Some(board.selected_bank()),
            Board::Mdm(board) => Some(board.selected_bank()),
            Board::X07(board) => Some(board.selected_bank()),
            _ => None,
        }
    }
}

/// A12 hands the bus to the cart. The port has no chip select, so this is the
/// board's own decode, not the console's — which is why a board is free to
/// watch the address lines below it too.
pub(crate) fn selects_window(address: u16) -> bool {
    address & 0x1000 != 0
}

impl Cartridge {
    /// Build a cartridge, honouring an explicit board type when the caller has
    /// one and inferring a best-effort board from ROM size otherwise.
    ///
    /// `clock_hz` is the rate the console will step the board at. A board with
    /// a clock of its own — the DPC's oscillator — needs it to convert its own
    /// free-running rate into those steps; every other board ignores it.
    ///
    /// `fit` grants tolerance for a catalogued overdump: the stated board says
    /// where its silicon ends, so an image longer than that is loaded as the
    /// board and the padding dropped. Without a stated board there is nothing
    /// to slice to, so the image is read whole and its size names the board.
    pub fn load(
        rom: &[u8],
        cart_type: Option<CartType>,
        clock_hz: f32,
        fit: DumpFit,
    ) -> Result<Cartridge, CartridgeError> {
        let Some(cart_type) = cart_type else {
            return Ok(Cartridge {
                board: Cartridge::build(rom, CartType::infer(rom)?, clock_hz)?,
            });
        };
        let rom = match fit {
            // A Supercharger image is a fastload container rather than the
            // board's contents, and a multi-load one is several of them, so its
            // length is no board size to trim to.
            DumpFit::Overdump if cart_type != CartType::Ar => {
                rom.get(..cart_type.image_size()).unwrap_or(rom)
            }
            _ => rom,
        };
        Ok(Cartridge {
            board: Cartridge::build(rom, cart_type, clock_hz)?,
        })
    }

    /// One console clock, for a board that runs to a clock of its own.
    pub fn tick(&mut self) {
        if let Board::Dpc(board) = &mut self.board {
            board.tick();
        }
    }

    fn build(rom: &[u8], cart_type: CartType, clock_hz: f32) -> Result<Board, CartridgeError> {
        if !cart_type.accepts(rom.len()) {
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
            CartType::Dpc => Board::Dpc(Dpc::new(rom, clock_hz)),
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
            // Boards that answer only inside the cart window.
            Board::Plain(board) => window.then(|| board.read(address)),
            Board::Atari(board) => window.then(|| board.read(address, residue)),
            Board::Fa(board) => window.then(|| board.read(address, residue)),
            Board::E0(board) => window.then(|| board.read(address)),
            Board::E7(board) => window.then(|| board.read(address, residue)),
            Board::Cv(board) => window.then(|| board.read(address, residue)),
            Board::Dpc(board) => window.then(|| board.read(address, residue)),
            Board::F0(board) => window.then(|| board.read(address)),
            Board::Jane(board) => window.then(|| board.read(address)),
            Board::Wf8(board) => window.then(|| board.peek(address)),
            Board::Fc(board) => window.then(|| board.read(address)),
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
            // No plugged board drives the bus.
            Board::Empty => None,
        }
    }

    /// A write cycle at the cart edge: no data lands in ROM, but the address
    /// still drives the hotspot decode and any write port.
    pub fn write_access(&mut self, address: u16, data: u8, residue: u8) {
        let window = selects_window(address);
        match &mut self.board {
            // Boards whose write port and hotspots sit inside the cart window.
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
            // Windowed boards ignore a write cycle outside the window.
            Board::Atari(_)
            | Board::Fa(_)
            | Board::E0(_)
            | Board::E7(_)
            | Board::Cv(_)
            | Board::Dpc(_)
            | Board::F0(_)
            | Board::Jane(_)
            | Board::Wf8(_)
            | Board::Fc(_) => {}
            // Boards that watch the whole address bus for hotspots.
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
            // A plain board and an empty slot latch nothing on a write.
            Board::Empty | Board::Plain(_) => {}
        }
    }

    /// A read-only view of the board for the debugger's Cartridge section.
    pub fn inspect(&self) -> CartridgeInspect {
        CartridgeInspect {
            board: self.board.name(),
            bank: self.board.selected_bank(),
            dpc: match &self.board {
                Board::Dpc(board) => Some(board.inspect()),
                _ => None,
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: f32 = 3_579_545.0;

    /// A 4 KB image whose second half is padding a dumper appended.
    fn padded_2k() -> Vec<u8> {
        let mut rom = vec![0xAA; 0x1000];
        rom[0x800..].fill(0xFF);
        rom
    }

    #[test]
    fn overdump_loads_the_stated_board_and_drops_the_padding() {
        let mut cart = Cartridge::load(
            &padded_2k(),
            Some(CartType::Plain2K),
            CLOCK,
            DumpFit::Overdump,
        )
        .unwrap();
        // A 2K board leaves A11 unwired, so both halves of the window answer
        // from the same 2 KB — never from the padding.
        assert_eq!(cart.read(0xF000, 0), Some(0xAA));
        assert_eq!(cart.read(0xF800, 0), Some(0xAA));
    }

    #[test]
    fn an_oversized_image_without_the_flag_is_still_refused() {
        assert_eq!(
            Cartridge::load(&padded_2k(), Some(CartType::Plain2K), CLOCK, DumpFit::Exact).err(),
            Some(CartridgeError::WrongSizeForBoard {
                cart_type: CartType::Plain2K,
                size: 0x1000,
            })
        );
    }

    #[test]
    fn the_flag_does_not_excuse_an_undersized_image() {
        assert_eq!(
            Cartridge::load(
                &vec![0u8; 0x800],
                Some(CartType::F8),
                CLOCK,
                DumpFit::Overdump
            )
            .err(),
            Some(CartridgeError::WrongSizeForBoard {
                cart_type: CartType::F8,
                size: 0x800,
            })
        );
    }

    #[test]
    fn without_a_stated_board_the_image_is_read_whole() {
        // Nothing states where the silicon ends, so the length names the board
        // and every byte belongs to it.
        let mut cart = Cartridge::load(&padded_2k(), None, CLOCK, DumpFit::Overdump).unwrap();
        assert_eq!(cart.read(0xF800, 0), Some(0xFF));
    }

    #[test]
    fn the_flag_is_inert_on_an_exact_image() {
        let rom: Vec<u8> = (0..0x800).map(|i| i as u8).collect();
        let mut cart =
            Cartridge::load(&rom, Some(CartType::Plain2K), CLOCK, DumpFit::Overdump).unwrap();
        assert_eq!(cart.read(0xF7FF, 0), Some(0xFF));
    }

    #[test]
    fn a_wrong_size_names_the_board_and_both_sizes() {
        let error = Cartridge::load(&padded_2k(), Some(CartType::Plain2K), CLOCK, DumpFit::Exact)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "image is 4096 bytes but a 2K board holds 2048"
        );
    }
}
