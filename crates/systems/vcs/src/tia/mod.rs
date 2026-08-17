//! TIA: beam timing, the five movable objects, playfield, collisions, and
//! the HMOVE motion mechanism.
//!
//! Motion is modelled as the hardware does it: HMOVE arms per-object
//! "more movement" latches; stuffed pulses ride the HSync counter's
//! line-fixed H@1 grid, each comparing a descending ripple against the
//! D7-inverted HM values captured at the H@2 edge until the latch clears;
//! the strobe also latches an 8-clock hblank extension (the HMOVE comb).
//! A stuffed pulse rides the object's own motion-clock node: while MOTCK
//! is gated it moves the object; coincident with a firing MOTCK it merges
//! into one pulse. Late/mid-line strobes reuse the same machinery, so the
//! classic "illegal HMOVE" positions emerge rather than being special-cased.

pub(crate) mod audio;
pub(crate) mod hsync;
pub(crate) mod objects;
pub(crate) mod registers;

mod bus;
mod compose;
mod input;
mod inspect;
mod motion;
mod palette;
mod rdy;
mod state;

use audio::Channel;
use compose::{Collisions, ColorMux, Pixels};
use hsync::{Beam, HSyncCounter};
use input::InputPorts;
use missingno_core::waveform::WaveRing;
use motion::{MOVABLES, MotionSequencer, MovableIndex, PerObject};
use objects::{Movables, Playfield};
use rdy::RdyLatch;

pub use inspect::{AudioRegisters, GraphicsRegisters};
pub use palette::{palette, palette_index};
pub(crate) use state::TiaState;

/// Waveform-capture ring depth: one field-window of audio ticks. The AUDx
/// circuits commit twice per line, so a field is ~524 ticks (NTSC, 262 lines)
/// or ~624 (PAL/SECAM, 312); 640 holds one with headroom, drained per frame.
const WAVE_CAPTURE_SAMPLES: usize = 640;

pub const CLOCKS_PER_LINE: u16 = 228;
pub const HBLANK_CLOCKS: u16 = 68;
pub const VISIBLE_CLOCKS: usize = 160;
const LATE_HBLANK_CLOCKS: u16 = HBLANK_CLOCKS + 8;
/// The audio tick positions (colour clock within the line). The sample clock
/// (N1421) holds the divider compare and the feedback/tap/hold decodes; the
/// commit clock (N325) lands the noise shift and the pulse capture. Against
/// N232, the line-start clear, the sample holds at CC 8/80 (a 72/156 line
/// split) and the commit becomes visible at 36/148 (112/116); each reads one
/// higher here because the decode runs before the counter advances.
/// N232 clears the decode's stage bank per line, so an RSYNC restart replays the
/// positions it had not yet passed.
const AUDIO_SAMPLE: [u16; 2] = [9, 81];
const AUDIO_COMMIT: [u16; 2] = [37, 149];

/// One finished scanline: 160 TIA colour indices plus its VSYNC state.
#[derive(Clone)]
pub struct Scanline {
    pub pixels: [u8; VISIBLE_CLOCKS],
    pub vsync: bool,
}

pub struct Tia {
    hsync: HSyncCounter,
    vsync: bool,
    vblank: bool,
    rdy: RdyLatch,

    movables: Movables,
    playfield: Playfield,

    mux: ColorMux,
    collisions: Collisions,

    motion: MotionSequencer,
    /// A merged live stuff stretches the object's clock node (NOR of MOTCK
    /// and the stuff, N90/N1118) high across the clock boundary: the second
    /// transfer has already committed, so the next MOTCK rise is the same
    /// pulse and delivers no advance (Sim2600 live-seam: 145 rises deliver
    /// 160 advances on a strobe line).
    subsume_next_edge: PerObject<bool>,

    audio: [Channel; 2],
    /// RSYNC's decoded level, up from two colour clocks before its wrap. While
    /// it holds it grounds the audio tap's sampling clock (N2057).
    rsync_asserted: bool,
    /// A commit whose window closed under that hold, waiting for the restart to
    /// supply the edge. (N1468's fall landing at CC 0 of the new line)
    audio_commit_held: bool,
    /// Per-channel DAC-code capture for the debugger's waveform scope. `None`
    /// when no consumer wants it: the commit tap is then one branch with no
    /// allocation.
    wave_capture: Option<[WaveRing; 2]>,

    input: InputPorts,

    line: [u8; VISIBLE_CLOCKS],
    finished_line: Option<Scanline>,
}

impl Default for Tia {
    fn default() -> Self {
        Self::new()
    }
}

impl Tia {
    pub fn new() -> Self {
        Tia {
            hsync: HSyncCounter::new(),
            vsync: false,
            vblank: false,
            rdy: RdyLatch::new(),
            movables: Movables::new(),
            playfield: Playfield::new(),
            mux: ColorMux::new(),
            collisions: Collisions::new(),
            motion: MotionSequencer::new(),
            subsume_next_edge: PerObject::splat(false),
            audio: [Channel::new(), Channel::new()],
            rsync_asserted: false,
            audio_commit_held: false,
            wave_capture: None,
            input: InputPorts::new(),
            line: [0; VISIBLE_CLOCKS],
            finished_line: None,
        }
    }

    /// Point a paddle knob: 0.0 charges instantly, 1.0 slowest.
    pub fn set_paddle(&mut self, index: usize, position: f32) {
        self.input.set_paddle(index, position);
    }

    /// A paddle reaches this pot pin, closing its charge path.
    pub fn connect_pot(&mut self, index: usize, position: f32) {
        self.input.connect_pot(index, position);
    }

    /// Nothing reaches this pot pin: it never charges.
    pub fn disconnect_pot(&mut self, index: usize) {
        self.input.disconnect_pot(index);
    }

    /// A controller holds this pot pin at a level — a keypad column, pulled up
    /// by the controller and grounded through a pressed key.
    pub fn drive_pot(&mut self, index: usize, low: bool) {
        self.input.drive_pot(index, low);
    }

    /// A trigger button's state into INPT4/5, true = pressed.
    pub fn set_trigger(&mut self, port: usize, pressed: bool) {
        self.input.set_trigger(port, pressed);
    }

    /// RDY: low while a WSYNC strobe parks the CPU.
    pub(crate) fn cpu_ready(&self) -> bool {
        self.rdy.ready()
    }

    /// The two channels' mixed output, 0.0-1.0. Their pads tie together at
    /// the pins, so the level follows the combined conductance of both DACs'
    /// legs rather than either channel alone.
    pub fn audio_level(&self) -> f32 {
        audio::summing_node_level(self.audio[0].conductance() + self.audio[1].conductance())
    }

    /// Enable or disable per-channel waveform capture. Enabling allocates the
    /// two rings once and starts each fresh; disabling frees them.
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

    /// The two channels' captured DAC-code windows (oldest first) and whether
    /// each channel's DAC is driving, or `None` when capture is off.
    pub fn wave_windows(&self) -> Option<([Vec<u8>; 2], [bool; 2])> {
        let rings = self.wave_capture.as_ref()?;
        let levels = [rings[0].to_vec(), rings[1].to_vec()];
        let active = [self.audio[0].volume > 0, self.audio[1].volume > 0];
        Some((levels, active))
    }

    /// Current colour clock within the line (0..228) — inspection only.
    pub fn beam(&self) -> u16 {
        self.hsync.position()
    }

    pub(crate) fn take_line(&mut self) -> Option<Scanline> {
        self.finished_line.take()
    }

    /// Advance one colour clock; completed lines surface via `take_line`.
    pub(crate) fn step_clock(&mut self) {
        self.rdy.step();
        self.motion.step_extension_decode();

        // A stuffed motion pulse ORs onto the object's own motion-clock node:
        // while MOTCK is gated the stuff is the standalone pulse that moves
        // the object, landing ahead of the sample; coincident with a firing
        // MOTCK it merges into one stretched pulse, handled at the edge phase
        // below.
        let motion_clock = self.hsync.motck_fires();
        let mut merge_advances = PerObject::splat(false);
        if let Some(ticks) = self.motion.step(self.hsync.phase()) {
            for (which, ticked) in ticks.iter() {
                if ticked {
                    if motion_clock {
                        merge_advances[which] = self.movables.merge_delivery_fires(which)
                            && !self.movables.merge_second_transfer_blocked(which);
                    } else {
                        self.movables.tick(which);
                    }
                }
            }
        }

        let beam = self.hsync.beam();
        if let Beam::Pixel(x) = beam
            && x.is_multiple_of(4)
        {
            self.playfield.latch_cell();
        }
        let px = Pixels {
            p0: self.movables.output(MovableIndex::P0),
            p1: self.movables.output(MovableIndex::P1),
            m0: self.movables.output(MovableIndex::M0),
            m1: self.movables.output(MovableIndex::M1),
            bl: self.movables.output(MovableIndex::Bl),
            // The playfield cell decode selects nothing outside the visible
            // counts, so its serial contribution while blanked is low.
            pf: match beam {
                Beam::Pixel(x) => self.playfield.pixel(x),
                Beam::Comb(_) | Beam::Blank => false,
            },
        };

        // The collision matrix samples the serial outputs on every clock;
        // only VBLANK gates it — horizontal blank and the comb do not.
        if !self.vblank {
            self.collisions.latch(px);
        }

        match beam {
            Beam::Pixel(x) => self.render_clock(x, px),
            // Inside the HMOVE comb: blanked output.
            Beam::Comb(x) => self.line[x as usize] = 0,
            Beam::Blank => {}
        }

        // A rise moves the object more than a colour clock before the pixel shows
        // it (Sim2600 live-seam: an advance delivered in clock 4k of a merged pulse
        // first shows at column 4k+2), so each clock's edge fires after its sample.
        // A merged pulse commits its second transfer here too and its stretched
        // high subsumes the object's next rise — unless the bit-0 guard consumes
        // the transfer, which then also leaves no subsume.
        if motion_clock {
            for which in MOVABLES {
                if self.subsume_next_edge[which] {
                    self.subsume_next_edge[which] = false;
                } else {
                    self.movables.tick(which);
                }
                if merge_advances[which] {
                    self.movables.tick(which);
                    self.subsume_next_edge[which] = true;
                }
            }
        }

        // The audio circuits clock twice per scanline (~31.4 kHz), each tick a
        // two-phase pair.
        let position = self.hsync.position();
        if AUDIO_SAMPLE.contains(&position) {
            self.audio[0].sample();
            self.audio[1].sample();
        } else if AUDIO_COMMIT.contains(&position) {
            // RSYNC holding N2057 down defers this commit past the restart.
            match self.rsync_asserted {
                true => self.audio_commit_held = true,
                false => self.commit_audio(),
            }
        }

        if self.hsync.advance(self.motion.extension_armed()) {
            self.end_line();
        }
    }

    /// The commit phase on both channels, plus the debugger's DAC tap once the
    /// commit settles (inert — one branch, no allocation — unless a consumer
    /// enabled capture).
    fn commit_audio(&mut self) {
        self.audio[0].commit_phase();
        self.audio[1].commit_phase();
        if let Some(rings) = &mut self.wave_capture {
            rings[0].push(self.audio[0].conductance());
            rings[1].push(self.audio[1].conductance());
        }
    }

    /// Whether the commit decode is satisfied but its bank clock has not yet
    /// arrived: the N2057 period (4 CLK) ending at a commit slot. A reset in that
    /// window supplies the missing edge; before it, the commit is simply lost.
    fn audio_commit_armed(&self) -> bool {
        let position = self.hsync.position();
        AUDIO_COMMIT
            .iter()
            .any(|&slot| (slot - 3..slot).contains(&position))
    }

    /// The HSync-counter wrap: one mechanism with two triggers — the
    /// natural end of line, and RSYNC forcing it early.
    fn end_line(&mut self) {
        self.hsync.reset_line();
        self.rdy.release();
        self.motion.release_extension();
        self.input.step_pot_charge();
        self.finished_line = Some(Scanline {
            pixels: self.line,
            vsync: self.vsync,
        });
    }

    fn render_clock(&mut self, x: u8, px: Pixels) {
        let color = if self.vblank {
            0
        } else {
            self.mux.compose(x, px)
        };
        self.line[x as usize] = color & 0xFE;
    }

    /// RSYNC's decoded level going up, two colour clocks before its wrap.
    pub(crate) fn rsync_assert(&mut self) {
        self.rsync_asserted = true;
    }

    /// The reset strobe's address-decoded rise, a clock before the fall.
    pub(crate) fn missile_reset_rise(&mut self, which: usize) {
        match which {
            0 => self.movables.m0.reset_rise(),
            _ => self.movables.m1.reset_rise(),
        }
    }

    /// The reset strobe's address-decoded rise, a clock before the fall.
    pub(crate) fn player_reset_rise(&mut self, which: usize) {
        match which {
            0 => self.movables.p0.reset_rise(),
            _ => self.movables.p1.reset_rise(),
        }
    }

    /// The reset strobe's leading scan-kill, one clock before it applies.
    pub(crate) fn missile_reset_kill(&mut self, which: usize) {
        match which {
            0 => self.movables.m0.reset_kill(),
            _ => self.movables.m1.reset_kill(),
        }
    }

    pub(crate) fn ball_reset_kill(&mut self) {
        self.movables.bl.reset_kill();
    }
}
