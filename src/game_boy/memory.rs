use crate::game_boy::{
    MemoryMapped, audio,
    interrupts::{self, InterruptFlags},
    serial_transfer, timers, video,
};

use super::save_state::Base64Array;
use nanoserde::{DeRon, DeRonErr, DeRonState, DeRonTok, SerRon, SerRonState};

#[derive(Clone)]
pub struct Ram {
    pub work_ram: [u8; 0x2000],
    pub high_ram: [u8; 0x80],
}

impl SerRon for Ram {
    fn ser_ron(&self, d: usize, s: &mut SerRonState) {
        s.st_pre();
        s.field(d + 1, "work_ram");
        Base64Array(self.work_ram).ser_ron(d + 1, s);
        s.conl();
        s.field(d + 1, "high_ram");
        Base64Array(self.high_ram).ser_ron(d + 1, s);
        s.conl();
        s.st_post(d);
    }
}

impl DeRon for Ram {
    fn de_ron(s: &mut DeRonState, i: &mut std::str::Chars<'_>) -> Result<Self, DeRonErr> {
        s.paren_open(i)?;
        let mut work_ram = None;
        let mut high_ram = None;
        while s.tok != DeRonTok::ParenClose {
            let field = s.identbuf.clone();
            s.next_colon(i)?;
            match field.as_str() {
                "work_ram" => work_ram = Some(Base64Array::<0x2000>::de_ron(s, i)?.0),
                "high_ram" => high_ram = Some(Base64Array::<0x80>::de_ron(s, i)?.0),
                _ => return Err(s.err_parse("unknown field")),
            }
            s.eat_comma_paren(i)?;
        }
        s.paren_close(i)?;
        Ok(Self {
            work_ram: work_ram.ok_or_else(|| s.err_parse("missing work_ram"))?,
            high_ram: high_ram.ok_or_else(|| s.err_parse("missing high_ram"))?,
        })
    }
}

impl Ram {
    pub fn new() -> Self {
        Self {
            work_ram: [0; 0x2000],
            high_ram: [0; 0x80],
        }
    }
}

/// Which physical data bus an address resides on, if any.
///
/// The Game Boy has two data buses:
/// - **External**: ROM, SRAM, WRAM, and WRAM echo
/// - **Vram**: Video RAM (0x8000-0x9FFF)
///
/// OAM, IO registers, and HRAM are internal to the CPU and not on
/// either bus. During OAM DMA the DMA controller occupies one bus,
/// and the CPU can still freely access the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    External,
    Vram,
}

impl Bus {
    /// Classify a 16-bit address by bus, or `None` for CPU-internal
    /// addresses (OAM, IO, HRAM, unmapped).
    pub fn of(address: u16) -> Option<Bus> {
        match address {
            0x0000..=0x7FFF => Some(Bus::External),
            0x8000..=0x9FFF => Some(Bus::Vram),
            0xA000..=0xBFFF => Some(Bus::External),
            0xC000..=0xFDFF => Some(Bus::External),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum MappedAddress {
    Cartridge(u16),
    WorkRam(u16),
    HighRam(u8),
    VideoRam(video::memory::MappedAddress),
    JoypadRegister,
    SerialTransferRegister(serial_transfer::Register),
    TimerRegister(timers::Register),
    InterruptRegister(interrupts::Register),
    AudioRegister(audio::Register),
    AudioWaveRam(u8),
    VideoRegister(video::Register),
    BeginDmaTransfer,
    Unmapped,
}

impl MappedAddress {
    pub fn map(address: u16) -> Self {
        match address {
            0x0000..=0x7fff => Self::Cartridge(address),
            0x8000..=0x9fff => Self::VideoRam(video::memory::MappedAddress::map(address)),
            0xa000..=0xbfff => Self::Cartridge(address),
            0xc000..=0xdfff => Self::WorkRam(address - 0xc000),
            0xe000..=0xfdff => Self::WorkRam(address - 0xe000),
            0xfe00..=0xfe9f => Self::VideoRam(video::memory::MappedAddress::map(address)),
            0xfea0..=0xfeff => Self::Unmapped,
            0xff00 => Self::JoypadRegister,
            0xff01 => Self::SerialTransferRegister(serial_transfer::Register::Data),
            0xff02 => Self::SerialTransferRegister(serial_transfer::Register::Control),
            0xff03 => Self::Unmapped,
            0xff04 => Self::TimerRegister(timers::Register::Divider),
            0xff05 => Self::TimerRegister(timers::Register::Counter),
            0xff06 => Self::TimerRegister(timers::Register::Modulo),
            0xff07 => Self::TimerRegister(timers::Register::Control),
            0xff08..=0xff0e => Self::Unmapped,
            0xff0f => Self::InterruptRegister(interrupts::Register::RequestedInterrupts),
            0xff10..=0xff14 => Self::AudioRegister(audio::Register::map(address)),
            0xff15 => Self::Unmapped,
            0xff16..=0xff1e => Self::AudioRegister(audio::Register::map(address)),
            0xff1f => Self::Unmapped,
            0xff20..=0xff26 => Self::AudioRegister(audio::Register::map(address)),
            0xff27..=0xff2f => Self::Unmapped,
            0xff30..=0xff3f => Self::AudioWaveRam((address - 0xff30) as u8),
            0xff40 => Self::VideoRegister(video::Register::Control),
            0xff41 => Self::VideoRegister(video::Register::Status),
            0xff42 => Self::VideoRegister(video::Register::BackgroundViewportY),
            0xff43 => Self::VideoRegister(video::Register::BackgroundViewportX),
            0xff44 => Self::VideoRegister(video::Register::CurrentScanline),
            0xff45 => Self::VideoRegister(video::Register::InterruptOnScanline),
            0xff46 => Self::BeginDmaTransfer,
            0xff47 => Self::VideoRegister(video::Register::BackgroundPalette),
            0xff48 => Self::VideoRegister(video::Register::Sprite0Palette),
            0xff49 => Self::VideoRegister(video::Register::Sprite1Palette),
            0xff4a => Self::VideoRegister(video::Register::WindowY),
            0xff4b => Self::VideoRegister(video::Register::WindowX),
            0xff4c..=0xff7f => Self::Unmapped,
            0xff80..=0xfffe => Self::HighRam((address - 0xff80) as u8),
            0xffff => Self::InterruptRegister(interrupts::Register::EnabledInterrupts),
        }
    }
}

impl MemoryMapped {
    pub fn read(&self, address: u16) -> u8 {
        if let Some(dma) = &self.dma {
            if !matches!(dma.delay, Some(super::DmaDelay::Startup(_))) {
                // OAM is being written to by DMA; CPU reads return $FF.
                if (0xFE00..=0xFE9F).contains(&address) {
                    return 0xFF;
                }
                // Bus conflict: if the CPU accesses the same bus the DMA
                // is reading from, the read returns the byte being transferred.
                if Bus::of(address) == Some(dma.source_bus) {
                    let src = dma.source + dma.byte_index as u16;
                    return self.read_mapped(MappedAddress::map(src));
                }
            }
        }
        self.read_mapped(MappedAddress::map(address))
    }

    /// Read a byte as the DMA controller would. Addresses not on either
    /// bus (OAM, IO, HRAM) are remapped to WRAM echo on the external bus.
    pub fn read_dma_source(&self, address: u16) -> u8 {
        let mapped = match Bus::of(address) {
            Some(_) => MappedAddress::map(address),
            None => MappedAddress::WorkRam(address.wrapping_sub(0xE000)),
        };
        self.read_mapped(mapped)
    }

    pub fn read_mapped(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::Cartridge(address) => self.cartridge.read(address),
            MappedAddress::WorkRam(address) => self.ram.work_ram[address as usize],
            MappedAddress::HighRam(address) => self.ram.high_ram[address as usize],
            MappedAddress::VideoRam(address) => self.video.read_memory(address),
            MappedAddress::JoypadRegister => {
                let mut value = self.joypad.read_register();
                if let Some(sgb) = &self.sgb {
                    if sgb.player_count > 1 {
                        let p14_selected = value & 0x10 == 0;
                        let p15_selected = value & 0x20 == 0;
                        if !p14_selected && !p15_selected {
                            value = (value & 0xF0) | (0x0F - sgb.current_player);
                        }
                    }
                }
                value
            }
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.data,
                serial_transfer::Register::Control => self.serial.control.bits() | 0x7E,
            },
            MappedAddress::TimerRegister(register) => self.timers.read_register(register),
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => self.interrupts.enabled.bits(),
                interrupts::Register::RequestedInterrupts => {
                    self.interrupts.requested.bits() | 0xE0
                }
            },
            MappedAddress::AudioRegister(register) => self.audio.read_register(register),
            MappedAddress::AudioWaveRam(offset) => self.audio.read_wave_ram(offset),
            MappedAddress::VideoRegister(register) => self.video.read_register(register),
            MappedAddress::BeginDmaTransfer => self.dma_source,

            MappedAddress::Unmapped => 0xFF,
        }
    }

    pub fn write_byte(&mut self, address: u16, value: u8) {
        if let Some(dma) = &self.dma {
            if !matches!(dma.delay, Some(super::DmaDelay::Startup(_))) {
                // OAM is being written to by DMA; CPU writes are ignored.
                if (0xFE00..=0xFE9F).contains(&address) {
                    return;
                }
                // Bus conflict: CPU writes on the same bus as DMA are ignored.
                if Bus::of(address) == Some(dma.source_bus) {
                    return;
                }
            }
        }
        self.write_mapped(MappedAddress::map(address), value);
    }

    pub fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Cartridge(address) => self.cartridge.write(address, value),
            MappedAddress::WorkRam(address) => self.ram.work_ram[address as usize] = value,
            MappedAddress::HighRam(address) => self.ram.high_ram[address as usize] = value,
            MappedAddress::VideoRam(address) => self.video.write_memory(address, value),
            MappedAddress::JoypadRegister => {
                if let Some(sgb) = &mut self.sgb {
                    sgb.write_joypad(value);
                }
                self.joypad.write_register(value);
            }
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.data = value,
                serial_transfer::Register::Control => {
                    self.serial.control = serial_transfer::Control::from_bits_retain(value);
                    self.serial.start_transfer();
                }
            },
            MappedAddress::TimerRegister(register) => {
                self.timers.write_register(register, value);
            }
            MappedAddress::AudioRegister(register) => self.audio.write_register(register, value),
            MappedAddress::AudioWaveRam(offset) => self.audio.write_wave_ram(offset, value),
            MappedAddress::VideoRegister(register) => self.video.write_register(register, value),
            MappedAddress::BeginDmaTransfer => self.begin_dma_transfer(value),
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => {
                    self.interrupts.enabled = InterruptFlags::from_bits_retain(value)
                }
                interrupts::Register::RequestedInterrupts => {
                    self.interrupts.requested = InterruptFlags::from_bits_retain(value)
                }
            },

            MappedAddress::Unmapped => {}
        }
    }

    fn begin_dma_transfer(&mut self, source: u8) {
        // When restarting DMA while a previous transfer is active (past startup),
        // bus conflicts remain in effect during the new startup period.
        let active_dma = self
            .dma
            .as_ref()
            .is_some_and(|d| !matches!(d.delay, Some(super::DmaDelay::Startup(_))));
        let source_addr = source as u16 * 0x100;
        self.dma_source = source;
        self.dma = Some(super::DmaTransfer {
            source: source_addr,
            source_bus: Bus::of(source_addr).unwrap_or(Bus::External),
            byte_index: 0,
            delay: Some(if active_dma {
                super::DmaDelay::Transfer(2)
            } else {
                super::DmaDelay::Startup(2)
            }),
        });
    }
}
