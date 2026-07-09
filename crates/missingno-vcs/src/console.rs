//! The console: 6507 + TIA + RIOT + cartridge on one colour-clock master.
//!
//! One CPU cycle = exactly three colour clocks. A CPU cycle's bus access
//! lands first, then its three TIA clocks — so a register write at CPU
//! cycle N shapes the beam from colour clock 3N, the coupling "racing the
//! beam" kernels depend on. WSYNC parks the CPU through the 6502 module's
//! RDY pin; the TIA raises it again as the beam wraps.

use crate::TvStandard;
use crate::cartridge::{Cartridge, CartridgeError};
use crate::cpu::{Bus, Cpu};
use crate::riot::Riot;
use crate::tia::{Scanline, Tia, VISIBLE_CLOCKS};

/// One VSYNC-delimited frame. Height is whatever the kernel produced —
/// there is no hardware frame, only the software's sync pattern.
pub struct Frame {
    pub lines: Vec<[u8; VISIBLE_CLOCKS]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JoystickDirection {
    Up,
    Down,
    Left,
    Right,
}

pub struct Vcs {
    pub cpu: Cpu,
    pub tia: Tia,
    pub riot: Riot,
    cartridge: Cartridge,
    region: TvStandard,
    clock_phase: u8,
    pending_tia_writes: [Option<TiaWrite>; 2],
    building: Vec<Scanline>,
    in_vsync: bool,
    finished_frame: Option<Frame>,
    sample_clock: f32,
    /// Colour clocks per 44.1 kHz sample, from the region's master clock.
    clocks_per_sample: f32,
    samples: Vec<(f32, f32)>,
}

/// Colour clocks from a CPU write until the TIA sees it: the data bus is
/// valid at φ2, two colour clocks into the CPU cycle.
const TIA_WRITE_CLOCKS: u8 = 3;
/// Position-counter resets land a two-phase-clock cycle later than other
/// writes; the residue vs the ordinary write path is 2 colour clocks.
/// Calibrated against the suite's oracle anchor, pending PAL hardware.
const TIA_RESET_STROBE_CLOCKS: u8 = TIA_WRITE_CLOCKS + 2;

/// The 6507's view of the board: A12 selects the cartridge; below it, A7
/// splits TIA from RIOT and A9 splits RIOT RAM from its I/O registers.
///
/// TIA writes are deferred through a two-slot pipe: a reset strobe and the
/// next instruction's write sit three clocks apart and can overlap.
struct BoardBus<'a> {
    tia: &'a mut Tia,
    riot: &'a mut Riot,
    cartridge: &'a Cartridge,
    pending_tia_writes: &'a mut [Option<TiaWrite>; 2],
}

pub(crate) struct TiaWrite {
    address: u16,
    data: u8,
    clocks_until_effective: u8,
}

impl Bus for BoardBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        if address & 0x1000 != 0 {
            self.cartridge.read(address)
        } else if address & 0x0080 == 0 {
            self.tia.read(address)
        } else if address & 0x0200 == 0 {
            self.riot.ram[(address & 0x7F) as usize]
        } else {
            self.riot.read(address)
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        use crate::tia::registers::{RESBL, RESM0, RESM1, RESP0, RESP1};
        if address & 0x1000 != 0 {
        } else if address & 0x0080 == 0 {
            let clocks = match address & 0x3F {
                RESP0 | RESP1 | RESM0 | RESM1 | RESBL => TIA_RESET_STROBE_CLOCKS,
                _ => TIA_WRITE_CLOCKS,
            };
            let slot = self
                .pending_tia_writes
                .iter_mut()
                .find(|slot| slot.is_none())
                .expect("more than two TIA writes in flight");
            *slot = Some(TiaWrite {
                address,
                data,
                clocks_until_effective: clocks,
            });
        } else if address & 0x0200 == 0 {
            self.riot.ram[(address & 0x7F) as usize] = data;
        } else {
            self.riot.write(address, data);
        }
    }
}

impl Vcs {
    pub fn new(rom: &[u8], region: TvStandard) -> Result<Vcs, CartridgeError> {
        let cartridge = Cartridge::load(rom)?;
        let mut cpu = Cpu::new();
        cpu.reset();
        Ok(Vcs {
            cpu,
            tia: Tia::new(),
            riot: Riot::new(),
            cartridge,
            region,
            clock_phase: 0,
            pending_tia_writes: [None, None],
            building: Vec::new(),
            in_vsync: false,
            finished_frame: None,
            sample_clock: 0.0,
            clocks_per_sample: region.clocks_per_sample(),
            samples: Vec::new(),
        })
    }

    /// The broadcast standard this console is wired to.
    pub fn tv_standard(&self) -> TvStandard {
        self.region
    }

    /// The region-correct 128-colour palette for rendering this console's frames.
    pub fn palette(&self) -> &'static [(u8, u8, u8); 128] {
        crate::tia::palette(self.region)
    }

    /// Advance one colour clock. Every third clock carries the CPU (and
    /// RIOT) cycle before the TIA sees its clock.
    pub fn step_clock(&mut self) {
        if self.clock_phase == 0 {
            self.cpu.rdy = self.tia.cpu_ready;
            let mut bus = BoardBus {
                tia: &mut self.tia,
                riot: &mut self.riot,
                cartridge: &self.cartridge,
                pending_tia_writes: &mut self.pending_tia_writes,
            };
            self.cpu.step_cycle(&mut bus);
            self.riot.tick();
        }
        self.clock_phase = (self.clock_phase + 1) % 3;

        for slot in &mut self.pending_tia_writes {
            if let Some(write) = slot {
                write.clocks_until_effective -= 1;
                if write.clocks_until_effective == 0 {
                    let write = slot.take().unwrap();
                    self.tia.write(write.address, write.data);
                }
            }
        }
        self.tia.step_clock();
        if let Some(line) = self.tia.take_line() {
            self.collect_line(line);
        }

        self.sample_clock += 1.0;
        if self.sample_clock >= self.clocks_per_sample {
            self.sample_clock -= self.clocks_per_sample;
            let level = self.tia.audio_level();
            self.samples.push((level, level));
        }
    }

    /// Accumulated 44.1 kHz stereo samples since the last drain.
    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.samples)
    }

    fn collect_line(&mut self, line: Scanline) {
        if line.vsync && !self.in_vsync {
            let lines = std::mem::take(&mut self.building);
            if !lines.is_empty() {
                self.finished_frame = Some(Frame {
                    lines: lines.into_iter().map(|l| l.pixels).collect(),
                });
            }
        }
        self.in_vsync = line.vsync;
        if !line.vsync {
            self.building.push(line);
        }
    }

    /// Side-effect-free bus read for inspection: the debugger's view of
    /// any address without perturbing latches or timer flags.
    pub fn peek(&self, address: u16) -> u8 {
        if address & 0x1000 != 0 {
            self.cartridge.read(address)
        } else if address & 0x0080 == 0 {
            self.tia.peek(address)
        } else if address & 0x0200 == 0 {
            self.riot.ram[(address & 0x7F) as usize]
        } else {
            self.riot.peek(address)
        }
    }

    /// Scanlines completed since the current frame began.
    pub fn scanline(&self) -> usize {
        self.building.len()
    }

    /// A frame completed since the last take, if any.
    pub fn take_frame(&mut self) -> Option<Frame> {
        self.finished_frame.take()
    }

    /// Power-cycle: fresh chip state, same cartridge.
    pub fn power_cycle(&mut self) {
        self.cpu = Cpu::new();
        self.cpu.reset();
        self.tia = Tia::new();
        self.riot = Riot::new();
        self.clock_phase = 0;
        self.pending_tia_writes = [None, None];
        self.building.clear();
        self.in_vsync = false;
        self.finished_frame = None;
        self.sample_clock = 0.0;
        self.samples.clear();
    }

    /// Player-0 joystick direction lines into RIOT port A, active-low.
    pub fn set_joystick(&mut self, direction: JoystickDirection, pressed: bool) {
        let bit = match direction {
            JoystickDirection::Right => 0x80,
            JoystickDirection::Left => 0x40,
            JoystickDirection::Down => 0x20,
            JoystickDirection::Up => 0x10,
        };
        if pressed {
            self.riot.port_a &= !bit;
        } else {
            self.riot.port_a |= bit;
        }
    }

    /// Paddle knob position, 0.0-1.0.
    pub fn set_paddle(&mut self, index: usize, position: f32) {
        self.tia.set_paddle(index, position);
    }

    /// Player-0 trigger into TIA INPT4.
    pub fn set_fire(&mut self, pressed: bool) {
        self.tia.triggers[0] = pressed;
    }

    /// The console's momentary Game Reset switch (SWCHB bit 0, active-low).
    pub fn set_console_reset(&mut self, pressed: bool) {
        if pressed {
            self.riot.port_b &= !0x01;
        } else {
            self.riot.port_b |= 0x01;
        }
    }

    /// The console's momentary Game Select switch (SWCHB bit 1, active-low).
    pub fn set_console_select(&mut self, pressed: bool) {
        if pressed {
            self.riot.port_b &= !0x02;
        } else {
            self.riot.port_b |= 0x02;
        }
    }

    /// Run to the next instruction boundary. A WSYNC-parked opcode fetch
    /// waits here until the beam wraps and the TIA releases RDY.
    /// Advance exactly one CPU cycle (three colour clocks), first
    /// aligning to the colour-clock phase so the CPU's bus access lands
    /// at phase 0.
    pub fn step_cpu_cycle(&mut self) {
        while self.clock_phase != 0 {
            self.step_clock();
        }
        self.step_clock();
        while self.clock_phase != 0 {
            self.step_clock();
        }
    }

    pub fn step_instruction(&mut self) {
        if self.cpu.halted() {
            return;
        }
        while self.cpu.at_instruction_boundary() {
            self.step_clock();
        }
        while !self.cpu.at_instruction_boundary() && !self.cpu.halted() {
            self.step_clock();
        }
    }

    /// Run until a frame completes, bounded so a kernel that never syncs
    /// cannot stall the caller. Returns `None` on budget exhaustion.
    pub fn step_frame(&mut self, budget_lines: usize) -> Option<Frame> {
        let budget_clocks = budget_lines * crate::tia::CLOCKS_PER_LINE as usize;
        for _ in 0..budget_clocks {
            self.step_clock();
            if let Some(frame) = self.finished_frame.take() {
                return Some(frame);
            }
        }
        None
    }
}
