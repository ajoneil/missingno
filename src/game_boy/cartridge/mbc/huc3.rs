use crate::game_boy::save_state::Base64Bytes;

#[derive(Clone, Copy)]
enum Mode {
    Rom,
    Ram,
    RtcCommand,
    RtcResponse,
    RtcSemaphore,
    Ir,
}

pub struct Huc3 {
    ram: Vec<[u8; 8 * 1024]>,
    rom_bank: u8,
    ram_bank: u8,
    mode: Mode,
    rtc_memory: [u8; 256],
    rtc_address: u8,
    rtc_last_command: u8,
    rtc_response: u8,
    rtc_semaphore: u8,
}

impl Huc3 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        let num_ram_banks = match rom[0x149] {
            2 => 1,
            3 => 4,
            _ => 0,
        };

        let mut ram = vec![[0u8; 8 * 1024]; num_ram_banks];
        if let Some(data) = &save_data {
            for (bank_idx, bank) in ram.iter_mut().enumerate() {
                let offset = bank_idx * 8 * 1024;
                if offset < data.len() {
                    let len = (data.len() - offset).min(bank.len());
                    bank[..len].copy_from_slice(&data[offset..offset + len]);
                }
            }
        }

        Self {
            ram,
            rom_bank: 1,
            ram_bank: 0,
            mode: Mode::Rom,
            rtc_memory: [0; 256],
            rtc_address: 0,
            rtc_last_command: 0,
            rtc_response: 0,
            rtc_semaphore: 1,
        }
    }

    fn handle_rtc_command(&mut self, value: u8) {
        let command = (value >> 4) & 0x07;
        let argument = value & 0x0f;

        self.rtc_last_command = command;

        match command {
            0x1 => {
                // Read and increment address
                self.rtc_response = self.rtc_memory[self.rtc_address as usize] & 0x0f;
                self.rtc_address = self.rtc_address.wrapping_add(1);
            }
            0x3 => {
                // Write and increment address
                self.rtc_memory[self.rtc_address as usize] = argument;
                self.rtc_address = self.rtc_address.wrapping_add(1);
            }
            0x4 => {
                // Set low nibble of address
                self.rtc_address = (self.rtc_address & 0xf0) | argument;
            }
            0x5 => {
                // Set high nibble of address
                self.rtc_address = (self.rtc_address & 0x0f) | (argument << 4);
            }
            0x6 => {
                // Extended command
                match argument {
                    0x0 => {
                        // Latch time — copy current time to locations 0x00-0x06
                        // We don't track real time, so leave as-is
                    }
                    0x1 => {
                        // Copy time from 0x00-0x06
                    }
                    0x2 => {
                        // Status check
                        self.rtc_response = 0x01;
                    }
                    0xe => {
                        // Tone generator — ignored
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub(crate) fn save_state(&self) -> crate::game_boy::save_state::MbcState {
        let mode = match self.mode {
            Mode::Rom => 0,
            Mode::Ram => 1,
            Mode::RtcCommand => 2,
            Mode::RtcResponse => 3,
            Mode::RtcSemaphore => 4,
            Mode::Ir => 5,
        };
        crate::game_boy::save_state::MbcState::Huc3 {
            ram: Base64Bytes::from_banks(&self.ram),
            rom_bank: self.rom_bank,
            ram_bank: self.ram_bank,
            mode,
            rtc_memory: Base64Bytes(self.rtc_memory.to_vec()),
            rtc_address: self.rtc_address,
            rtc_last_command: self.rtc_last_command,
            rtc_response: self.rtc_response,
            rtc_semaphore: self.rtc_semaphore,
        }
    }

    pub(crate) fn from_state(rom: &[u8], state: crate::game_boy::save_state::MbcState) -> Self {
        let crate::game_boy::save_state::MbcState::Huc3 {
            ram: ram_data,
            rom_bank,
            ram_bank,
            mode: mode_val,
            rtc_memory: rtc_memory_data,
            rtc_address,
            rtc_last_command,
            rtc_response,
            rtc_semaphore,
        } = state
        else {
            unreachable!();
        };
        let num_ram_banks = match rom[0x149] {
            2 => 1,
            3 => 4,
            _ => 0,
        };
        let mode = match mode_val {
            0 => Mode::Rom,
            1 => Mode::Ram,
            2 => Mode::RtcCommand,
            3 => Mode::RtcResponse,
            4 => Mode::RtcSemaphore,
            _ => Mode::Ir,
        };
        let mut rtc_memory = [0u8; 256];
        let len = rtc_memory_data.len().min(256);
        rtc_memory[..len].copy_from_slice(&rtc_memory_data[..len]);
        Self {
            ram: ram_data.into_banks(num_ram_banks),
            rom_bank,
            ram_bank,
            mode,
            rtc_memory,
            rtc_address,
            rtc_last_command,
            rtc_response,
            rtc_semaphore,
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
                let bank = self.rom_bank.max(1) as usize;
                let addr = bank * 0x4000 + (address - 0x4000) as usize;
                rom[addr % rom.len()]
            }
            0xa000..=0xbfff => match self.mode {
                Mode::Ram => {
                    let bank = self.ram_bank as usize;
                    if bank < self.ram.len() {
                        self.ram[bank][(address - 0xa000) as usize]
                    } else {
                        0xff
                    }
                }
                Mode::RtcResponse => {
                    0x80 | (self.rtc_last_command << 4) | (self.rtc_response & 0x0f)
                }
                Mode::RtcSemaphore => 0x80 | self.rtc_semaphore,
                Mode::Ir => {
                    // No remote device
                    0xc0
                }
                _ => 0xff,
            },
            _ => 0xff,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1fff => {
                self.mode = match value & 0x0f {
                    0x00 => Mode::Rom,
                    0x0a => Mode::Ram,
                    0x0b => Mode::RtcCommand,
                    0x0c => Mode::RtcResponse,
                    0x0d => Mode::RtcSemaphore,
                    0x0e => Mode::Ir,
                    _ => self.mode,
                };
            }
            0x2000..=0x3fff => self.rom_bank = value & 0x7f,
            0x4000..=0x5fff => self.ram_bank = value & 0x03,
            0xa000..=0xbfff => match self.mode {
                Mode::Ram => {
                    let bank = self.ram_bank as usize;
                    if bank < self.ram.len() {
                        self.ram[bank][(address - 0xa000) as usize] = value;
                    }
                }
                Mode::RtcCommand => {
                    self.handle_rtc_command(value);
                }
                Mode::RtcSemaphore => {
                    if value == 0xfe {
                        // Trigger execution — set semaphore to ready
                        self.rtc_semaphore = 1;
                    }
                }
                Mode::Ir => {
                    // IR transmitter control — ignored
                }
                _ => {}
            },
            _ => {}
        }
    }
}
