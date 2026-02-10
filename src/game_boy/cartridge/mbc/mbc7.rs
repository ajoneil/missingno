use crate::game_boy::cartridge::MemoryBankController;
use crate::game_boy::save_state::Base64Bytes;

enum EepromState {
    Idle,
    ReceivingCommand {
        bits: u16,
        count: u8,
    },
    Reading {
        address: u8,
        bits_remaining: u8,
    },
    Writing {
        address: u8,
        value: u16,
        bits_remaining: u8,
    },
}

struct Eeprom {
    data: [u16; 128],
    state: EepromState,
    write_enabled: bool,
    do_bit: u8,
    last_clk: bool,
    last_cs: bool,
}

impl Eeprom {
    fn new(save_data: Option<&Vec<u8>>) -> Self {
        let mut data = [0xffffu16; 128];
        if let Some(save) = save_data {
            for (i, word) in data.iter_mut().enumerate() {
                let offset = i * 2;
                if offset + 1 < save.len() {
                    *word = u16::from_be_bytes([save[offset], save[offset + 1]]);
                }
            }
        }

        Self {
            data,
            state: EepromState::Idle,
            write_enabled: false,
            do_bit: 1,
            last_clk: false,
            last_cs: false,
        }
    }

    fn to_vec(&self) -> Vec<u8> {
        self.data.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn write(&mut self, value: u8) {
        let cs = value & 0x80 != 0;
        let clk = value & 0x40 != 0;
        let di = (value >> 1) & 1;

        if !cs {
            self.state = EepromState::Idle;
            self.do_bit = 1;
            self.last_clk = clk;
            self.last_cs = cs;
            return;
        }

        // Detect rising edge of clock
        if !clk || self.last_clk {
            self.last_clk = clk;
            self.last_cs = cs;
            return;
        }
        self.last_clk = clk;
        self.last_cs = cs;

        match &mut self.state {
            EepromState::Idle => {
                if di == 1 {
                    // Start bit received
                    self.state = EepromState::ReceivingCommand { bits: 0, count: 0 };
                }
            }
            EepromState::ReceivingCommand { bits, count } => {
                *bits = (*bits << 1) | di as u16;
                *count += 1;

                if *count == 10 {
                    let command = *bits;
                    let opcode = (command >> 8) & 0x03;
                    let address = (command & 0x7f) as u8;

                    match opcode {
                        0b10 => {
                            // READ
                            self.do_bit = 0; // Dummy bit
                            self.state = EepromState::Reading {
                                address,
                                bits_remaining: 17, // 1 dummy + 16 data
                            };
                        }
                        0b01 => {
                            // WRITE
                            self.state = EepromState::Writing {
                                address,
                                value: 0,
                                bits_remaining: 16,
                            };
                        }
                        0b11 => {
                            // ERASE
                            if self.write_enabled {
                                self.data[address as usize] = 0xffff;
                            }
                            self.do_bit = 1;
                            self.state = EepromState::Idle;
                        }
                        0b00 => {
                            // Special commands based on upper address bits
                            match (address >> 5) & 0x03 {
                                0b11 => {
                                    // EWEN - enable writes
                                    self.write_enabled = true;
                                }
                                0b00 => {
                                    // EWDS - disable writes
                                    self.write_enabled = false;
                                }
                                0b10 => {
                                    // ERAL - erase all
                                    if self.write_enabled {
                                        self.data = [0xffff; 128];
                                    }
                                }
                                0b01 => {
                                    // WRAL - write all (need 16 more bits)
                                    self.state = EepromState::Writing {
                                        address: 0xff, // Sentinel for write-all
                                        value: 0,
                                        bits_remaining: 16,
                                    };
                                    return;
                                }
                                _ => unreachable!(),
                            }
                            self.do_bit = 1;
                            self.state = EepromState::Idle;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            EepromState::Reading {
                address,
                bits_remaining,
            } => {
                *bits_remaining -= 1;
                if *bits_remaining == 16 {
                    // Dummy bit done, now output data MSB first
                    self.do_bit = ((self.data[*address as usize] >> 15) & 1) as u8;
                } else if *bits_remaining > 0 {
                    self.do_bit =
                        ((self.data[*address as usize] >> *bits_remaining as u32) & 1) as u8;
                } else {
                    self.do_bit = (self.data[*address as usize] & 1) as u8;
                    self.state = EepromState::Idle;
                }
            }
            EepromState::Writing {
                address,
                value,
                bits_remaining,
            } => {
                *value = (*value << 1) | di as u16;
                *bits_remaining -= 1;

                if *bits_remaining == 0 {
                    if self.write_enabled {
                        if *address == 0xff {
                            // WRAL
                            self.data = [*value; 128];
                        } else {
                            self.data[*address as usize] = *value;
                        }
                    }
                    self.do_bit = 1;
                    self.state = EepromState::Idle;
                }
            }
        }
    }

    fn read(&self) -> u8 {
        self.do_bit
    }
}

enum LatchState {
    Idle,
    WroteErase,
}

pub struct Mbc7 {
    rom: Vec<u8>,
    eeprom: Eeprom,
    ram_enabled_1: bool,
    ram_enabled_2: bool,
    rom_bank: u8,
    accel_x: u16,
    accel_y: u16,
    latch_state: LatchState,
}

impl Mbc7 {
    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Self {
        Self {
            eeprom: Eeprom::new(save_data.as_ref()),
            rom,
            ram_enabled_1: false,
            ram_enabled_2: false,
            rom_bank: 1,
            accel_x: 0x8000,
            accel_y: 0x8000,
            latch_state: LatchState::Idle,
        }
    }

    fn ram_accessible(&self) -> bool {
        self.ram_enabled_1 && self.ram_enabled_2
    }

    pub(crate) fn save_state(&self) -> crate::game_boy::save_state::MbcState {
        crate::game_boy::save_state::MbcState::Mbc7 {
            eeprom_data: Base64Bytes(self.eeprom.to_vec()),
            eeprom_write_enabled: self.eeprom.write_enabled,
            ram_enabled_1: self.ram_enabled_1,
            ram_enabled_2: self.ram_enabled_2,
            rom_bank: self.rom_bank,
            accel_x: self.accel_x,
            accel_y: self.accel_y,
        }
    }

    pub(crate) fn from_state(rom: Vec<u8>, state: crate::game_boy::save_state::MbcState) -> Self {
        let crate::game_boy::save_state::MbcState::Mbc7 {
            eeprom_data,
            eeprom_write_enabled,
            ram_enabled_1,
            ram_enabled_2,
            rom_bank,
            accel_x,
            accel_y,
        } = state
        else {
            unreachable!();
        };
        let mut eeprom = Eeprom::new(Some(&eeprom_data.0));
        eeprom.write_enabled = eeprom_write_enabled;
        Self {
            rom,
            eeprom,
            ram_enabled_1,
            ram_enabled_2,
            rom_bank,
            accel_x,
            accel_y,
            latch_state: LatchState::Idle,
        }
    }
}

impl MemoryBankController for Mbc7 {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn ram(&self) -> Option<Vec<u8>> {
        Some(self.eeprom.to_vec())
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => self.rom[address as usize],
            0x4000..=0x7fff => {
                let bank = self.rom_bank.max(1) as usize;
                let addr = bank * 0x4000 + (address - 0x4000) as usize;
                if addr < self.rom.len() {
                    self.rom[addr]
                } else {
                    0xff
                }
            }
            0xa000..=0xafff if self.ram_accessible() => {
                let register = (address >> 4) & 0x0f;
                match register {
                    0x2 => (self.accel_x & 0xff) as u8,
                    0x3 => (self.accel_x >> 8) as u8,
                    0x4 => (self.accel_y & 0xff) as u8,
                    0x5 => (self.accel_y >> 8) as u8,
                    0x6 => 0x00,
                    0x7 => 0xff,
                    0x8 => self.eeprom.read(),
                    _ => 0xff,
                }
            }
            0xb000..=0xbfff => 0xff,
            _ => 0xff,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1fff => self.ram_enabled_1 = value & 0x0f == 0x0a,
            0x2000..=0x3fff => self.rom_bank = value & 0x7f,
            0x4000..=0x5fff => self.ram_enabled_2 = value == 0x40,
            0xa000..=0xafff if self.ram_accessible() => {
                let register = (address >> 4) & 0x0f;
                match register {
                    0x0 => {
                        if value == 0x55 {
                            self.latch_state = LatchState::WroteErase;
                            self.accel_x = 0x8000;
                            self.accel_y = 0x8000;
                        }
                    }
                    0x1 => {
                        if let LatchState::WroteErase = self.latch_state {
                            if value == 0xaa {
                                // Latch accelerometer — return center values (no tilt)
                                self.accel_x = 0x81d0;
                                self.accel_y = 0x81d0;
                            }
                        }
                        self.latch_state = LatchState::Idle;
                    }
                    0x8 => self.eeprom.write(value),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
