//! The board: a Z80, a TMS9928A or TMS9929A, an SN76489AN, 1 KB of RAM, the
//! 8 KB BIOS ahead of the cartridge, and two hand controllers. One 7.15909 MHz
//! crystal divided by two is the system clock the Z80 and the PSG take; the
//! VDP runs on that clock's third harmonic, so it takes three of its periods
//! and the PSG one CLOCK per Z80 T-state. The board contributes decode (two
//! '138s), a wait state on every M1 cycle, the VDP's interrupt on /NMI, and
//! the controller mode latch.

use missingno_core::ClockRatio;
use missingno_core::TvStandard;
use missingno_core::ports::PortId;
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};
use missingno_core::waveform::{ChannelWave, WaveRing};
use missingno_ti_psg::{MUTE_ATTENUATION, Psg, Variant};
use missingno_ti_vdp::{Frame, Standard, Vdp};
use missingno_zilog_z80::{Bus, Cpu};

use crate::cartridge::{CartWindow, Cartridge, CartridgeError, UNDRIVEN};
use crate::controllers::{ControllerMode, HandController};
use crate::firmware::BIOS_SIZE;

/// The VDP's clock periods in one system-clock period: the tank's third
/// harmonic.
const XTALS_PER_TSTATE: u32 = 3;
const TSTATES_PER_LINE: u32 = 228;

/// One frame of the part's raster: 262 lines on NTSC, 313 on PAL.
pub fn tstates_per_frame(standard: Standard) -> u32 {
    TSTATES_PER_LINE * standard.lines_per_frame() as u32
}

/// The VDP the board fits for a broadcast standard: a TMS9928A for NTSC, a
/// TMS9929A for PAL. PAL-M is System M's 525-line raster, so the NTSC part;
/// its PAL colour encoding is off-chip.
pub fn part_for(standard: TvStandard) -> Option<Standard> {
    match standard {
        TvStandard::Ntsc | TvStandard::PalM => Some(Standard::Ntsc),
        TvStandard::Pal => Some(Standard::Pal),
        _ => None,
    }
}

/// Two 2114s: 1 KB on A0-A9, selected across the whole $6000-$7FFF window.
pub const RAM_SIZE: usize = 0x400;
const RAM_MASK: usize = RAM_SIZE - 1;

/// The 7.15909 MHz crystal ÷ 2: the Z80's CLK and the PSG's CLOCK.
pub(crate) const CLOCK_HZ: u32 = 3_579_545;
/// 44.1 kHz output from that clock.
const SAMPLE_RATE: u32 = 44_100;

fn sample_clock() -> ClockRatio {
    ClockRatio::new(SAMPLE_RATE as u64, CLOCK_HZ as u64)
}

const PSG_CHANNELS: usize = 4;
/// Width of the amplitude code each channel hands its DAC.
const PSG_CODE_BITS: u8 = 4;
/// Waveform-capture ring depth: one frame-window of output samples with
/// headroom — a PAL frame is ~879 samples at 44.1 kHz.
const WAVE_CAPTURE_SAMPLES: usize = 1000;
/// The channels' display names, in capture order.
const WAVE_LABELS: [&str; PSG_CHANNELS] = ["Tone 1", "Tone 2", "Tone 3", "Noise"];

/// J5, read at $FC (A1 = 0), and J6 at $FF (A1 = 1).
pub const PORT1: PortId = PortId(0);
pub const PORT2: PortId = PortId(1);

/// The eight 8 KB windows U5 decodes from A13-A15.
enum MemorySelect {
    Bios,
    /// `EX_20_3F` and `EX_40_5F`: the expansion connector's selects.
    Expansion20,
    Expansion40,
    Ram,
    Cart(CartWindow),
}

impl MemorySelect {
    fn of(address: u16) -> MemorySelect {
        match address >> 13 {
            0 => MemorySelect::Bios,
            1 => MemorySelect::Expansion20,
            2 => MemorySelect::Expansion40,
            3 => MemorySelect::Ram,
            window => MemorySelect::Cart(CartWindow(window as u8 - 4)),
        }
    }
}

/// Offset within an 8 KB window: A0-A12.
const WINDOW_OFFSET: u16 = 0x1FFF;

/// What U6 decodes from A5-A7 and /WR.
enum IoSelect {
    /// $80-$9F write: the mode latch set, segment 1.
    KeypadMode,
    /// $A0-$BF, A0 on the VDP's MODE pin.
    VdpWrite,
    VdpRead,
    /// $C0-$DF write: the mode latch reset, segment 0.
    JoystickMode,
    /// $E0-$FF write.
    PsgWrite,
    /// $E0-$FF read, A1 picking J5 or J6.
    ControllerRead(PortId),
    /// $00-$7F, and the read halves of $80-$9F and $C0-$DF.
    Unselected,
}

/// A1: which connector's buffer a controller read enables.
const CONTROLLER_SELECT: u16 = 0x02;
/// A0: the VDP's MODE pin.
const VDP_MODE: u16 = 0x01;

impl IoSelect {
    fn of(port: u16, write: bool) -> IoSelect {
        match (port as u8 & 0xE0, write) {
            (0x80, true) => IoSelect::KeypadMode,
            (0xA0, true) => IoSelect::VdpWrite,
            (0xA0, false) => IoSelect::VdpRead,
            (0xC0, true) => IoSelect::JoystickMode,
            (0xE0, true) => IoSelect::PsgWrite,
            (0xE0, false) => IoSelect::ControllerRead(match port & CONTROLLER_SELECT {
                0 => PORT1,
                _ => PORT2,
            }),
            _ => IoSelect::Unselected,
        }
    }
}

/// U8A: one half of a 74LS74 with D tied to /Q, clocked by the system clock
/// and held clear while /M1 is high. Q pulls /WAIT low through U7B.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct M1WaitLatch {
    q: bool,
}

impl M1WaitLatch {
    /// One clock edge: whether /M1 was asserted at the edge and whether it
    /// stays asserted after it. The edge where /M1 falls still finds the clear
    /// active.
    fn clock(&mut self, m1_at_edge: bool, m1_after: bool) {
        self.q = m1_after && m1_at_edge && !self.q;
    }
}

pub struct ColecoVision {
    pub cpu: Cpu,
    board: Board,
    /// Whether /M1 was asserted through the T-state just run.
    m1_level: bool,
    sample_clock: ClockRatio,
    audio: Vec<(f32, f32)>,
    /// Per-channel DAC codes for the debugger's scope, present only while a
    /// consumer wants them.
    wave_capture: Option<[WaveRing; PSG_CHANNELS]>,
    /// Whether a consumer wants the VDP's memory decoded into the debugger's
    /// graphics surfaces.
    graphics_capture: bool,
    /// VDP frames already handed out.
    frames_seen: u64,
    #[cfg(feature = "morepork")]
    pub(crate) rom_sha256: [u8; 32],
}

/// The board's own state beside the chips': the mode latch, the wait latch,
/// the controllers' switches, the output tap's phase and the fields handed
/// out.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BoardState {
    pub mode: ControllerMode,
    pub wait_latch_q: bool,
    pub controllers: [HandController; 2],
    pub sample_phase: u32,
    pub fields_taken: u64,
}

struct Board {
    bios: Box<[u8; BIOS_SIZE]>,
    ram: [u8; RAM_SIZE],
    cart: Cartridge,
    vdp: Vdp,
    psg: Psg,
    mode: ControllerMode,
    controllers: [HandController; 2],
    wait_latch: M1WaitLatch,
    /// The last RAM write, as the CPU addressed it, for a trace entry.
    #[cfg(feature = "morepork")]
    last_ram_write: Option<(u16, u8)>,
}

impl Board {
    fn memory(&self, address: u16) -> u8 {
        match MemorySelect::of(address) {
            MemorySelect::Bios => self.bios[address as usize & (BIOS_SIZE - 1)],
            MemorySelect::Expansion20 | MemorySelect::Expansion40 => UNDRIVEN,
            MemorySelect::Ram => self.ram[address as usize & RAM_MASK],
            MemorySelect::Cart(window) => self
                .cart
                .read(window, address & WINDOW_OFFSET)
                .unwrap_or(UNDRIVEN),
        }
    }

    fn controller(&self, port: PortId) -> &HandController {
        &self.controllers[port.0 as usize]
    }
}

impl Bus for Board {
    fn read(&mut self, address: u16) -> u8 {
        self.memory(address)
    }

    /// /WE reaches the RAM alone; the BIOS and the cartridge edge carry no
    /// write strobe.
    fn write(&mut self, address: u16, data: u8) {
        if let MemorySelect::Ram = MemorySelect::of(address) {
            self.ram[address as usize & RAM_MASK] = data;
            #[cfg(feature = "morepork")]
            {
                self.last_ram_write = Some((address, data));
            }
        }
    }

    fn input(&mut self, port: u16) -> u8 {
        match IoSelect::of(port, false) {
            IoSelect::VdpRead if port & VDP_MODE == 0 => self.vdp.read_data(),
            IoSelect::VdpRead => self.vdp.read_status(),
            IoSelect::ControllerRead(connector) => self.controller(connector).read(self.mode),
            _ => UNDRIVEN,
        }
    }

    /// The mode latch takes the strobe alone; the data byte goes nowhere.
    fn output(&mut self, port: u16, data: u8) {
        match IoSelect::of(port, true) {
            IoSelect::KeypadMode => self.mode = ControllerMode::Keypad,
            IoSelect::JoystickMode => self.mode = ControllerMode::Joystick,
            IoSelect::VdpWrite if port & VDP_MODE == 0 => self.vdp.write_data(data),
            IoSelect::VdpWrite => self.vdp.write_control(data),
            IoSelect::PsgWrite => self.psg.write(data),
            _ => {}
        }
    }

    /// The wait latch and the PSG's READY are both open-collector onto the
    /// pulled-up /WAIT line.
    fn wait_requested(&self) -> bool {
        self.wait_latch.q || !self.psg.ready()
    }
}

impl ColecoVision {
    /// A console fitted with the part cut for `standard`, the BIOS in its
    /// socket, and the cartridge in its slot.
    pub fn new(
        rom: &[u8],
        standard: Standard,
        bios: [u8; BIOS_SIZE],
    ) -> Result<ColecoVision, CartridgeError> {
        Ok(ColecoVision {
            cpu: Cpu::new(),
            board: Board {
                bios: Box::new(bios),
                ram: [0; RAM_SIZE],
                cart: Cartridge::load(rom)?,
                vdp: Vdp::new(standard),
                psg: Psg::new(Variant::DiscreteTi),
                mode: ControllerMode::Joystick,
                controllers: [HandController::default(); 2],
                wait_latch: M1WaitLatch::default(),
                #[cfg(feature = "morepork")]
                last_ram_write: None,
            },
            m1_level: false,
            sample_clock: sample_clock(),
            audio: Vec::new(),
            wave_capture: None,
            graphics_capture: false,
            frames_seen: 0,
            #[cfg(feature = "morepork")]
            rom_sha256: missingno_core::machine::rom_fingerprint(rom),
        })
    }

    /// The standard the fitted VDP is cut for.
    pub fn standard(&self) -> Standard {
        self.board.vdp.standard()
    }

    pub fn tv_standard(&self) -> TvStandard {
        match self.standard() {
            Standard::Ntsc => TvStandard::Ntsc,
            Standard::Pal => TvStandard::Pal,
        }
    }

    pub fn vdp(&self) -> &Vdp {
        &self.board.vdp
    }

    pub fn vdp_mut(&mut self) -> &mut Vdp {
        &mut self.board.vdp
    }

    pub fn psg(&self) -> &Psg {
        &self.board.psg
    }

    pub fn psg_mut(&mut self) -> &mut Psg {
        &mut self.board.psg
    }

    /// The 2114 pair's kilobyte, before the decode mirrors it.
    pub fn ram(&self) -> &[u8] {
        &self.board.ram
    }

    /// The RAM write held since the last take, cleared by taking it.
    #[cfg(feature = "morepork")]
    pub(crate) fn take_ram_write(&mut self) -> Option<(u16, u8)> {
        self.board.last_ram_write.take()
    }

    pub fn restore_ram(&mut self, bytes: &[u8]) {
        let len = self.board.ram.len().min(bytes.len());
        self.board.ram[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn board_state(&self) -> BoardState {
        BoardState {
            mode: self.board.mode,
            wait_latch_q: self.board.wait_latch.q,
            controllers: self.board.controllers,
            sample_phase: self.sample_clock.phase() as u32,
            fields_taken: self.frames_seen,
        }
    }

    /// Reseat the board at an instruction boundary, where no M1 cycle is in
    /// flight. Samples already accumulated belong to the timeline being left.
    pub fn restore_board(&mut self, state: &BoardState) {
        self.board.mode = state.mode;
        self.board.wait_latch.q = state.wait_latch_q;
        self.board.controllers = state.controllers;
        self.m1_level = false;
        self.sample_clock.set_phase(state.sample_phase as u64);
        self.frames_seen = state.fields_taken;
        self.audio.clear();
    }

    pub fn at_instruction_boundary(&self) -> bool {
        self.cpu.at_instruction_boundary()
    }

    /// One Z80 T-state, and with it the VDP's three periods and the PSG's one
    /// CLOCK. The VDP runs ahead of the CPU so a port access lands against the
    /// instant it fires on; the wait latch takes the edge that opens the T.
    fn tick(&mut self) {
        let m1_next = self.cpu.m1();
        self.board.vdp.tick(XTALS_PER_TSTATE);
        self.board.psg.tick();
        self.board.wait_latch.clock(self.m1_level, m1_next);
        self.m1_level = m1_next;
        self.cpu.tick(&mut self.board);
        self.cpu.set_nmi(self.board.vdp.interrupt_asserted());

        for _ in 0..self.sample_clock.advance(1) {
            let level = self.board.psg.level();
            self.audio.push((level, level));
            if let Some(rings) = &mut self.wave_capture {
                for (ring, code) in rings.iter_mut().zip(self.board.psg.dac_codes()) {
                    ring.push(code);
                }
            }
        }
    }

    /// One T-state of the board — the grid every chip is stepped on.
    pub fn step_tstate(&mut self) {
        self.tick();
    }

    pub fn step_instruction(&mut self) {
        self.tick();
        while !self.cpu.at_instruction_boundary() {
            self.tick();
        }
    }

    /// Run T-states until the raster leaves the visible picture and the CPU
    /// reaches an instruction boundary, bounded so runaway code cannot stall
    /// the caller.
    pub fn step_frame(&mut self, budget_tstates: u32) -> Option<&Frame> {
        for _ in 0..budget_tstates {
            self.tick();
            if self.board.vdp.frames_completed() != self.frames_seen
                && self.cpu.at_instruction_boundary()
            {
                break;
            }
        }
        self.take_frame()
    }

    /// The completed frame not yet handed out, borrowed from the chip that
    /// rendered it.
    pub fn take_frame(&mut self) -> Option<&Frame> {
        let completed = self.board.vdp.frames_completed();
        if completed == self.frames_seen {
            return None;
        }
        self.frames_seen = completed;
        Some(self.board.vdp.frame())
    }

    /// Accumulated 44.1 kHz stereo samples since the last drain.
    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.audio)
    }

    /// Enable or disable per-channel waveform capture.
    pub fn set_wave_capture(&mut self, on: bool) {
        match (on, self.wave_capture.is_some()) {
            (true, false) => {
                self.wave_capture =
                    Some(std::array::from_fn(|_| WaveRing::new(WAVE_CAPTURE_SAMPLES)));
            }
            (false, true) => self.wave_capture = None,
            _ => {}
        }
    }

    /// Enable or disable the VDP's decode into the debugger's graphics
    /// surfaces.
    pub fn set_graphics_capture(&mut self, on: bool) {
        self.graphics_capture = on;
    }

    pub fn graphics_capture(&self) -> bool {
        self.graphics_capture
    }

    /// The four channels' captured waveforms, or `None` when capture is off.
    pub fn channel_waves(&self) -> Option<Vec<ChannelWave>> {
        let rings = self.wave_capture.as_ref()?;
        let attenuations = self.board.psg.attenuations();
        Some(
            (0..PSG_CHANNELS)
                .map(|channel| ChannelWave {
                    label: WAVE_LABELS[channel],
                    levels: rings[channel].to_vec(),
                    depth_bits: PSG_CODE_BITS,
                    rate: SAMPLE_RATE,
                    active: attenuations[channel] != MUTE_ATTENUATION,
                })
                .collect(),
        )
    }

    /// Side-effect-free bus read for inspection.
    pub fn peek(&self, address: u16) -> u8 {
        self.board.memory(address)
    }

    /// The panel's Reset: `CPU_RESET` reaches the Z80, and the wait and mode
    /// latches with it. The VDP's /RESET is not on that chain, and the RAM
    /// keeps its contents.
    pub fn reset(&mut self) {
        self.cpu.reset();
        self.board.wait_latch = M1WaitLatch::default();
        self.board.mode = ControllerMode::Joystick;
        self.m1_level = false;
    }

    /// Power-cycle: fresh chip state, same cartridge and BIOS, the same
    /// switches held.
    pub fn power_cycle(&mut self) {
        self.cpu = Cpu::new();
        self.board.vdp.reset();
        self.board.psg = Psg::new(Variant::DiscreteTi);
        self.board.ram = [0; RAM_SIZE];
        self.board.mode = ControllerMode::Joystick;
        self.board.wait_latch = M1WaitLatch::default();
        self.m1_level = false;
        self.sample_clock = sample_clock();
        self.audio.clear();
        if let Some(rings) = &mut self.wave_capture {
            rings.iter_mut().for_each(WaveRing::clear);
        }
        self.frames_seen = 0;
    }

    pub fn apply_control(&mut self, control: ControlId, input: ControlInput) {
        let ControlInput::Digital(pressed) = input else {
            return;
        };
        match control.site {
            ControlSite::Panel if control.role == ControlRole::Reset && pressed => self.reset(),
            ControlSite::Port(port @ (PORT1 | PORT2)) => {
                self.board.controllers[port.0 as usize].apply(control.role, pressed);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console(rom: &[u8]) -> ColecoVision {
        ColecoVision::new(rom, Standard::Ntsc, [0; BIOS_SIZE]).expect("a flat image")
    }

    #[test]
    fn the_ram_repeats_every_kilobyte_across_its_window() {
        let mut console = console(&[]);
        console.board.write(0x6000, 0x5A);
        assert_eq!(console.peek(0x6400), 0x5A);
        assert_eq!(console.peek(0x7C00), 0x5A);
        assert_eq!(console.ram()[0], 0x5A);
    }

    #[test]
    fn the_bios_cartridge_and_expansion_windows_take_no_writes() {
        let mut console = console(&[0x11; 0x2000]);
        for address in [0x0000, 0x2000, 0x4000, 0x8000, 0xE000] {
            console.board.write(address, 0x5A);
        }
        assert_eq!(console.peek(0x0000), 0x00);
        assert_eq!(console.peek(0x8000), 0x11);
        assert_eq!(console.ram(), [0; RAM_SIZE]);
    }

    #[test]
    fn an_unselected_window_reads_undriven() {
        let console = console(&[0x11; 0x2000]);
        assert_eq!(console.peek(0x2000), UNDRIVEN);
        assert_eq!(console.peek(0x5FFF), UNDRIVEN);
        assert_eq!(console.peek(0x8000), 0x11);
        assert_eq!(console.peek(0xA000), UNDRIVEN);
    }

    #[test]
    fn only_the_e0_read_half_reaches_the_controllers() {
        let mut console = console(&[]);
        console.apply_control(
            ControlId::port(PORT1, ControlRole::Up),
            ControlInput::Digital(true),
        );
        assert_eq!(console.board.input(0xFC), 0x7E);
        assert_eq!(console.board.input(0xE0), 0x7E);
        assert_eq!(console.board.input(0xE2), 0x7F);
        for port in [0x00, 0x80, 0xC0] {
            assert_eq!(console.board.input(port), UNDRIVEN);
        }
    }

    #[test]
    fn one_write_moves_both_ports_to_the_keypad() {
        let mut console = console(&[]);
        console.apply_control(
            ControlId::port(PORT1, ControlRole::Key(4)),
            ControlInput::Digital(true),
        );
        console.apply_control(
            ControlId::port(PORT2, ControlRole::Key(10)),
            ControlInput::Digital(true),
        );
        console.board.output(0x80, 0x00);
        assert_eq!(console.board.input(0xFC) & 0x0F, 0x3);
        assert_eq!(console.board.input(0xFF) & 0x0F, 0xA);
        console.board.output(0xDF, 0xFF);
        assert_eq!(console.board.input(0xFC), 0x7F);
    }

    /// Clear while /M1 is high, then a toggle per edge: set on the edge after
    /// /M1 falls, clear on the next.
    #[test]
    fn the_wait_latch_rises_once_inside_an_m1_cycle() {
        let mut latch = M1WaitLatch::default();
        let mut trail = Vec::new();
        let m1 = [false, true, true, true, false, false, false];
        for pair in m1.windows(2) {
            latch.clock(pair[0], pair[1]);
            trail.push(latch.q);
        }
        assert_eq!(trail, [false, true, false, false, false, false]);
    }
}
