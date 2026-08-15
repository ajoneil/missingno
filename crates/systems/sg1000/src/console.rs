//! The board: a Z80, a TMS9918A, an SN76489AN and a kilobyte of SRAM on one
//! 10.738635 MHz crystal. The crystal is the model's grid — the VDP takes
//! three periods and the PSG one CLOCK per Z80 T-state — and the board itself
//! contributes only decode (two halves of one '139), the joystick
//! multiplexers, and the pause switch on /NMI.

use missingno_core::ports::PortId;
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};
use missingno_core::waveform::{ChannelWave, WaveRing};
use missingno_ti_psg::{Psg, Variant};
use missingno_ti_vdp::{Frame, Standard, Vdp, XTALS_PER_TSTATE};
use missingno_zilog_z80::{Bus, Cpu};

use crate::cartridge::{Cartridge, CartridgeError, UNDRIVEN};

/// The TMM2009 work RAM: 1 KB with only A0-A9 brought out, selected across the
/// whole top 16 KB, so it repeats every kilobyte to $FFFF.
const RAM_SIZE: usize = 0x400;
const RAM_MASK: usize = RAM_SIZE - 1;
/// Where `/CS WRAM` takes over from the cartridge selects.
const RAM_BASE: u16 = 0xC000;

/// 44.1 kHz output from the 3.579545 MHz CPU/PSG clock.
const SAMPLE_RATE: u32 = 44_100;
const TSTATES_PER_SAMPLE: f32 = 3_579_545.0 / SAMPLE_RATE as f32;

const PSG_CHANNELS: usize = 4;
/// The attenuation that switches a channel off.
const MUTE_ATTENUATION: u8 = 0x0F;
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
    sample_clock: f32,
    audio: Vec<(f32, f32)>,
    /// Per-channel DAC codes for the debugger's scope, present only while a
    /// consumer wants them.
    wave_capture: Option<[WaveRing; PSG_CHANNELS]>,
    /// VDP frames already handed out; `take_frame` compares against the
    /// VDP's counter so nothing frame-sized moves on the tick path.
    frames_seen: u64,
}

struct Board {
    cart: Cartridge,
    ram: [u8; RAM_SIZE],
    vdp: Vdp,
    psg: Psg,
    joy_dc: u8,
    joy_dd: u8,
}

impl Bus for Board {
    fn read(&mut self, address: u16) -> u8 {
        if address < RAM_BASE {
            self.cart.read(address)
        } else {
            self.ram[address as usize & RAM_MASK]
        }
    }

    /// Nothing below the RAM select is writable: no SG-1000 cart in this cut
    /// carries RAM.
    fn write(&mut self, address: u16, data: u8) {
        if address >= RAM_BASE {
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
    pub fn new(rom: &[u8]) -> Result<Sg1000, CartridgeError> {
        Ok(Sg1000 {
            cpu: Cpu::new(),
            board: Board {
                cart: Cartridge::load(rom)?,
                ram: [0; RAM_SIZE],
                vdp: Vdp::new(Standard::Ntsc),
                psg: Psg::new(Variant::DiscreteTi),
                joy_dc: RELEASED,
                joy_dd: RELEASED,
            },
            sample_clock: 0.0,
            audio: Vec::new(),
            wave_capture: None,
            frames_seen: 0,
        })
    }

    pub fn vdp(&self) -> &Vdp {
        &self.board.vdp
    }

    pub fn psg(&self) -> &Psg {
        &self.board.psg
    }

    /// One Z80 T-state, and with it the three crystal periods the VDP takes
    /// and the one CLOCK the PSG takes. The VDP runs ahead of the CPU so a
    /// port access lands against the instant it fires on.
    fn tick(&mut self) {
        self.board.vdp.tick(XTALS_PER_TSTATE);
        self.board.psg.tick();
        self.cpu.tick(&mut self.board);
        self.cpu.set_irq(self.board.vdp.interrupt_asserted());

        self.sample_clock += 1.0;
        while self.sample_clock >= TSTATES_PER_SAMPLE {
            self.sample_clock -= TSTATES_PER_SAMPLE;
            let level = self.board.psg.level();
            self.audio.push((level, level));
            if let Some(rings) = &mut self.wave_capture {
                for (ring, code) in rings.iter_mut().zip(self.board.psg.dac_codes()) {
                    ring.push(code);
                }
            }
        }
    }

    pub fn step_instruction(&mut self) {
        self.tick();
        while !self.cpu.at_instruction_boundary() {
            self.tick();
        }
    }

    /// Run T-states until the raster leaves the visible picture, bounded so
    /// runaway code cannot stall the caller.
    pub fn step_frame(&mut self, budget_tstates: u32) -> Option<Frame> {
        for _ in 0..budget_tstates {
            self.tick();
            if self.board.vdp.frames_completed() != self.frames_seen {
                return self.take_frame();
            }
        }
        None
    }

    /// The completed frame not yet handed out, copied once at consumption.
    pub fn take_frame(&mut self) -> Option<Frame> {
        let completed = self.board.vdp.frames_completed();
        (completed != self.frames_seen).then(|| {
            self.frames_seen = completed;
            self.board.vdp.frame().clone()
        })
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
        if address < RAM_BASE {
            self.board.cart.read(address)
        } else {
            self.board.ram[address as usize & RAM_MASK]
        }
    }

    /// Power-cycle: fresh chip state, same cartridge and the same lines held.
    pub fn power_cycle(&mut self) {
        self.cpu = Cpu::new();
        self.board.vdp = Vdp::new(Standard::Ntsc);
        self.board.psg = Psg::new(Variant::DiscreteTi);
        self.board.ram = [0; RAM_SIZE];
        self.sample_clock = 0.0;
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
