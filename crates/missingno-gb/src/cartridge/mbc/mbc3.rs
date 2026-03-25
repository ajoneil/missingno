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

#[derive(Clone, Copy, Default)]
struct ClockRegisters {
    seconds: u8,
    minutes: u8,
    hours: u8,
    days_lower: u8,
    days_upper: u8,
}

impl ClockRegisters {
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

struct Clock {
    registers: ClockRegisters,
    latched: ClockRegisters,
    latch_ready: bool,
}

impl Clock {
    pub fn get_register(&self, register: ClockRegister) -> u8 {
        self.latched.get(register)
    }

    pub fn set_register(&mut self, register: ClockRegister, value: u8) {
        self.registers.set(register, value);
    }

    pub fn latch(&mut self) {
        self.latched = self.registers;
    }
}

pub struct Mbc3 {
    ram: Vec<[u8; 8 * 1024]>,
    clock: Option<Clock>,
    ram_and_clock_enabled: bool,
    bank: u8,
    mapped: Mapped,
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
            }),
            _ => None,
        };

        Self {
            ram,
            clock,
            ram_and_clock_enabled: false,
            bank: 1,
            mapped: Mapped::Ram(0),
        }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        if self.ram.is_empty() {
            None
        } else {
            Some(self.ram.iter().flatten().copied().collect())
        }
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => rom[address as usize],
            0x4000..=0x7fff => {
                let bank = if self.bank == 0 { 1 } else { self.bank } as usize;
                rom[bank * 0x4000 + (address - 0x4000) as usize]
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

    pub fn write(&mut self, address: u16, value: u8) {
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
                if let Some(clock) = &mut self.clock {
                    if value & 1 == 0 {
                        clock.latch_ready = true;
                    } else if clock.latch_ready {
                        clock.latch();
                        clock.latch_ready = false;
                    }
                }
            }

            0xa000..=0xbfff if self.ram_and_clock_enabled => match self.mapped {
                Mapped::Ram(ram_bank) if (ram_bank as usize) < self.ram.len() => {
                    self.ram[ram_bank as usize][(address - 0xa000) as usize] = value;
                }
                Mapped::Clock(register) => {
                    if let Some(clock) = &mut self.clock {
                        clock.set_register(register, value);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}
