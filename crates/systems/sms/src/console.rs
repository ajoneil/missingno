//! The console: Z80 + VDP + PSG + cartridge on one crystal. Two CPU
//! T-states carry exactly three VDP dots; the CPU steps a whole
//! instruction at a time and the VDP catches up afterwards on the carried
//! ratio, so instruction-granular timing is the current resolution —
//! refining the catch-up to the VDP-port membrane is later accuracy work.

use missingno_core::ClockRatio;
use missingno_ti_psg::{Psg, Variant};
use missingno_zilog_z80::{Bus, Cpu};

use crate::cartridge::{Cartridge, CartridgeError};
use crate::vdp::{Frame, Vdp};

/// The CPU/PSG clock, and the 44.1 kHz output taken from it.
const TSTATE_HZ: u64 = 3_579_545;
const SAMPLE_RATE: u64 = 44_100;

fn dot_clock() -> ClockRatio {
    ClockRatio::new(3, 2)
}

fn sample_clock() -> ClockRatio {
    ClockRatio::new(SAMPLE_RATE, TSTATE_HZ)
}

pub struct Sms {
    pub cpu: Cpu,
    pub vdp: Vdp,
    pub psg: Psg,
    cartridge: Cartridge,
    ram: Box<[u8; 0x2000]>,
    memory_control: u8,
    io_control: u8,
    /// Pad lines for ports $DC/$DD, active low.
    pub port_dc: u8,
    pub port_dd: u8,

    /// Three VDP dots per two T-states.
    dot_clock: ClockRatio,
    sample_clock: ClockRatio,
    samples: Vec<(f32, f32)>,
}

struct BoardBus<'a> {
    cartridge: &'a mut Cartridge,
    ram: &'a mut [u8; 0x2000],
    vdp: &'a mut Vdp,
    psg: &'a mut Psg,
    memory_control: &'a mut u8,
    io_control: &'a mut u8,
    port_dc: u8,
    port_dd: u8,
}

impl Bus for BoardBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        if address < 0xC000 {
            self.cartridge.read(address)
        } else {
            self.ram[(address & 0x1FFF) as usize]
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        if address >= 0xC000 {
            self.ram[(address & 0x1FFF) as usize] = data;
        }
        // The mapper latches sit behind the RAM mirror: writes land in
        // both, reads see the RAM.
        if address >= 0xFFFD {
            self.cartridge.write_bank((address - 0xFFFD) as usize, data);
        }
    }

    /// The I/O decode uses only A7, A6, and A0 — the rest are mirrors.
    fn input(&mut self, port: u16) -> u8 {
        match (port & 0xC0, port & 1) {
            (0x00, _) => 0xFF,
            (0x40, 0) => self.vdp.v_counter(),
            (0x40, _) => self.vdp.h_counter(),
            (0x80, 0) => self.vdp.read_data(),
            (0x80, _) => self.vdp.read_status(),
            (_, 0) => self.port_dc,
            // Bit 4 is the SMS1 reset button, unpressed; TH echoes read
            // high on export hardware.
            (_, _) => self.port_dd,
        }
    }

    fn output(&mut self, port: u16, data: u8) {
        match (port & 0xC0, port & 1) {
            (0x00, 0) => *self.memory_control = data,
            (0x00, _) => *self.io_control = data,
            (0x40, _) => self.psg.write(data),
            (0x80, 0) => self.vdp.write_data(data),
            (0x80, _) => self.vdp.write_control(data),
            _ => {}
        }
    }
}

impl Sms {
    pub fn new(rom: &[u8]) -> Result<Sms, CartridgeError> {
        // Some dumps carry a 512-byte copier header before the pages.
        let rom = if rom.len() % 0x4000 == 512 {
            &rom[512..]
        } else {
            rom
        };
        Ok(Sms {
            cpu: Cpu::new(),
            vdp: Vdp::new(),
            psg: Psg::new(Variant::SegaIntegrated),
            cartridge: Cartridge::load(rom)?,
            ram: Box::new([0; 0x2000]),
            memory_control: 0,
            io_control: 0,
            port_dc: 0xFF,
            port_dd: 0xFF,
            dot_clock: dot_clock(),
            sample_clock: sample_clock(),
            samples: Vec::new(),
        })
    }

    /// Execute one instruction and bring the rest of the board up to it.
    pub fn step_instruction(&mut self) {
        let mut bus = BoardBus {
            cartridge: &mut self.cartridge,
            ram: &mut self.ram,
            vdp: &mut self.vdp,
            psg: &mut self.psg,
            memory_control: &mut self.memory_control,
            io_control: &mut self.io_control,
            port_dc: self.port_dc,
            port_dd: self.port_dd,
        };
        let tstates = self.cpu.step(&mut bus);

        for _ in 0..self.dot_clock.advance(tstates as u64) {
            self.vdp.step_dot();
        }
        for _ in 0..tstates {
            self.psg.tick();
        }
        for _ in 0..self.sample_clock.advance(tstates as u64) {
            let level = self.psg.level();
            self.samples.push((level, level));
        }

        self.cpu.set_irq(self.vdp.interrupt_asserted());
    }

    /// Run until a frame completes, bounded so runaway code cannot stall
    /// the caller.
    pub fn step_frame(&mut self, budget_instructions: u32) -> Option<Frame> {
        for _ in 0..budget_instructions {
            self.step_instruction();
            if let Some(frame) = self.vdp.take_frame() {
                return Some(frame);
            }
        }
        None
    }

    pub fn take_frame(&mut self) -> Option<Frame> {
        self.vdp.take_frame()
    }

    /// Accumulated 44.1 kHz stereo samples since the last drain.
    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.samples)
    }

    /// Side-effect-free bus read for inspection.
    pub fn peek(&self, address: u16) -> u8 {
        if address < 0xC000 {
            self.cartridge.read(address)
        } else {
            self.ram[(address & 0x1FFF) as usize]
        }
    }

    /// Power-cycle: fresh chip state, same cartridge.
    pub fn power_cycle(&mut self) {
        self.cpu = Cpu::new();
        self.vdp = Vdp::new();
        self.psg = Psg::new(Variant::SegaIntegrated);
        *self.ram = [0; 0x2000];
        self.memory_control = 0;
        self.io_control = 0;
        self.dot_clock = dot_clock();
        self.sample_clock = sample_clock();
        self.samples.clear();
        let banks: [u8; 3] = [0, 1, 2];
        for (slot, bank) in banks.into_iter().enumerate() {
            self.cartridge.write_bank(slot, bank);
        }
    }
}
