//! The TIA's movable objects — two players, two missiles, one ball — plus the
//! playfield. Each movable runs on its own MOTCK grid built from the shared
//! [`counter`] machinery; the playfield is beam-derived and has none.
//!
//! The movables free-run continuously, so at an instruction boundary their
//! counters, ÷4 ring phases, START-pending latches, and serialiser/gate state
//! are all live — full Tier-2a hardware state a bit-exact restore needs. Each
//! object owns its own `capture`/`restore` pair beside its silicon.

mod ball;
mod counter;
mod missile;
mod player;
mod playfield;

pub use ball::Ball;
pub use missile::Missile;
pub use player::Player;
pub use playfield::Playfield;

pub(crate) use ball::BallState;
pub(crate) use missile::MissileState;
pub(crate) use player::{PlayerState, ScanState};
pub(crate) use playfield::PlayfieldState;

use super::motion::{MovableIndex, PerObject};

/// Visible clocks per line — the objects' colour-clock position space.
pub const COUNTER_RANGE: u8 = 160;

/// The five objects the HMOVE engine can move, addressed by [`MovableIndex`]
/// so the motion loops need no per-object branching of their own.
pub(super) struct Movables {
    pub(super) p0: Player,
    pub(super) p1: Player,
    pub(super) m0: Missile,
    pub(super) m1: Missile,
    pub(super) bl: Ball,
}

impl Movables {
    pub(super) fn new() -> Self {
        Movables {
            p0: Player::new(),
            p1: Player::new(),
            m0: Missile::new(),
            m1: Missile::new(),
            bl: Ball::new(),
        }
    }

    pub(super) fn tick(&mut self, which: MovableIndex) {
        match which {
            MovableIndex::P0 => self.p0.tick(),
            MovableIndex::P1 => self.p1.tick(),
            MovableIndex::M0 => self.m0.tick(),
            MovableIndex::M1 => self.m1.tick(),
            MovableIndex::Bl => self.bl.tick(),
        }
    }

    pub(super) fn output(&self, which: MovableIndex) -> bool {
        match which {
            MovableIndex::P0 => self.p0.output(),
            MovableIndex::P1 => self.p1.output(),
            MovableIndex::M0 => self.m0.output(),
            MovableIndex::M1 => self.m1.output(),
            MovableIndex::Bl => self.bl.output(),
        }
    }

    /// One tick ahead of the live state — the merged pulse's early window.
    fn peek(&self, which: MovableIndex) -> bool {
        match which {
            MovableIndex::P0 => {
                let mut ghost = self.p0.clone();
                ghost.tick();
                ghost.output()
            }
            MovableIndex::P1 => {
                let mut ghost = self.p1.clone();
                ghost.tick();
                ghost.output()
            }
            MovableIndex::M0 => {
                let mut ghost = self.m0.clone();
                ghost.tick();
                ghost.output()
            }
            MovableIndex::M1 => {
                let mut ghost = self.m1.clone();
                ghost.tick();
                ghost.output()
            }
            MovableIndex::Bl => {
                let mut ghost = self.bl.clone();
                ghost.tick();
                ghost.output()
            }
        }
    }

    /// A merged stuff previews the serialiser only at phase classes its
    /// clocking derives from: the player at its scan clock's class, the
    /// missile at every class but the pulse class, the ball at every class.
    pub(super) fn seam_preview_fires(&self, which: MovableIndex, final_stuff_slot: bool) -> bool {
        match which {
            MovableIndex::P0 => self.p0.seam_preview_fires(final_stuff_slot),
            MovableIndex::P1 => self.p1.seam_preview_fires(final_stuff_slot),
            MovableIndex::M0 => self.m0.seam_preview_fires(),
            MovableIndex::M1 => self.m1.seam_preview_fires(),
            MovableIndex::Bl => true,
        }
    }

    pub(super) fn pixel(&self, which: MovableIndex, seam: PerObject<bool>) -> bool {
        if seam[which] {
            self.peek(which)
        } else {
            self.output(which)
        }
    }
}
