pub enum Mapped {
    Ram(u8),
    Clock(ClockRegister),
}

#[derive(Clone, Copy)]
pub enum Mbc3Chip {
    Mbc3,
    Mbc30,
}

impl Mbc3Chip {
    fn rom_bank_mask(self) -> u8 {
        match self {
            Mbc3Chip::Mbc3 => 0x7f,
            Mbc3Chip::Mbc30 => 0xff,
        }
    }
}

#[derive(Clone, Copy)]
pub enum ClockRegister {
    Seconds,
    Minutes,
    Hours,
    DayLower,
    DayUpper,
}

#[derive(Clone, Copy, Default)]
pub struct ClockRegisters {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub days_lower: u8,
    pub days_upper: u8,
}

/// RTCDH bit 6 halts the RTC; bit 0 is day bit 8; bit 7 is the sticky day-overflow flag.
const HALT_BIT: u8 = 0x40;
/// Base master-clock dots per RTC second: the 32768 Hz crystal × 128 = the 2^22-dot base clock.
const DOTS_PER_RTC_SECOND: u32 = 1 << 22;

impl ClockRegisters {
    /// Carry chain: each stage carries into the next ONLY on its true boundary
    /// value, so a written out-of-range value (e.g. seconds 63) wraps to 0
    /// without a carry.
    fn increment_second(&mut self) {
        if self.seconds == 59 {
            self.seconds = 0;
            self.increment_minute();
        } else {
            self.seconds = (self.seconds + 1) & 0x3f;
        }
    }

    fn increment_minute(&mut self) {
        if self.minutes == 59 {
            self.minutes = 0;
            self.increment_hour();
        } else {
            self.minutes = (self.minutes + 1) & 0x3f;
        }
    }

    fn increment_hour(&mut self) {
        if self.hours == 23 {
            self.hours = 0;
            self.increment_day();
        } else {
            self.hours = (self.hours + 1) & 0x1f;
        }
    }

    fn increment_day(&mut self) {
        let day = (((self.days_upper & 1) as u16) << 8) | self.days_lower as u16;
        if day == 0x1ff {
            self.days_lower = 0;
            // Clear day bit 8, set the sticky overflow flag, keep the halt bit.
            self.days_upper = (self.days_upper & 0xc0) | 0x80;
        } else {
            let day = day + 1;
            self.days_lower = day as u8;
            self.days_upper = (self.days_upper & 0xc0) | ((day >> 8) as u8 & 1);
        }
    }

    fn get(&self, register: ClockRegister) -> u8 {
        match register {
            ClockRegister::Seconds => self.seconds,
            ClockRegister::Minutes => self.minutes,
            ClockRegister::Hours => self.hours,
            ClockRegister::DayLower => self.days_lower,
            ClockRegister::DayUpper => self.days_upper,
        }
    }

    fn set(&mut self, register: ClockRegister, value: u8) {
        match register {
            ClockRegister::Seconds => self.seconds = value & 0x3f,
            ClockRegister::Minutes => self.minutes = value & 0x3f,
            ClockRegister::Hours => self.hours = value & 0x1f,
            ClockRegister::DayLower => self.days_lower = value,
            ClockRegister::DayUpper => self.days_upper = value & 0xc1,
        }
    }
}

pub struct Clock {
    pub registers: ClockRegisters,
    pub latched: ClockRegisters,
    pub latch_ready: bool,
    /// Master-clock dots accrued toward the next RTC-second increment.
    sub_second_dots: u32,
}

impl Clock {
    pub fn get_register(&self, register: ClockRegister) -> u8 {
        self.latched.get(register)
    }

    pub fn set_register(&mut self, register: ClockRegister, value: u8) {
        self.registers.set(register, value);
        // A write to the seconds register re-phases the next tick to a full second.
        if matches!(register, ClockRegister::Seconds) {
            self.sub_second_dots = 0;
        }
    }

    pub fn latch(&mut self) {
        self.latched = self.registers;
    }

    /// Advance the RTC by `dots` of real master-clock time. Halted by RTCDH
    /// bit 6, in which case the sub-second counter freezes and resumes in place.
    fn tick(&mut self, dots: u32) {
        if self.registers.days_upper & HALT_BIT != 0 {
            return;
        }
        self.sub_second_dots += dots;
        while self.sub_second_dots >= DOTS_PER_RTC_SECOND {
            self.sub_second_dots -= DOTS_PER_RTC_SECOND;
            self.registers.increment_second();
        }
    }
}

pub struct Mbc3 {
    pub ram: Vec<[u8; 8 * 1024]>,
    pub clock: Option<Clock>,
    pub ram_and_clock_enabled: bool,
    pub bank: u8,
    pub mapped: Mapped,
    pub chip: Mbc3Chip,
}

impl Mbc3 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        let ram = match rom[0x149] {
            2 => {
                let mut banks = vec![[0u8; 8 * 1024]; 1];
                if let Some(data) = &save_data {
                    let len = data.len().min(8 * 1024);
                    banks[0][..len].copy_from_slice(&data[..len]);
                }
                banks
            }
            3 => {
                let mut banks = vec![[0u8; 8 * 1024]; 4];
                if let Some(data) = &save_data {
                    for (bank_idx, bank) in banks.iter_mut().enumerate() {
                        let offset = bank_idx * 8 * 1024;
                        if offset < data.len() {
                            let len = (data.len() - offset).min(bank.len());
                            bank[..len].copy_from_slice(&data[offset..offset + len]);
                        }
                    }
                }
                banks
            }
            _ => vec![],
        };

        let clock = match rom[0x147] {
            0x0f | 0x10 => Some(Clock {
                registers: ClockRegisters::default(),
                latched: ClockRegisters::default(),
                latch_ready: false,
                sub_second_dots: 0,
            }),
            _ => None,
        };

        let chip = if rom.len() > 0x200000 || matches!(rom[0x149], 0x04 | 0x05) {
            Mbc3Chip::Mbc30
        } else {
            Mbc3Chip::Mbc3
        };

        Self {
            ram,
            clock,
            ram_and_clock_enabled: false,
            bank: 1,
            mapped: Mapped::Ram(0),
            chip,
        }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        if self.ram.is_empty() {
            None
        } else {
            Some(self.ram.iter().flatten().copied().collect())
        }
    }

    pub fn tick_rtc(&mut self, dots: u32) {
        if let Some(clock) = &mut self.clock {
            clock.tick(dots);
        }
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => rom[address as usize],
            0x4000..=0x7fff => {
                let bank = if self.bank == 0 { 1 } else { self.bank } as usize;
                let addr = (bank * 0x4000 + (address - 0x4000) as usize) % rom.len();
                rom[addr]
            }
            0xa000..=0xbfff if self.ram_and_clock_enabled => match self.mapped {
                Mapped::Ram(ram_bank) if (ram_bank as usize) < self.ram.len() => {
                    self.ram[ram_bank as usize][(address - 0xa000) as usize]
                }
                Mapped::Clock(register) => self.clock.as_ref().unwrap().get_register(register),
                _ => 0xff,
            },
            _ => 0xff,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) -> bool {
        match address {
            0x0000..=0x1fff => {
                self.ram_and_clock_enabled = value & 0xf == 0xa;
                false
            }
            0x2000..=0x3fff => {
                let bank = value & self.chip.rom_bank_mask();
                self.bank = if bank == 0 { 1 } else { bank };
                false
            }
            0x4000..=0x5fff => {
                self.mapped = match value & 0xf {
                    0x00..=0x07 => Mapped::Ram(value & 0x07),
                    0x08 => Mapped::Clock(ClockRegister::Seconds),
                    0x09 => Mapped::Clock(ClockRegister::Minutes),
                    0x0a => Mapped::Clock(ClockRegister::Hours),
                    0x0b => Mapped::Clock(ClockRegister::DayLower),
                    0x0c => Mapped::Clock(ClockRegister::DayUpper),
                    _ => panic!("Invalid bank select {:2x}", value),
                };
                false
            }
            0x6000..=0x7fff => {
                if let Some(clock) = &mut self.clock {
                    if value & 1 == 0 {
                        clock.latch_ready = true;
                    } else if clock.latch_ready {
                        clock.latch();
                        clock.latch_ready = false;
                    }
                }
                false
            }

            0xa000..=0xbfff if self.ram_and_clock_enabled => match self.mapped {
                Mapped::Ram(ram_bank) if (ram_bank as usize) < self.ram.len() => {
                    self.ram[ram_bank as usize][(address - 0xa000) as usize] = value;
                    true
                }
                Mapped::Clock(register) => {
                    if let Some(clock) = &mut self.clock {
                        clock.set_register(register, value);
                    }
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }
}
