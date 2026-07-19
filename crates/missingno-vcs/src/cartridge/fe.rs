//! The Activision FE board (Robot Tank, Decathlon): no hotspot, no opcode
//! watching — a dumb address comparator wired to one data line.
//!
//! Both 4 KB banks answer the same window; nothing in the address picks between
//! them. A latch does, and it is driven from address $01FE and data bit D5:
//! any access to $01FE arms it, and one cycle later it captures D5 — set
//! selects bank 0, clear selects bank 1. The cart decodes all 13 lines, so
//! $01FE arms even though the access is really to RIOT RAM up in the stack page.
//!
//! This is why JSR and RTS appear to switch banks with the stack near the top
//! of RAM: JSR pushes the return low byte to $01FE (arming), and its next cycle
//! fetches the target's high address byte, whose D5 picks the bank. The cart has
//! no idea an opcode ran — with the stack pointer far from $FE, the same JSR
//! switches nothing. Crane's patent application (EP 84300730.3) and cartridge
//! measurements refute the earlier opcode-watching model.
//!
//! The patent describes a three-bit latch (D7-D5) feeding a demultiplexer sized
//! for eight banks; D5 alone is the economy reading for the two-bank board
//! Activision shipped. The readings only diverge for an armed byte outside
//! $Cx-$Fx, which no shipped game produces.

const BANK_SIZE: usize = 0x1000;
/// The address whose access arms the latch, on the board's full 13-line decode.
const ARM_ADDRESS: u16 = 0x01FE;
/// The captured line: set selects bank 0, clear selects bank 1.
const BANK_SELECT: u8 = 0x20;

pub struct Fe {
    image: Vec<u8>,
    bank: usize,
    /// The access just seen was to $01FE.
    armed: bool,
    /// The byte the latch captures is on the bus during the coming cycle.
    capturing: bool,
}

impl Fe {
    pub fn new(rom: &[u8]) -> Fe {
        Fe {
            image: rom.to_vec(),
            bank: 0,
            armed: false,
            capturing: false,
        }
    }

    /// Every cycle at the cart edge. `residue` is the byte the bus carries
    /// entering this cycle — the one the previous cycle settled on, which is
    /// what an armed latch captured.
    fn cycle(&mut self, address: u16, residue: u8) {
        if self.capturing {
            self.bank = match residue & BANK_SELECT != 0 {
                true => 0,
                false => 1,
            };
        }
        self.capturing = self.armed;
        self.armed = address & 0x1FFF == ARM_ADDRESS;
    }

    pub fn read(&mut self, address: u16, residue: u8) -> Option<u8> {
        self.cycle(address, residue);
        super::selects_window(address).then(|| self.peek(address))
    }

    pub fn write_access(&mut self, address: u16, residue: u8) {
        self.cycle(address, residue);
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
