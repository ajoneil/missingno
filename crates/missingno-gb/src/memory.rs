use crate::{
    Console, Model, audio,
    cpu_bus::{BusAccess, BusAccessKind},
    dmg_sram,
    interrupts::{self, InterruptFlags},
    ppu, serial_transfer, timers,
};

use super::cartridge::Cartridge;
use ppu::memory::{OamAddress, Vram, VramAddress};

/// M-cycles before the external data bus decays to 0xFF. The bus
/// retains its last driven value via parasitic capacitance; 12 M-cycles
/// (~2.86 µs) is a board-independent approximation.
const EXTERNAL_BUS_DECAY_MCYCLES: u8 = 12;

/// High RAM (0xFF80–0xFFFE): 127 bytes of SoC-internal SRAM. Not on
/// either bus — always accessible to the CPU, even during OAM DMA.
pub struct HighRam([u8; 0x7F]);

impl HighRam {
    pub fn new() -> Self {
        Self([0; 0x7F])
    }

    pub fn from_bytes(data: &[u8]) -> Self {
        let mut hram = Self([0; 0x7F]);
        let len = data.len().min(0x7F);
        hram.0[..len].copy_from_slice(&data[..len]);
        hram
    }

    pub fn read(&self, offset: u8) -> u8 {
        self.0[offset as usize]
    }

    pub fn write(&mut self, offset: u8, value: u8) {
        self.0[offset as usize] = value;
    }

    pub fn data(&self) -> &[u8; 0x7F] {
        &self.0
    }
}

/// Address on the external data bus: cartridge or work RAM.
#[derive(Debug)]
pub enum ExternalAddress {
    Cartridge(u16),
    WorkRam(u16),
}

/// A boot ROM and the address range it overlays over the cartridge while
/// mapped. The variant carries the layout: the DMG boot ROM is 256 bytes
/// over 0x0000–0x00FF; the CGB boot ROM is 2304 bytes over 0x0000–0x00FF
/// AND 0x0200–0x08FF, leaving the 0x0100–0x01FF cartridge-header window
/// visible so the boot ROM can read the header it inspects.
pub enum BootRom {
    Dmg(Box<[u8; 0x100]>),
    Cgb(Box<[u8; 0x900]>),
}

impl BootRom {
    /// The boot-ROM byte overlaying `addr`, or `None` if `addr` shows the
    /// cartridge through (always, or the CGB header window).
    fn overlay_byte(&self, addr: u16) -> Option<u8> {
        match self {
            BootRom::Dmg(rom) if addr <= 0x00FF => Some(rom[addr as usize]),
            BootRom::Cgb(rom) if addr <= 0x00FF || (0x0200..=0x08FF).contains(&addr) => {
                Some(rom[addr as usize])
            }
            _ => None,
        }
    }
}

/// The external data bus connects the SoC to the cartridge and (on
/// DMG) to work RAM. The bus retains its last driven value through
/// parasitic capacitance, decaying toward 0xFF when idle.
pub struct ExternalBus {
    pub cartridge: Cartridge,
    pub(crate) work_ram: [u8; 0x2000],

    /// Retained value on the data bus. Updated on every CPU read/write
    /// to an external-bus address and by DMA when reading from this bus.
    pub(crate) latch: u8,
    /// M-cycles remaining before `latch` decays to 0xFF.
    pub(crate) decay: u8,

    /// Boot ROM (DMG or CGB). When present and `boot_rom_mapped` is true,
    /// reads from the overlaid range return boot ROM data instead of
    /// cartridge ROM.
    boot_rom: Option<BootRom>,
    /// True while the boot ROM overlay is active. Cleared by writing
    /// to 0xFF50.
    boot_rom_mapped: bool,
}

impl ExternalBus {
    pub fn new(cartridge: Cartridge, boot_rom: Option<BootRom>) -> Self {
        let boot_rom_mapped = boot_rom.is_some();
        let mut work_ram = [0; 0x2000];
        dmg_sram::fill(&mut work_ram);
        Self {
            cartridge,
            work_ram,
            latch: 0xFF,
            decay: 0,
            boot_rom,
            boot_rom_mapped,
        }
    }

    /// Read from a device on this bus (cartridge or WRAM). Does NOT
    /// update the latch — callers decide when to latch.
    pub fn read(&self, address: ExternalAddress) -> u8 {
        match address {
            ExternalAddress::Cartridge(addr) if self.boot_rom_mapped => self
                .boot_rom
                .as_ref()
                .and_then(|rom| rom.overlay_byte(addr))
                .unwrap_or_else(|| self.cartridge.read(addr)),
            ExternalAddress::Cartridge(addr) => self.cartridge.read(addr),
            ExternalAddress::WorkRam(addr) => self.work_ram[addr as usize],
        }
    }

    pub fn has_boot_rom(&self) -> bool {
        self.boot_rom.is_some()
    }

    pub fn boot_rom_mapped(&self) -> bool {
        self.boot_rom_mapped
    }

    pub fn unmap_boot_rom(&mut self) {
        self.boot_rom_mapped = false;
    }

    /// Reset volatile state for a power-cycle: clear WRAM (filled with
    /// the same DMG SRAM pattern as a fresh power-on), clear the data-
    /// bus latch and decay timer, and re-map the boot ROM if present.
    /// Preserves the cartridge (including its MBC/SRAM state) and the
    /// boot ROM contents.
    pub fn reset(&mut self) {
        self.work_ram = [0; 0x2000];
        dmg_sram::fill(&mut self.work_ram);
        self.latch = 0xFF;
        self.decay = 0;
        self.boot_rom_mapped = self.boot_rom.is_some();
    }

    pub fn write(&mut self, address: ExternalAddress, value: u8) {
        match address {
            ExternalAddress::Cartridge(addr) => self.cartridge.write(addr, value),
            ExternalAddress::WorkRam(addr) => self.work_ram[addr as usize] = value,
        }
    }

    /// Drive `value` onto the bus latch and reset the decay counter.
    pub fn drive(&mut self, value: u8) {
        self.latch = value;
        self.decay = EXTERNAL_BUS_DECAY_MCYCLES;
    }

    pub fn latch(&self) -> u8 {
        self.latch
    }

    /// Tick the decay counter once per M-cycle. When it reaches zero
    /// the latch falls back to 0xFF.
    pub fn tick_decay(&mut self) {
        if self.decay > 0 {
            self.decay -= 1;
            if self.decay == 0 {
                self.latch = 0xFF;
            }
        }
    }

    /// Advance the cartridge RTC (if any) by `dots` of master-clock time.
    pub fn tick_rtc(&mut self, dots: u32) {
        self.cartridge.tick_rtc(dots);
    }
}

/// The VRAM data bus connects the SoC to video RAM (0x8000–0x9FFF).
/// The bus retains its last driven value as a latch (no decay).
pub struct VramBus<V: Vram> {
    pub vram: V,
    /// Retained value on the VRAM data bus.
    pub latch: u8,
}

impl<V: Vram> VramBus<V> {
    pub fn new() -> Self {
        Self {
            vram: V::default(),
            latch: 0xFF,
        }
    }

    /// Drive `value` onto the bus latch.
    pub fn drive(&mut self, value: u8) {
        self.latch = value;
    }
}

/// Which physical data bus an address resides on, if any.
///
/// - **External**: ROM, SRAM, WRAM, WRAM echo (0x0000-0x7FFF, 0xA000-0xFDFF)
/// - **Vram**: Video RAM (0x8000-0x9FFF)
///
/// OAM, IO registers, and HRAM are CPU-internal and not on either bus.
/// During OAM DMA the controller occupies one bus and the CPU can
/// still freely access the other.
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
            0x8000..=0x9FFF => Some(Bus::Vram),
            0x0000..=0x7FFF | 0xA000..=0xFDFF => Some(Bus::External),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum MappedAddress {
    External(ExternalAddress),
    HighRam(u8),
    Vram(VramAddress),
    Oam(OamAddress),
    /// Region $FEA0–$FEFF, just above OAM. Not connected to any
    /// storage on DMG — reads return 0x00, writes are dropped.
    OamExtra,
    JoypadRegister,
    SerialTransferRegister(serial_transfer::Register),
    TimerRegister(timers::Register),
    InterruptRegister(interrupts::Register),
    AudioRegister(audio::Register),
    AudioWaveRam(u8),
    AudioPcm12,
    AudioPcm34,
    PpuRegister(ppu::Register),
    BeginDmaTransfer,
    BootRomUnmap,
    Unmapped,
}

impl MappedAddress {
    pub fn map(address: u16) -> Self {
        match address {
            0x0000..=0x7fff => Self::External(ExternalAddress::Cartridge(address)),
            0x8000..=0x9fff => match ppu::memory::MappedAddress::map(address) {
                ppu::memory::MappedAddress::Vram(addr) => Self::Vram(addr),
                ppu::memory::MappedAddress::Oam(_) => unreachable!(),
            },
            0xa000..=0xbfff => Self::External(ExternalAddress::Cartridge(address)),
            0xc000..=0xdfff => Self::External(ExternalAddress::WorkRam(address - 0xc000)),
            0xe000..=0xfdff => Self::External(ExternalAddress::WorkRam(address - 0xe000)),
            0xfe00..=0xfe9f => match ppu::memory::MappedAddress::map(address) {
                ppu::memory::MappedAddress::Oam(addr) => Self::Oam(addr),
                ppu::memory::MappedAddress::Vram(_) => unreachable!(),
            },
            0xfea0..=0xfeff => Self::OamExtra,
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
            0xff40 => Self::PpuRegister(ppu::Register::Control),
            0xff41 => Self::PpuRegister(ppu::Register::Status),
            0xff42 => Self::PpuRegister(ppu::Register::BackgroundViewportY),
            0xff43 => Self::PpuRegister(ppu::Register::BackgroundViewportX),
            0xff44 => Self::PpuRegister(ppu::Register::CurrentScanline),
            0xff45 => Self::PpuRegister(ppu::Register::InterruptOnScanline),
            0xff46 => Self::BeginDmaTransfer,
            0xff47 => Self::PpuRegister(ppu::Register::BackgroundPalette),
            0xff48 => Self::PpuRegister(ppu::Register::Sprite0Palette),
            0xff49 => Self::PpuRegister(ppu::Register::Sprite1Palette),
            0xff4a => Self::PpuRegister(ppu::Register::WindowY),
            0xff4b => Self::PpuRegister(ppu::Register::WindowX),
            // $FF4F (VBK), $FF68-6B (CRAM), $FF6C (OPRI) are CGB registers — the
            // CGB model map resolves them; the DMG base map leaves them unmapped.
            0xff4c..=0xff4f => Self::Unmapped,
            0xff50 => Self::BootRomUnmap,
            // PCM12/PCM34 exist only on CGB silicon (HAS_PCM_REGISTERS).
            0xff76 => Self::AudioPcm12,
            0xff77 => Self::AudioPcm34,
            0xff51..=0xff75 | 0xff78..=0xff7f => Self::Unmapped,
            0xff80..=0xfffe => Self::HighRam((address - 0xff80) as u8),
            0xffff => Self::InterruptRegister(interrupts::Register::EnabledInterrupts),
        }
    }
}

impl<M: Model> Console<M> {
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

    /// Read a byte without side effects. Same value as a real CPU read
    /// would see, but the bus latch is not updated. Used by the
    /// debugger, test helpers, and any non-emulation peek.
    pub fn read(&self, address: u16) -> u8 {
        if let Some(value) = self.dma_read_conflict(address) {
            return value;
        }
        if self.ppu.read_locked(address) {
            return 0xFF;
        }
        self.read_addr(address)
    }

    /// Resolve an address to a value: this console's own memory map (the
    /// model's CGB registers) first, then the shared `MappedAddress` map.
    /// The single chokepoint for the model-before-shared-map contract.
    fn read_addr(&self, address: u16) -> u8 {
        if let Some(value) = self.model.map_read(address, &self.ppu, &self.vram_bus.vram) {
            return value;
        }
        self.read_mapped(MappedAddress::map(address))
    }

    /// Read a byte bypassing all bus conflicts and PPU mode gating.
    /// Used by the debugger to inspect memory that would normally be
    /// hidden.
    pub fn peek(&self, address: u16) -> u8 {
        self.read_addr(address)
    }

    /// Value the addressed peripheral first drives onto the CPU bus at
    /// the driver-enable edge (tobe↑ / wafu↑ early in T-cycle 2). DMA
    /// bus redirection happens here; the OAM/VRAM lock is a property
    /// of the driver's state at the LATCH edge and is resolved in
    /// `bus_value_at_latch` below.
    pub fn bus_value_at_drive_enable(&self, address: u16) -> u8 {
        if let Some(value) = self.dma_read_conflict(address) {
            return value;
        }
        self.read_addr(self.dma_wram_remapped(address))
    }

    /// A WRAM-bus access taken during an active OAM-DMA sourcing from the cart
    /// bus is address-remapped by the DMA driving the bus (CGB only). Shared by
    /// the read (drive-enable) and write (commit) paths.
    fn dma_wram_remapped(&self, address: u16) -> u16 {
        if self.dma.is_active_on_bus().is_some()
            && let Some(remapped) = self.model.oam_dma_wram_remap(address, self.dma.source())
        {
            return remapped;
        }
        address
    }

    /// Value the CPU latches from the bus at `data_phase_n↑` (near the
    /// end of T-cycle 3). Resolves the drive-enable snapshot against
    /// per-address mid-M-cycle flux: OAM/VRAM lock (full-byte 0xFF
    /// override when the access-control gates assert at the latch
    /// edge) and STAT/LY per-bit flux (NOT_IF0 / NOT_IF1 driver
    /// settling within the drive window).
    pub fn bus_value_at_latch(&self, address: u16, snapshot: u8, ly_at_latch: Option<u8>) -> u8 {
        match address {
            // OAM/VRAM read locks: the on-chip OAM / off-chip VRAM
            // drivers tri-state at the latch edge, so the bus floats
            // high (0xFF).
            _ if self.ppu.read_locked(address) => 0xFF,

            // LY: the byte fluxes via NOT_IF0 drivers fed by the
            // RUTU-clocked ripple counter. A latch coincident with the
            // counter edge catches the ripple mid-settle: falling bits
            // are visible, rising bits not yet ("0 wins"), so the read
            // resolves to pre AND post.
            0xFF44 => ly_at_latch.unwrap_or(snapshot) & self.read(address),

            // CH3 wave RAM: under the DMG fetch-strobe coupling the
            // `wave_data_latch` pulse can open and close within the
            // T=2..T=3 read window, so the T=2 drive-enable snapshot
            // captures it where a live latch re-read would float to
            // 0xFF. Under the CGB channel-position coupling the same
            // snapshot pins the read to the T=2 playback byte.
            0xFF30..=0xFF3F => snapshot,

            // STAT bits 0-2 (mode + LYC=LY) drive cpu_port_d via dmg_not_if1
            // cells, resolved "0 wins" (AND with the live latch). The mode-2 bit
            // (WUGA) carries the slow BESU companion-driver contention and reads
            // its settling bus value; the vblank bit (TEBY) and LYC bit (SEGO)
            // resolve fast off the drive-enable snapshot. Bits 3-7 are stable
            // enable outputs.
            0xFF41 => {
                let live = self.read(address);
                const FAST_BITS: u8 = 0b0000_0101;
                let mode2 = if self.ppu.stat_mode2_bus() { 0b0000_0010 } else { 0 };
                (snapshot & !0b0000_0111) | (snapshot & live & FAST_BITS) | mode2
            }

            _ => snapshot,
        }
    }

    /// Read a byte as the DMA controller would. Addresses not on either
    /// bus (OAM, IO, HRAM) are remapped to WRAM echo on the external bus.
    pub fn read_dma_source(&self, address: u16) -> u8 {
        if let Some(value) = self.model.dma_source_open_bus(address) {
            return value;
        }
        if let Some(value) = self.model.map_read(address, &self.ppu, &self.vram_bus.vram) {
            return value;
        }
        let mapped = match Bus::of(address) {
            Some(_) => MappedAddress::map(address),
            None => MappedAddress::External(ExternalAddress::WorkRam(address.wrapping_sub(0xE000))),
        };
        self.read_mapped(mapped)
    }

    /// If DMA is driving a bus that conflicts with `address`, return
    /// the override value the CPU sees: 0xFF for an OAM read during
    /// DMA, otherwise the source byte DMA is about to commit this
    /// M-cycle (the value being driven on the bus right now). Falls
    /// back to the bus latch during DMA's restart-delay window when
    /// no byte will commit this M-cycle.
    fn dma_read_conflict(&self, address: u16) -> Option<u8> {
        let bus = self.dma.is_active_on_bus()?;
        if (0xFE00..=0xFEFF).contains(&address) {
            return Some(0xFF);
        }
        if !self.model.oam_dma_bus_conflict(address, self.dma.source()) {
            return None;
        }
        Some(match self.dma.peek_transfer() {
            Some((src, _)) => self.read_dma_source(src),
            None => match bus {
                Bus::External => self.external.latch(),
                Bus::Vram => self.vram_bus.latch,
            },
        })
    }

    /// Drive `value` onto whichever physical bus `address` resides on.
    /// CPU-internal addresses (OAM, IO, HRAM) don't update a latch.
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
            MappedAddress::Vram(address) => self.vram_bus.vram.cpu_read(address),
            MappedAddress::Oam(address) => self.ppu.read_oam(address),
            MappedAddress::JoypadRegister => self.model.read_joypad(self.joypad.read_register()),
            MappedAddress::SerialTransferRegister(register) => match register {
                serial_transfer::Register::Data => self.serial.registers.data,
                serial_transfer::Register::Control => self
                    .serial
                    .registers
                    .read_control(self.model.has_serial_fast_clock()),
            },
            MappedAddress::TimerRegister(register) => self.timers.read_register(register),
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => self.interrupts.enabled.bits(),
                interrupts::Register::RequestedInterrupts => {
                    self.interrupts.requested.bits() | 0xE0
                }
            },
            MappedAddress::AudioRegister(register) => self.audio.read_register(register),
            MappedAddress::AudioWaveRam(offset) => {
                self.audio.read_wave_ram(offset, M::WAVE_RAM_COUPLING)
            }
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
            MappedAddress::AudioPcm12 => {
                if M::HAS_PCM_REGISTERS {
                    self.audio.pcm12()
                } else {
                    0xFF
                }
            }
            MappedAddress::AudioPcm34 => {
                if M::HAS_PCM_REGISTERS {
                    self.audio.pcm34()
                } else {
                    0xFF
                }
            }
            MappedAddress::Unmapped => 0xFF,
        }
    }

    /// CPU write pulse drives the data bus during T-cycles 2-3. PPU
    /// register DFF cells latch combinationally during this window.
    /// Returns true if the write triggered a STAT interrupt (FF41
    /// write quirk).
    pub fn drive_ppu_bus(&mut self, address: u16, value: u8) -> bool {
        if let MappedAddress::PpuRegister(register) = MappedAddress::map(address) {
            let halt_wake_active = self.cpu.is_halt_wake_active();
            self.ppu.write_register(register, value, halt_wake_active)
        } else {
            false
        }
    }

    /// CPU bus write commit. `locked_at_snapshot` / `locked_at_mid` are
    /// the OAM/VRAM lock states sampled at CUPA-rising (rise of T-cycle
    /// 2) and mid-CUPA (fall of T-cycle 2 after AVAP). The commit-time
    /// live lock is read here. The write is blocked iff locked at ALL
    /// THREE samples — modelling hardware's "AJUJ high at ANY edge
    /// during CUPA strobes the per-byte write." `None` lock samples
    /// mean a non-CUPA write path; the live lock alone decides.
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
        if self.dma.is_active_on_bus().is_some() {
            // The OAM block (extra rows included) is being written by DMA;
            // CPU writes are ignored.
            if (0xFE00..=0xFEFF).contains(&address) {
                return;
            }
            // Source-bus conflict: the CPU's write strobe collides with
            // DMA's on the source bus. Stash the CPU value and the source
            // byte DMA fetched this M-cycle so the M-cycle-boundary commit
            // can land the model's resolved byte at the OAM slot DMA is
            // depositing. The CPU also drives the bus latch.
            if self.model.oam_dma_bus_conflict(address, self.dma.source()) {
                if let Some((src_addr, dst_offset)) = self.dma.peek_transfer() {
                    if self
                        .model
                        .oam_dma_conflict_zeroes_oam(address, self.dma.source())
                    {
                        self.dma_conflict_oam_zero = Some(dst_offset);
                    } else {
                        let src_byte = self.read_dma_source(src_addr);
                        self.dma_conflict_write_pending = Some((dst_offset, src_byte, value));
                    }
                }
                self.drive_bus(address, value);
                return;
            }
        }

        // PPU mode gating: block if locked at all three CUPA samples
        // (snapshot at rise of T-cycle 2, mid at fall of T-cycle 2
        // after AVAP, live at fall of T-cycle 3). Models the AJUJ-
        // glitch window for OAM at Mode 2→3 straddles.
        if let Some(locked_now) = self.ppu.write_lock(address) {
            let blocked = match (locked_at_snapshot, locked_at_mid) {
                (Some(snap), Some(mid)) => snap && mid && locked_now,
                _ => locked_now,
            };
            if blocked {
                return;
            }
        }

        // The CPU drives the data bus with the value it's writing,
        // regardless of whether the target device stores it.
        self.drive_bus(address, value);

        // The byte stores into the DMA-remapped WRAM cell when the CPU writes
        // the WRAM bus during a cart-source DMA; the bus drive above keeps the
        // CPU's original address.
        let store = self.dma_wram_remapped(address);
        if self
            .model
            .map_write(store, value, &mut self.ppu, &mut self.vram_bus.vram)
        {
            return;
        }

        let mapped = MappedAddress::map(store);
        if !matches!(mapped, MappedAddress::PpuRegister(_)) {
            self.write_mapped(mapped, value);
        }
    }

    fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::External(addr) => self.external.write(addr, value),
            MappedAddress::HighRam(offset) => self.high_ram.write(offset, value),
            MappedAddress::Vram(address) => self.vram_bus.vram.cpu_write(address, value),
            MappedAddress::Oam(address) => self.ppu.write_oam(address, value),
            MappedAddress::JoypadRegister => {
                self.model.on_joypad_write(value);
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
                    if let Some(interrupt) = self
                        .serial
                        .on_div_write(old_counter, self.model.has_serial_fast_clock())
                    {
                        self.interrupts.request(interrupt);
                    }
                } else {
                    self.timers.write_register(register, value);
                }
            }
            MappedAddress::AudioRegister(register) => {
                self.audio
                    .write_register(register, value, self.timers.internal_counter())
            }
            MappedAddress::AudioWaveRam(offset) => {
                self.audio
                    .write_wave_ram(offset, value, M::WAVE_RAM_COUPLING)
            }
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
            MappedAddress::AudioPcm12 | MappedAddress::AudioPcm34 => {}
            MappedAddress::Unmapped => {}
        }
    }
}
