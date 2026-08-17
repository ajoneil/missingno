//! The HMOVE motion engine: the SEC decode, the ripple counter, the per-object
//! "more movement" latches, and the hblank extension the same strobe arms.

use std::ops::{Index, IndexMut};

/// Colour clocks from an HMOVE write reaching the TIA to its SEC decode.
const HBLANK_EXTENSION_DECODE_CLOCKS: u8 = 3;
/// H@1 (visible x ≡ 1 mod 4): each latched object stuffs a clock and its
/// comparator applies the H@2-captured HM value.
const MOTION_STUFF_PHASE: u16 = 1;
/// H@2 (x ≡ 3 = H@1 + 2): the ripple counter decrements.
const MOTION_DECREMENT_PHASE: u16 = 3;
/// The motion ripple counter's value between sequences (%1111).
const RESTING_RIPPLE: u8 = 15;

#[derive(Clone, Copy)]
pub(super) enum MovableIndex {
    P0,
    P1,
    M0,
    M1,
    Bl,
}

pub(super) const MOVABLES: [MovableIndex; 5] = [
    MovableIndex::P0,
    MovableIndex::P1,
    MovableIndex::M0,
    MovableIndex::M1,
    MovableIndex::Bl,
];

/// A value per movable object: named fields for a known object, `Index` by a
/// runtime [`MovableIndex`] for the sequencer's loops, `iter` to visit all five.
#[derive(Clone, Copy)]
pub(super) struct PerObject<T> {
    pub(super) p0: T,
    pub(super) p1: T,
    pub(super) m0: T,
    pub(super) m1: T,
    pub(super) bl: T,
}

impl<T: Copy> PerObject<T> {
    pub(super) fn splat(value: T) -> Self {
        PerObject {
            p0: value,
            p1: value,
            m0: value,
            m1: value,
            bl: value,
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (MovableIndex, T)> + '_ {
        MOVABLES.into_iter().map(|which| (which, self[which]))
    }
}

impl<T> Index<MovableIndex> for PerObject<T> {
    type Output = T;
    fn index(&self, which: MovableIndex) -> &T {
        match which {
            MovableIndex::P0 => &self.p0,
            MovableIndex::P1 => &self.p1,
            MovableIndex::M0 => &self.m0,
            MovableIndex::M1 => &self.m1,
            MovableIndex::Bl => &self.bl,
        }
    }
}

impl<T> IndexMut<MovableIndex> for PerObject<T> {
    fn index_mut(&mut self, which: MovableIndex) -> &mut T {
        match which {
            MovableIndex::P0 => &mut self.p0,
            MovableIndex::P1 => &mut self.p1,
            MovableIndex::M0 => &mut self.m0,
            MovableIndex::M1 => &mut self.m1,
            MovableIndex::Bl => &mut self.bl,
        }
    }
}

/// [SEC] propagating through the HSync two-phase clock after an HMOVE strobe.
/// The strobe sets the latch transparently; it is sampled at the first H@2
/// strictly after it set (a set coincident with the slot start misses), clocks
/// through the next H@1, then arms on the following H@2 — a three-stage
/// two-phase shift, so the arm delay falls out of the grid per strobe parity.
#[derive(Clone, Copy)]
pub(super) enum MotionArmDecode {
    Idle,
    /// Latched by the strobe; the next strict H@2 samples it.
    Set,
    /// Sampled at H@2; the next H@1 clocks it forward.
    Sampled,
    /// Clocked through H@1; the next H@2 arms the latches.
    Clocked,
}

/// The HMOVE motion engine. A strobe latches [SEC], which clocks through the
/// two-phase clock ([`MotionArmDecode`]) to arm every object's "more movement" latch
/// and load a 4-bit ripple counter to 15. The ripple counts down one step per
/// H@2; each H@1 every latched object gets a stuffed motion clock, and each
/// object's comparator clears its latch when the ripple reaches that object's
/// HM nibble with D7 inverted (so the stuffed count is 0..15 = 8 − net move).
/// The compare reads the HM value captured at the previous H@2 edge — 2 CLK
/// before the pulse — during descent and at rest alike, so a mid-sequence
/// rewrite that dodges every remaining ripple value never clears the latch —
/// it stuffs a clock every line until the next HMOVE (the Cosmic Ark
/// starfield) — and a resting rewrite to $8x clears only from the capture
/// after its write. HMCLR zeroes the HM values only.
///
/// The same strobe latches the 8-clock hblank extension (the HMOVE comb): its
/// SEC decode counts down to arming the reset-select the HSync counter samples.
pub(super) struct MotionSequencer {
    /// [SEC] shifting through the two-phase clock to arm the motion.
    pub(super) arm_stage: MotionArmDecode,
    /// Set for the strobe's own colour clock: its H@2 must not sample it.
    pub(super) just_strobed: bool,
    /// The 4-bit ripple counter, 15 down to 0; `None` once exhausted (the
    /// comparator then rests against %1111).
    pub(super) ripple: Option<u8>,
    pub(super) more_movement: PerObject<bool>,
    pub(super) hm_values: PerObject<u8>,
    /// HM values as of the last H@2 (decrement) edge; the comparator reads
    /// this capture, never the register file directly.
    pub(super) captured_hm_values: PerObject<u8>,
    /// HMOVE's SEC decode on its way to (or holding at) the reset-select.
    pub(super) extension_pending: Option<u8>,
    pub(super) extension_armed: bool,
}

impl MotionSequencer {
    pub(super) fn new() -> Self {
        MotionSequencer {
            arm_stage: MotionArmDecode::Idle,
            just_strobed: false,
            ripple: None,
            more_movement: PerObject::splat(false),
            hm_values: PerObject::splat(0),
            captured_hm_values: PerObject::splat(0),
            extension_pending: None,
            extension_armed: false,
        }
    }

    pub(super) fn strobe(&mut self) {
        self.arm_stage = MotionArmDecode::Set;
        self.just_strobed = true;
        self.extension_pending = Some(HBLANK_EXTENSION_DECODE_CLOCKS);
    }

    pub(super) fn set_hm(&mut self, which: MovableIndex, value: u8) {
        self.hm_values[which] = value;
    }

    pub(super) fn clear_hm(&mut self) {
        self.hm_values = PerObject::splat(0);
    }

    /// Whether the comb is holding the blank late, for the HSync counter's
    /// RHB decode.
    pub(super) fn extension_armed(&self) -> bool {
        self.extension_armed
    }

    /// The line wrap releases the comb; a pending decode rides through.
    pub(super) fn release_extension(&mut self) {
        self.extension_armed = false;
    }

    /// One colour clock of the extension's SEC decode, ahead of the motion
    /// step it shares the strobe with.
    pub(super) fn step_extension_decode(&mut self) {
        if let Some(remaining) = self.extension_pending {
            if remaining == 0 {
                self.extension_pending = None;
                self.extension_armed = true;
            } else {
                self.extension_pending = Some(remaining - 1);
            }
        }
    }

    /// Advance one colour clock; `Some(ticks)` on an H@1 stuff, where a set
    /// latch requests an extra motion clock for its object.
    pub(super) fn step(&mut self, phase: u16) -> Option<PerObject<bool>> {
        // Every H@2 edge captures the register file, even when the SEC shift
        // consumes the clock — the arm H@2 provides the first pulse's capture
        // and quiet H@2s keep it tracking the resting value.
        if phase == MOTION_DECREMENT_PHASE {
            self.captured_hm_values = self.hm_values;
        }
        // [SEC] shifts H@2 → H@1 → H@2; on the final H@2 the more-movement
        // latches arm for every object and the ripple counter loads to 15.
        let strobed_this_clock = self.just_strobed;
        self.just_strobed = false;
        match (self.arm_stage, phase) {
            (MotionArmDecode::Set, MOTION_DECREMENT_PHASE) if !strobed_this_clock => {
                self.arm_stage = MotionArmDecode::Sampled;
                return None;
            }
            (MotionArmDecode::Sampled, MOTION_STUFF_PHASE) => {
                self.arm_stage = MotionArmDecode::Clocked;
                return None;
            }
            (MotionArmDecode::Clocked, MOTION_DECREMENT_PHASE) => {
                self.arm_stage = MotionArmDecode::Idle;
                self.more_movement = PerObject::splat(true);
                self.ripple = Some(15);
                // The load edge that arms the counter is not a count edge.
                return None;
            }
            _ => {}
        }

        // H@2: the ripple counter decrements one step — the phase after H@1's
        // stuff, so each H@1 sees 15, 14, 13, … in turn.
        if phase == MOTION_DECREMENT_PHASE {
            self.ripple = match self.ripple {
                Some(0) | None => None,
                Some(r) => Some(r - 1),
            };
            return None;
        }

        // H@1: each latched object stuffs a clock, unless its comparator
        // matches the ripple this step — then it clears instead. The
        // comparator's input is the H@2 capture for descent and rest alike:
        // a rewrite landing after that capture edge is honoured one H@ cycle
        // later. Clearing before the tick keeps the matching step from
        // stuffing (HM $8x → 0 stuffs).
        if phase != MOTION_STUFF_PHASE || !self.any_movement() {
            return None;
        }
        let ripple = self.ripple.unwrap_or(RESTING_RIPPLE);
        let mut ticks = PerObject::splat(false);
        for which in MOVABLES {
            if self.more_movement[which] && ripple == (self.captured_hm_values[which] >> 4) ^ 0x07 {
                self.more_movement[which] = false;
            }
            ticks[which] = self.more_movement[which];
        }
        Some(ticks)
    }

    fn any_movement(&self) -> bool {
        self.more_movement.iter().any(|(_, m)| m)
    }
}
