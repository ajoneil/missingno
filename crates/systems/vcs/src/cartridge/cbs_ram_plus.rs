//! The FA board (CBS RAM Plus): three 4 KB banks and 256 bytes of cart RAM,
//! with a bankswitch the data bus can veto.
//!
//! $1FF8/$1FF9/$1FFA select bank 0/1/2, but unlike the Atari boards the decoder
//! only switches when data-bus bit D0 is high during the hotspot access (US
//! Patent 4,485,457). A write strobe gates on the CPU's value; a read strobe
//! gates on the byte the cart itself drives at that address, so the image
//! decides whether its own hotspots switch.
//!
//! The RAM splits into a write port at $F000-$F0FF and a read port at
//! $F100-$F1FF — the cart edge has no R/W line — and sits outside the banked
//! ROM, shadowing the image beneath it.

const HOTSPOT_BASE: u16 = 0x1FF8;
const BANKS: usize = 3;
const BANK_SIZE: usize = 0x1000;
pub const RAM_SIZE: usize = 0x100;

pub struct CbsRamPlus {
    image: Vec<u8>,
    bank: usize,
    ram: Box<[u8; RAM_SIZE]>,
}

impl CbsRamPlus {
    pub fn new(rom: &[u8]) -> CbsRamPlus {
        CbsRamPlus {
            image: rom.to_vec(),
            bank: 0,
            ram: Box::new([0; RAM_SIZE]),
        }
    }

    /// The switch only fires when the byte on the data bus has D0 set.
    fn hotspot(&mut self, address: u16, bus: u8) {
        let decoded = address & 0x1FFF;
        let offset = decoded.wrapping_sub(HOTSPOT_BASE) as usize;
        if decoded >= HOTSPOT_BASE && offset < BANKS && bus & 0x01 != 0 {
            self.bank = offset;
        }
    }

    /// The RAM cell an address touches, and whether it is the write port.
    fn ram_port(address: u16) -> Option<(usize, bool)> {
        let offset = (address & 0x0FFF) as usize;
        (offset < 2 * RAM_SIZE).then_some((offset % RAM_SIZE, offset < RAM_SIZE))
    }

    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        if let Some((cell, write_port)) = CbsRamPlus::ram_port(address) {
            // No R/W line: a write-port read still stores, latching the
            // floating bus byte the CPU also sees.
            if write_port {
                self.ram[cell] = bus;
                return bus;
            }
            return self.ram[cell];
        }
        // The cart drives the byte first; its D0 is what gates the switch.
        let driven = self.peek(address);
        self.hotspot(address, driven);
        driven
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        self.hotspot(address, data);
        if let Some((cell, true)) = CbsRamPlus::ram_port(address) {
            self.ram[cell] = data;
        }
    }

    /// The 256-byte cart RAM, all of it.
    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        self.ram.as_mut_slice()
    }

    pub(super) fn ram(&self) -> &[u8] {
        self.ram.as_slice()
    }

    /// The selected bank, for a state save.
    pub(super) fn bank_state(&self) -> Vec<u8> {
        vec![self.bank as u8]
    }

    pub(super) fn restore_bank_state(&mut self, bytes: &[u8]) {
        if let Some(&bank) = bytes.first() {
            self.bank = (bank as usize).min(BANKS - 1);
        }
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    pub fn peek(&self, address: u16) -> u8 {
        match CbsRamPlus::ram_port(address) {
            Some((cell, _)) => self.ram[cell],
            None => self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize],
        }
    }
}
