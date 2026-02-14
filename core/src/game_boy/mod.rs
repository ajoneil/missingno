use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use dma::Dma;
use joypad::{Button, Joypad};
use memory::Ram;
use video::{Video, screen::Screen};

pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod dma;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod recording;
pub mod serial_transfer;
pub mod sgb;
pub mod timers;
pub mod video;

/// M-cycles before the external data bus decays to 0xFF.
///
/// On real hardware the external bus retains its last driven value
/// through parasitic capacitance. With no device driving the bus
/// the charge leaks and the value trends toward 0xFF. The exact
/// rate is board-dependent; 12 M-cycles (~2.86 µs) is a reasonable
/// approximation.
const EXTERNAL_BUS_DECAY_MCYCLES: u8 = 12;

// Anything accessible via a memory address is stored in a separate
// struct to allow borrowing independently of the Cpu
pub struct MemoryMapped {
    cartridge: Cartridge,
    ram: Ram,
    video: Video,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
    timers: timers::Timers,
    dma: Dma,
    sgb: Option<sgb::Sgb>,

    /// Retained value on the external data bus (ROM, cart RAM, WRAM).
    /// Updated on every CPU read from or write to an external-bus
    /// address. Decays toward 0xFF when the bus is idle.
    external_bus: u8,
    /// Retained value on the VRAM data bus (0x8000–0x9FFF).
    /// Updated on every CPU read from or write to a VRAM address.
    vram_bus: u8,
    /// M-cycles remaining before `external_bus` decays to 0xFF.
    /// Reset to [`EXTERNAL_BUS_DECAY_MCYCLES`] on every external bus
    /// access.
    external_bus_decay: u8,
}

pub struct GameBoy {
    cpu: Cpu,
    screen: Screen,
    mapped: MemoryMapped,
    /// Counts T-cycles 0–3 within each M-cycle, used to call
    /// audio/serial/DMA once per M-cycle (every 4th T-cycle).
    mcycle_counter: u8,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> GameBoy {
        let cpu = Cpu::new(cartridge.header_checksum());
        let sgb = if cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };

        GameBoy {
            cpu,
            screen: Screen::new(),
            mcycle_counter: 0,
            mapped: MemoryMapped {
                cartridge,
                ram: Ram::new(),
                video: Video::new(),
                audio: Audio::new(),
                joypad: Joypad::new(),
                interrupts: interrupts::Registers::new(),
                serial: serial_transfer::Registers::new(),
                timers: timers::Timers::new(),
                dma: Dma::new(),
                sgb,
                external_bus: 0xFF,
                vram_bus: 0xFF,
                external_bus_decay: 0,
            },
        }
    }

    pub fn reset(&mut self) {
        self.cpu = Cpu::new(self.mapped.cartridge.header_checksum());
        self.screen = Screen::new();
        self.mcycle_counter = 0;
        self.mapped.ram = Ram::new();
        self.mapped.video = Video::new();
        self.mapped.audio = Audio::new();
        self.mapped.joypad = Joypad::new();
        self.mapped.interrupts = interrupts::Registers::new();
        self.mapped.serial = serial_transfer::Registers::new();
        self.mapped.timers = timers::Timers::new();
        self.mapped.dma = Dma::new();
        self.mapped.external_bus = 0xFF;
        self.mapped.vram_bus = 0xFF;
        self.mapped.external_bus_decay = 0;
        self.mapped.sgb = if self.mapped.cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };
    }

    pub fn memory_mapped(&self) -> &MemoryMapped {
        &self.mapped
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.mapped.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn video(&self) -> &Video {
        &self.mapped.video
    }

    pub fn audio(&self) -> &Audio {
        &self.mapped.audio
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.mapped.audio.drain_samples()
    }

    pub fn press_button(&mut self, button: Button) {
        self.mapped.joypad.press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.mapped.joypad.release_button(button);
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.mapped.interrupts
    }

    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.mapped.sgb.as_ref()
    }

    #[allow(dead_code)]
    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.mapped.serial.output)
    }
}
