pub mod mbc;

use mbc::mbc3::ClockRegisters;
use mbc::{
    Mbc, dbz_trans::DbzTrans, huc1::Huc1, huc3::Huc3, mbc1::Mbc1, mbc2::Mbc2, mbc3::Mbc3,
    mbc5::Mbc5, mbc6::Mbc6, mbc7::Mbc7, no_mbc::NoMbc,
};

/// Real-time-clock state as saved alongside SRAM.
#[derive(Clone, Copy)]
pub struct RtcSnapshot {
    pub registers: ClockRegisters,
    pub latched: ClockRegisters,
}

pub struct Cartridge {
    title: String,
    has_battery: bool,
    sgb_flag: bool,
    rom: Vec<u8>,
    mbc: Mbc,
    pub(crate) sram_dirty: bool,
}

/// The boot logo at $0104; the boot ROM refuses to unmap without it.
pub(crate) const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

pub fn parse_title(rom: &[u8]) -> String {
    if rom.len() < 0x144 {
        return String::new();
    }

    let mut title = String::new();
    for character in rom[0x134..0x144].iter() {
        if *character == 0u8 {
            break;
        }
        title.push(*character as char)
    }
    title
}

pub fn parse_header(rom: &[u8]) -> (String, bool, bool) {
    let title = parse_title(rom);
    let sgb_flag = rom[0x146] == 0x03;
    let cartridge_type = rom[0x147];
    let has_battery = matches!(
        cartridge_type,
        0x03 | 0x06 | 0x09 | 0x10 | 0x13 | 0x1b | 0x1e | 0x22 | 0xfe | 0xff
    );
    (title, sgb_flag, has_battery)
}

impl Cartridge {
    /// The 16KB ROM bank currently mapped at 0x4000–0x7FFF; `None` for
    /// half-window mappers.
    pub fn switchable_rom_bank(&self) -> Option<u16> {
        self.mbc.switchable_rom_bank(self.rom.len())
    }

    pub fn rom_len(&self) -> usize {
        self.rom.len()
    }

    /// The real-time clock's register state, on carts that have one.
    pub fn rtc(&self) -> Option<RtcSnapshot> {
        match &self.mbc {
            Mbc::Mbc3(m) => m.clock.as_ref().map(|clock| RtcSnapshot {
                registers: clock.registers,
                latched: clock.latched,
            }),
            _ => None,
        }
    }

    /// Restore a saved clock and advance it by the wall-clock seconds that
    /// passed since the save was written.
    pub fn restore_rtc(&mut self, snapshot: RtcSnapshot, elapsed_seconds: u64) {
        if let Mbc::Mbc3(m) = &mut self.mbc
            && let Some(clock) = &mut m.clock
        {
            clock.registers = snapshot.registers;
            clock.latched = snapshot.latched;
            clock.advance_seconds(elapsed_seconds);
        }
    }

    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Cartridge {
        let (title, sgb_flag, has_battery) = parse_header(&rom);
        let cartridge_type = rom[0x147];
        let save = if has_battery { save_data } else { None };

        // The unlicensed "GB DBZ GOKOU 2" cart declares MBC5 but uses the
        // DbzTrans half-bank-switch mapper.
        let mbc = if title == "GB DBZ GOKOU 2" && rom[0x148] == 0x05 {
            Mbc::DbzTrans(DbzTrans::new(&rom, save))
        } else {
            match cartridge_type {
                0x00 | 0x08 | 0x09 => Mbc::NoMbc(NoMbc::new(&rom, save)),
                0x01..=0x03 => Mbc::Mbc1(Mbc1::new(&rom, save)),
                0x05 | 0x06 => Mbc::Mbc2(Mbc2::new(&rom, save)),
                0x0f..=0x13 => Mbc::Mbc3(Mbc3::new(&rom, save)),
                0x19..=0x1b => Mbc::Mbc5(Mbc5::new(&rom, save)),
                0x1c..=0x1e => Mbc::Mbc5(Mbc5::new_rumble(&rom, save)),
                0x20 => Mbc::Mbc6(Mbc6::new(&rom, save)),
                0x22 => Mbc::Mbc7(Mbc7::new(&rom, save)),
                0xfe => Mbc::Huc3(Huc3::new(&rom, save)),
                0xff => Mbc::Huc1(Huc1::new(&rom, save)),

                _ => panic!("nyi: mbc {:2x}", cartridge_type),
            }
        };

        Cartridge {
            title,
            has_battery,
            sgb_flag,
            sram_dirty: false,
            rom,
            mbc,
        }
    }

    pub fn peek_title(rom: &[u8]) -> String {
        parse_title(rom)
    }

    /// Whether `rom` carries the boot logo (the CGB boot ROM checks only its
    /// first half, so that is the validity bar).
    pub fn peek_valid_header(rom: &[u8]) -> bool {
        rom.len() >= 0x150 && rom[0x104..0x11C] == NINTENDO_LOGO[..24]
    }

    /// CGB flag (header $0143): $C0 marks media that requires the CGB, as
    /// opposed to $80's dual-mode enhancement.
    pub fn peek_cgb_only(rom: &[u8]) -> bool {
        rom.get(0x143).is_some_and(|flag| flag & 0xC0 == 0xC0)
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn has_battery(&self) -> bool {
        self.has_battery
    }

    pub fn supports_sgb(&self) -> bool {
        self.sgb_flag
    }

    /// CGB flag (header $0143): bit 7 set ($80 enhanced, $C0 CGB-only) marks a
    /// CGB-aware cartridge. Any other value is a DMG cartridge, which the CGB
    /// runs in DMG-compatibility mode.
    pub fn is_cgb(&self) -> bool {
        self.rom[0x143] & 0x80 != 0
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        self.mbc.ram()
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn header_checksum(&self) -> u8 {
        self.rom[0x14d]
    }

    pub fn read(&self, address: u16) -> u8 {
        self.mbc.read(&self.rom, address)
    }

    pub fn write(&mut self, address: u16, value: u8) {
        let ram_written = self.mbc.write(address, value);
        if self.has_battery && ram_written {
            self.sram_dirty = true;
        }
    }

    /// Advance the cartridge RTC (if any) by `dots` of master-clock time.
    pub fn tick_rtc(&mut self, dots: u32) {
        self.mbc.tick_rtc(dots);
    }

    /// Returns true if SRAM has been written to since the last call.
    pub fn take_sram_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.sram_dirty, false)
    }

    pub fn mbc(&self) -> &Mbc {
        &self.mbc
    }

    pub fn mbc_mut(&mut self) -> &mut Mbc {
        &mut self.mbc
    }
}
