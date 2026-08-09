//! The console: 2A03 + 2C02 + cartridge on one crystal. The CPU is
//! cycle-stepped and every CPU cycle carries exactly three PPU dots, so
//! the interleave is cycle-granular. OAM DMA freezes the CPU for the
//! 513-cycle transfer while the PPU keeps running.

use missingno_mos_6502::{Bus, Cpu};

use crate::apu::Apu;
use crate::cartridge::{Cartridge, CartridgeError};
use crate::ppu::{Frame, Ppu};

/// 44.1 kHz output from the 1.789773 MHz CPU clock.
const CYCLES_PER_SAMPLE: f32 = 1_789_773.0 / 44_100.0;

pub struct Nes {
    pub cpu: Cpu,
    pub ppu: Ppu,
    pub apu: Apu,
    cartridge: Cartridge,
    ram: Box<[u8; 0x800]>,
    controller_state: u8,
    controller_shift: u8,
    controller_strobe: bool,
    pending_oam_dma: Option<u8>,
    sample_clock: f32,
    samples: Vec<(f32, f32)>,
    finished_frame: Option<Frame>,
}

struct BoardBus<'a> {
    ram: &'a mut [u8; 0x800],
    ppu: &'a mut Ppu,
    apu: &'a mut Apu,
    cartridge: &'a mut Cartridge,
    controller_state: u8,
    controller_shift: &'a mut u8,
    controller_strobe: &'a mut bool,
    pending_oam_dma: &'a mut Option<u8>,
}

impl Bus for BoardBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[(address & 0x7FF) as usize],
            0x2000..=0x3FFF => self.ppu.read_register(address, self.cartridge),
            0x4016 => {
                if *self.controller_strobe {
                    *self.controller_shift = self.controller_state;
                }
                let bit = *self.controller_shift & 1;
                *self.controller_shift = (*self.controller_shift >> 1) | 0x80;
                // Upper bits are open bus from the $40xx page.
                0x40 | bit
            }
            0x4017 => 0x40,
            0x4000..=0x401F => self.apu.read(address),
            0x4020..=0x7FFF => 0,
            0x8000..=0xFFFF => self.cartridge.read_prg(address),
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        match address {
            0x0000..=0x1FFF => self.ram[(address & 0x7FF) as usize] = data,
            0x2000..=0x3FFF => self.ppu.write_register(address, data, self.cartridge),
            0x4014 => *self.pending_oam_dma = Some(data),
            0x4016 => {
                *self.controller_strobe = data & 1 != 0;
                if *self.controller_strobe {
                    *self.controller_shift = self.controller_state;
                }
            }
            0x4000..=0x401F => self.apu.write(address, data),
            _ => {}
        }
    }
}

impl Nes {
    pub fn new(rom: &[u8]) -> Result<Nes, CartridgeError> {
        let cartridge = Cartridge::load(rom)?;
        let mut cpu = Cpu::new_without_decimal();
        cpu.reset();
        Ok(Nes {
            cpu,
            ppu: Ppu::new(),
            apu: Apu::new(),
            cartridge,
            ram: Box::new([0; 0x800]),
            controller_state: 0,
            controller_shift: 0,
            controller_strobe: false,
            pending_oam_dma: None,
            sample_clock: 0.0,
            samples: Vec::new(),
            finished_frame: None,
        })
    }

    /// One CPU cycle and its three PPU dots.
    pub fn step_cycle(&mut self) {
        let mut bus = BoardBus {
            ram: &mut self.ram,
            ppu: &mut self.ppu,
            apu: &mut self.apu,
            cartridge: &mut self.cartridge,
            controller_state: self.controller_state,
            controller_shift: &mut self.controller_shift,
            controller_strobe: &mut self.controller_strobe,
            pending_oam_dma: &mut self.pending_oam_dma,
        };
        self.cpu.step_cycle(&mut bus);
        self.after_cpu_cycle();

        if let Some(page) = self.pending_oam_dma.take() {
            self.run_oam_dma(page);
        }
    }

    fn after_cpu_cycle(&mut self) {
        for _ in 0..3 {
            self.ppu.step_dot(&self.cartridge);
        }
        if self.ppu.take_nmi() {
            self.cpu.trigger_nmi();
        }
        if let Some(frame) = self.ppu.take_frame() {
            self.finished_frame = Some(frame);
        }

        self.apu.tick();
        self.sample_clock += 1.0;
        if self.sample_clock >= CYCLES_PER_SAMPLE {
            self.sample_clock -= CYCLES_PER_SAMPLE;
            let level = self.apu.level();
            self.samples.push((level, level));
        }
    }

    /// The DMA engine copies a page into OAM while the CPU is held;
    /// the rest of the board keeps its own time.
    fn run_oam_dma(&mut self, page: u8) {
        let base = (page as u16) << 8;
        self.after_cpu_cycle();
        for offset in 0..256u16 {
            let value = match base + offset {
                address @ 0x0000..=0x1FFF => self.ram[(address & 0x7FF) as usize],
                address @ 0x8000..=0xFFFF => self.cartridge.read_prg(address),
                _ => 0,
            };
            self.after_cpu_cycle();
            self.write_oam(value);
            self.after_cpu_cycle();
        }
    }

    fn write_oam(&mut self, value: u8) {
        self.ppu.write_register(0x2004, value, &mut self.cartridge);
    }

    /// Run to the next instruction boundary.
    pub fn step_instruction(&mut self) {
        if self.cpu.halted() {
            return;
        }
        while self.cpu.at_instruction_boundary() {
            self.step_cycle();
        }
        while !self.cpu.at_instruction_boundary() && !self.cpu.halted() {
            self.step_cycle();
        }
    }

    /// Run until a frame completes, bounded against runaway code.
    pub fn step_frame(&mut self, budget_cycles: u32) -> Option<Frame> {
        for _ in 0..budget_cycles {
            self.step_cycle();
            if let Some(frame) = self.finished_frame.take() {
                return Some(frame);
            }
        }
        None
    }

    pub fn take_frame(&mut self) -> Option<Frame> {
        self.finished_frame.take()
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.samples)
    }

    /// Controller 1's buttons, in the shift order
    /// A, B, Select, Start, Up, Down, Left, Right (bit 0 first).
    pub fn set_controller(&mut self, state: u8) {
        self.controller_state = state;
    }

    pub fn controller(&self) -> u8 {
        self.controller_state
    }

    /// Side-effect-free bus read for inspection ($2000-$3FFF excluded:
    /// PPU register reads acknowledge; peek those through the PPU).
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[(address & 0x7FF) as usize],
            0x8000..=0xFFFF => self.cartridge.read_prg(address),
            _ => 0,
        }
    }

    /// Power-cycle: fresh chip state, same cartridge.
    pub fn power_cycle(&mut self) {
        self.cpu = Cpu::new_without_decimal();
        self.cpu.reset();
        self.ppu = Ppu::new();
        self.apu = Apu::new();
        *self.ram = [0; 0x800];
        self.controller_shift = 0;
        self.controller_strobe = false;
        self.pending_oam_dma = None;
        self.sample_clock = 0.0;
        self.samples.clear();
        self.finished_frame = None;
    }
}
