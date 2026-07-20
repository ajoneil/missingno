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

// Mapper state (including RAM) lives inline; one Mbc exists per console.
#[allow(clippy::large_enum_variant)]
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
    /// The mapper's display name for the debugger.
    pub fn name(&self) -> &'static str {
        match self {
            Mbc::NoMbc(_) => "None",
            Mbc::Mbc1(_) => "MBC1",
            Mbc::Mbc2(_) => "MBC2",
            Mbc::Mbc3(_) => "MBC3",
            Mbc::Mbc5(_) => "MBC5",
            Mbc::Mbc6(_) => "MBC6",
            Mbc::Mbc7(_) => "MBC7",
            Mbc::Huc1(_) => "HuC1",
            Mbc::Huc3(_) => "HuC3",
            Mbc::DbzTrans(_) => "DbzTrans",
        }
    }

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

    /// The cartridge RAM size in bytes, all banks linearised — the length the
    /// debugger's bank-complete `sram` region spans. Zero when the mapper has
    /// no RAM.
    pub fn ram_len(&self) -> usize {
        match self {
            Mbc::NoMbc(m) => m.ram.map_or(0, |ram| ram.len()),
            Mbc::Mbc1(m) => m.ram.len(),
            Mbc::Mbc2(m) => m.ram.len(),
            Mbc::Mbc3(m) => m.ram.len() * 8 * 1024,
            Mbc::Mbc5(m) => m.ram.len() * 8 * 1024,
            Mbc::Mbc6(m) => m.ram.len() * 4 * 1024,
            Mbc::Mbc7(m) => m.eeprom.data.len() * 2,
            Mbc::Huc1(m) => m.ram.len() * 8 * 1024,
            Mbc::Huc3(m) => m.ram.len() * 8 * 1024,
            Mbc::DbzTrans(m) => m.ram.len() * 8 * 1024,
        }
    }

    /// A side-effect-free read of linearised cartridge RAM at `offset` — the raw
    /// backing store, bypassing the enable latch and bank selection, so the
    /// debugger sees every bank regardless of what the CPU has mapped. `0xFF`
    /// past the end.
    pub fn peek_ram(&self, offset: usize) -> u8 {
        match self {
            Mbc::NoMbc(m) => m
                .ram
                .as_ref()
                .and_then(|ram| ram.get(offset).copied())
                .unwrap_or(0xff),
            Mbc::Mbc1(m) => m.ram.peek(offset),
            Mbc::Mbc2(m) => m.ram.get(offset).copied().unwrap_or(0xff),
            Mbc::Mbc3(m) => peek_banked(&m.ram, offset),
            Mbc::Mbc5(m) => peek_banked(&m.ram, offset),
            Mbc::Mbc6(m) => peek_banked(&m.ram, offset),
            Mbc::Mbc7(m) => m
                .eeprom
                .data
                .get(offset / 2)
                .map_or(0xff, |word| word.to_be_bytes()[offset % 2]),
            Mbc::Huc1(m) => peek_banked(&m.ram, offset),
            Mbc::Huc3(m) => peek_banked(&m.ram, offset),
            Mbc::DbzTrans(m) => peek_banked(&m.ram, offset),
        }
    }
}

/// Read byte `offset` from a bank-major RAM store as one linear space.
fn peek_banked<const N: usize>(banks: &[[u8; N]], offset: usize) -> u8 {
    banks.get(offset / N).map_or(0xff, |bank| bank[offset % N])
}

/// Copy a saved span into a flat store, truncating to whichever is shorter.
pub(super) fn restore_flat(dst: &mut [u8], src: &[u8]) {
    let len = dst.len().min(src.len());
    dst[..len].copy_from_slice(&src[..len]);
}

/// Restore a bank-major RAM store from one linear span, filling banks in order —
/// the write counterpart to [`peek_banked`].
pub(super) fn restore_banked<const N: usize>(banks: &mut [[u8; N]], src: &[u8]) {
    for (i, bank) in banks.iter_mut().enumerate() {
        let start = i * N;
        if start >= src.len() {
            break;
        }
        let len = (src.len() - start).min(N);
        bank[..len].copy_from_slice(&src[start..start + len]);
    }
}

impl Mbc {
    /// Restore linearised cartridge RAM from a saved span across every bank,
    /// bypassing the enable latch and the bank selection — the write counterpart
    /// to [`peek_ram`](Self::peek_ram). It never routes through the mapper's
    /// banked `$A000` window (so it cannot overflow the 16-bit bus address or
    /// drop banks past the mapped one) and never sets the SRAM-dirty flag.
    pub fn restore_ram(&mut self, bytes: &[u8]) {
        match self {
            Mbc::NoMbc(m) => {
                if let Some(ram) = &mut m.ram {
                    restore_flat(ram, bytes);
                }
            }
            Mbc::Mbc1(m) => m.ram.restore(bytes),
            Mbc::Mbc2(m) => restore_flat(&mut m.ram, bytes),
            Mbc::Mbc3(m) => restore_banked(&mut m.ram, bytes),
            Mbc::Mbc5(m) => restore_banked(&mut m.ram, bytes),
            Mbc::Mbc6(m) => restore_banked(&mut m.ram, bytes),
            Mbc::Mbc7(m) => m.eeprom.restore(bytes),
            Mbc::Huc1(m) => restore_banked(&mut m.ram, bytes),
            Mbc::Huc3(m) => restore_banked(&mut m.ram, bytes),
            Mbc::DbzTrans(m) => restore_banked(&mut m.ram, bytes),
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
