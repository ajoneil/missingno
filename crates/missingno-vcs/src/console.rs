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
    pending_tia_writes: [Option<TiaWrite>; MAX_TIA_WRITES_IN_FLIGHT],
    last_bus_value: u8,
    building: Vec<Scanline>,
    in_vsync: bool,
    finished_frame: Option<Frame>,
    sample_clock: f32,
    /// Colour clocks per 44.1 kHz sample, from the region's master clock.
    clocks_per_sample: f32,
    samples: Vec<(f32, f32)>,
}

/// Half-clocks from a CPU write until the TIA sees it: the data bus is valid at
/// φ2 — the colour clock's high half, two colour clocks into the CPU cycle.
const TIA_WRITE_HC: u8 = 4;
/// A reset strobe (RESxx) re-phases the object counters on the strobe level
/// release — the low half one half-clock after φ2.
const TIA_RESET_HC: u8 = TIA_WRITE_HC + 1;
/// The VBLANK gate and the player graphics consume a write one colour clock
/// behind the combinational colour path.
const TIA_GATED_WRITE_HC: u8 = TIA_WRITE_HC + 2;
/// Playfield registers reach the serialiser a colour clock later still; the
/// per-cell latch in the playfield completes the in-flight cell.
const TIA_CELL_WRITE_HC: u8 = TIA_WRITE_HC + 4;
/// RSYNC's counter reset requantises onto the next H@1-H@2 cycle.
const TIA_RSYNC_HC: u8 = TIA_WRITE_HC + 6;

/// A write and the next can overlap; ≤6-clock delays never make three (BRK's
/// mirror-push triple is the binding case).
const MAX_TIA_WRITES_IN_FLIGHT: usize = 2;

// The 6507's 13-line board decode: A12 selects the cartridge; below it,
// A7 splits TIA from RIOT and A9 splits RIOT RAM from its I/O registers.
fn selects_cartridge(address: u16) -> bool {
    address & 0x1000 != 0
}
fn selects_tia(address: u16) -> bool {
    address & 0x0080 == 0
}
fn selects_riot_ram(address: u16) -> bool {
    address & 0x0200 == 0
}

/// TIA writes are deferred through a two-slot pipe: a write and the next
/// instruction's write can overlap in flight.
struct BoardBus<'a> {
    tia: &'a mut Tia,
    riot: &'a mut Riot,
    cartridge: &'a mut Cartridge,
    pending_tia_writes: &'a mut [Option<TiaWrite>; MAX_TIA_WRITES_IN_FLIGHT],
    /// The data bus holds its last driven byte (bus capacitance).
    last_bus_value: &'a mut u8,
}

#[derive(Clone, Copy)]
pub(crate) struct TiaWrite {
    register: u8,
    data: u8,
    hc_until_effective: u8,
}

impl Bus for BoardBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        let value = if selects_cartridge(address) {
            self.cartridge.read(address)
        } else if selects_tia(address) {
            // The TIA drives only D7-D6; the rest floats to the bus's byte.
            self.tia.read(address, *self.last_bus_value)
        } else if selects_riot_ram(address) {
            self.riot.ram[(address & 0x7F) as usize]
        } else {
            self.riot.read(address)
        };
        *self.last_bus_value = value;
        value
    }

    fn write(&mut self, address: u16, data: u8) {
        *self.last_bus_value = data;
        use crate::tia::registers::{
            GRP0, GRP1, PF0, PF1, PF2, RESBL, RESM0, RESM1, RESP0, RESP1, RSYNC, VBLANK,
        };
        if selects_cartridge(address) {
            self.cartridge.write_access(address);
        } else if selects_tia(address) {
            let register = (address & 0x3F) as u8;
            // Data commits at φ2 (the high half); a reset strobe re-phases the
            // object counters on the strobe-level release, the next low half.
            let hc = match u16::from(register) {
                RSYNC => TIA_RSYNC_HC,
                RESP0 | RESP1 | RESM0 | RESM1 | RESBL => TIA_RESET_HC,
                VBLANK | GRP0 | GRP1 => TIA_GATED_WRITE_HC,
                PF0 | PF1 | PF2 => TIA_CELL_WRITE_HC,
                _ => TIA_WRITE_HC,
            };
            let slot = self
                .pending_tia_writes
                .iter_mut()
                .find(|slot| slot.is_none())
                .expect("more than two TIA writes in flight");
            *slot = Some(TiaWrite {
                register,
                data,
                hc_until_effective: hc,
            });
        } else if selects_riot_ram(address) {
            self.riot.ram[(address & 0x7F) as usize] = data;
        } else {
            self.riot.write(address, data);
        }
    }
}

impl Vcs {
    pub fn new(rom: &[u8], region: TvStandard) -> Result<Vcs, CartridgeError> {
        Ok(Vcs::with_cartridge(Cartridge::load(rom)?, region))
    }

    fn with_cartridge(cartridge: Cartridge, region: TvStandard) -> Vcs {
        let mut cpu = Cpu::new();
        cpu.reset();
        Vcs {
            cpu,
            tia: Tia::new(),
            riot: Riot::new(),
            cartridge,
            region,
            clock_phase: 0,
            pending_tia_writes: [None; MAX_TIA_WRITES_IN_FLIGHT],
            last_bus_value: 0,
            building: Vec::new(),
            in_vsync: false,
            finished_frame: None,
            sample_clock: 0.0,
            clocks_per_sample: region.clocks_per_sample(),
            samples: Vec::new(),
        }
    }

    /// The broadcast standard this console is wired to.
    pub fn tv_standard(&self) -> TvStandard {
        self.region
    }

    /// The region-correct 128-colour palette for rendering this console's frames.
    pub fn palette(&self) -> &'static [(u8, u8, u8); 128] {
        crate::tia::palette(self.region)
    }

    /// Advance one colour clock as its two half-clocks: the CPU bus access lands
    /// on the high (φ2) half, the TIA render and MOTCK on the low half.
    pub fn step_clock(&mut self) {
        self.step_half_high();
        self.step_half_low();
    }

    /// The colour clock's high half: pending writes tick a half-clock (data
    /// commits at φ2 here), then the CPU (and RIOT) cycle runs, once per three
    /// colour clocks, so its write registers ahead of the low-half render.
    fn step_half_high(&mut self) {
        self.advance_pending_writes();
        if self.clock_phase == 0 {
            self.cpu.rdy = self.tia.cpu_ready;
            let mut bus = BoardBus {
                tia: &mut self.tia,
                riot: &mut self.riot,
                cartridge: &mut self.cartridge,
                pending_tia_writes: &mut self.pending_tia_writes,
                last_bus_value: &mut self.last_bus_value,
            };
            self.cpu.step_cycle(&mut bus);
            self.riot.tick();
        }
    }

    /// The colour clock's low half: pending writes tick a half-clock (a reset
    /// strobe releases here), MOTCK fires and the TIA renders the pixel.
    fn step_half_low(&mut self) {
        self.clock_phase = (self.clock_phase + 1) % 3;
        self.advance_pending_writes();
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

    /// Tick every in-flight TIA write one half-clock; a write reaching its φ2
    /// commits, and a reset strobe's scan-kill leads its plant by one half-clock.
    fn advance_pending_writes(&mut self) {
        for slot in &mut self.pending_tia_writes {
            if let Some(write) = slot {
                write.hc_until_effective -= 1;
                if write.hc_until_effective == 1 {
                    // The missile reset's scan-kill leads its plant.
                    match u16::from(write.register) {
                        crate::tia::registers::RESM0 => self.tia.missile_reset_kill(0),
                        crate::tia::registers::RESM1 => self.tia.missile_reset_kill(1),
                        crate::tia::registers::RESBL => self.tia.ball_reset_kill(),
                        _ => {}
                    }
                } else if write.hc_until_effective == 0 {
                    let write = slot.take().unwrap();
                    self.tia.write(u16::from(write.register), write.data);
                }
            }
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
        if selects_cartridge(address) {
            self.cartridge.peek(address)
        } else if selects_tia(address) {
            self.tia.read(address, self.last_bus_value)
        } else if selects_riot_ram(address) {
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

    /// Power-cycle: fresh chip state, same cartridge (bank state included).
    pub fn power_cycle(&mut self) {
        let placeholder = Cartridge::Rom2K(Box::new([0; 0x800]));
        let cartridge = std::mem::replace(&mut self.cartridge, placeholder);
        *self = Vcs::with_cartridge(cartridge, self.region);
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
            self.riot.set_pin_a(bit, false);
        } else {
            self.riot.set_pin_a(bit, true);
        }
    }

    /// Paddle knob position, 0.0-1.0.
    pub fn set_paddle(&mut self, index: usize, position: f32) {
        debug_assert!(index < 4, "the TIA has four pot inputs");
        self.tia.set_paddle(index, position);
    }

    /// Player-0 trigger into TIA INPT4.
    pub fn set_fire(&mut self, pressed: bool) {
        self.tia.set_trigger(0, pressed);
    }

    /// The console's momentary Game Reset switch (SWCHB bit 0, active-low).
    pub fn set_console_reset(&mut self, pressed: bool) {
        if pressed {
            self.riot.set_pin_b(0x01, false);
        } else {
            self.riot.set_pin_b(0x01, true);
        }
    }

    /// The console's momentary Game Select switch (SWCHB bit 1, active-low).
    pub fn set_console_select(&mut self, pressed: bool) {
        if pressed {
            self.riot.set_pin_b(0x02, false);
        } else {
            self.riot.set_pin_b(0x02, true);
        }
    }

    /// A player difficulty switch (SWCHB: P0 = bit 6, P1 = bit 7). Pro (A)
    /// drives the pin high, amateur (B) low.
    pub fn set_difficulty(&mut self, player: usize, pro: bool) {
        let mask = if player == 0 { 0x40 } else { 0x80 };
        self.riot.set_pin_b(mask, pro);
    }

    /// The colour / black-and-white switch (SWCHB bit 3). Colour is high.
    pub fn set_color_mode(&mut self, color: bool) {
        self.riot.set_pin_b(0x08, color);
    }

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

    /// Run to the next instruction boundary. A WSYNC-parked opcode fetch
    /// waits here until the beam wraps and the TIA releases RDY.
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
