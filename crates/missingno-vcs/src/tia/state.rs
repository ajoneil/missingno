//! The TIA's save/restore boundary state.

use super::audio::ChannelState;
use super::motion::{MotionArmDecode, PerObject};
use super::objects::{BallState, MissileState, PlayerState, PlayfieldState};
use super::{Tia, VISIBLE_CLOCKS};

/// The TIA's boundary state for a save/restore. Every object counter, ring
/// phase, serialiser latch, motion stage, collision latch, and audio-tick
/// latch is live at an instruction boundary (the beam never idles), so this is
/// the full Tier-2a hardware set. The line output buffer and the finished-line
/// handoff are frame-assembly, not hardware, and are reconstructed empty.
pub(crate) struct TiaState {
    pub vsync: bool,
    pub vblank: bool,
    pub cpu_ready: bool,
    pub wsync_reset_hold: u8,
    pub hsync_position: u16,
    pub hblank_release: u16,
    pub color_p0: u8,
    pub color_p1: u8,
    pub color_pf: u8,
    pub color_bk: u8,
    pub playfield_priority: bool,
    pub score_mode: bool,
    /// HMOVE SEC shift stage (0 idle / 1 set / 2 sampled / 3 clocked).
    pub mot_arm_stage: u8,
    pub mot_just_strobed: bool,
    /// The 4-bit ripple counter, live only while `mot_ripple_active`.
    pub mot_ripple_active: bool,
    pub mot_ripple: u8,
    /// Per-object [P0, P1, M0, M1, Bl] motion state.
    pub mot_more_movement: [bool; 5],
    pub mot_hm_values: [u8; 5],
    pub mot_captured_hm: [u8; 5],
    pub hblank_ext_pending_active: bool,
    pub hblank_ext_pending: u8,
    pub hblank_ext_armed: bool,
    pub seam_lookahead: [bool; 5],
    pub collisions: [u8; 8],
    pub player0: PlayerState,
    pub player1: PlayerState,
    pub missile0: MissileState,
    pub missile1: MissileState,
    pub ball: BallState,
    pub playfield: PlayfieldState,
    pub audio: [ChannelState; 2],
    pub triggers: [bool; 2],
    pub trigger_latch_enabled: bool,
    pub trigger_latches: [bool; 2],
    /// Paddle knob positions quantised to 16 bits, and the RC-charge countdown.
    pub pot_position: [u16; 4],
    pub pot_countdown: [u16; 4],
    pub pot_dumped: bool,
}

fn flatten<T: Copy>(values: &PerObject<T>) -> [T; 5] {
    [values.p0, values.p1, values.m0, values.m1, values.bl]
}

fn unflatten<T: Copy>(values: &mut PerObject<T>, saved: [T; 5]) {
    values.p0 = saved[0];
    values.p1 = saved[1];
    values.m0 = saved[2];
    values.m1 = saved[3];
    values.bl = saved[4];
}

impl Tia {
    /// The TIA's full boundary state for a save. The line output buffer and the
    /// finished-line handoff are frame-assembly (reconstructed empty on restore),
    /// and waveform capture is a debugger tap, so neither travels here.
    pub(crate) fn capture(&self) -> TiaState {
        let (hsync_position, hblank_release) = self.hsync.capture();
        let (cpu_ready, wsync_reset_hold) = self.rdy.capture();
        TiaState {
            vsync: self.vsync,
            vblank: self.vblank,
            cpu_ready,
            wsync_reset_hold,
            hsync_position,
            hblank_release,
            color_p0: self.mux.color_p0,
            color_p1: self.mux.color_p1,
            color_pf: self.mux.color_pf,
            color_bk: self.mux.color_bk,
            playfield_priority: self.mux.playfield_priority,
            score_mode: self.mux.score_mode,
            mot_arm_stage: match self.motion.arm_stage {
                MotionArmDecode::Idle => 0,
                MotionArmDecode::Set => 1,
                MotionArmDecode::Sampled => 2,
                MotionArmDecode::Clocked => 3,
            },
            mot_just_strobed: self.motion.just_strobed,
            mot_ripple_active: self.motion.ripple.is_some(),
            mot_ripple: self.motion.ripple.unwrap_or(0),
            mot_more_movement: flatten(&self.motion.more_movement),
            mot_hm_values: flatten(&self.motion.hm_values),
            mot_captured_hm: flatten(&self.motion.captured_hm_values),
            hblank_ext_pending_active: self.motion.extension_pending.is_some(),
            hblank_ext_pending: self.motion.extension_pending.unwrap_or(0),
            hblank_ext_armed: self.motion.extension_armed,
            seam_lookahead: flatten(&self.seam_lookahead),
            collisions: self.collisions.0,
            player0: self.movables.p0.capture(),
            player1: self.movables.p1.capture(),
            missile0: self.movables.m0.capture(),
            missile1: self.movables.m1.capture(),
            ball: self.movables.bl.capture(),
            playfield: self.playfield.capture(),
            audio: [self.audio[0].capture(), self.audio[1].capture()],
            triggers: self.input.triggers,
            trigger_latch_enabled: self.input.trigger_latch_enabled,
            trigger_latches: self.input.trigger_latches,
            pot_position: std::array::from_fn(|i| {
                (self.input.pots[i].position().clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
            }),
            pot_countdown: std::array::from_fn(|i| self.input.pots[i].countdown()),
            pot_dumped: self.input.pot_dumped,
        }
    }

    /// Rebuild the TIA in place from a saved boundary state. The line output
    /// buffer, finished-line handoff, and waveform capture reset to their idle
    /// defaults — they are frame-assembly and debugger surfaces, not hardware.
    pub(crate) fn restore(&mut self, s: &TiaState) {
        self.hsync.restore(s.hsync_position, s.hblank_release);
        self.rdy.restore(s.cpu_ready, s.wsync_reset_hold);
        self.vsync = s.vsync;
        self.vblank = s.vblank;
        self.mux.color_p0 = s.color_p0;
        self.mux.color_p1 = s.color_p1;
        self.mux.color_pf = s.color_pf;
        self.mux.color_bk = s.color_bk;
        self.mux.playfield_priority = s.playfield_priority;
        self.mux.score_mode = s.score_mode;
        self.motion.arm_stage = match s.mot_arm_stage {
            1 => MotionArmDecode::Set,
            2 => MotionArmDecode::Sampled,
            3 => MotionArmDecode::Clocked,
            _ => MotionArmDecode::Idle,
        };
        self.motion.just_strobed = s.mot_just_strobed;
        self.motion.ripple = s.mot_ripple_active.then_some(s.mot_ripple & 0x0F);
        unflatten(&mut self.motion.more_movement, s.mot_more_movement);
        unflatten(&mut self.motion.hm_values, s.mot_hm_values);
        unflatten(&mut self.motion.captured_hm_values, s.mot_captured_hm);
        self.motion.extension_pending = s.hblank_ext_pending_active.then_some(s.hblank_ext_pending);
        self.motion.extension_armed = s.hblank_ext_armed;
        unflatten(&mut self.seam_lookahead, s.seam_lookahead);
        self.collisions.0 = s.collisions;
        self.movables.p0.restore(&s.player0);
        self.movables.p1.restore(&s.player1);
        self.movables.m0.restore(&s.missile0);
        self.movables.m1.restore(&s.missile1);
        self.movables.bl.restore(&s.ball);
        self.playfield.restore(&s.playfield);
        self.audio[0].restore(&s.audio[0]);
        self.audio[1].restore(&s.audio[1]);
        self.input.triggers = s.triggers;
        self.input.trigger_latch_enabled = s.trigger_latch_enabled;
        self.input.trigger_latches = s.trigger_latches;
        for i in 0..4 {
            self.input.restore_pot(
                i,
                s.pot_position[i] as f32 / u16::MAX as f32,
                s.pot_countdown[i],
            );
        }
        self.input.pot_dumped = s.pot_dumped;
        self.line = [0; VISIBLE_CLOCKS];
        self.finished_line = None;
    }
}
