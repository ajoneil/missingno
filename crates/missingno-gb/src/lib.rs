use crate::audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use cpu::mcycle::{BusDot, DotAction};
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{ExternalBus, HighRam, VramBus};
use ppu::{Ppu, screen::Screen};

pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod debugger;
pub mod dma;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod ppu;
pub mod recording;
pub mod serial_transfer;
pub mod sgb;
pub mod timers;
#[cfg(feature = "gbtrace")]
pub mod snapshot;
#[cfg(feature = "gbtrace")]
pub mod trace;

/// Master clock signal level. The clock alternates High → Low
/// uniformly. Edge logic runs at transitions: `rise()` at the
/// Low→High edge, `fall()` at the High→Low edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPhase {
    High,
    Low,
}

impl ClockPhase {
    pub fn next(self) -> ClockPhase {
        match self {
            ClockPhase::High => ClockPhase::Low,
            ClockPhase::Low => ClockPhase::High,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusAccessKind {
    Read,
    Write,
    DmaRead,
    DmaWrite,
}

#[derive(Clone, Copy, Debug)]
pub struct BusAccess {
    pub address: u16,
    pub value: u8,
    pub kind: BusAccessKind,
}

pub struct GameBoy {
    cpu: Cpu,
    screen: Screen,

    external: ExternalBus,
    high_ram: HighRam,
    ppu: Ppu,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
    timers: timers::Timers,
    dma: Dma,
    sgb: Option<sgb::Sgb>,
    vram_bus: VramBus,

    /// Last read value from the bus, persisted across dots for step_dot().
    last_read_value: u8,
    bus_trace: Option<Vec<BusAccess>>,

    /// Master clock phase — alternates Rising/Falling uniformly.
    clock_phase: ClockPhase,
    /// Action for the current dot, set on Rising and consumed during Falling.
    current_dot_action: DotAction,
    /// Dot position for the current dot, set on Rising and consumed during Falling.
    current_dot: BusDot,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge, boot_rom: Option<Box<[u8; 256]>>) -> GameBoy {
        let has_boot_rom = boot_rom.is_some();

        let cpu = if has_boot_rom {
            Cpu::power_on()
        } else {
            Cpu::new(cartridge.header_checksum())
        };
        let sgb = if cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };

        let mut gb = GameBoy {
            cpu,
            screen: Screen::default(),
            external: ExternalBus::new(cartridge, boot_rom),
            high_ram: HighRam::new(),
            ppu: if has_boot_rom {
                Ppu::power_on()
            } else {
                Ppu::new()
            },
            audio: if has_boot_rom {
                Audio::power_on()
            } else {
                Audio::new()
            },
            joypad: Joypad::new(),
            interrupts: interrupts::Registers::new(),
            serial: serial_transfer::Registers::new(),
            timers: if has_boot_rom {
                timers::Timers::power_on()
            } else {
                timers::Timers::new()
            },
            dma: Dma::new(),
            sgb,
            vram_bus: VramBus::new(),
            last_read_value: 0,
            bus_trace: None,
            clock_phase: ClockPhase::Low,
            current_dot_action: DotAction::Idle,
            current_dot: BusDot::ZERO,
        };
        if !has_boot_rom {
            gb.init_post_boot_vram();
        }
        gb
    }

    pub fn reset(&mut self) {
        let has_boot_rom = self.external.has_boot_rom();
        if has_boot_rom {
            self.cpu = Cpu::power_on();
            self.external.remap_boot_rom();
        } else {
            self.cpu = Cpu::new(self.external.cartridge.header_checksum());
        }
        self.screen = Screen::default();
        self.external.work_ram = [0; 0x2000];
        self.external.latch = 0xFF;
        self.external.decay = 0;
        self.high_ram = HighRam::new();
        self.ppu = if has_boot_rom {
            Ppu::power_on()
        } else {
            Ppu::new()
        };
        self.audio = if has_boot_rom {
            Audio::power_on()
        } else {
            Audio::new()
        };
        self.joypad = Joypad::new();
        self.interrupts = interrupts::Registers::new();
        self.serial = serial_transfer::Registers::new();
        self.timers = if has_boot_rom {
            timers::Timers::power_on()
        } else {
            timers::Timers::new()
        };
        self.dma = Dma::new();
        self.vram_bus = VramBus::new();
        self.sgb = if self.external.cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };
        if !has_boot_rom {
            self.init_post_boot_vram();
        }
        self.bus_trace = None;
        self.clock_phase = ClockPhase::Low;
        self.current_dot_action = DotAction::Idle;
        self.current_dot = BusDot::ZERO;
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.external.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    pub fn vram(&self) -> &ppu::memory::Vram {
        &self.vram_bus.vram
    }

    /// Read a contiguous range of memory via peek (bypasses bus conflicts).
    pub fn peek_range(&self, start: u16, len: u16) -> Vec<u8> {
        (0..len).map(|i| self.peek(start.wrapping_add(i))).collect()
    }

    pub fn ppu_mut(&mut self) -> &mut Ppu {
        &mut self.ppu
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    pub fn audio_mut(&mut self) -> &mut Audio {
        &mut self.audio
    }

    pub fn clock_phase(&self) -> ClockPhase {
        self.clock_phase
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// The bus action for the most recently completed dot.
    /// Use after `step_phase()` to detect memory writes (e.g. VRAM writes).
    pub fn last_dot_action(&self) -> &cpu::mcycle::DotAction {
        &self.current_dot_action
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.audio.drain_samples()
    }

    pub fn press_button(&mut self, button: Button) {
        self.joypad.press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.joypad.release_button(button);
    }

    pub fn timers(&self) -> &timers::Timers {
        &self.timers
    }

    pub fn timers_mut(&mut self) -> &mut timers::Timers {
        &mut self.timers
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.interrupts
    }

    pub fn interrupts_mut(&mut self) -> &mut interrupts::Registers {
        &mut self.interrupts
    }

    pub fn dma(&self) -> &Dma {
        &self.dma
    }

    pub fn dma_mut(&mut self) -> &mut Dma {
        &mut self.dma
    }

    pub fn serial(&self) -> &serial_transfer::Registers {
        &self.serial
    }

    pub fn serial_mut(&mut self) -> &mut serial_transfer::Registers {
        &mut self.serial
    }

    pub fn joypad(&self) -> &Joypad {
        &self.joypad
    }

    pub fn joypad_mut(&mut self) -> &mut Joypad {
        &mut self.joypad
    }

    pub fn external_bus(&self) -> &ExternalBus {
        &self.external
    }

    pub fn external_bus_mut(&mut self) -> &mut ExternalBus {
        &mut self.external
    }

    pub fn high_ram(&self) -> &HighRam {
        &self.high_ram
    }

    pub fn high_ram_mut(&mut self) -> &mut HighRam {
        &mut self.high_ram
    }

    pub fn vram_bus(&self) -> &VramBus {
        &self.vram_bus
    }

    pub fn vram_bus_mut(&mut self) -> &mut VramBus {
        &mut self.vram_bus
    }

    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.sgb.as_ref()
    }

    #[allow(dead_code)]
    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.serial.output)
    }

    /// Populate VRAM with the data the DMG boot ROM would have left:
    /// decompressed Nintendo logo tiles (1-24), ® symbol (tile 25),
    /// and tile map entries for the logo display.
    fn init_post_boot_vram(&mut self) {
        use ppu::types::tiles::TileIndex;

        // 1. Decompress Nintendo logo from cartridge header (0x0104-0x0133)
        // into tiles 1-24 in tile block 0.
        //
        // Each of the 48 logo bytes contains two nibbles. Each nibble is
        // expanded horizontally (each bit doubled to 2 pixels = 1 byte)
        // and vertically (each row written twice), producing 4 VRAM bytes
        // per nibble (low bitplane only, high bitplane stays zero).
        let mut vram_offset: usize = 0x10; // tile 1 starts at byte 16
        for addr in 0x0104u16..=0x0133 {
            let logo_byte = self.external.cartridge.read(addr);
            for &nibble in &[logo_byte >> 4, logo_byte & 0x0F] {
                let expanded = (((nibble >> 3) & 1) * 0xC0)
                    | (((nibble >> 2) & 1) * 0x30)
                    | (((nibble >> 1) & 1) * 0x0C)
                    | ((nibble & 1) * 0x03);
                // Row A: low bitplane
                self.vram_bus.vram.tiles[0].data[vram_offset] = expanded;
                // Row A: high bitplane (zero, skip)
                // Row B (vertical double): low bitplane
                self.vram_bus.vram.tiles[0].data[vram_offset + 2] = expanded;
                // Row B: high bitplane (zero, skip)
                vram_offset += 4;
            }
        }

        // 2. Write ® symbol into tile 25 (offset 0x190 in tile block 0).
        const REGISTERED_SYMBOL: [u8; 8] = [0x3C, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x3C];
        let tile_25_offset: usize = 25 * 16;
        for (i, &byte) in REGISTERED_SYMBOL.iter().enumerate() {
            self.vram_bus.vram.tiles[0].data[tile_25_offset + i * 2] = byte;
            // High bitplane (odd offset) stays zero
        }

        // 3. Write tile map entries for the logo display.
        // Row 8, cols 4-15: tiles 1-12
        for col in 0u16..12 {
            let map_offset = (8 * 32 + 4 + col) as usize;
            self.vram_bus.vram.tile_maps[0].data[map_offset] = TileIndex((col + 1) as u8);
        }
        // Row 8, col 16: tile 25 (® symbol)
        self.vram_bus.vram.tile_maps[0].data[(8 * 32 + 16) as usize] = TileIndex(25);
        // Row 9, cols 4-15: tiles 13-24
        for col in 0u16..12 {
            let map_offset = (9 * 32 + 4 + col) as usize;
            self.vram_bus.vram.tile_maps[0].data[map_offset] = TileIndex((col + 13) as u8);
        }
    }
}
