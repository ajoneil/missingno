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

mod compose;
mod input;
mod motion;
mod palette;
mod state;

use audio::Channel;
use compose::{Collisions, ColorMux, Pixels};
use hsync::{Beam, HSyncCounter};
use input::InputPorts;
use missingno_core::waveform::WaveRing;
use motion::{MOVABLES, MotionSequencer, MovableIndex, PerObject};
use objects::{Movables, Player, Playfield};

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
/// SHB's latched reset absorbs a WSYNC set through the wrap's first CPU cycle.
const WSYNC_RESET_HOLD_CLOCKS: u8 = 3;
/// The audio two-phase tick positions (colour clock within the line): phase0
/// is the sample window (divider compare, feedback/tap/hold latches), phase1
/// the commit window (noise shift lands, pulse captures, output updates).
/// The 72/156 (phase0) and 112/116 (phase1) splits are the die-measured values.
const AUDIO_PHASE0: [u16; 2] = [9, 81];
const AUDIO_PHASE1: [u16; 2] = [37, 149];

pub(crate) mod registers {
    pub const VSYNC: u16 = 0x00;
    pub const VBLANK: u16 = 0x01;
    pub const WSYNC: u16 = 0x02;
    pub const RSYNC: u16 = 0x03;
    pub const NUSIZ0: u16 = 0x04;
    pub const NUSIZ1: u16 = 0x05;
    pub const COLUP0: u16 = 0x06;
    pub const COLUP1: u16 = 0x07;
    pub const COLUPF: u16 = 0x08;
    pub const COLUBK: u16 = 0x09;
    pub const CTRLPF: u16 = 0x0A;
    pub const REFP0: u16 = 0x0B;
    pub const REFP1: u16 = 0x0C;
    pub const PF0: u16 = 0x0D;
    pub const PF1: u16 = 0x0E;
    pub const PF2: u16 = 0x0F;
    pub const RESP0: u16 = 0x10;
    pub const RESP1: u16 = 0x11;
    pub const RESM0: u16 = 0x12;
    pub const RESM1: u16 = 0x13;
    pub const RESBL: u16 = 0x14;
    pub const AUDC0: u16 = 0x15;
    pub const AUDC1: u16 = 0x16;
    pub const AUDF0: u16 = 0x17;
    pub const AUDF1: u16 = 0x18;
    pub const AUDV0: u16 = 0x19;
    pub const AUDV1: u16 = 0x1A;
    pub const GRP0: u16 = 0x1B;
    pub const GRP1: u16 = 0x1C;
    pub const ENAM0: u16 = 0x1D;
    pub const ENAM1: u16 = 0x1E;
    pub const ENABL: u16 = 0x1F;
    pub const HMP0: u16 = 0x20;
    pub const HMP1: u16 = 0x21;
    pub const HMM0: u16 = 0x22;
    pub const HMM1: u16 = 0x23;
    pub const HMBL: u16 = 0x24;
    pub const VDELP0: u16 = 0x25;
    pub const VDELP1: u16 = 0x26;
    pub const VDELBL: u16 = 0x27;
    pub const RESMP0: u16 = 0x28;
    pub const RESMP1: u16 = 0x29;
    pub const HMOVE: u16 = 0x2A;
    pub const HMCLR: u16 = 0x2B;
    pub const CXCLR: u16 = 0x2C;
}

/// One finished scanline: 160 TIA colour indices plus its VSYNC state.
#[derive(Clone)]
pub struct Scanline {
    pub pixels: [u8; VISIBLE_CLOCKS],
    pub vsync: bool,
}

/// The WSYNC RDY latch. A strobe drops RDY to park the CPU until the line
/// wrap releases it; SHB's latched reset outlasts that wrap, and while it
/// holds a WSYNC set is overridden and never reaches RDY.
struct RdyLatch {
    ready: bool,
    reset_hold: u8,
}

impl RdyLatch {
    fn new() -> Self {
        RdyLatch {
            ready: true,
            reset_hold: 0,
        }
    }

    fn ready(&self) -> bool {
        self.ready
    }

    fn step(&mut self) {
        self.reset_hold = self.reset_hold.saturating_sub(1);
    }

    fn strobe(&mut self) {
        if self.reset_hold == 0 {
            self.ready = false;
        }
    }

    fn release(&mut self) {
        self.ready = true;
        self.reset_hold = WSYNC_RESET_HOLD_CLOCKS;
    }

    fn capture(&self) -> (bool, u8) {
        (self.ready, self.reset_hold)
    }

    fn restore(&mut self, ready: bool, reset_hold: u8) {
        self.ready = ready;
        self.reset_hold = reset_hold;
    }
}

/// The TIA graphics registers driving the picture, copied for the debugger's
/// pixel strips. Write-only on the bus, so the debugger reads them here.
pub struct GraphicsRegisters {
    /// Effective player patterns (the VDELP-selected GRP copy) and their REFP.
    pub grp0: u8,
    pub reflect_p0: bool,
    pub grp1: u8,
    pub reflect_p1: bool,
    /// The playfield's three pattern registers and its CTRLPF reflect bit.
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,
    pub pf_mirrored: bool,
    /// Whether each missile / the ball currently draws.
    pub missile0: bool,
    pub missile1: bool,
    pub ball: bool,
    /// The object colour bytes (COLUP0/COLUP1/COLUPF), TIA-palette indices.
    pub color_p0: u8,
    pub color_p1: u8,
    pub color_pf: u8,
}

/// One audio channel's AUDC/AUDF/AUDV register bytes, copied for the debugger.
/// Write-only on the bus, so the debugger reads them here.
pub struct AudioRegisters {
    /// AUDC waveform/tone class (low 4 bits).
    pub control: u8,
    /// AUDF frequency divider (5 bits).
    pub frequency: u8,
    /// AUDV volume (4 bits).
    pub volume: u8,
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
    /// A merged live stuff stretches the object's motion-clock high through
    /// the stuff slot; the serialiser shows the NEXT clock's output one clock
    /// early, at the ring phase classes each object's clocking derives from
    /// (console-measured: the w1 dot swallows on a final-leading-slot merge
    /// and widens on a wrap-slot merge; nothing persists to the next line).
    seam_lookahead: PerObject<bool>,

    audio: [Channel; 2],
    /// Per-channel DAC-code capture for the debugger's waveform scope. `None`
    /// when no consumer wants it: the phase1 tap is then one branch with no
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
            seam_lookahead: PerObject::splat(false),
            audio: [Channel::new(), Channel::new()],
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

    /// The graphics registers driving the picture, for the debugger's pixel
    /// strips: the two player patterns (effective GRP after VDELP, plus REFP),
    /// the playfield's three pattern registers and its reflect bit, the
    /// missile/ball enables, and each object's colour byte. Inspection only.
    pub fn graphics_registers(&self) -> GraphicsRegisters {
        let player = |p: &Player| {
            (
                if p.vertical_delay {
                    p.graphics_old
                } else {
                    p.graphics_new
                },
                p.reflect,
            )
        };
        let (grp0, reflect_p0) = player(&self.movables.p0);
        let (grp1, reflect_p1) = player(&self.movables.p1);
        GraphicsRegisters {
            grp0,
            reflect_p0,
            grp1,
            reflect_p1,
            pf0: self.playfield.pf0,
            pf1: self.playfield.pf1,
            pf2: self.playfield.pf2,
            pf_mirrored: self.playfield.mirrored,
            missile0: self.movables.m0.enabled && !self.movables.m0.locked_to_player,
            missile1: self.movables.m1.enabled && !self.movables.m1.locked_to_player,
            ball: self.movables.bl.effective_enabled(),
            color_p0: self.mux.color_p0,
            color_p1: self.mux.color_p1,
            color_pf: self.mux.color_pf,
        }
    }

    /// The two audio channels' AUDC/AUDF/AUDV register bytes. Write-only on the
    /// bus, so the debugger reads them here. Inspection only.
    pub fn audio_registers(&self) -> [AudioRegisters; 2] {
        std::array::from_fn(|i| AudioRegisters {
            control: self.audio[i].control,
            frequency: self.audio[i].frequency,
            volume: self.audio[i].volume,
        })
    }

    pub(crate) fn take_line(&mut self) -> Option<Scanline> {
        self.finished_line.take()
    }

    /// Advance one colour clock; completed lines surface via `take_line`.
    pub(crate) fn step_clock(&mut self) {
        self.rdy.step();
        self.motion.step_extension_decode();

        // A stuffed motion pulse ORs onto the object's own motion-clock node:
        // coincident with a firing MOTCK it merges into one stretched pulse
        // (no extra advance, but the serialiser shows the next clock's output
        // one clock early); while MOTCK is gated the stuff is the pulse that
        // moves the object.
        let seam = self.seam_lookahead;
        self.seam_lookahead = PerObject::splat(false);
        let motion_clock = self.hsync.motck_fires();
        if let Some(ticks) = self.motion.step(self.hsync.phase()) {
            for (which, ticked) in ticks.iter() {
                if ticked {
                    if motion_clock {
                        let final_slot = self.hsync.final_stuff_slot();
                        self.seam_lookahead[which] =
                            self.movables.seam_preview_fires(which, final_slot);
                    } else {
                        self.movables.tick(which);
                    }
                }
            }
        }

        if motion_clock {
            for which in MOVABLES {
                self.movables.tick(which);
            }
        }

        match self.hsync.beam() {
            // A visible clock whose motion tick is N90-deferred past the wrap
            // (the line's last pixel) previews it like the merge ghost — the
            // die shows one serialiser tick per clock (m11 wrap runs).
            Beam::Pixel(x) => self.render_clock(x, seam.map(|s| s || !motion_clock)),
            // Inside the HMOVE comb: blanked output.
            Beam::Comb(x) => self.line[x as usize] = 0,
            Beam::Blank => {}
        }

        // The audio circuits clock twice per scanline (~31.4 kHz), each tick a
        // two-phase pair.
        let position = self.hsync.position();
        if AUDIO_PHASE0.contains(&position) {
            self.audio[0].phase0();
            self.audio[1].phase0();
        } else if AUDIO_PHASE1.contains(&position) {
            self.audio[0].phase1();
            self.audio[1].phase1();
            // Tap each channel's DAC conductance once the commit settles. Inert
            // (one branch, no allocation) unless a consumer enabled capture.
            if let Some(rings) = &mut self.wave_capture {
                rings[0].push(self.audio[0].conductance());
                rings[1].push(self.audio[1].conductance());
            }
        }

        if self.hsync.advance(self.motion.extension_armed()) {
            self.end_line();
        }
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

    fn render_clock(&mut self, x: u8, seam: PerObject<bool>) {
        if x.is_multiple_of(4) {
            self.playfield.latch_cell();
        }
        let px = Pixels {
            p0: self.movables.pixel(MovableIndex::P0, seam),
            p1: self.movables.pixel(MovableIndex::P1, seam),
            m0: self.movables.pixel(MovableIndex::M0, seam),
            m1: self.movables.pixel(MovableIndex::M1, seam),
            bl: self.movables.pixel(MovableIndex::Bl, seam),
            pf: self.playfield.pixel(x),
        };

        self.collisions.latch(px);

        let color = if self.vblank {
            0
        } else {
            self.mux.compose(x, px)
        };
        self.line[x as usize] = color & 0xFE;
    }

    /// The reset strobe's address-decoded rise, a clock before the fall.
    pub(crate) fn missile_reset_rise(&mut self, which: usize) {
        match which {
            0 => self.movables.m0.reset_rise(),
            _ => self.movables.m1.reset_rise(),
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

    pub(crate) fn write(&mut self, address: u16, value: u8) {
        use registers::*;
        let objects = &mut self.movables;
        match address & 0x3F {
            VSYNC => self.vsync = value & 0x02 != 0,
            VBLANK => {
                self.vblank = value & 0x02 != 0;
                self.input.write_vblank(value);
            }
            WSYNC => self.rdy.strobe(),
            RSYNC => {
                // The forced wrap ends the line where it stands: the TV
                // gets a short line — undrawn pixels never left the gun.
                let drawn = self.hsync.columns_drawn();
                self.line[drawn..].fill(0);
                self.end_line();
            }
            NUSIZ0 => {
                objects.p0.nusiz = value;
                objects.m0.nusiz = value;
            }
            NUSIZ1 => {
                objects.p1.nusiz = value;
                objects.m1.nusiz = value;
            }
            COLUP0 => self.mux.color_p0 = value,
            COLUP1 => self.mux.color_p1 = value,
            COLUPF => self.mux.color_pf = value,
            COLUBK => self.mux.color_bk = value,
            CTRLPF => {
                self.playfield.mirrored = value & 0x01 != 0;
                self.mux.score_mode = value & 0x02 != 0;
                self.mux.playfield_priority = value & 0x04 != 0;
                objects.bl.width_exponent = (value >> 4) & 0x03;
            }
            REFP0 => objects.p0.reflect = value & 0x08 != 0,
            REFP1 => objects.p1.reflect = value & 0x08 != 0,
            PF0 => self.playfield.pf0 = value,
            PF1 => self.playfield.pf1 = value,
            PF2 => self.playfield.pf2 = value,
            // The strobe grounds the object's ÷4 ring; hblank-vs-visible
            // landings emerge from the MOTCK gating alone.
            RESP0 => objects.p0.reset_position(),
            RESP1 => objects.p1.reset_position(),
            RESM0 => objects.m0.reset_position(),
            RESM1 => objects.m1.reset_position(),
            RESBL => objects.bl.reset_position(),
            AUDC0 => self.audio[0].control = value,
            AUDC1 => self.audio[1].control = value,
            AUDF0 => self.audio[0].frequency = value & 0x1F,
            AUDF1 => self.audio[1].frequency = value & 0x1F,
            AUDV0 => self.audio[0].volume = value & 0x0F,
            AUDV1 => self.audio[1].volume = value & 0x0F,
            // The vertical-delay latches cross-couple: a GRP0 write
            // freezes player 1's old graphics, a GRP1 write freezes
            // player 0's and the ball's.
            GRP0 => {
                objects.p0.graphics_new = value;
                objects.p1.graphics_old = objects.p1.graphics_new;
            }
            GRP1 => {
                objects.p1.graphics_new = value;
                objects.p0.graphics_old = objects.p0.graphics_new;
                objects.bl.enabled_old = objects.bl.enabled_new;
            }
            ENAM0 => objects.m0.enabled = value & 0x02 != 0,
            ENAM1 => objects.m1.enabled = value & 0x02 != 0,
            ENABL => objects.bl.enabled_new = value & 0x02 != 0,
            HMP0 => self.motion.set_hm(MovableIndex::P0, value),
            HMP1 => self.motion.set_hm(MovableIndex::P1, value),
            HMM0 => self.motion.set_hm(MovableIndex::M0, value),
            HMM1 => self.motion.set_hm(MovableIndex::M1, value),
            HMBL => self.motion.set_hm(MovableIndex::Bl, value),
            VDELP0 => objects.p0.vertical_delay = value & 0x01 != 0,
            VDELP1 => objects.p1.vertical_delay = value & 0x01 != 0,
            VDELBL => objects.bl.vertical_delay = value & 0x01 != 0,
            RESMP0 => {
                let lock = value & 0x02 != 0;
                if objects.m0.locked_to_player && !lock {
                    objects
                        .m0
                        .release_at(objects.p0.counter(), objects.p0.nusiz);
                }
                objects.m0.locked_to_player = lock;
            }
            RESMP1 => {
                let lock = value & 0x02 != 0;
                if objects.m1.locked_to_player && !lock {
                    objects
                        .m1
                        .release_at(objects.p1.counter(), objects.p1.nusiz);
                }
                objects.m1.locked_to_player = lock;
            }
            HMOVE => self.motion.strobe(),
            HMCLR => self.motion.clear_hm(),
            CXCLR => self.collisions.clear(),
            _ => {}
        }
    }

    /// What a read returns with `floating` held on the data bus: the TIA
    /// drives D7-D6 on collision reads and D7 on input reads; every
    /// undriven line keeps the bus's retained byte. Side-effect-free.
    pub fn read(&self, address: u16, floating: u8) -> u8 {
        match address & 0x0F {
            reg @ 0x00..=0x07 => self.collisions.read(reg as usize) | (floating & 0x3F),
            reg @ 0x08..=0x0B => self.input.pot_level((reg - 0x08) as usize) | (floating & 0x7F),
            reg @ (0x0C | 0x0D) => {
                self.input.trigger_level((reg - 0x0C) as usize) | (floating & 0x7F)
            }
            _ => floating,
        }
    }
}
