//! The FC board (Amiga Power Play Arcade): latching a bank and switching to it
//! are separate acts.
//!
//! Two registers assemble a target bank number — $1FF8 takes its low two bits
//! from a write, $1FF9 the rest — and nothing moves until the commit hotspot
//! $1FFC is touched. The target survives the commit, so touching $1FFC again
//! re-commits the same bank with no fresh writes.
//!
//! Only a *read* of $1FFC commits here. That follows Stella, whose poke path
//! masks the address down to the bank before comparing and so never matches;
//! real Amiga carts are known to commit with `sta $FFFC` too (Surf's Up does),
//! which makes the write-inertness a modelling choice rather than a hardware
//! fact. A socketed board is the arbiter.

const BANK_SIZE: usize = 0x1000;
const BANKS: usize = 8;

const LATCH_LOW: u16 = 0x1FF8;
const LATCH_HIGH: u16 = 0x1FF9;
const COMMIT: u16 = 0x1FFC;

pub struct Fc {
    image: Vec<u8>,
    bank: usize,
    /// The bank a commit would make live.
    target: usize,
}

impl Fc {
    pub fn new(rom: &[u8]) -> Fc {
        Fc {
            image: rom.to_vec(),
            bank: 0,
            target: 0,
        }
    }

    pub fn read(&mut self, address: u16) -> u8 {
        if address & 0x1FFF == COMMIT {
            self.bank = self.target % BANKS;
        }
        self.peek(address)
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        match address & 0x1FFF {
            LATCH_LOW => self.target = self.target & !0x03 | usize::from(data & 0x03),
            LATCH_HIGH => self.target = usize::from(data) << 2 | self.target & 0x03,
            _ => {}
        }
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
