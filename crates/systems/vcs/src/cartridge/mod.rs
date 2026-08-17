//! The cartridge: whatever board sits in the slot.
//!
//! The 6507 hands the slot a 4 KB window — every access with A12 high — and the
//! board decides what answers there. A plain ROM mirrors up into it; the rest
//! page it, shadow it with RAM, or both. One module per board.

mod activision;
mod amiga_power_play;
mod atari;
mod cbs_ram_plus;
mod coleco_wf8;
mod commavid;
pub mod dpc;
mod econobanking;
mod fotomania;
mod jane;
mod low_bank_select;
mod m_network;
mod megaboy;
mod menu_driven_megacart;
mod parker_bros;
mod parker_bros_brazil;
mod plain;
mod superbanking;
pub mod supercharger;
mod tigervision;
mod tigervision_ram;
mod tigervision_ram_plus;
mod ua_ltd;
mod wickstead_design;
mod x07;

mod cart_type;
mod stores;

pub use cart_type::{CartType, CartridgeError, DumpFit};

use activision::Activision;
use amiga_power_play::AmigaPowerPlay;
use atari::Atari;
use cbs_ram_plus::CbsRamPlus;
use coleco_wf8::ColecoWf8;
use commavid::Commavid;
use dpc::Dpc;
use jane::Jane;
use low_bank_select::LowBankSelect;
use m_network::MNetwork;
use megaboy::Megaboy;
use menu_driven_megacart::MenuDrivenMegacart;
use parker_bros::ParkerBros;
use parker_bros_brazil::ParkerBrosBrazil;
use plain::Plain;
use superbanking::Superbanking;
use supercharger::Supercharger;
use tigervision::Tigervision;
use tigervision_ram::TigervisionRam;
use tigervision_ram_plus::TigervisionRamPlus;
use wickstead_design::WicksteadDesign;
use x07::X07;

/// The board in the slot. Bank state and cart RAM live inline, so one board
/// exists per console and survives a power cycle exactly as the silicon does.
/// The Supercharger's 6 KB dwarfs a plain board's state; one exists per
/// console, so the spread costs nothing.
#[allow(clippy::large_enum_variant)]
enum Board {
    /// Nothing in the slot: no silicon drives the window, so it floats.
    Empty,
    Supercharger(Supercharger),
    Plain(Plain),
    Atari(Atari),
    CbsRamPlus(CbsRamPlus),
    ParkerBros(ParkerBros),
    MNetwork(MNetwork),
    Commavid(Commavid),
    Dpc(Dpc),
    Megaboy(Megaboy),
    Jane(Jane),
    ColecoWf8(ColecoWf8),
    WicksteadDesign(WicksteadDesign),
    AmigaPowerPlay(AmigaPowerPlay),
    Fotomania(LowBankSelect),
    ParkerBrosBrazil(ParkerBrosBrazil),
    Activision(Activision),
    UaLtd(LowBankSelect),
    Tigervision(Tigervision),
    TigervisionRam(TigervisionRam),
    TigervisionRamPlus(TigervisionRamPlus),
    Superbanking(Superbanking),
    Econobanking(LowBankSelect),
    X07(X07),
    MenuDrivenMegacart(MenuDrivenMegacart),
}

pub struct Cartridge {
    board: Board,
    /// The board the slot holds; `None` when nothing is plugged in.
    cart_type: Option<CartType>,
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
    /// The single 4 KB bank paged into the window, on boards that keep one.
    fn selected_bank(&self) -> Option<usize> {
        match self {
            Board::Atari(board) => Some(board.selected_bank()),
            Board::Megaboy(board) => Some(board.selected_bank()),
            Board::Jane(board) => Some(board.selected_bank()),
            Board::ColecoWf8(board) => Some(board.selected_bank()),
            Board::AmigaPowerPlay(board) => Some(board.selected_bank()),
            Board::Superbanking(board) => Some(board.selected_bank()),
            Board::MenuDrivenMegacart(board) => Some(board.selected_bank()),
            Board::X07(board) => Some(board.selected_bank()),
            _ => None,
        }
    }
}

/// A12 hands the bus to the cart. The port has no chip select, so this is the
/// board's own decode, not the console's — which is why a board is free to
/// watch the address lines below it too.
fn selects_window(address: u16) -> bool {
    address & 0x1000 != 0
}

/// A paged bank fills the whole window.
const BANK_SIZE: usize = 0x1000;

/// The image byte a window address reads on a board that pages a whole bank
/// into it.
fn banked_byte(image: &[u8], bank: usize, address: u16) -> u8 {
    image[bank * BANK_SIZE + (address & 0x0FFF) as usize]
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
            let inferred = CartType::infer(rom)?;
            return Ok(Cartridge {
                board: Cartridge::build(rom, inferred, clock_hz)?,
                cart_type: Some(inferred),
            });
        };
        let rom = match fit {
            // A board that sizes itself from the image — a Supercharger
            // fastload container, a 3E cart's bank count — states no length for
            // the padding to be trimmed back to.
            DumpFit::Overdump => match cart_type.image_size() {
                Some(size) => rom.get(..size).unwrap_or(rom),
                None => rom,
            },
            _ => rom,
        };
        Ok(Cartridge {
            board: Cartridge::build(rom, cart_type, clock_hz)?,
            cart_type: Some(cart_type),
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
            CartType::Atari8K => atari(atari::F8_HOTSPOT_BASE, false),
            CartType::Atari8KSuperchip => atari(atari::F8_HOTSPOT_BASE, true),
            CartType::Atari16K => atari(atari::F6_HOTSPOT_BASE, false),
            CartType::Atari16KSuperchip => atari(atari::F6_HOTSPOT_BASE, true),
            CartType::Atari32K => atari(atari::F4_HOTSPOT_BASE, false),
            CartType::Atari32KSuperchip => atari(atari::F4_HOTSPOT_BASE, true),
            CartType::Atari64K => atari(atari::EF_HOTSPOT_BASE, false),
            CartType::Atari128K => Board::Atari(
                Atari::new(rom, atari::DF_HOTSPOT_BASE, false).waking_in(atari::DF_START_BANK),
            ),
            CartType::Atari256K => atari(atari::BF_HOTSPOT_BASE, false),
            CartType::CbsRamPlus => Board::CbsRamPlus(CbsRamPlus::new(rom)),
            CartType::ParkerBros => Board::ParkerBros(ParkerBros::new(rom)),
            CartType::MNetwork => Board::MNetwork(MNetwork::new(rom)),
            CartType::Commavid => Board::Commavid(Commavid::new(rom)),
            CartType::UaLtd => Board::UaLtd(LowBankSelect::new(rom, ua_ltd::DECODE)),
            CartType::Tigervision => Board::Tigervision(Tigervision::new(rom)),
            CartType::Activision => Board::Activision(Activision::new(rom)),
            CartType::Dpc => Board::Dpc(Dpc::new(rom, clock_hz)),
            CartType::Supercharger => Board::Supercharger(Supercharger::new(rom)?),
            CartType::Megaboy => Board::Megaboy(Megaboy::new(rom)),
            CartType::Jane => Board::Jane(Jane::new(rom)),
            CartType::ColecoWf8 => Board::ColecoWf8(ColecoWf8::new(rom)),
            CartType::WicksteadDesign => Board::WicksteadDesign(WicksteadDesign::new(rom)),
            CartType::AmigaPowerPlay => Board::AmigaPowerPlay(AmigaPowerPlay::new(rom)),
            CartType::Fotomania => Board::Fotomania(LowBankSelect::new(rom, fotomania::DECODE)),
            CartType::ParkerBrosBrazil => Board::ParkerBrosBrazil(ParkerBrosBrazil::new(rom)),
            CartType::TigervisionRam => Board::TigervisionRam(TigervisionRam::new(rom)),
            CartType::TigervisionRamPlus => Board::TigervisionRamPlus(TigervisionRamPlus::new(rom)),
            CartType::Superbanking => Board::Superbanking(Superbanking::new(rom)),
            CartType::Econobanking => {
                Board::Econobanking(LowBankSelect::new(rom, econobanking::DECODE))
            }
            CartType::X07 => Board::X07(X07::new(rom)),
            CartType::MenuDrivenMegacart => Board::MenuDrivenMegacart(MenuDrivenMegacart::new(rom)),
            CartType::DpcPlus | CartType::GameLine | CartType::Fa2 | CartType::FourA50 => {
                return Err(CartridgeError::BoardNotBuilt(cart_type));
            }
        })
    }

    /// An empty slot. Nothing drives the window, so it reads as the floating
    /// bus.
    pub fn unplugged() -> Cartridge {
        Cartridge {
            board: Board::Empty,
            cart_type: None,
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
            Board::CbsRamPlus(board) => window.then(|| board.read(address, residue)),
            Board::ParkerBros(board) => window.then(|| board.read(address)),
            Board::MNetwork(board) => window.then(|| board.read(address, residue)),
            Board::Commavid(board) => window.then(|| board.read(address, residue)),
            Board::Dpc(board) => window.then(|| board.read(address, residue)),
            Board::Megaboy(board) => window.then(|| board.read(address)),
            Board::Jane(board) => window.then(|| board.read(address)),
            Board::ColecoWf8(board) => window.then(|| board.peek(address)),
            Board::AmigaPowerPlay(board) => window.then(|| board.read(address)),
            // Boards that watch the whole address bus — for hotspots below the
            // window, or to count its transitions — and answer only inside it.
            Board::Supercharger(board) => board.read(address, residue),
            Board::WicksteadDesign(board) => board.read(address, residue),
            Board::Fotomania(board) => board.read(address),
            Board::ParkerBrosBrazil(board) => board.read(address),
            Board::Superbanking(board) => board.read(address),
            Board::Econobanking(board) => board.read(address),
            Board::X07(board) => board.read(address),
            Board::MenuDrivenMegacart(board) => board.read(address),
            Board::UaLtd(board) => board.read(address),
            Board::Tigervision(board) => board.read(address, residue),
            Board::TigervisionRam(board) => board.read(address, residue),
            Board::TigervisionRamPlus(board) => board.read(address, residue),
            Board::Activision(board) => board.read(address, residue),
            // No plugged board drives the bus.
            Board::Empty => None,
        }
    }

    /// A write cycle at the cart edge: no data lands in ROM, but the address
    /// still drives the hotspot decode and any write port.
    pub fn write_access(&mut self, address: u16, data: u8, residue: u8) {
        let window = selects_window(address);
        match &mut self.board {
            // Boards whose write port and hotspots sit inside the cart window,
            // so a cycle outside it passes them by.
            Board::Atari(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            Board::CbsRamPlus(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            Board::ParkerBros(board) => {
                if window {
                    board.write_access(address)
                }
            }
            Board::MNetwork(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            Board::Commavid(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            Board::Dpc(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            Board::Megaboy(board) => {
                if window {
                    board.write_access(address)
                }
            }
            Board::Jane(board) => {
                if window {
                    board.write_access(address)
                }
            }
            Board::ColecoWf8(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            Board::AmigaPowerPlay(board) => {
                if window {
                    board.write_access(address, data)
                }
            }
            // Boards that watch the whole address bus for hotspots.
            Board::Supercharger(board) => board.write_access(address),
            Board::WicksteadDesign(board) => board.write_access(address, data),
            Board::Fotomania(board) => board.write_access(address),
            Board::ParkerBrosBrazil(board) => board.write_access(address),
            Board::Superbanking(board) => board.write_access(address),
            Board::Econobanking(board) => board.write_access(address),
            Board::X07(board) => board.write_access(address),
            Board::MenuDrivenMegacart(board) => board.write_access(address),
            Board::UaLtd(board) => board.write_access(address),
            Board::Tigervision(board) => board.write_access(address, residue),
            Board::TigervisionRam(board) => board.write_access(address, residue, data),
            Board::TigervisionRamPlus(board) => board.write_access(address, residue, data),
            Board::Activision(board) => board.write_access(address, residue),
            // A plain board and an empty slot latch nothing on a write.
            Board::Empty | Board::Plain(_) => {}
        }
    }

    /// A read-only view of the board for the debugger's Cartridge section.
    pub fn inspect(&self) -> CartridgeInspect {
        CartridgeInspect {
            board: self.cart_type.map_or("empty", CartType::display_name),
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
            Board::CbsRamPlus(board) => board.peek(address),
            Board::ParkerBros(board) => board.peek(address),
            Board::MNetwork(board) => board.peek(address),
            Board::Commavid(board) => board.peek(address),
            Board::Dpc(board) => board.peek(address),
            Board::Megaboy(board) => board.peek(address),
            Board::Jane(board) => board.peek(address),
            Board::ColecoWf8(board) => board.peek(address),
            Board::WicksteadDesign(board) => board.peek(address),
            Board::AmigaPowerPlay(board) => board.peek(address),
            Board::Fotomania(board) => board.peek(address),
            Board::ParkerBrosBrazil(board) => board.peek(address),
            Board::Superbanking(board) => board.peek(address),
            Board::Econobanking(board) => board.peek(address),
            Board::X07(board) => board.peek(address),
            Board::MenuDrivenMegacart(board) => board.peek(address),
            Board::Supercharger(board) => board.peek(address).unwrap_or(0),
            Board::UaLtd(board) => board.peek(address),
            Board::Tigervision(board) => board.peek(address),
            Board::TigervisionRam(board) => board.peek(address),
            Board::TigervisionRamPlus(board) => board.peek(address),
            Board::Activision(board) => board.peek(address),
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
    fn an_unbuilt_board_loads_as_a_stated_refusal() {
        for cart_type in [
            CartType::DpcPlus,
            CartType::GameLine,
            CartType::Fa2,
            CartType::FourA50,
        ] {
            assert!(!cart_type.built());
            assert_eq!(
                Cartridge::load(&vec![0; 0x8000], Some(cart_type), CLOCK, DumpFit::Exact).err(),
                Some(CartridgeError::BoardNotBuilt(cart_type))
            );
        }
        assert_eq!(
            CartridgeError::BoardNotBuilt(CartType::DpcPlus).to_string(),
            "the DPC+ board is not modelled yet"
        );
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
                Some(CartType::Atari8K),
                CLOCK,
                DumpFit::Overdump
            )
            .err(),
            Some(CartridgeError::WrongSizeForBoard {
                cart_type: CartType::Atari8K,
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

    #[test]
    fn a_3e_image_is_as_many_banks_as_the_cart_carries() {
        // 240 banks — no power of two, and far past the eight the commercial
        // boards stop at.
        let banks = 240;
        let rom: Vec<u8> = (0..banks).flat_map(|bank| [bank as u8; 0x800]).collect();
        let mut cart =
            Cartridge::load(&rom, Some(CartType::TigervisionRam), CLOCK, DumpFit::Exact).unwrap();
        // The upper half is the last bank, whatever the lower half shows.
        assert_eq!(cart.read(0xF800, 0), Some(banks as u8 - 1));
        for bank in [0, 17, banks - 1] {
            // `sta $3F` arms the latch; the next cycle's A12 rise clocks the
            // bank number the bus still carries.
            cart.write_access(0x003F, bank as u8, 0);
            cart.read(0xF800, bank as u8);
            assert_eq!(cart.read(0xF000, 0), Some(bank as u8));
        }
    }

    #[test]
    fn a_3e_image_short_of_a_whole_bank_is_refused() {
        let error = Cartridge::load(
            &vec![0u8; 0x900],
            Some(CartType::TigervisionRam),
            CLOCK,
            DumpFit::Exact,
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "image is 2304 bytes, no 3E image");
    }

    #[test]
    fn whole_supercharger_load_units_name_the_board() {
        assert_eq!(
            CartType::infer(&[0u8; supercharger::IMAGE_SIZE]),
            Ok(CartType::Supercharger)
        );
        assert_eq!(
            CartType::infer(&vec![0u8; 4 * supercharger::IMAGE_SIZE]),
            Ok(CartType::Supercharger)
        );
        // Past the cap, and short of a whole unit, the length names nothing.
        let beyond = (supercharger::MAX_LOADS + 1) * supercharger::IMAGE_SIZE;
        assert_eq!(
            CartType::infer(&vec![0u8; beyond]),
            Err(CartridgeError::UnsupportedSize(beyond))
        );
        assert_eq!(
            CartType::infer(&[0u8; supercharger::IMAGE_SIZE - 1]),
            Err(CartridgeError::UnsupportedSize(
                supercharger::IMAGE_SIZE - 1
            ))
        );
    }
}
