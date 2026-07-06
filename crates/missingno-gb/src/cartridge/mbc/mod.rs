pub mod dbz_trans;
pub mod huc1;
pub mod huc3;
pub mod mbc1;
pub mod mbc2;
pub mod mbc3;
pub mod mbc5;
pub mod mbc6;
pub mod mbc7;
pub mod no_mbc;

pub enum Mbc {
    NoMbc(no_mbc::NoMbc),
    Mbc1(mbc1::Mbc1),
    Mbc2(mbc2::Mbc2),
    Mbc3(mbc3::Mbc3),
    Mbc5(mbc5::Mbc5),
    Mbc6(mbc6::Mbc6),
    Mbc7(mbc7::Mbc7),
    Huc1(huc1::Huc1),
    Huc3(huc3::Huc3),
    DbzTrans(dbz_trans::DbzTrans),
}

impl Mbc {
    pub fn ram(&self) -> Option<Vec<u8>> {
        match self {
            Mbc::NoMbc(m) => m.ram(),
            Mbc::Mbc1(m) => m.ram(),
            Mbc::Mbc2(m) => m.ram(),
            Mbc::Mbc3(m) => m.ram(),
            Mbc::Mbc5(m) => m.ram(),
            Mbc::Mbc6(m) => m.ram(),
            Mbc::Mbc7(m) => m.ram(),
            Mbc::Huc1(m) => m.ram(),
            Mbc::Huc3(m) => m.ram(),
            Mbc::DbzTrans(m) => m.ram(),
        }
    }

    /// The 16KB ROM bank mapped at 0x4000–0x7FFF, as debug-symbol files
    /// count banks; `None` where the mapper switches 8KB half-windows
    /// instead (MBC6, the DBZ multicart).
    pub fn switchable_rom_bank(&self, rom_len: usize) -> Option<u16> {
        match self {
            Mbc::NoMbc(_) => Some(1),
            Mbc::Mbc1(m) => Some(m.switchable_rom_bank(rom_len)),
            Mbc::Mbc2(m) => Some(m.switchable_rom_bank(rom_len)),
            Mbc::Mbc3(m) => Some(m.switchable_rom_bank()),
            Mbc::Mbc5(m) => Some(m.switchable_rom_bank()),
            Mbc::Mbc6(_) | Mbc::DbzTrans(_) => None,
            Mbc::Mbc7(m) => Some(m.switchable_rom_bank()),
            Mbc::Huc1(m) => Some(m.switchable_rom_bank()),
            Mbc::Huc3(m) => Some(m.switchable_rom_bank()),
        }
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match self {
            Mbc::NoMbc(m) => m.read(rom, address),
            Mbc::Mbc1(m) => m.read(rom, address),
            Mbc::Mbc2(m) => m.read(rom, address),
            Mbc::Mbc3(m) => m.read(rom, address),
            Mbc::Mbc5(m) => m.read(rom, address),
            Mbc::Mbc6(m) => m.read(rom, address),
            Mbc::Mbc7(m) => m.read(rom, address),
            Mbc::Huc1(m) => m.read(rom, address),
            Mbc::Huc3(m) => m.read(rom, address),
            Mbc::DbzTrans(m) => m.read(rom, address),
        }
    }

    /// Write to cartridge address space. Returns true if SRAM was written.
    pub fn write(&mut self, address: u16, value: u8) -> bool {
        match self {
            Mbc::NoMbc(m) => m.write(address, value),
            Mbc::Mbc1(m) => m.write(address, value),
            Mbc::Mbc2(m) => m.write(address, value),
            Mbc::Mbc3(m) => m.write(address, value),
            Mbc::Mbc5(m) => m.write(address, value),
            Mbc::Mbc6(m) => m.write(address, value),
            Mbc::Mbc7(m) => m.write(address, value),
            Mbc::Huc1(m) => m.write(address, value),
            Mbc::Huc3(m) => m.write(address, value),
            Mbc::DbzTrans(m) => m.write(address, value),
        }
    }

    /// Advance any real-time clock by `dots` of master-clock time. Only MBC3's
    /// RTC counts; all other cartridge types ignore it.
    pub fn tick_rtc(&mut self, dots: u32) {
        if let Mbc::Mbc3(m) = self {
            m.tick_rtc(dots);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM where every byte of a bank holds the bank's low index byte, so a
    /// read through the mapper reveals which bank is mapped.
    fn bank_stamped_rom(banks: usize) -> Vec<u8> {
        (0..banks).flat_map(|bank| [bank as u8; 0x4000]).collect()
    }

    /// The accessor must agree with what the read path actually maps.
    fn assert_bank_matches_read(mbc: &Mbc, rom: &[u8]) {
        let bank = mbc
            .switchable_rom_bank(rom.len())
            .expect("mapper reports a 16KB bank");
        assert_eq!(mbc.read(rom, 0x4000), bank as u8);
        assert_eq!(mbc.read(rom, 0x7fff), bank as u8);
    }

    #[test]
    fn mbc1_bank_tracks_register_and_zero_maps_to_one() {
        let rom = bank_stamped_rom(32);
        let mut mbc = Mbc::Mbc1(mbc1::Mbc1::new(&rom, None));
        assert_bank_matches_read(&mbc, &rom);
        mbc.write(0x2000, 0x12);
        assert_eq!(mbc.switchable_rom_bank(rom.len()), Some(0x12));
        assert_bank_matches_read(&mbc, &rom);
        mbc.write(0x2000, 0);
        assert_eq!(mbc.switchable_rom_bank(rom.len()), Some(1));
        assert_bank_matches_read(&mbc, &rom);
    }

    #[test]
    fn mbc1_large_rom_upper_bits_apply() {
        let rom = bank_stamped_rom(128);
        let mut mbc = Mbc::Mbc1(mbc1::Mbc1::new(&rom, None));
        mbc.write(0x2000, 0x03);
        mbc.write(0x4000, 0x01);
        assert_bank_matches_read(&mbc, &rom);
    }

    #[test]
    fn mbc2_bank_matches_read() {
        let rom = bank_stamped_rom(16);
        let mut mbc = Mbc::Mbc2(mbc2::Mbc2::new(&rom, None));
        mbc.write(0x2100, 5);
        assert_eq!(mbc.switchable_rom_bank(rom.len()), Some(5));
        assert_bank_matches_read(&mbc, &rom);
    }

    #[test]
    fn mbc3_bank_matches_read() {
        let rom = bank_stamped_rom(64);
        let mut mbc = Mbc::Mbc3(mbc3::Mbc3::new(&rom, None));
        mbc.write(0x2000, 0x21);
        assert_eq!(mbc.switchable_rom_bank(rom.len()), Some(0x21));
        assert_bank_matches_read(&mbc, &rom);
        mbc.write(0x2000, 0);
        assert_eq!(mbc.switchable_rom_bank(rom.len()), Some(1));
    }

    #[test]
    fn mbc5_maps_bank_zero_literally() {
        let rom = bank_stamped_rom(64);
        let mut mbc = Mbc::Mbc5(mbc5::Mbc5::new(&rom, None));
        mbc.write(0x2000, 0);
        assert_eq!(mbc.switchable_rom_bank(rom.len()), Some(0));
        assert_bank_matches_read(&mbc, &rom);
        mbc.write(0x2000, 0x2a);
        assert_bank_matches_read(&mbc, &rom);
    }

    #[test]
    fn huc1_and_mbc7_map_zero_to_one() {
        let rom = bank_stamped_rom(32);
        let mut huc1 = Mbc::Huc1(huc1::Huc1::new(&rom, None));
        huc1.write(0x2000, 0);
        assert_eq!(huc1.switchable_rom_bank(rom.len()), Some(1));
        assert_bank_matches_read(&huc1, &rom);
        let mut mbc7 = Mbc::Mbc7(mbc7::Mbc7::new(&rom, None));
        mbc7.write(0x2000, 7);
        assert_bank_matches_read(&mbc7, &rom);
    }

    #[test]
    fn half_window_mappers_report_no_single_bank() {
        let rom = bank_stamped_rom(32);
        let mbc6 = Mbc::Mbc6(mbc6::Mbc6::new(&rom, None));
        assert_eq!(mbc6.switchable_rom_bank(rom.len()), None);
        let dbz = Mbc::DbzTrans(dbz_trans::DbzTrans::new(&rom, None));
        assert_eq!(dbz.switchable_rom_bank(rom.len()), None);
    }
}
