//! The Atari hotspot boards (F8/F6/F4), optionally with Superchip cart RAM.
//!
//! A run of fixed addresses at the top of the window are hotspots: any bus
//! access landing on one selects its 4 KB bank, and the data bus is irrelevant.
//! The cart decodes 13 lines, so the CPU reaches them through their $FFFx
//! mirror. Bank count is what separates the boards; the mechanism does not.
//!
//! Superchip (SARA) adds 128 bytes of static RAM under the bottom of the
//! window. The cart edge has no read/write strobe, so the RAM splits into two
//! ports: the low half is the write port (an access latches the data bus), the
//! high half is the read port. The RAM sits outside the banked ROM, so a bank
//! switch never disturbs it, and it shadows the ROM bytes under it.

pub const F8_HOTSPOT_BASE: u16 = 0x1FF8;
pub const F6_HOTSPOT_BASE: u16 = 0x1FF6;
pub const F4_HOTSPOT_BASE: u16 = 0x1FF4;

pub const SUPERCHIP_RAM_SIZE: usize = 0x80;

const BANK_SIZE: usize = 0x1000;

pub struct Atari {
    image: Vec<u8>,
    bank: usize,
    banks: usize,
    hotspot_base: u16,
    ram: Option<Box<[u8; SUPERCHIP_RAM_SIZE]>>,
}

impl Atari {
    pub fn new(rom: &[u8], hotspot_base: u16, superchip: bool) -> Atari {
        Atari {
            image: rom.to_vec(),
            bank: 0,
            banks: rom.len() / BANK_SIZE,
            hotspot_base,
            ram: superchip.then(|| Box::new([0; SUPERCHIP_RAM_SIZE])),
        }
    }

    /// The board decodes 13 lines; one hotspot per bank, counting up from the
    /// board's base address.
    fn hotspot(&mut self, address: u16) {
        let decoded = address & 0x1FFF;
        let offset = decoded.wrapping_sub(self.hotspot_base) as usize;
        if decoded >= self.hotspot_base && offset < self.banks {
            self.bank = offset;
        }
    }

    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        self.hotspot(address);
        if let Some(ram) = &mut self.ram {
            let offset = (address & 0x0FFF) as usize;
            // The cart slot has no R/W line: a write-port read still stores,
            // latching the floating bus byte the CPU also sees.
            if offset < SUPERCHIP_RAM_SIZE {
                ram[offset] = bus;
                return bus;
            }
        }
        self.peek(address)
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        self.hotspot(address);
        if let Some(ram) = &mut self.ram {
            let offset = (address & 0x0FFF) as usize;
            if offset < SUPERCHIP_RAM_SIZE {
                ram[offset] = data;
            }
        }
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        if let Some(ram) = &self.ram
            && offset < 2 * SUPERCHIP_RAM_SIZE
        {
            return ram[offset % SUPERCHIP_RAM_SIZE];
        }
        self.image[self.bank * BANK_SIZE + offset]
    }
}
