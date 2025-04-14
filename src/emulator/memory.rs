use super::{
    MemoryMapped, audio,
    interrupts::{self, InterruptFlags},
    serial_transfer, video,
};

pub struct Ram {
    pub work_ram: [u8; 0x2000],
    pub high_ram: [u8; 0x80],
}

impl Ram {
    pub fn new() -> Self {
        Self {
            work_ram: [0; 0x2000],
            high_ram: [0; 0x80],
        }
    }
}

pub enum MappedAddress {
    Cartridge(u16),
    WorkRam(u16),
    HighRam(u8),
    VideoRam(video::memory::MappedAddress),
    InterruptRegister(interrupts::Register),
    AudioRegister(audio::Register),
    VideoRegister(video::Register),
    SerialTransferRegister(serial_transfer::Register),
    Unmapped,
}

impl MappedAddress {
    pub fn map(address: u16) -> Self {
        match address {
            0x0000..=0x7fff => Self::Cartridge(address),
            0x8000..=0x9fff => Self::VideoRam(video::memory::MappedAddress::map(address)),
            0xc000..=0xdfff => Self::WorkRam(address - 0xc000),
            0xfe00..=0xfe9f => Self::VideoRam(video::memory::MappedAddress::map(address)),
            0xfea0..=0xfeff => Self::Unmapped,
            0xff01 => Self::SerialTransferRegister(serial_transfer::Register::Data),
            0xff02 => Self::SerialTransferRegister(serial_transfer::Register::Control),
            0xff0f => Self::InterruptRegister(interrupts::Register::RequestedInterrupts),
            0xff24 => Self::AudioRegister(audio::Register::Volume),
            0xff25 => Self::AudioRegister(audio::Register::Panning),
            0xff26 => Self::AudioRegister(audio::Register::Control),
            0xff40 => Self::VideoRegister(video::Register::Control),
            0xff41 => Self::VideoRegister(video::Register::Status),
            0xff42 => Self::VideoRegister(video::Register::BackgroundViewportY),
            0xff43 => Self::VideoRegister(video::Register::BackgroundViewportX),
            0xff44 => Self::VideoRegister(video::Register::CurrentScanline),
            0xff47 => Self::VideoRegister(video::Register::BackgroundPalette),
            0xff48 => Self::VideoRegister(video::Register::Sprite0Palette),
            0xff49 => Self::VideoRegister(video::Register::Sprite1Palette),
            0xff4c..=0xff7f => Self::Unmapped,
            0xff80..=0xfffe => Self::HighRam((address - 0xff80) as u8),
            0xffff => Self::InterruptRegister(interrupts::Register::EnabledInterrupts),
            _ => todo!("Unmapped address {:04x}", address),
        }
    }
}

pub enum MemoryWrite {
    Write8(MappedAddress, u8),
    //Write16((MappedAddress, u8), (MappedAddress, u8)),
}

impl MemoryMapped {
    pub fn read(&self, address: u16) -> u8 {
        self.read_mapped(MappedAddress::map(address))
    }

    pub fn read_mapped(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::Cartridge(address) => self.cartridge.read(address),
            MappedAddress::WorkRam(address) => self.ram.work_ram[address as usize],
            MappedAddress::HighRam(address) => self.ram.high_ram[address as usize],
            MappedAddress::VideoRam(address) => self.video.read_memory(address),
            MappedAddress::AudioRegister(register) => self.audio.read_register(register),
            MappedAddress::VideoRegister(register) => self.video.read_register(register),
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => self.interrupts.enabled.bits(),
                interrupts::Register::RequestedInterrupts => self.interrupts.requested.bits(),
            },
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.data,
                serial_transfer::Register::Control => self.serial.control.bits(),
            },
            MappedAddress::Unmapped => 0x00,
        }
    }

    pub fn write(&mut self, write: MemoryWrite) {
        match write {
            MemoryWrite::Write8(address, value) => self.write_mapped(address, value),
            // MemoryWrite::Write16((address1, value1), (address2, value2)) => {
            //     self.write_mapped(address1, value1);
            //     self.write_mapped(address2, value2);
            // }
        }
    }

    pub fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Cartridge(address) => self.cartridge.write(address, value),
            MappedAddress::WorkRam(address) => self.ram.work_ram[address as usize] = value,
            MappedAddress::HighRam(address) => self.ram.work_ram[address as usize] = value,
            MappedAddress::VideoRam(address) => self.video.write_memory(address, value),
            MappedAddress::AudioRegister(register) => self.audio.write_register(register, value),
            MappedAddress::VideoRegister(register) => self.video.write_register(register, value),
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => {
                    self.interrupts.enabled = InterruptFlags::from_bits_retain(value)
                }
                interrupts::Register::RequestedInterrupts => {
                    self.interrupts.requested = InterruptFlags::from_bits_retain(value)
                }
            },
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.data = value,
                serial_transfer::Register::Control => {
                    self.serial.control = serial_transfer::Control::from_bits_retain(value)
                }
            },
            MappedAddress::Unmapped => {}
        }
    }
}
