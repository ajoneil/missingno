pub mod cart_type;
pub mod mbc;

pub use cart_type::{GbCartType, GbCartridgeError};

use mbc::mbc3::{ClockRegisters, Mapped, Mbc3Chip};
use mbc::{
    Mbc, dbz_trans::DbzTrans, huc1::Huc1, huc3::Huc3, mbc1::Mbc1, mbc2::Mbc2, mbc3::Mbc3,
    mbc5::Mbc5, mbc6::Mbc6, mbc7::Mbc7, no_mbc::NoMbc,
};

/// A read-only view of the cartridge's mapper and clock state, for the
/// debugger's Cartridge sidebar section. A plain data copy the running snapshot
/// takes cheaply and the paused console builds live.
#[derive(Clone, Debug)]
pub struct CartridgeView {
    pub mapper: &'static str,
    /// The 16 KB bank at $4000; `None` for a half-window mapper.
    pub rom_bank: Option<u16>,
    /// The selected RAM bank, where the mapper banks RAM.
    pub ram_bank: Option<u8>,
    /// The RAM/clock enable latch, where the mapper has one.
    pub ram_enabled: Option<bool>,
    /// The MBC1 banking mode; `None` for every other mapper.
    pub mode1: Option<bool>,
    /// The real-time clock, on MBC3 carts that carry one.
    pub rtc: Option<RtcView>,
}

/// A read-only view of the MBC3 real-time clock's live register state.
#[derive(Clone, Debug)]
pub struct RtcView {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub day: u16,
    /// RTCDH bit 6 — the clock is halted.
    pub halted: bool,
    /// A $6000 latch is armed, awaiting its completing write.
    pub latch_ready: bool,
    /// RTCDH bit 7 — the sticky day-counter overflow.
    pub day_carry: bool,
}

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
    let has_battery = GbCartType::from_header(rom[0x147]).is_ok_and(GbCartType::has_battery);
    (title, sgb_flag, has_battery)
}

/// The unlicensed "GB DBZ GOKOU 2" cart declares MBC5 but wires the DbzTrans
/// half-bank switch, and only its title gives it away.
fn is_dbz_trans(rom: &[u8], title: &str) -> bool {
    title == "GB DBZ GOKOU 2" && rom[0x148] == 0x05
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

    /// The RAM bank currently mapped at 0xA000–0xBFFF, where the mapper banks
    /// RAM; `None` for a mapper with no banked RAM.
    pub fn ram_bank(&self) -> Option<u8> {
        match &self.mbc {
            Mbc::Mbc1(m) => Some(m.ram_bank),
            Mbc::Mbc3(m) => match m.mapped {
                Mapped::Ram(bank) => Some(bank),
                Mapped::Clock(_) => None,
            },
            Mbc::Mbc5(m) => Some(m.ram_bank),
            Mbc::Mbc6(m) => Some(m.ram_bank_a),
            Mbc::Huc1(m) => Some(m.ram_bank),
            Mbc::Huc3(m) => Some(m.ram_bank),
            Mbc::DbzTrans(m) => Some(m.ram_bank),
            Mbc::NoMbc(_) | Mbc::Mbc2(_) | Mbc::Mbc7(_) => None,
        }
    }

    /// The RAM bank a `$A000` access currently targets, for matching a synthetic
    /// SRAM row's bank watch. A single-RAM-bank mapper (NoMbc/MBC2/MBC7) has no
    /// bank register but always targets bank 0, so a bank-0 watch should match;
    /// MBC3 in clock mode maps no RAM, so `None` — and a bank watch correctly
    /// does not match there.
    pub fn mapped_ram_bank(&self) -> Option<u8> {
        match &self.mbc {
            Mbc::NoMbc(_) | Mbc::Mbc2(_) | Mbc::Mbc7(_) => (self.ram_len() > 0).then_some(0),
            _ => self.ram_bank(),
        }
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

    /// The cartridge a ROM image is built into. The header names the board;
    /// a caller who knows the header lies may state one instead, which also
    /// stands in for the boards no header byte names.
    pub fn new(
        rom: Vec<u8>,
        stated: Option<GbCartType>,
        save_data: Option<Vec<u8>>,
    ) -> Result<Cartridge, GbCartridgeError> {
        let title = parse_title(&rom);
        let sgb_flag = rom[0x146] == 0x03;
        let cart_type = match stated {
            Some(stated) => stated,
            None if is_dbz_trans(&rom, &title) => GbCartType::DbzTrans,
            None => GbCartType::from_header(rom[0x147]).map_err(GbCartridgeError::UnknownMapper)?,
        };

        let has_battery = cart_type.has_battery();
        let save = if has_battery { save_data } else { None };

        let mbc = match cart_type {
            GbCartType::Rom | GbCartType::RomRam | GbCartType::RomRamBattery => {
                Mbc::NoMbc(NoMbc::new(&rom, save))
            }
            GbCartType::Mbc1 | GbCartType::Mbc1Ram | GbCartType::Mbc1RamBattery => {
                // Only a header-named MBC1 is probed for the multicart logo: a
                // stated board has already settled the question.
                let multicart = stated.is_none() && mbc::mbc1::detect_multicart(&rom);
                Mbc::Mbc1(Mbc1::new(&rom, save, multicart))
            }
            GbCartType::Mbc1Multicart => Mbc::Mbc1(Mbc1::new(&rom, save, true)),
            GbCartType::Mbc2 | GbCartType::Mbc2Battery => Mbc::Mbc2(Mbc2::new(&rom, save)),
            GbCartType::Mbc3TimerBattery
            | GbCartType::Mbc3TimerRamBattery
            | GbCartType::Mbc3
            | GbCartType::Mbc3Ram
            | GbCartType::Mbc3RamBattery => Mbc::Mbc3(Mbc3::new(
                &rom,
                save,
                Mbc3Chip::for_rom(&rom),
                cart_type.has_timer(),
            )),
            // MBC30 names the chip, not the whole board, so the header still
            // says whether the clock is populated beside it.
            GbCartType::Mbc30 => {
                let timer = GbCartType::from_header(rom[0x147]).is_ok_and(GbCartType::has_timer);
                Mbc::Mbc3(Mbc3::new(&rom, save, Mbc3Chip::Mbc30, timer))
            }
            GbCartType::Mbc5 | GbCartType::Mbc5Ram | GbCartType::Mbc5RamBattery => {
                Mbc::Mbc5(Mbc5::new(&rom, save))
            }
            GbCartType::Mbc5Rumble
            | GbCartType::Mbc5RumbleRam
            | GbCartType::Mbc5RumbleRamBattery => Mbc::Mbc5(Mbc5::new_rumble(&rom, save)),
            GbCartType::Mbc6 => Mbc::Mbc6(Mbc6::new(&rom, save)),
            GbCartType::Mbc7 => Mbc::Mbc7(Mbc7::new(&rom, save)),
            GbCartType::Huc3 => Mbc::Huc3(Huc3::new(&rom, save)),
            GbCartType::Huc1 => Mbc::Huc1(Huc1::new(&rom, save)),
            GbCartType::DbzTrans => Mbc::DbzTrans(DbzTrans::new(&rom, save)),
        };

        Ok(Cartridge {
            title,
            has_battery,
            sgb_flag,
            sram_dirty: false,
            rom,
            mbc,
        })
    }

    /// A read-only view of the mapper and clock state for the debugger.
    pub fn inspect(&self) -> CartridgeView {
        let (ram_enabled, ram_bank, mode1) = match &self.mbc {
            Mbc::NoMbc(_) => (None, None, None),
            Mbc::Mbc1(m) => (Some(m.ram_enabled), Some(m.ram_bank), Some(m.mode1)),
            Mbc::Mbc2(m) => (Some(m.ram_enabled), None, None),
            Mbc::Mbc3(m) => {
                let ram_bank = match m.mapped {
                    Mapped::Ram(bank) => Some(bank),
                    Mapped::Clock(_) => None,
                };
                (Some(m.ram_and_clock_enabled), ram_bank, None)
            }
            Mbc::Mbc5(m) => (Some(m.ram_enabled), Some(m.ram_bank), None),
            Mbc::Mbc6(m) => (Some(m.ram_enabled), Some(m.ram_bank_a), None),
            Mbc::Mbc7(m) => (Some(m.ram_enabled_1 && m.ram_enabled_2), None, None),
            Mbc::Huc1(m) => (None, Some(m.ram_bank), None),
            Mbc::Huc3(m) => (None, Some(m.ram_bank), None),
            Mbc::DbzTrans(m) => (Some(m.ram_enabled), Some(m.ram_bank), None),
        };
        CartridgeView {
            mapper: self.mbc.name(),
            rom_bank: self.switchable_rom_bank(),
            ram_bank,
            ram_enabled,
            mode1,
            rtc: self.rtc_view(),
        }
    }

    /// The MBC3 clock's live registers as a debugger view, if the cart has one.
    fn rtc_view(&self) -> Option<RtcView> {
        let Mbc::Mbc3(mbc) = &self.mbc else {
            return None;
        };
        let clock = mbc.clock.as_ref()?;
        let r = clock.registers;
        Some(RtcView {
            seconds: r.seconds,
            minutes: r.minutes,
            hours: r.hours,
            day: ((r.days_upper as u16 & 1) << 8) | r.days_lower as u16,
            halted: r.days_upper & 0x40 != 0,
            latch_ready: clock.latch_ready,
            day_carry: r.days_upper & 0x80 != 0,
        })
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

    /// $C0 in the CGB flag: the game runs on no Game Boy but a Color.
    pub fn requires_cgb(&self) -> bool {
        Cartridge::peek_cgb_only(&self.rom)
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        self.mbc.ram()
    }

    /// The cartridge RAM size in bytes, all banks linearised; zero with no RAM.
    pub fn ram_len(&self) -> usize {
        self.mbc.ram_len()
    }

    /// A side-effect-free read of linearised cartridge RAM — the raw backing
    /// store across every bank, past the enable latch and bank selection.
    pub fn peek_ram(&self, offset: usize) -> u8 {
        self.mbc.peek_ram(offset)
    }

    /// Restore linearised cartridge RAM across every bank from a saved span,
    /// past the enable latch and bank selection — the write counterpart to
    /// [`peek_ram`](Self::peek_ram). Independent of the mapper latch state, so a
    /// state restore can call it before or after reseating the mapper, and it
    /// leaves the SRAM-dirty flag untouched.
    pub fn restore_ram(&mut self, bytes: &[u8]) {
        self.mbc.restore_ram(bytes);
    }

    /// A side-effect-free read of the full ROM image at a linear `offset`,
    /// independent of the current bank; `0xFF` past the end.
    pub fn peek_rom(&self, offset: usize) -> u8 {
        self.rom.get(offset).copied().unwrap_or(0xff)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_declaring(cartridge_type: u8, ram_size: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = cartridge_type;
        rom[0x149] = ram_size;
        rom
    }

    #[test]
    fn a_cartridge_type_naming_no_board_is_refused() {
        let Err(error) = Cartridge::new(rom_declaring(0xdd, 0), None, None) else {
            panic!("a cartridge type naming no board was accepted");
        };
        assert_eq!(error, GbCartridgeError::UnknownMapper(0xdd));
        assert!(error.to_string().contains("$dd"));
    }

    #[test]
    fn a_stated_board_outranks_the_declared_one() {
        let rom = rom_declaring(0x00, 3);
        let cartridge = Cartridge::new(rom, Some(GbCartType::Mbc5RamBattery), None).unwrap();
        assert_eq!(cartridge.mbc().name(), "MBC5");
        assert!(cartridge.has_battery());
    }

    #[test]
    fn a_stated_board_stands_in_for_a_type_that_names_none() {
        let rom = rom_declaring(0x99, 0);
        let cartridge = Cartridge::new(rom, Some(GbCartType::DbzTrans), None).unwrap();
        assert_eq!(cartridge.mbc().name(), "DbzTrans");
    }

    #[test]
    fn a_stated_mbc1_settles_the_multicart_question_either_way() {
        let mut multicart_rom = vec![0u8; 1024 * 1024];
        multicart_rom[0x147] = 0x01;
        let logo = 0x10 * 0x4000 + 0x104;
        multicart_rom[logo..logo + 48].copy_from_slice(&NINTENDO_LOGO);

        let probed = Cartridge::new(multicart_rom.clone(), None, None).unwrap();
        let Mbc::Mbc1(mbc1) = probed.mbc() else {
            panic!("MBC1 cart built a different mapper");
        };
        assert!(mbc1.multicart);

        let stated = Cartridge::new(multicart_rom, Some(GbCartType::Mbc1), None).unwrap();
        let Mbc::Mbc1(mbc1) = stated.mbc() else {
            panic!("MBC1 cart built a different mapper");
        };
        assert!(!mbc1.multicart);

        let stated = Cartridge::new(
            rom_declaring(0x01, 0),
            Some(GbCartType::Mbc1Multicart),
            None,
        )
        .unwrap();
        let Mbc::Mbc1(mbc1) = stated.mbc() else {
            panic!("MBC1 cart built a different mapper");
        };
        assert!(mbc1.multicart);
    }

    #[test]
    fn a_stated_mbc30_keeps_the_wide_bank_register_on_a_small_rom() {
        let cartridge =
            Cartridge::new(rom_declaring(0x10, 3), Some(GbCartType::Mbc30), None).unwrap();
        let Mbc::Mbc3(mbc3) = cartridge.mbc() else {
            panic!("MBC30 cart built a different mapper");
        };
        assert!(matches!(mbc3.chip, Mbc3Chip::Mbc30));
        assert!(mbc3.clock.is_some());
    }

    #[test]
    fn a_save_is_restored_only_where_the_battery_holds_the_ram() {
        let save = vec![0xa5; 8 * 1024];
        let backed = Cartridge::new(rom_declaring(0x13, 2), None, Some(save.clone())).unwrap();
        assert_eq!(backed.peek_ram(0), 0xa5);

        let unbacked = Cartridge::new(rom_declaring(0x12, 2), None, Some(save)).unwrap();
        assert_eq!(unbacked.peek_ram(0), 0x00);
    }
}
