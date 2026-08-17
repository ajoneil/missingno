//! The cartridge as the board sees it: whatever silicon sits behind the two
//! chip selects the '139 decodes — `/EXM2` over $0000-$7FFF and `/EXM1` over
//! $8000-$BFFF — and the `/DSRAM` line on edge pin B3, which is the console
//! work RAM's own chip select: a cart holding it high deselects the console's
//! kilobyte and answers over it. No SG-1000 board switches banks; the mapper
//! carts are Mark III-era. One module per board.

mod cart_type;
mod dahjee_a;
mod dahjee_b;
mod flat;
mod sega_ram;

pub use cart_type::{CartType, CartridgeError};

use dahjee_a::DahjeeA;
use dahjee_b::DahjeeB;
use flat::Flat;
use sega_ram::SegaRam;

/// The `/EXM2` window: $0000-$7FFF.
const EXM2_WINDOW: usize = 0x8000;
/// Both cartridge windows together: $0000-$BFFF.
const CARTRIDGE_SPAN: usize = 0xC000;
/// The base of the `/EXM1` window.
const EXM1_BASE: u16 = 0x8000;
/// Where `/CS WRAM` — and `/DSRAM` with it — takes over from the cart selects.
const WRAM_BASE: u16 = 0xC000;

/// What the Z80 reads where nothing on the board drives the data bus.
pub const UNDRIVEN: u8 = 0xFF;

/// The board in the slot. Cart RAM lives inline, so it exists for as long as
/// the board does and, carrying no battery, wakes cleared.
enum Board {
    Flat(Flat),
    SegaRam(SegaRam),
    DahjeeA(DahjeeA),
    DahjeeB(DahjeeB),
}

pub struct Cartridge {
    board: Board,
}

impl Cartridge {
    /// Build the stated board, or a plain ROM image where nothing states one —
    /// an SG-1000 dump carries no header, and no length distinguishes a
    /// RAM-bearing board, so there is nothing to infer a board from.
    pub fn load(rom: &[u8], cart_type: Option<CartType>) -> Result<Cartridge, CartridgeError> {
        let cart_type = cart_type.unwrap_or(CartType::Flat);
        if rom.is_empty() || rom.len() > CARTRIDGE_SPAN {
            return Err(CartridgeError::UnsupportedSize(rom.len()));
        }
        if rom.len() > cart_type.rom_window() {
            return Err(CartridgeError::WrongSizeForBoard {
                cart_type,
                size: rom.len(),
            });
        }
        Ok(Cartridge {
            board: match cart_type {
                CartType::Flat => Board::Flat(Flat::new(rom)),
                CartType::OthelloRam => Board::SegaRam(SegaRam::new(rom, sega_ram::OTHELLO_RAM)),
                CartType::CastleRam => Board::SegaRam(SegaRam::new(rom, sega_ram::CASTLE_RAM)),
                CartType::DahjeeA => Board::DahjeeA(DahjeeA::new(rom)),
                CartType::DahjeeB => Board::DahjeeB(DahjeeB::new(rom)),
            },
        })
    }

    /// The byte the cartridge drives, or `None` where it leaves the bus to the
    /// console.
    pub fn read(&self, address: u16) -> Option<u8> {
        match &self.board {
            Board::Flat(board) => board.read(address),
            Board::SegaRam(board) => board.read(address),
            Board::DahjeeA(board) => board.read(address),
            Board::DahjeeB(board) => board.read(address),
        }
    }

    /// A write cycle at the cart edge; nothing lands where the board has no RAM.
    pub fn write(&mut self, address: u16, value: u8) {
        match &mut self.board {
            Board::Flat(_) => {}
            Board::SegaRam(board) => board.write(address, value),
            Board::DahjeeA(board) => board.write(address, value),
            Board::DahjeeB(board) => board.write(address, value),
        }
    }

    /// Whether the cart is holding `/DSRAM` high for this access, taking the
    /// console work RAM's chip select away.
    pub fn disables_console_ram(&self, address: u16) -> bool {
        match &self.board {
            Board::Flat(_) | Board::SegaRam(_) => false,
            Board::DahjeeA(board) => board.disables_console_ram(address),
            Board::DahjeeB(board) => board.disables_console_ram(address),
        }
    }

    /// The cart's own RAM, its chips in decode order — the blob a save state
    /// carries. `None` for a board with none.
    pub fn ram(&self) -> Option<Vec<u8>> {
        let chips = self.ram_chips();
        (!chips.is_empty()).then(|| chips.concat())
    }

    /// Fill the cart's RAM from a save state's blob, chip by chip.
    pub fn restore_ram(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        for chip in self.ram_chips_mut() {
            let len = chip.len().min(rest.len());
            chip[..len].copy_from_slice(&rest[..len]);
            rest = &rest[len..];
        }
    }

    /// Cart RAM is plain SRAM with no battery behind it, so it wakes cleared.
    pub fn power_cycle(&mut self) {
        for chip in self.ram_chips_mut() {
            chip.fill(0);
        }
    }

    fn ram_chips(&self) -> Vec<&[u8]> {
        match &self.board {
            Board::Flat(_) => Vec::new(),
            Board::SegaRam(board) => vec![board.ram()],
            Board::DahjeeA(board) => board.ram(),
            Board::DahjeeB(board) => vec![board.ram()],
        }
    }

    fn ram_chips_mut(&mut self) -> Vec<&mut [u8]> {
        match &mut self.board {
            Board::Flat(_) => Vec::new(),
            Board::SegaRam(board) => vec![board.ram_mut()],
            Board::DahjeeA(board) => board.ram_mut(),
            Board::DahjeeB(board) => vec![board.ram_mut()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(rom: &[u8]) -> Cartridge {
        Cartridge::load(rom, None).expect("a flat image")
    }

    fn board(cart_type: CartType) -> Cartridge {
        Cartridge::load(&[0x11; 0x8000], Some(cart_type)).expect("an image the board holds")
    }

    #[test]
    fn a_small_image_repeats_through_the_lower_window() {
        let mut rom = vec![0; 0x2000];
        rom[0] = 0x42;
        let cart = flat(&rom);
        assert_eq!(cart.read(0x0000), Some(0x42));
        assert_eq!(cart.read(0x2000), Some(0x42));
        assert_eq!(cart.read(0x6000), Some(0x42));
        // ROM1's region belongs to a second chip this image does not carry.
        assert_eq!(cart.read(0x8000), None);
    }

    #[test]
    fn a_large_image_runs_flat_into_the_upper_window() {
        let mut rom = vec![0; 0xC000];
        rom[0xA000] = 0x5A;
        let cart = flat(&rom);
        assert_eq!(cart.read(0xA000), Some(0x5A));
        assert_eq!(cart.read(0x2000), Some(0x00));
    }

    /// A plain board drives nothing above the ROM, holds no RAM and never
    /// touches `/DSRAM`.
    #[test]
    fn a_flat_board_leaves_the_console_its_own_windows() {
        let mut cart = flat(&[0x11; 0x2000]);
        cart.write(0x8000, 0x99);
        cart.write(0xC000, 0x99);
        assert_eq!(cart.read(0x8000), None);
        assert_eq!(cart.read(0xC000), None);
        assert!(!cart.disables_console_ram(0xC000));
        assert_eq!(cart.ram(), None);
    }

    #[test]
    fn an_image_past_the_windows_is_rejected() {
        assert_eq!(
            Cartridge::load(&vec![0; 0x10000], None).err(),
            Some(CartridgeError::UnsupportedSize(0x10000))
        );
        assert_eq!(
            Cartridge::load(&[], None).err(),
            Some(CartridgeError::UnsupportedSize(0))
        );
    }

    #[test]
    fn an_image_past_the_stated_boards_rom_window_is_rejected() {
        for cart_type in [CartType::OthelloRam, CartType::CastleRam, CartType::DahjeeA] {
            let error = Cartridge::load(&vec![0; 0x8001], Some(cart_type))
                .err()
                .expect("the image runs past the board's ROM window");
            assert_eq!(
                error,
                CartridgeError::WrongSizeForBoard {
                    cart_type,
                    size: 0x8001
                }
            );
        }
        assert_eq!(
            Cartridge::load(&vec![0; 0x8001], Some(CartType::DahjeeB)).err(),
            None
        );
        assert_eq!(
            Cartridge::load(&vec![0; 0x8001], Some(CartType::OthelloRam))
                .err()
                .map(|error| error.to_string()),
            Some("image is 32769 bytes but a OTHELLO board holds at most 32768".to_string())
        );
    }

    /// 2 KB with A0-A10 wired, selected by `/EXM1` alone: the same kilobyte pair
    /// answers all eight slots of $8000-$BFFF.
    #[test]
    fn othellos_two_kilobytes_repeat_eight_times_through_exm1() {
        let mut cart = board(CartType::OthelloRam);
        cart.write(0x8000, 0x5A);
        cart.write(0x87FF, 0xA5);
        for slot in 0..8 {
            let base = 0x8000 + slot * 0x800;
            assert_eq!(cart.read(base), Some(0x5A), "slot at {base:04X}");
            assert_eq!(cart.read(base + 0x7FF), Some(0xA5));
        }
        // A write through any mirror lands in the one 2 KB.
        cart.write(0xB800, 0x3C);
        assert_eq!(cart.read(0x8000), Some(0x3C));
        assert_eq!(cart.ram().map(|ram| ram.len()), Some(0x800));
    }

    /// 8 KB with A0-A12 wired: the window holds it twice.
    #[test]
    fn the_castles_eight_kilobytes_repeat_twice_through_exm1() {
        let mut cart = board(CartType::CastleRam);
        cart.write(0x8000, 0x5A);
        cart.write(0x9FFF, 0xA5);
        assert_eq!(cart.read(0xA000), Some(0x5A));
        assert_eq!(cart.read(0xBFFF), Some(0xA5));
        cart.write(0xA000, 0x3C);
        assert_eq!(cart.read(0x8000), Some(0x3C));
        assert_eq!(cart.ram().map(|ram| ram.len()), Some(0x2000));
    }

    /// Neither Sega board brings `/DSRAM` onto the sheet, and the ROM keeps the
    /// whole `/EXM2` window.
    #[test]
    fn the_sega_boards_leave_the_console_ram_selected() {
        for cart_type in [CartType::OthelloRam, CartType::CastleRam] {
            let cart = board(cart_type);
            assert!(!cart.disables_console_ram(0xC000));
            assert_eq!(cart.read(0xC000), None);
            assert_eq!(cart.read(0x0000), Some(0x11));
            assert_eq!(cart.read(0x7FFF), Some(0x11));
        }
    }

    /// Type A: the expander answers $2000-$3FFF, the ROM keeps the rest of
    /// `/EXM2`, and the kilobyte repeats through the whole `/DSRAM` window.
    #[test]
    fn type_a_answers_its_expansion_window_and_the_console_ram_window() {
        let mut cart = board(CartType::DahjeeA);
        cart.write(0x2000, 0x5A);
        cart.write(0x3FFF, 0xA5);
        assert_eq!(cart.read(0x2000), Some(0x5A));
        assert_eq!(cart.read(0x3FFF), Some(0xA5));
        // The ROM answers its own addresses either side of the expansion.
        assert_eq!(cart.read(0x1FFF), Some(0x11));
        assert_eq!(cart.read(0x4000), Some(0x11));
        // Nothing on the board drives `/EXM1`.
        assert_eq!(cart.read(0x8000), None);

        cart.write(0xC000, 0x3C);
        for slot in 0..16 {
            assert_eq!(cart.read(0xC000 + slot * 0x400), Some(0x3C));
        }
        assert!(cart.disables_console_ram(0xC000));
        assert!(cart.disables_console_ram(0xFFFF));
        assert_eq!(cart.ram().map(|ram| ram.len()), Some(0x2400));
    }

    /// Type B: ROM through both cartridge windows, 8 KB twice over the console's.
    #[test]
    fn type_b_answers_the_console_ram_window_twice() {
        let mut cart = Cartridge::load(&[0x11; 0xC000], Some(CartType::DahjeeB)).unwrap();
        assert_eq!(cart.read(0x0000), Some(0x11));
        assert_eq!(cart.read(0xBFFF), Some(0x11));

        cart.write(0xC000, 0x5A);
        cart.write(0xDFFF, 0xA5);
        assert_eq!(cart.read(0xE000), Some(0x5A));
        assert_eq!(cart.read(0xFFFF), Some(0xA5));
        assert!(cart.disables_console_ram(0xC000));
        assert_eq!(cart.ram().map(|ram| ram.len()), Some(0x2000));
    }

    /// The state blob carries the board's chips in decode order, and a power
    /// cycle clears them — cart SRAM has no battery behind it.
    #[test]
    fn cart_ram_rides_a_blob_and_wakes_cleared() {
        let mut cart = board(CartType::DahjeeA);
        cart.write(0x2000, 0x5A);
        cart.write(0xC000, 0xA5);
        let saved = cart.ram().expect("the board carries RAM");
        assert_eq!(saved[0], 0x5A);
        assert_eq!(saved[0x2000], 0xA5);

        cart.power_cycle();
        assert_eq!(cart.read(0x2000), Some(0));
        assert_eq!(cart.read(0xC000), Some(0));

        cart.restore_ram(&saved);
        assert_eq!(cart.read(0x2000), Some(0x5A));
        assert_eq!(cart.read(0xC000), Some(0xA5));
    }
}
