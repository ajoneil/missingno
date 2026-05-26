//! STAT register state and LALU edge-detection.
//! LYC pipeline: PALY (LY==LYC) → ROPO (TALU-rising DFF) → RUPO (transparent NOR latch).

use bitflags::bitflags;

bitflags! {
    #[derive(Copy, Clone, PartialEq, Eq)]
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const OAM_SCAN             = 0b00100000;
        const VERTICAL_BLANK       = 0b00010000;
        const HORIZONTAL_BLANK     = 0b00001000;
    }
}

pub struct StatInterrupt {
    /// LYC register ($FF45).
    pub(in crate::ppu) lyc: u8,
    /// PALY combinational comparator; recomputed at TALU fall and on LYC writes.
    pub(in crate::ppu) comparison_pending: bool,
    /// ROPO DFF — latched LY==LYC. Reset only by SYS_RST (not VID_RST). Drives STAT bit 2 via transparent RUPO.
    pub(in crate::ppu) comparison_latched: bool,
    /// FF41 bits 3-6 enables + DUMMY pull-up on bit 7.
    pub(in crate::ppu) enables: InterruptFlags,
    /// LALU per-leg previous values — one bit per SUKO source AND-term.
    pub(in crate::ppu) legs_was_high: InterruptFlags,
}

impl StatInterrupt {
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            lyc: 0,
            comparison_pending: true,
            comparison_latched: true,
            enables: InterruptFlags::DUMMY,
            legs_was_high: InterruptFlags::empty(),
        }
    }

    /// PALY recompute: `pending = (ly == lyc)`.
    pub(in crate::ppu) fn update_comparison(&mut self, ly: u8) {
        self.comparison_pending = ly == self.lyc;
    }

    /// ROPO captures comparison_pending on TALU rising.
    pub(in crate::ppu) fn latch_comparison(&mut self) {
        self.comparison_latched = self.comparison_pending;
    }

    /// ROPO.Q — drives STAT bit 2 via the transparent RUPO latch, and also feeds the
    /// LYC-match arm of the STAT-interrupt edge detector.
    pub(in crate::ppu) fn ly_eq_lyc(&self) -> bool {
        self.comparison_latched
    }

    pub(in crate::ppu) fn lyc(&self) -> u8 {
        self.lyc
    }

    pub(in crate::ppu) fn enables(&self) -> InterruptFlags {
        self.enables
    }

    pub(in crate::ppu) fn legs_was_high(&self) -> InterruptFlags {
        self.legs_was_high
    }

    /// Used by the STAT write glitch path to install the transient all-bits-high state.
    pub(in crate::ppu) fn set_enables(&mut self, flags: InterruptFlags) {
        self.enables = flags;
    }

    pub(in crate::ppu) fn write_lyc(&mut self, value: u8, ly: u8) {
        self.lyc = value;
        self.update_comparison(ly);
    }

    pub(in crate::ppu) fn write_stat_bits(&mut self, value: u8) {
        self.enables = InterruptFlags::from_bits_truncate(value);
    }

    /// LALU edge detect: SUKO 0→1 fires, with pulse-width filtering on TALU↑ leg-swaps.
    ///
    /// Real silicon's LALU dffsr captures the SUKO rising edge only if the SUKO low pulse
    /// preceding it exceeds the dffsr's effective minimum-pulse threshold. Sub-threshold
    /// glitches (~1.5 ns from TOLU-stage TALU-edge swaps) are ignored. dmg-sim doesn't model
    /// this; per-leg / two-phase logic over-fires on Case 4 (1,524 ps glitch).
    ///
    /// The filter applies only when a TALU↑ DFF capture happened in this evaluation — that's
    /// the regime where multiple legs can transition within sub-ns of each other. Off-TALU
    /// leg-swaps (register writes, WODU edges) are well-separated in time and use the boolean
    /// SUKO rising-edge rule.
    pub(in crate::ppu) fn detect_suko_edge(
        &mut self,
        legs: InterruptFlags,
        talu_rising_in_this_eval: bool,
    ) -> bool {
        let prev = self.legs_was_high;
        self.legs_was_high = legs;

        let rising = legs - prev;
        let falling = prev - legs;
        let surviving = prev & legs;

        if rising.is_empty() {
            return false;
        }
        if !surviving.is_empty() {
            return false;
        }
        if falling.is_empty() {
            return true;
        }
        if !talu_rising_in_this_eval {
            return true;
        }
        let min_falling_ps = falling
            .iter()
            .map(|l| arrival(l).falling_ps as i32)
            .min()
            .unwrap();
        let max_rising_ps = rising
            .iter()
            .map(|l| arrival(l).rising_ps as i32)
            .max()
            .unwrap();
        max_rising_ps - min_falling_ps >= SUKO_CAPTURE_THRESHOLD_PS
    }

    /// Prime the per-leg baseline at LCD-enable or snapshot restore to avoid a spurious first edge.
    pub(in crate::ppu) fn prime_legs(&mut self, legs: InterruptFlags) {
        self.legs_was_high = legs;
    }
}

/// Gate-prop arrival time of each SUKO source leg at the AO2222 inputs, in ps from the
/// triggering TALU↑. Constants come from spec §8.5.1 Cases 1, 3, 4.
#[derive(Copy, Clone)]
struct LegArrival {
    rising_ps: u16,
    falling_ps: u16,
}

const LYC_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 874,
    falling_ps: 874,
};
const MODE_1_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 2_822,
    falling_ps: 2_300,
};
const MODE_0_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 4_038,
    falling_ps: 4_038,
};
const MODE_2_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 3_970,
    falling_ps: 3_970,
};

/// dffsr LALU capture threshold. Sits between Case 4 (1,524 ps, no fire) and Case 1
/// (1,802 ps, fires); 1,700 ps is midway.
const SUKO_CAPTURE_THRESHOLD_PS: i32 = 1_700;

fn arrival(leg: InterruptFlags) -> LegArrival {
    if leg == InterruptFlags::CURRENT_LINE_COMPARE {
        LYC_ARRIVAL
    } else if leg == InterruptFlags::VERTICAL_BLANK {
        MODE_1_ARRIVAL
    } else if leg == InterruptFlags::HORIZONTAL_BLANK {
        MODE_0_ARRIVAL
    } else if leg == InterruptFlags::OAM_SCAN {
        MODE_2_ARRIVAL
    } else {
        LegArrival {
            rising_ps: 0,
            falling_ps: 0,
        }
    }
}
