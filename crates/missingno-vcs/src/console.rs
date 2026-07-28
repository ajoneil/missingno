//! The console: 6507 + TIA + RIOT + cartridge on one colour-clock master.
//!
//! One CPU cycle = exactly three colour clocks. A CPU cycle's bus access
//! lands first, then its three TIA clocks — so a register write at CPU
//! cycle N shapes the beam from colour clock 3N, the coupling "racing the
//! beam" kernels depend on. WSYNC parks the CPU through the 6502 module's
//! RDY pin; the TIA raises it again as the beam wraps.

use missingno_core::system::{ControlInput, ControlRole};

use crate::TvStandard;
use crate::cartridge::{CartType, Cartridge, CartridgeError, DumpFit};
use crate::controllers::{Controller, ControllerKind, Jack, release_jack};
use crate::cpu::{Bus, Cpu};
use crate::riot::Riot;
use crate::tia::{Scanline, Tia, VISIBLE_CLOCKS};

/// One VSYNC-delimited frame. Height is whatever the kernel produced —
/// there is no hardware frame, only the software's sync pattern.
pub struct Frame {
    pub lines: Vec<[u8; VISIBLE_CLOCKS]>,
}

pub struct Vcs {
    pub cpu: Cpu,
    pub tia: Tia,
    pub riot: Riot,
    /// The two controller jacks, left then right.
    controllers: [Controller; 2],
    cartridge: Cartridge,
    region: TvStandard,
    pending_tia_writes: [Option<TiaWrite>; MAX_TIA_WRITES_IN_FLIGHT],
    last_bus_value: u8,
    building: Vec<Scanline>,
    in_vsync: bool,
    finished_frame: Option<Frame>,
    /// The most recently completed scanline, for the frontend's television:
    /// it integrates VSYNC across scanlines to decide field boundaries, so it
    /// consumes the raw per-line stream rather than the `Frame` the suite uses.
    last_line: Option<Scanline>,
    sample_clock: f32,
    /// Colour clocks per 44.1 kHz sample, from the region's master clock.
    clocks_per_sample: f32,
    /// The audio level integrated across the sample window so far, and the
    /// clocks it spans — the window is fractional, so its width alternates.
    sample_accum: f32,
    sample_accum_clocks: u32,
    samples: Vec<(f32, f32)>,
}

/// Half-clocks from a CPU write until the TIA sees it: the data bus is valid at
/// φ2 — the colour clock's high half, two colour clocks into the CPU cycle.
const TIA_WRITE_HC: u8 = 4;
/// A reset strobe (RESxx) re-phases the object counters on the strobe level
/// release — the low half one half-clock after φ2.
const TIA_RESET_HC: u8 = TIA_WRITE_HC + 1;
/// The colour registers are transparent latches read combinationally by the
/// output mux: the new colour reaches the pads while the write still drives
/// the bus, so the effect lands on the write cycle's own pixel.
const TIA_COLOUR_WRITE_HC: u8 = TIA_WRITE_HC - 2;
/// The playfield serialiser's cell latch samples the PF registers one colour
/// clock behind the φ2 latch: a write half a clock ahead of a cell boundary
/// leaves that cell on the old value; one and a half clocks ahead lands it.
const TIA_PF_WRITE_HC: u8 = TIA_WRITE_HC + 2;
/// A strobe's address-decoded level rises one colour clock before the write's
/// φ2 fall; HMOVE's SEC latch sets on that rise.
const TIA_STROBE_RISE_HC: u8 = TIA_WRITE_HC - 2;
/// RSYNC's counter reset lands the wrap 3.5 colour clocks after the write end.
const TIA_RSYNC_HC: u8 = TIA_WRITE_HC + 7;
/// RSYNC's decoded level rises two colour clocks before that wrap (rise 144 →
/// wrap 146, rise 147 → wrap 149). While it is up it grounds N2057, the audio
/// tap's sampling clock, so a commit falling in that window is held and lands
/// after the restart instead.
const RSYNC_ASSERT_HC: u8 = 4;
/// Reset-strobe countdown milestone: the scan-kill leads the counter plant by
/// one half-clock. The START-decode hold engages at the write's enqueue — the
/// decoded strobe level grips the divider wrap preceding the plant
/// (PAL-console-measured draw-at-NEW window −3..0; the decap sim resolves
/// this sub-clock race one clock later).
const RESET_KILL_HC: u8 = 1;

/// The TIA's φ0 (÷3) divider phase: CPU cycles begin where the line position
/// ≡ this (mod 3). RSYNC requantises the divider with the counter, so the
/// phase is line-locked; the value is the power-on settle, fixed at the
/// verified reference convention.
const PHI0_GRID_PHASE: u16 = 2;

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
        // The cart port has no chip select: every cycle reaches the edge, and
        // the board decides whether it answers. Boards whose hotspots live
        // below the window fire here without driving the bus.
        let value = if let Some(driven) = self.cartridge.read(address, *self.last_bus_value) {
            driven
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
        use crate::tia::registers::{
            COLUBK, COLUP0, COLUP1, COLUPF, HMOVE, PF0, PF1, PF2, RESBL, RESM0, RESM1, RESP0,
            RESP1, RSYNC,
        };
        // The residue is what the bus carries entering the cycle, before the
        // CPU drives its byte — what a latch clocked by an address edge sees.
        self.cartridge
            .write_access(address, data, *self.last_bus_value);
        *self.last_bus_value = data;
        if selects_cartridge(address) {
            // ROM takes no data; the address has already driven the board.
        } else if selects_tia(address) {
            let register = (address & 0x3F) as u8;
            // Data commits at φ2 (the high half); a reset strobe re-phases the
            // object counters on the strobe-level release, the next low half.
            let hc = match u16::from(register) {
                RSYNC => TIA_RSYNC_HC,
                RESP0 | RESP1 | RESM0 | RESM1 | RESBL => TIA_RESET_HC,
                COLUP0 | COLUP1 | COLUPF | COLUBK => TIA_COLOUR_WRITE_HC,
                PF0 | PF1 | PF2 => TIA_PF_WRITE_HC,
                HMOVE => TIA_STROBE_RISE_HC,
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
            // A reset's decoded strobe level holds the START decode from the
            // bus cycle's tail through the plant, gripping the divider wrap
            // before it (PAL-console-measured: missile window −3..0; player
            // via the merge-delivery leg-3 wrap-spanning kill).
            match u16::from(register) {
                RESP0 => self.tia.player_reset_rise(0),
                RESP1 => self.tia.player_reset_rise(1),
                RESM0 => self.tia.missile_reset_rise(0),
                RESM1 => self.tia.missile_reset_rise(1),
                _ => {}
            }
        } else if selects_riot_ram(address) {
            self.riot.ram[(address & 0x7F) as usize] = data;
        } else {
            self.riot.write(address, data);
        }
    }
}

impl Vcs {
    pub fn new(
        rom: &[u8],
        region: TvStandard,
        cart_type: Option<CartType>,
        fit: DumpFit,
    ) -> Result<Vcs, CartridgeError> {
        let clock_hz = crate::tv_standard::master_clock_hz(region);
        Ok(Vcs::with_cartridge(
            Cartridge::load(rom, cart_type, clock_hz, fit)?,
            region,
        ))
    }

    fn with_cartridge(cartridge: Cartridge, region: TvStandard) -> Vcs {
        let mut cpu = Cpu::new();
        cpu.reset();
        let mut vcs = Vcs {
            cpu,
            tia: Tia::new(),
            riot: Riot::new(),
            controllers: [Controller::Unplugged, Controller::Unplugged],
            cartridge,
            region,
            pending_tia_writes: [None; MAX_TIA_WRITES_IN_FLIGHT],
            last_bus_value: 0,
            building: Vec::new(),
            in_vsync: false,
            finished_frame: None,
            last_line: None,
            sample_clock: 0.0,
            clocks_per_sample: crate::tv_standard::clocks_per_sample(region),
            sample_accum: 0.0,
            sample_accum_clocks: 0,
            samples: Vec::new(),
        };
        for jack in [Jack::Left, Jack::Right] {
            vcs.plug(jack, ControllerKind::Joystick);
        }
        vcs
    }

    /// The broadcast standard this console is wired to.
    pub fn tv_standard(&self) -> TvStandard {
        self.region
    }

    /// Advance one colour clock as its two half-clocks: the CPU bus access lands
    /// on the high (φ2) half, the TIA render and MOTCK on the low half.
    pub fn step_clock(&mut self) {
        self.step_half_high();
        self.step_half_low();
        // A board with a clock of its own runs whether or not the CPU is
        // talking to it.
        self.cartridge.tick();
    }

    /// The colour clock's high half: pending writes tick a half-clock (data
    /// commits at φ2 here), then the CPU (and RIOT) cycle runs, once per three
    /// colour clocks, so its write registers ahead of the low-half render.
    fn step_half_high(&mut self) {
        self.advance_pending_writes();
        if self.tia.beam() % 3 == PHI0_GRID_PHASE {
            self.cpu.rdy = self.tia.cpu_ready();
            let port_a_before = self.riot.port_a_level();
            let mut bus = BoardBus {
                tia: &mut self.tia,
                riot: &mut self.riot,
                cartridge: &mut self.cartridge,
                pending_tia_writes: &mut self.pending_tia_writes,
                last_bus_value: &mut self.last_bus_value,
            };
            self.cpu.step_cycle(&mut bus);
            self.riot.tick();
            if self.riot.port_a_level() != port_a_before {
                self.refresh_controllers();
            }
        }
    }

    /// Port A's pin levels moved, so anything scanned from them — a keypad's
    /// rows — re-derives what it drives.
    fn refresh_controllers(&mut self) {
        for jack in [Jack::Left, Jack::Right] {
            self.controllers[jack.index()].refresh(jack, &self.riot, &mut self.tia);
        }
    }

    /// The colour clock's low half: pending writes tick a half-clock (a reset
    /// strobe releases here), MOTCK fires and the TIA renders the pixel.
    fn step_half_low(&mut self) {
        self.advance_pending_writes();
        self.tia.step_clock();
        if let Some(line) = self.tia.take_line() {
            self.last_line = Some(line.clone());
            self.collect_line(line);
        }

        // The coupling network integrates the node's level, so the window's
        // average is taken over the level itself — averaging the channels'
        // conductances first would run the saturating divider off its mean.
        self.sample_accum += self.tia.audio_level();
        self.sample_accum_clocks += 1;
        self.sample_clock += 1.0;
        if self.sample_clock >= self.clocks_per_sample {
            self.sample_clock -= self.clocks_per_sample;
            let level = self.sample_accum / self.sample_accum_clocks as f32;
            self.samples.push((level, level));
            self.sample_accum = 0.0;
            self.sample_accum_clocks = 0;
        }
    }

    /// Tick every in-flight TIA write one half-clock; a write reaching its φ2
    /// commits, and a reset strobe's scan-kill leads its plant by one half-clock.
    fn advance_pending_writes(&mut self) {
        for slot in &mut self.pending_tia_writes {
            if let Some(write) = slot {
                write.hc_until_effective -= 1;
                if write.hc_until_effective == RSYNC_ASSERT_HC
                    && u16::from(write.register) == crate::tia::registers::RSYNC
                {
                    self.tia.rsync_assert();
                } else if write.hc_until_effective == RESET_KILL_HC {
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

    /// Enable or disable the debugger's per-channel waveform capture.
    pub fn set_wave_capture(&mut self, on: bool) {
        self.tia.set_wave_capture(on);
    }

    /// The captured per-channel waveforms, or `None` when capture is off. The
    /// AUDx circuits commit twice per line, so the capture rate is twice the
    /// region's line rate (master clock ÷ 228 clocks/line).
    pub fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        let (levels, active) = self.tia.wave_windows()?;
        let line_rate =
            crate::tv_standard::master_clock_hz(self.region) / crate::tia::CLOCKS_PER_LINE as f32;
        let rate = (2.0 * line_rate).round() as u32;
        let labels = ["CH0", "CH1"];
        Some(
            (0..2)
                .map(|i| missingno_core::waveform::ChannelWave {
                    label: labels[i],
                    levels: levels[i].clone(),
                    depth_bits: 4,
                    rate,
                    active: active[i],
                })
                .collect(),
        )
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

    /// The cartridge, for the debugger's board and cart-RAM inspection.
    pub fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    /// The cartridge for a state restore (bank + cart-RAM reseat).
    pub fn cartridge_mut(&mut self) -> &mut Cartridge {
        &mut self.cartridge
    }

    /// True between instructions — the only boundary a state save is taken at.
    pub fn at_instruction_boundary(&self) -> bool {
        self.cpu.at_instruction_boundary()
    }

    /// The in-flight TIA write pipe as `(register, data, half_clocks_remaining)`
    /// tuples — deferred writes that are live boundary state when a save lands
    /// between two instructions.
    pub fn pending_tia_writes(&self) -> Vec<(u8, u8, u8)> {
        self.pending_tia_writes
            .iter()
            .flatten()
            .map(|w| (w.register, w.data, w.hc_until_effective))
            .collect()
    }

    /// The byte the data bus still carries (bus capacitance) — boundary state a
    /// latch clocked by an address edge reads.
    pub fn last_bus_value(&self) -> u8 {
        self.last_bus_value
    }

    /// Reseat the deferred-write pipe, the bus-capacitance byte, and clear the
    /// frame-assembly buffers. The completed-line accumulator, the in-progress
    /// field flag, and the audio resampler window are the frontend Television's
    /// off-chip integration surface, not hardware, so a restore starts them
    /// empty and the field re-locks on the next VSYNC.
    pub fn restore_console(&mut self, pending: &[(u8, u8, u8)], last_bus_value: u8) {
        self.pending_tia_writes = [None; MAX_TIA_WRITES_IN_FLIGHT];
        for (slot, &(register, data, hc)) in self.pending_tia_writes.iter_mut().zip(pending) {
            *slot = Some(TiaWrite {
                register,
                data,
                hc_until_effective: hc,
            });
        }
        self.last_bus_value = last_bus_value;
        self.building.clear();
        self.in_vsync = false;
        self.finished_frame = None;
        self.last_line = None;
        self.sample_clock = 0.0;
        self.sample_accum = 0.0;
        self.sample_accum_clocks = 0;
        self.samples.clear();
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
    /// Flipping the power switch does not unplug the controllers.
    pub fn power_cycle(&mut self) {
        let plugged = [self.plugged(Jack::Left), self.plugged(Jack::Right)];
        let cartridge = std::mem::replace(&mut self.cartridge, Cartridge::unplugged());
        *self = Vcs::with_cartridge(cartridge, self.region);
        for (jack, kind) in [Jack::Left, Jack::Right].into_iter().zip(plugged) {
            self.plug(jack, kind);
        }
    }

    /// What is in a controller jack.
    pub fn plugged(&self, jack: Jack) -> ControllerKind {
        self.controllers[jack.index()].kind()
    }

    /// Swap what a jack carries. The departing controller lets go of every line
    /// it was driving; the arriving one takes up the lines it owns.
    pub fn plug(&mut self, jack: Jack, kind: ControllerKind) {
        release_jack(jack, &mut self.riot, &mut self.tia);
        let controller = Controller::new(kind);
        controller.connect(jack, &mut self.riot, &mut self.tia);
        self.controllers[jack.index()] = controller;
    }

    /// Hand a control to whatever is in the jack; a controller ignores roles
    /// it has no part for.
    pub fn set_controller_input(&mut self, jack: Jack, role: ControlRole, input: ControlInput) {
        self.controllers[jack.index()].apply(jack, role, input, &mut self.riot, &mut self.tia);
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
    /// aligning to the φ0 grid so the CPU's bus access lands on its
    /// boundary clock.
    pub fn step_cpu_cycle(&mut self) {
        while self.tia.beam() % 3 != PHI0_GRID_PHASE {
            self.step_clock();
        }
        self.step_clock();
        while self.tia.beam() % 3 != PHI0_GRID_PHASE {
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

    /// Advance until the TIA completes a scanline, returning it with its raw
    /// VSYNC state. The frontend's television integrates VSYNC across scanlines
    /// to decide field boundaries — that lock is off-chip, in the set.
    pub fn step_scanline(&mut self) -> Scanline {
        loop {
            self.step_clock();
            if let Some(line) = self.last_line.take() {
                return line;
            }
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
