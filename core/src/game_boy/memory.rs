use crate::game_boy::{
    GameBoy, audio,
    interrupts::{self, InterruptFlags},
    serial_transfer, timers, video,
};

use super::cartridge::Cartridge;
use video::memory::{OamAddress, Vram, VramAddress};

/// M-cycles before the external data bus decays to 0xFF.
///
/// On real hardware the external bus retains its last driven value
/// through parasitic capacitance. With no device driving the bus
/// the charge leaks and the value trends toward 0xFF. The exact
/// rate is board-dependent; 12 M-cycles (~2.86 µs) is a reasonable
/// approximation.
const EXTERNAL_BUS_DECAY_MCYCLES: u8 = 12;

/// The external data bus connects the SoC to the cartridge and, on
/// DMG, to work RAM. The bus retains its last driven value through
/// parasitic capacitance, decaying toward 0xFF when idle.
pub struct ExternalBus {
    pub(super) cartridge: Cartridge,
    pub(super) work_ram: [u8; 0x2000],

    /// Retained value on the data bus. Updated on every CPU read/write
    /// to an external-bus address and by DMA when reading from this bus.
    pub(super) latch: u8,
    /// M-cycles remaining before `latch` decays to 0xFF.
    pub(super) decay: u8,
}

impl ExternalBus {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cartridge,
            work_ram: [0; 0x2000],
            latch: 0xFF,
            decay: 0,
        }
    }

    /// Read from a device on this bus (cartridge or WRAM).
    /// Does NOT update the latch — callers decide when to latch.
    pub fn read(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::Cartridge(addr) => self.cartridge.read(addr),
            MappedAddress::WorkRam(addr) => self.work_ram[addr as usize],
            _ => unreachable!("ExternalBus::read called with non-external address"),
        }
    }

    /// Write to a device on this bus (cartridge or WRAM).
    pub fn write(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Cartridge(addr) => self.cartridge.write(addr, value),
            MappedAddress::WorkRam(addr) => self.work_ram[addr as usize] = value,
            _ => unreachable!("ExternalBus::write called with non-external address"),
        }
    }

    /// Update the bus latch to `value` and reset the decay counter.
    pub fn drive(&mut self, value: u8) {
        self.latch = value;
        self.decay = EXTERNAL_BUS_DECAY_MCYCLES;
    }

    /// Return the current latch value (for DMA bus conflict reads).
    pub fn latch(&self) -> u8 {
        self.latch
    }

    /// Tick decay: call once per M-cycle. If the counter reaches zero,
    /// the latch decays to 0xFF.
    pub fn tick_decay(&mut self) {
        if self.decay > 0 {
            self.decay -= 1;
            if self.decay == 0 {
                self.latch = 0xFF;
            }
        }
    }
}

/// The VRAM data bus connects the SoC to video RAM (0x8000–0x9FFF).
/// The bus retains its last driven value as a latch (no decay).
pub struct VramBus {
    pub(super) vram: Vram,
    /// Retained value on the VRAM data bus.
    latch: u8,
}

impl VramBus {
    pub fn new() -> Self {
        Self {
            vram: Vram::new(),
            latch: 0xFF,
        }
    }

    /// Read from VRAM. Does NOT update the latch.
    pub fn read(&self, address: VramAddress) -> u8 {
        self.vram.read(address)
    }

    /// Write to VRAM.
    pub fn write(&mut self, address: VramAddress, value: u8) {
        self.vram.write(address, value);
    }

    /// Update the bus latch to `value`.
    pub fn drive(&mut self, value: u8) {
        self.latch = value;
    }

    /// Return the current latch value.
    pub fn latch(&self) -> u8 {
        self.latch
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
    Vram(VramAddress),
    Oam(OamAddress),
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
            0x8000..=0x9fff => match video::memory::MappedAddress::map(address) {
                video::memory::MappedAddress::Vram(addr) => Self::Vram(addr),
                video::memory::MappedAddress::Oam(_) => unreachable!(),
            },
            0xa000..=0xbfff => Self::Cartridge(address),
            0xc000..=0xdfff => Self::WorkRam(address - 0xc000),
            0xe000..=0xfdff => Self::WorkRam(address - 0xe000),
            0xfe00..=0xfe9f => match video::memory::MappedAddress::map(address) {
                video::memory::MappedAddress::Oam(addr) => Self::Oam(addr),
                video::memory::MappedAddress::Vram(_) => unreachable!(),
            },
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

impl GameBoy {
    /// Read a byte as the CPU sees it, updating the data bus latch.
    ///
    /// This is the "real" CPU read path: it checks DMA bus conflicts,
    /// PPU mode gating, and updates the bus latch on the appropriate
    /// physical bus. Use [`read`] for non-emulation reads (debugger,
    /// tests) that should not mutate bus state.
    pub fn cpu_read(&mut self, address: u16) -> u8 {
        if let Some(bus) = self.dma.is_active_on_bus() {
            // OAM is being written to by DMA; CPU reads return $FF.
            if (0xFE00..=0xFE9F).contains(&address) {
                return 0xFF;
            }
            // Bus conflict: the DMA controller is driving this bus,
            // so the CPU sees whatever value the DMA last placed on
            // it — which is the bus latch.
            if Bus::of(address) == Some(bus) {
                return match bus {
                    Bus::External => self.external.latch(),
                    Bus::Vram => self.vram_bus.latch(),
                };
            }
        }

        // PPU mode-based memory gating: the PPU locks OAM during Mode 2
        // and Mode 3, and locks VRAM during Mode 3. Reads return 0xFF.
        // The bus latch is NOT updated — no device drove the bus.
        let mode = self.video.gating_mode();
        match address {
            0xFE00..=0xFE9F => match mode {
                video::ppu::Mode::PreparingScanline | video::ppu::Mode::DrawingPixels => {
                    return 0xFF;
                }
                _ => {}
            },
            0x8000..=0x9FFF => {
                if mode == video::ppu::Mode::DrawingPixels {
                    return 0xFF;
                }
            }
            _ => {}
        }

        let value = self.read_mapped(MappedAddress::map(address));

        // Update the bus latch for whichever physical bus this address
        // resides on. CPU-internal addresses (OAM, IO, HRAM) are not
        // on either bus and do not update a latch.
        match Bus::of(address) {
            Some(Bus::External) => {
                self.external.drive(value);
            }
            Some(Bus::Vram) => {
                self.vram_bus.drive(value);
            }
            None => {}
        }

        value
    }

    /// Read a byte without side effects.
    ///
    /// Returns the same value as [`cpu_read`] but does NOT update the
    /// data bus latch. Used by the debugger, test helpers, and any
    /// context where a non-emulation peek is needed.
    pub fn read(&self, address: u16) -> u8 {
        if let Some(bus) = self.dma.is_active_on_bus() {
            if (0xFE00..=0xFE9F).contains(&address) {
                return 0xFF;
            }
            if Bus::of(address) == Some(bus) {
                return match bus {
                    Bus::External => self.external.latch(),
                    Bus::Vram => self.vram_bus.latch(),
                };
            }
        }

        let mode = self.video.gating_mode();
        match address {
            0xFE00..=0xFE9F => match mode {
                video::ppu::Mode::PreparingScanline | video::ppu::Mode::DrawingPixels => {
                    return 0xFF;
                }
                _ => {}
            },
            0x8000..=0x9FFF => {
                if mode == video::ppu::Mode::DrawingPixels {
                    return 0xFF;
                }
            }
            _ => {}
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
            MappedAddress::Cartridge(_) | MappedAddress::WorkRam(_) => self.external.read(address),
            MappedAddress::HighRam(offset) => self.high_ram[offset as usize],
            MappedAddress::Vram(address) => self.vram_bus.read(address),
            MappedAddress::Oam(address) => self.video.read_oam(address),
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
            MappedAddress::BeginDmaTransfer => self.dma.source_register(),

            MappedAddress::Unmapped => 0xFF,
        }
    }

    /// Trigger OAM bug write corruption if the address is in the OAM
    /// range (0xFE00-0xFEFF) and the PPU is in Mode 2.
    pub fn oam_bug_write(&mut self, address: u16) {
        if (0xFE00..=0xFEFF).contains(&address) {
            self.video.oam_bug_write();
        }
    }

    /// Trigger OAM bug read corruption if the address is in the OAM
    /// range (0xFE00-0xFEFF) and the PPU is in Mode 2.
    pub fn oam_bug_read(&mut self, address: u16) {
        if (0xFE00..=0xFEFF).contains(&address) {
            self.video.oam_bug_read();
        }
    }

    pub fn write_byte(&mut self, address: u16, value: u8) {
        if let Some(bus) = self.dma.is_active_on_bus() {
            // OAM is being written to by DMA; CPU writes are ignored.
            if (0xFE00..=0xFE9F).contains(&address) {
                return;
            }
            // Bus conflict: CPU writes on the same bus as DMA are ignored.
            // The bus latch is NOT updated — DMA is driving the bus.
            if Bus::of(address) == Some(bus) {
                return;
            }
        }

        // PPU mode-based memory gating for writes. Writes use different
        // timing than reads: no early OAM/VRAM locks, and mode 2 releases
        // OAM 4 dots early (at dot 76).
        // The bus latch is NOT updated — the write was blocked.
        let mode = self.video.write_gating_mode();
        match address {
            0xFE00..=0xFE9F => match mode {
                video::ppu::Mode::PreparingScanline | video::ppu::Mode::DrawingPixels => {
                    return;
                }
                _ => {}
            },
            0x8000..=0x9FFF => {
                if mode == video::ppu::Mode::DrawingPixels {
                    return;
                }
            }
            _ => {}
        }

        // Update the bus latch to the written value. The CPU drives
        // the data bus with the value it's writing, regardless of
        // whether the target device actually stores it.
        match Bus::of(address) {
            Some(Bus::External) => {
                self.external.drive(value);
            }
            Some(Bus::Vram) => {
                self.vram_bus.drive(value);
            }
            None => {}
        }

        self.write_mapped(MappedAddress::map(address), value);
    }

    pub fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Cartridge(_) | MappedAddress::WorkRam(_) => {
                self.external.write(address, value)
            }
            MappedAddress::HighRam(offset) => self.high_ram[offset as usize] = value,
            MappedAddress::Vram(address) => self.vram_bus.write(address, value),
            MappedAddress::Oam(address) => self.video.write_oam(address, value),
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
                if matches!(register, timers::Register::Divider) {
                    let old_counter = self.timers.internal_counter();
                    self.timers.write_register(register, value);
                    self.audio.on_div_write(old_counter);
                } else {
                    self.timers.write_register(register, value);
                }
            }
            MappedAddress::AudioRegister(register) => self.audio.write_register(register, value),
            MappedAddress::AudioWaveRam(offset) => self.audio.write_wave_ram(offset, value),
            MappedAddress::VideoRegister(register) => {
                self.video
                    .write_register(register, value, &self.vram_bus.vram)
            }
            MappedAddress::BeginDmaTransfer => self.dma.begin_transfer(value),
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
}
