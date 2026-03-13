use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use cpu::mcycle::{BusDot, Processor};
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{ExternalBus, HighRam, VramBus};
use ppu::{Ppu, screen::Screen};

pub mod audio;
pub mod cartridge;
pub mod cpu;
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

/// Models the sequencer DFF (g42) pipeline for interrupt dispatch.
///
/// On hardware, an IF flag set during M-cycle N is captured in the
/// sequencer DFF at N's boundary but doesn't reach the dispatch
/// decision until M-cycle N+1. `promote()` advances Fresh→Ready.
/// For the Running path, promote runs after the leading fetch's
/// ticking — the leading fetch M-cycle IS the DFF propagation
/// delay. For the Halted path, dispatch is deferred so that a
/// wakeup NOP runs first. When `take_ready()` succeeds, ISR
/// dispatch begins immediately.
#[derive(Clone, Copy)]
enum InterruptLatch {
    /// No interrupt pending in the sequencer pipeline.
    Empty,
    /// Interrupt captured during this step's M-cycle boundary ticks.
    /// The DFF has latched the value but it hasn't propagated to the
    /// dispatch output yet.
    Fresh(interrupts::Interrupt),
    /// Interrupt that has propagated through the DFF pipeline (carried
    /// over from a previous step). Dispatch can consume it.
    Ready(interrupts::Interrupt),
}

impl InterruptLatch {
    /// Advance the DFF pipeline: Fresh becomes Ready.
    fn promote(&mut self) {
        if let InterruptLatch::Fresh(interrupt) = *self {
            *self = InterruptLatch::Ready(interrupt);
        }
    }

    /// Take the interrupt if it has propagated (Ready). Returns None
    /// and leaves the latch unchanged for Fresh and Empty.
    fn take_ready(&mut self) -> Option<interrupts::Interrupt> {
        if let InterruptLatch::Ready(interrupt) = *self {
            *self = InterruptLatch::Empty;
            Some(interrupt)
        } else {
            None
        }
    }
}

/// Where the CPU is in its instruction execution lifecycle.
/// Used by `step_dot()` to pause and resume between individual dots.
enum ExecutionState {
    /// Between instructions. The next `step_dot()` begins the
    /// leading fetch (Running path) or builds a HaltedNop (Halted path).
    Ready,
    /// Leading fetch: ticking hardware for 4 dots then reading the
    /// opcode at PC. After the read, the dispatch decision is made
    /// and a Processor is built.
    LeadingFetch {
        dot: BusDot,
        fetch_addr: u16,
    },
    /// Mid-instruction: the Processor is yielding dots.
    Running {
        processor: Processor,
        read_value: u8,
        dot: BusDot,
        pending_oam_bug: Option<execute::OamBugKind>,
        was_halted: bool,
    },
    /// HALT dummy fetch: ticking hardware for 4 dots then reading
    /// at PC (result discarded), transitioning to Halted.
    HaltDummyFetch {
        dot: BusDot,
        fetch_addr: u16,
    },
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

    interrupt_latch: InterruptLatch,
    execution: ExecutionState,
    bus_trace: Option<Vec<BusAccess>>,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> GameBoy {
        let cpu = Cpu::new(cartridge.header_checksum());
        let sgb = if cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };

        let mut gb = GameBoy {
            cpu,
            screen: Screen::new(),
            external: ExternalBus::new(cartridge),
            high_ram: HighRam::new(),
            ppu: Ppu::new(),
            audio: Audio::new(),
            joypad: Joypad::new(),
            interrupts: interrupts::Registers::new(),
            serial: serial_transfer::Registers::new(),
            timers: timers::Timers::new(),
            dma: Dma::new(),
            sgb,
            vram_bus: VramBus::new(),
            interrupt_latch: InterruptLatch::Empty,
            execution: ExecutionState::Ready,
            bus_trace: None,
        };
        gb.init_post_boot_vram();
        gb
    }

    pub fn reset(&mut self) {
        self.cpu = Cpu::new(self.external.cartridge.header_checksum());
        self.screen = Screen::new();
        self.external.work_ram = [0; 0x2000];
        self.external.latch = 0xFF;
        self.external.decay = 0;
        self.high_ram = HighRam::new();
        self.ppu = Ppu::new();
        self.audio = Audio::new();
        self.joypad = Joypad::new();
        self.interrupts = interrupts::Registers::new();
        self.serial = serial_transfer::Registers::new();
        self.timers = timers::Timers::new();
        self.dma = Dma::new();
        self.vram_bus = VramBus::new();
        self.sgb = if self.external.cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };
        self.init_post_boot_vram();
        self.interrupt_latch = InterruptLatch::Empty;
        self.execution = ExecutionState::Ready;
        self.bus_trace = None;
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.external.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    pub fn vram(&self) -> &ppu::memory::Vram {
        &self.vram_bus.vram
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
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

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.interrupts
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
        use ppu::tiles::TileIndex;

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
