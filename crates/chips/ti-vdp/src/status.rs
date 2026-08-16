//! The status register: the three flags, their read-strobe races, and the
//! byte a read composes.

use crate::Vdp;
use crate::registers::r1;
use crate::standard::XTALS_PER_TSTATE;

mod bits {
    pub const FRAME: u8 = 0x80;
    pub const FIFTH_SPRITE: u8 = 0x40;
    pub const COINCIDENCE: u8 = 0x20;
}

/// The flags a status read presents and clears, with what each set needs
/// to resolve its race against that read.
pub(crate) struct Status {
    pub(crate) frame: bool,
    pub(crate) coincidence: bool,
    pub(crate) fifth_sprite: bool,
    /// XTAL instant of the most recent F / 5S set. A clearing read whose
    /// access strobe contains the instant resolves clear-dominant: the
    /// read reports the pre-set value and its clear swallows the set.
    pub(crate) frame_set_at: u64,
    pub(crate) fifth_sprite_set_at: u64,
    /// Latched fifth-sprite index, presented while 5S is set.
    pub(crate) sprite_field: u8,
}

impl Status {
    pub(crate) const POWER_ON: Self = Status {
        frame: false,
        coincidence: false,
        fifth_sprite: false,
        frame_set_at: 0,
        fifth_sprite_set_at: 0,
        sprite_field: 0,
    };
}

impl Vdp {
    pub fn interrupt_asserted(&self) -> bool {
        self.status.frame && self.registers[1] & r1::INTERRUPT_ENABLE != 0
    }

    /// The status byte as it stands, disturbing nothing — no flag clear, no
    /// latch reset, and no read strobe for a set to lose to.
    pub fn peek_status(&self) -> u8 {
        self.status_byte(self.status.frame, self.status.fifth_sprite)
    }

    pub fn read_status(&mut self) -> u8 {
        self.port.awaiting_second_byte = false;
        // F and 5S sets landing inside the read's strobe lose to its
        // clear (dynamic-node contention). C is set-dominant.
        let value = self.status_byte(
            self.status.frame && !self.set_in_read_strobe(self.status.frame_set_at),
            self.status.fifth_sprite && !self.set_in_read_strobe(self.status.fifth_sprite_set_at),
        );
        self.status.frame = false;
        self.status.fifth_sprite = false;
        self.status.coincidence = false;
        value
    }

    /// The byte the two presented flags and the live scanner field compose.
    fn status_byte(&self, frame: bool, fifth_sprite: bool) -> u8 {
        let mut value = self.scanned_field();
        if frame {
            value |= bits::FRAME;
        }
        if fifth_sprite {
            value |= bits::FIFTH_SPRITE;
        }
        if self.status.coincidence {
            value |= bits::COINCIDENCE;
        }
        value
    }

    /// Whether a set at `set_at` landed inside this read's access strobe
    /// (the T-state whose crystal periods have just elapsed).
    fn set_in_read_strobe(&self, set_at: u64) -> bool {
        self.xtal_total - set_at < XTALS_PER_TSTATE as u64
    }
}
