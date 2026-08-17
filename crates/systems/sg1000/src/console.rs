//! The board: a Z80, a TMS9918A, an SN76489AN and a kilobyte of SRAM on one
//! 10.738635 MHz crystal. The crystal is the model's grid — the VDP takes
//! three periods and the PSG one CLOCK per Z80 T-state — and the board itself
//! contributes only decode (two halves of one '139), the joystick
//! multiplexers, and the pause switch on /NMI.

use missingno_core::ClockRatio;
use missingno_core::ports::PortId;
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};
use missingno_core::waveform::{ChannelWave, WaveRing};
use missingno_ti_psg::{MUTE_ATTENUATION, Psg, Variant};
use missingno_ti_vdp::{Frame, Standard, Vdp};
use missingno_zilog_z80::{Bus, Cpu};

use crate::cartridge::{CartType, Cartridge, CartridgeError, UNDRIVEN};

/// The board is cut for NTSC: a TMS9918A, and a 262-line frame.
pub(crate) const STANDARD: Standard = Standard::Ntsc;
/// The 10.738635 MHz crystal over the 3.579545 MHz the Z80 runs at.
const XTALS_PER_TSTATE: u32 = 3;
/// One frame on that grid: 262 lines of 228 T-states.
pub const TSTATES_PER_FRAME: u32 = 228 * 262;

/// The TMM2009 work RAM: 1 KB with only A0-A9 brought out, selected across the
/// whole top 16 KB, so it repeats every kilobyte to $FFFF.
const RAM_SIZE: usize = 0x400;
const RAM_MASK: usize = RAM_SIZE - 1;
/// Where `/CS WRAM` takes over from the cartridge selects.
const RAM_BASE: u16 = 0xC000;

/// The clock the Z80 and the PSG's CLOCK pin share, the crystal divided by
/// three.
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
/// headroom — an NTSC frame is ~736 samples at 44.1 kHz.
const WAVE_CAPTURE_SAMPLES: usize = 800;
/// The channels' display names, in capture order.
const WAVE_LABELS: [&str; PSG_CHANNELS] = ["Tone 1", "Tone 2", "Tone 3", "Noise"];

/// A6 and A7 are the only address lines the I/O half of the '139 decodes, so
/// each select covers a whole $40 block and everything inside it aliases.
const IO_BLOCK: u16 = 0xC0;
/// A0: the VDP's MODE pin, and the joystick multiplexers' select input.
const IO_SELECT: u16 = 0x01;

/// Every joystick line has a pull-up and switches to ground: 1 is released.
const RELEASED: u8 = 0xFF;

/// The two joystick connectors, in the order the multiplexers present them.
pub const JOY1: PortId = PortId(0);
pub const JOY2: PortId = PortId(1);

/// The $DC multiplexer byte: player 1's six lines, then player 2's two
/// vertical ones.
mod dc {
    pub const P1_UP: u8 = 0x01;
    pub const P1_DOWN: u8 = 0x02;
    pub const P1_LEFT: u8 = 0x04;
    pub const P1_RIGHT: u8 = 0x08;
    pub const P1_BUTTON_1: u8 = 0x10;
    pub const P1_BUTTON_2: u8 = 0x20;
    pub const P2_UP: u8 = 0x40;
    pub const P2_DOWN: u8 = 0x80;
}

/// The $DD multiplexer byte: player 2's remaining four lines. `CON` (d4,
/// function undocumented) and three unconnected multiplexer inputs (d5-d7)
/// are tied high and never driven low.
mod dd {
    pub const P2_LEFT: u8 = 0x01;
    pub const P2_RIGHT: u8 = 0x02;
    pub const P2_BUTTON_1: u8 = 0x04;
    pub const P2_BUTTON_2: u8 = 0x08;
}

/// The four blocks the '139's second half decodes from A6/A7.
enum IoBlock {
    /// $00-$3F: no select is wired.
    Unused,
    /// $40-$7F: `/CS PSG`, qualified by nothing but /IORQ.
    Psg,
    /// $80-$BF: `/VDP RD` and `/VDP WR`.
    Vdp,
    /// $C0-$FF: `/JOY SEL`, ORed with /RD so writes here do nothing.
    Joysticks,
}

impl IoBlock {
    fn of(port: u16) -> IoBlock {
        match port & IO_BLOCK {
            0x00 => IoBlock::Unused,
            0x40 => IoBlock::Psg,
            0x80 => IoBlock::Vdp,
            _ => IoBlock::Joysticks,
        }
    }
}

/// Which multiplexer byte a pad line lands in.
enum MuxByte {
    Dc,
    Dd,
}

pub struct Sg1000 {
    pub cpu: Cpu,
    board: Board,
    sample_clock: ClockRatio,
    audio: Vec<(f32, f32)>,
    /// Per-channel DAC codes for the debugger's scope, present only while a
    /// consumer wants them.
    wave_capture: Option<[WaveRing; PSG_CHANNELS]>,
    /// Whether a consumer wants the VDP's memory decoded into the debugger's
    /// graphics surfaces; the walk runs only while it does.
    graphics_capture: bool,
    /// VDP frames already handed out; `take_frame` compares against the
    /// VDP's counter so nothing frame-sized moves on the tick path.
    frames_seen: u64,
}

/// The board's own state beside the chips': the multiplexer bytes the pads
/// drive, the carried phase of the 44.1 kHz output tap, and the fields handed
/// out.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BoardState {
    pub joystick_dc: u8,
    pub joystick_dd: u8,
    pub sample_phase: u32,
    pub fields_taken: u64,
}

struct Board {
    cart: Cartridge,
    ram: [u8; RAM_SIZE],
    vdp: Vdp,
    psg: Psg,
    joy_dc: u8,
    joy_dd: u8,
}

impl Board {
    /// The memory map the '139's first half decodes: the cartridge selects
    /// below `/CS WRAM`, the work RAM mirrored above it — unless the cart is
    /// holding `/DSRAM` high, which takes that select away from the console.
    fn memory(&self, address: u16) -> u8 {
        if let Some(driven) = self.cart.read(address) {
            return driven;
        }
        if self.console_ram_selected(address) {
            return self.ram[address as usize & RAM_MASK];
        }
        UNDRIVEN
    }

    fn console_ram_selected(&self, address: u16) -> bool {
        address >= RAM_BASE && !self.cart.disables_console_ram(address)
    }
}

impl Bus for Board {
    fn read(&mut self, address: u16) -> u8 {
        self.memory(address)
    }

    /// The cycle reaches the cart edge whatever the address; the console's own
    /// RAM takes it only where its select survives.
    fn write(&mut self, address: u16, data: u8) {
        self.cart.write(address, data);
        if self.console_ram_selected(address) {
            self.ram[address as usize & RAM_MASK] = data;
        }
    }

    fn input(&mut self, port: u16) -> u8 {
        match (IoBlock::of(port), port & IO_SELECT) {
            // The PSG has no data outputs, and no select answers $00-$3F.
            (IoBlock::Unused | IoBlock::Psg, _) => UNDRIVEN,
            (IoBlock::Vdp, 0) => self.vdp.read_data(),
            (IoBlock::Vdp, _) => self.vdp.read_status(),
            (IoBlock::Joysticks, 0) => self.joy_dc,
            (IoBlock::Joysticks, _) => self.joy_dd,
        }
    }

    fn output(&mut self, port: u16, data: u8) {
        match (IoBlock::of(port), port & IO_SELECT) {
            (IoBlock::Psg, _) => self.psg.write(data),
            (IoBlock::Vdp, 0) => self.vdp.write_data(data),
            (IoBlock::Vdp, _) => self.vdp.write_control(data),
            (IoBlock::Unused | IoBlock::Joysticks, _) => {}
        }
    }

    /// READY sits on the /WAIT net, so the CPU stalls for as long as the PSG
    /// takes to load the byte it was just handed.
    fn wait_requested(&self) -> bool {
        !self.psg.ready()
    }
}

impl Sg1000 {
    /// A console with the stated board in its slot; without one the image
    /// loads as a plain ROM.
    pub fn new(rom: &[u8], cart_type: Option<CartType>) -> Result<Sg1000, CartridgeError> {
        Ok(Sg1000 {
            cpu: Cpu::new(),
            board: Board {
                cart: Cartridge::load(rom, cart_type)?,
                ram: [0; RAM_SIZE],
                vdp: Vdp::new(STANDARD),
                psg: Psg::new(Variant::DiscreteTi),
                joy_dc: RELEASED,
                joy_dd: RELEASED,
            },
            sample_clock: sample_clock(),
            audio: Vec::new(),
            wave_capture: None,
            graphics_capture: false,
            frames_seen: 0,
        })
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

    /// The TMM2009's kilobyte, before the decode mirrors it.
    pub fn work_ram(&self) -> &[u8] {
        &self.board.ram
    }

    pub fn restore_work_ram(&mut self, bytes: &[u8]) {
        let len = self.board.ram.len().min(bytes.len());
        self.board.ram[..len].copy_from_slice(&bytes[..len]);
    }

    /// The cartridge's own RAM, its chips in decode order; `None` for a board
    /// that carries none.
    pub fn cart_ram(&self) -> Option<Vec<u8>> {
        self.board.cart.ram()
    }

    pub fn restore_cart_ram(&mut self, bytes: &[u8]) {
        self.board.cart.restore_ram(bytes);
    }

    pub fn board_state(&self) -> BoardState {
        BoardState {
            joystick_dc: self.board.joy_dc,
            joystick_dd: self.board.joy_dd,
            sample_phase: self.sample_clock.phase() as u32,
            fields_taken: self.frames_seen,
        }
    }

    /// Reseat the board. Samples already accumulated belong to the timeline
    /// being left, so the pending buffer starts empty.
    pub fn restore_board(&mut self, state: &BoardState) {
        self.board.joy_dc = state.joystick_dc;
        self.board.joy_dd = state.joystick_dd;
        self.sample_clock.set_phase(state.sample_phase as u64);
        self.frames_seen = state.fields_taken;
        self.audio.clear();
    }

    pub fn at_instruction_boundary(&self) -> bool {
        self.cpu.at_instruction_boundary()
    }

    /// One Z80 T-state, and with it the three crystal periods the VDP takes
    /// and the one CLOCK the PSG takes. The VDP runs ahead of the CPU so a
    /// port access lands against the instant it fires on.
    fn tick(&mut self) {
        self.board.vdp.tick(XTALS_PER_TSTATE);
        self.board.psg.tick();
        self.cpu.tick(&mut self.board);
        self.cpu.set_irq(self.board.vdp.interrupt_asserted());

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
    /// reaches an instruction boundary — so a frame handoff is a point a state
    /// can be captured at — bounded so runaway code cannot stall the caller.
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
    /// rendered it so nothing frame-sized is copied on the way out.
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

    /// Enable or disable per-channel waveform capture. Enabling allocates the
    /// four rings once and starts each fresh; disabling frees them, leaving the
    /// sample tap inert.
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
    /// surfaces. Nothing is retained: the decode runs at the instant it is
    /// asked for, so the flag only says whether to run it at all.
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

    /// Power-cycle: fresh chip state, same cartridge and the same lines held.
    /// Cart RAM is unbacked SRAM, so it clears with the console's own.
    pub fn power_cycle(&mut self) {
        self.cpu = Cpu::new();
        self.board.vdp.reset();
        self.board.psg = Psg::new(Variant::DiscreteTi);
        self.board.ram = [0; RAM_SIZE];
        self.board.cart.power_cycle();
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
        if control.site == ControlSite::Panel {
            // SW3 pulls /NMI down; it is not a controller line.
            if control.role == ControlRole::Pause && pressed {
                self.cpu.trigger_nmi();
            }
            return;
        }
        let ControlSite::Port(port) = control.site else {
            return;
        };
        let Some((mux, line)) = pad_line(port, control.role) else {
            return;
        };
        let byte = match mux {
            MuxByte::Dc => &mut self.board.joy_dc,
            MuxByte::Dd => &mut self.board.joy_dd,
        };
        if pressed {
            *byte &= !line;
        } else {
            *byte |= line;
        }
    }
}

/// Where a pad control sits in the multiplexer pair.
fn pad_line(port: PortId, role: ControlRole) -> Option<(MuxByte, u8)> {
    match (port, role) {
        (JOY1, ControlRole::Up) => Some((MuxByte::Dc, dc::P1_UP)),
        (JOY1, ControlRole::Down) => Some((MuxByte::Dc, dc::P1_DOWN)),
        (JOY1, ControlRole::Left) => Some((MuxByte::Dc, dc::P1_LEFT)),
        (JOY1, ControlRole::Right) => Some((MuxByte::Dc, dc::P1_RIGHT)),
        (JOY1, ControlRole::Action(0)) => Some((MuxByte::Dc, dc::P1_BUTTON_1)),
        (JOY1, ControlRole::Action(1)) => Some((MuxByte::Dc, dc::P1_BUTTON_2)),
        (JOY2, ControlRole::Up) => Some((MuxByte::Dc, dc::P2_UP)),
        (JOY2, ControlRole::Down) => Some((MuxByte::Dc, dc::P2_DOWN)),
        (JOY2, ControlRole::Left) => Some((MuxByte::Dd, dd::P2_LEFT)),
        (JOY2, ControlRole::Right) => Some((MuxByte::Dd, dd::P2_RIGHT)),
        (JOY2, ControlRole::Action(0)) => Some((MuxByte::Dd, dd::P2_BUTTON_1)),
        (JOY2, ControlRole::Action(1)) => Some((MuxByte::Dd, dd::P2_BUTTON_2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console(cart_type: Option<CartType>) -> Sg1000 {
        Sg1000::new(&[0x11; 0x8000], cart_type).expect("an image the board holds")
    }

    fn write(console: &mut Sg1000, address: u16, data: u8) {
        console.board.write(address, data);
    }

    /// The kilobyte repeats through `/CS WRAM` on a board that leaves the
    /// select alone — a plain ROM, and both Sega RAM boards.
    #[test]
    fn the_console_ram_keeps_its_window_where_no_cart_drives_dsram() {
        for cart_type in [None, Some(CartType::OthelloRam), Some(CartType::CastleRam)] {
            let mut console = console(cart_type);
            write(&mut console, 0xC000, 0x5A);
            assert_eq!(console.peek(0xC000), 0x5A);
            assert_eq!(console.peek(0xC400), 0x5A);
            assert_eq!(console.peek(0xFC00), 0x5A);
            assert_eq!(console.work_ram()[0], 0x5A);
        }
    }

    /// The cart's own RAM behind `/EXM1` and the console's above it are
    /// separate stores.
    #[test]
    fn a_sega_boards_ram_sits_beside_the_consoles() {
        let mut console = console(Some(CartType::OthelloRam));
        write(&mut console, 0x8000, 0x5A);
        write(&mut console, 0xC000, 0xA5);
        assert_eq!(console.peek(0x8000), 0x5A);
        assert_eq!(console.peek(0xC000), 0xA5);
        assert_eq!(console.cart_ram().map(|ram| ram[0]), Some(0x5A));
        assert_eq!(console.work_ram()[0], 0xA5);
    }

    /// An expander holding `/DSRAM` high answers the whole top window itself;
    /// the console's kilobyte is deselected for reads and writes alike.
    #[test]
    fn an_expander_takes_the_console_ram_window() {
        for cart_type in [CartType::DahjeeA, CartType::DahjeeB] {
            let mut console = console(Some(cart_type));
            write(&mut console, 0xC000, 0x5A);
            assert_eq!(console.peek(0xC000), 0x5A);
            assert_eq!(console.work_ram()[0], 0x00, "{cart_type:?}");
        }
    }

    /// Cart RAM carries no battery, so a power cycle wakes it cleared.
    #[test]
    fn a_power_cycle_clears_cart_ram() {
        let mut console = console(Some(CartType::CastleRam));
        write(&mut console, 0x8000, 0x5A);
        console.power_cycle();
        assert_eq!(console.peek(0x8000), 0x00);
    }
}
