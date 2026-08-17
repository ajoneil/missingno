//! The TIA's face on the CPU bus: the write decode's arm per register, and
//! the handful of addresses that drive data back.

use super::Tia;
use super::motion::MovableIndex;

impl Tia {
    pub(crate) fn write(&mut self, address: u16, value: u8) {
        use super::registers::*;
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
                // The restart supplies the edge N2057 was denied: a commit whose
                // window closed under the hold, or one whose decode was already
                // satisfied and waiting, lands now. Earlier in the line the
                // pending commit has no decode yet and is simply lost.
                let deferred = self.audio_commit_held || self.audio_commit_armed();
                self.rsync_asserted = false;
                self.audio_commit_held = false;
                self.end_line();
                if deferred {
                    self.commit_audio();
                }
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
            reg @ 0x00..=0x07 => self.collisions.0[reg as usize] | (floating & 0x3F),
            reg @ 0x08..=0x0B => self.input.pot_level((reg - 0x08) as usize) | (floating & 0x7F),
            reg @ (0x0C | 0x0D) => {
                self.input.trigger_level((reg - 0x0C) as usize) | (floating & 0x7F)
            }
            _ => floating,
        }
    }
}
