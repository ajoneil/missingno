//! Memory access helpers for `GameBoyColor`.
//!
//! Copied from `missingno-gb`'s `impl GameBoy` block with the SGB
//! joypad-multiplexing branches removed (CGB has no SGB co-processor).
//! Routing for CGB-specific registers (`$FF4F` VRAM bank, `$FF70` WRAM
//! bank, `$FF68-6B` palette I/O, `$FF51-55` HDMA, `KEY1`) will land here
//! when those features are implemented.

use missingno_gb::{
    cpu_bus::{BusAccess, BusAccessKind},
    interrupts::{self, InterruptFlags},
    memory::{Bus, ExternalAddress, MappedAddress},
    serial_transfer, timers,
};

use crate::GameBoyColor;

impl GameBoyColor {
    /// Apply the side effects of a CPU bus read whose value was already
    /// captured into `cpu_bus.data` at the driver-enable edge: drive
    /// the appropriate physical bus latch and record the read in the
    /// trace. Called at the CPU's data-latch edge.
    pub fn commit_bus_read(&mut self, address: u16, value: u8) {
        self.bus_trace.record(BusAccess {
            address,
            value,
            kind: BusAccessKind::Read,
        });
        self.drive_bus(address, value);
    }

    /// Read a byte without side effects.
    pub fn read(&self, address: u16) -> u8 {
        if let Some(value) = self.dma_read_conflict(address) {
            return value;
        }
        if self.ppu.read_locked(address) {
            return 0xFF;
        }
        self.read_mapped(MappedAddress::map(address))
    }

    /// Read a byte bypassing all bus conflicts and PPU mode gating.
    pub fn peek(&self, address: u16) -> u8 {
        self.read_mapped(MappedAddress::map(address))
    }

    pub fn bus_value_at_drive_enable(&self, address: u16) -> u8 {
        if let Some(value) = self.dma_read_conflict(address) {
            return value;
        }
        self.read_mapped(MappedAddress::map(address))
    }

    pub fn bus_value_at_latch(&self, address: u16, snapshot: u8) -> u8 {
        match address {
            _ if self.ppu.read_locked(address) => 0xFF,
            0xFF44 => self.read(address),
            0xFF41 => {
                let live = self.read(address);
                const X_WINDOW: u8 = 0b0000_0111;
                (snapshot & !X_WINDOW) | (snapshot & live & X_WINDOW)
            }
            // CH3 wave RAM: re-read live at latch so wave_data_latch
            // windows opening after drive-enable (§14.8.4) are caught.
            0xFF30..=0xFF3F => self.read(address),
            _ => snapshot,
        }
    }

    pub fn read_dma_source(&self, address: u16) -> u8 {
        let mapped = match Bus::of(address) {
            Some(_) => MappedAddress::map(address),
            None => MappedAddress::External(ExternalAddress::WorkRam(address.wrapping_sub(0xE000))),
        };
        self.read_mapped(mapped)
    }

    fn dma_read_conflict(&self, address: u16) -> Option<u8> {
        let bus = self.dma.is_active_on_bus()?;
        if (0xFE00..=0xFE9F).contains(&address) {
            return Some(0xFF);
        }
        if Bus::of(address) == Some(bus) {
            return Some(match bus {
                Bus::External => self.external.latch(),
                Bus::Vram => self.vram_bus.latch,
            });
        }
        None
    }

    fn drive_bus(&mut self, address: u16, value: u8) {
        match Bus::of(address) {
            Some(Bus::External) => self.external.drive(value),
            Some(Bus::Vram) => self.vram_bus.drive(value),
            None => {}
        }
    }

    fn read_mapped(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::External(addr) => self.external.read(addr),
            MappedAddress::HighRam(offset) => self.high_ram.read(offset),
            MappedAddress::Vram(address) => self.vram_bus.vram.read(address),
            MappedAddress::Oam(address) => self.ppu.read_oam(address),
            MappedAddress::JoypadRegister => self.joypad.read_register(),
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.registers.data,
                serial_transfer::Register::Control => self.serial.registers.control.bits() | 0x7E,
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
            MappedAddress::PpuRegister(register) => self.ppu.read_register(register),
            MappedAddress::BeginDmaTransfer => self.dma.source_register(),
            MappedAddress::BootRomUnmap => {
                if self.external.boot_rom_mapped() {
                    0xFE
                } else {
                    0xFF
                }
            }
            MappedAddress::OamExtra => 0x00,
            MappedAddress::Unmapped => 0xFF,
        }
    }

    /// CPU write pulse drives the data bus during T-cycles 2-3. PPU
    /// register DFF cells latch combinationally during this window.
    /// Returns true if the write triggered a STAT interrupt.
    pub fn drive_ppu_bus(&mut self, address: u16, value: u8) -> bool {
        if let MappedAddress::PpuRegister(register) = MappedAddress::map(address) {
            let halt_wake_active = self.cpu.is_halt_wake_active();
            self.ppu.write_register(register, value, halt_wake_active)
        } else {
            false
        }
    }

    /// CPU bus write commit. `locked_at_snapshot` / `locked_at_mid` are
    /// the OAM/VRAM lock states sampled at CUPA-rising and mid-CUPA.
    pub fn write_byte_with_cupa_lock(
        &mut self,
        address: u16,
        value: u8,
        locked_at_snapshot: Option<bool>,
        locked_at_mid: Option<bool>,
    ) {
        self.bus_trace.record(BusAccess {
            address,
            value,
            kind: BusAccessKind::Write,
        });
        if let Some(bus) = self.dma.is_active_on_bus() {
            if (0xFE00..=0xFE9F).contains(&address) {
                return;
            }
            if Bus::of(address) == Some(bus) {
                return;
            }
        }

        if let Some(locked_now) = self.ppu.write_lock(address) {
            let blocked = match (locked_at_snapshot, locked_at_mid) {
                (Some(snap), Some(mid)) => snap && mid && locked_now,
                _ => locked_now,
            };
            if blocked {
                return;
            }
        }

        self.drive_bus(address, value);

        let mapped = MappedAddress::map(address);
        if !matches!(mapped, MappedAddress::PpuRegister(_)) {
            self.write_mapped(mapped, value);
        }
    }

    fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::External(addr) => self.external.write(addr, value),
            MappedAddress::HighRam(offset) => self.high_ram.write(offset, value),
            MappedAddress::Vram(address) => self.vram_bus.vram.write(address, value),
            MappedAddress::Oam(address) => self.ppu.write_oam(address, value),
            MappedAddress::JoypadRegister => {
                let before = self.joypad.input_lines();
                self.joypad.write_register(value);
                if before & !self.joypad.input_lines() != 0 {
                    self.interrupts.request(interrupts::Interrupt::Joypad);
                }
            }
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.registers.data = value,
                serial_transfer::Register::Control => {
                    self.serial.registers.control =
                        serial_transfer::Control::from_bits_retain(value);
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
            MappedAddress::PpuRegister(register) => {
                let halt_wake_active = self.cpu.is_halt_wake_active();
                if self.ppu.write_register(register, value, halt_wake_active) {
                    self.interrupts
                        .requested
                        .insert(InterruptFlags::VIDEO_STATUS);
                }
            }
            MappedAddress::BeginDmaTransfer => self.dma.begin_transfer(value),
            MappedAddress::BootRomUnmap => {
                if value & 0x01 != 0 {
                    self.external.unmap_boot_rom();
                }
            }
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => {
                    self.interrupts.enabled = InterruptFlags::from_bits_retain(value)
                }
                interrupts::Register::RequestedInterrupts => {
                    self.interrupts.requested = InterruptFlags::from_bits_retain(value);
                }
            },

            MappedAddress::OamExtra => {}
            MappedAddress::Unmapped => {}
        }
    }
}

