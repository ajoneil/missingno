use crate::emulator::cartridge::MemoryBankController;

enum Mapped {
    Ram(u8),
    Clock(ClockRegister),
}

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
    day_lower: u8,
    day_upper: u8,
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
                day_lower: 0,
                day_upper: 0,
            }),
            _ => None,
        };

        Self {
            rom,
            ram,
            clock,
            ram_and_clock_enabled: false,
            bank: 0,
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
                self.rom[(self.bank as usize * 0x4000) + (address - 0x4000) as usize]
            }
            0xa000..=0xbfff => match self.mapped {
                Mapped::Ram(ram_bank) => self.ram[ram_bank as usize][(address - 0xa000) as usize],
                _ => {
                    println!("nyi: rtc");
                    0x00
                }
            },
            _ => 0xff,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1ff => self.ram_and_clock_enabled = value & 0xf == 0xa,
            0x2000..=0x3fff => {
                self.bank = value & 0x7f;
            }
            0x4000..=0x5fff => {
                self.mapped = match value & 0xf {
                    0x00..=0x07 => Mapped::Ram(value),
                    0x08..=0x0c => {
                        println!("nyi: rtc");
                        Mapped::Clock(ClockRegister::Seconds)
                    }
                    _ => panic!("Invalid bank select {:2x}", value),
                };
            }
            0x6000..=0x7fff => {
                println!("todo: rtc latch")
            }

            _ => {}
        }
    }
}
