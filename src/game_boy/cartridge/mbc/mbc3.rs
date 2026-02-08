use crate::game_boy::cartridge::MemoryBankController;

enum Mapped {
    Ram(u8),
    Clock(ClockRegister),
}

#[derive(Clone, Copy)]
enum ClockRegister {
    Seconds,
    Minutes,
    Hours,
    DayLower,
    DayUpper,
}

struct Clock {
    seconds: u8,
    minutes: u8,
    hours: u8,
    days_lower: u8,
    days_upper: u8,
}

impl Clock {
    pub fn get_register(&self, register: ClockRegister) -> u8 {
        match register {
            ClockRegister::Seconds => self.seconds,
            ClockRegister::Minutes => self.minutes,
            ClockRegister::Hours => self.hours,
            ClockRegister::DayLower => self.days_lower,
            ClockRegister::DayUpper => self.days_upper,
        }
    }
}

pub struct Mbc3 {
    rom: Vec<u8>,
    ram: Vec<[u8; 8 * 1024]>,
    clock: Option<Clock>,
    ram_and_clock_enabled: bool,
    bank: u8,
    mapped: Mapped,
}

impl Mbc3 {
    pub fn new(rom: Vec<u8>) -> Self {
        let ram = match rom[0x149] {
            2 => vec![[0; 8 * 1024]; 1],
            3 => vec![[0; 8 * 1024]; 4],
            _ => vec![],
        };

        let clock = match rom[0x147] {
            0x0f | 0x10 => Some(Clock {
                seconds: 0,
                minutes: 0,
                hours: 0,
                days_lower: 0,
                days_upper: 0,
            }),
            _ => None,
        };

        Self {
            rom,
            ram,
            clock,
            ram_and_clock_enabled: false,
            bank: 1,
            mapped: Mapped::Ram(0),
        }
    }
}

impl MemoryBankController for Mbc3 {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => self.rom[address as usize],
            0x4000..=0x7fff => {
                let bank = if self.bank == 0 { 1 } else { self.bank } as usize;
                self.rom[bank * 0x4000 + (address - 0x4000) as usize]
            }
            0xa000..=0xbfff => {
                if !self.ram_and_clock_enabled || self.ram.is_empty() {
                    return 0xff;
                }
                match self.mapped {
                    Mapped::Ram(ram_bank) => {
                        self.ram[ram_bank as usize][(address - 0xa000) as usize]
                    }
                    Mapped::Clock(register) => self.clock.as_ref().unwrap().get_register(register),
                }
            }
            _ => 0xff,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1fff => self.ram_and_clock_enabled = value & 0xf == 0xa,
            0x2000..=0x3fff => {
                let bank = value & 0x7f;
                self.bank = if bank == 0 { 1 } else { bank };
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
            }
            0x6000..=0x7fff => {
                println!("todo: rtc latch")
            }

            0xa000..=0xbfff => {
                if !self.ram_and_clock_enabled || self.ram.is_empty() {
                    return;
                }
                match self.mapped {
                    Mapped::Ram(ram_bank) => {
                        self.ram[ram_bank as usize][(address - 0xa000) as usize] = value;
                    }
                    Mapped::Clock(_register) => {
                        // TODO: RTC register writes
                    }
                }
            }
            _ => {}
        }
    }
}
