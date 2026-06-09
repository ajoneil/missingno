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

    /// Post-boot comparator settled against the live LY (LYC=0).
    pub(in crate::ppu) fn post_boot_at_line(ly: u8) -> Self {
        let matches = ly == 0;
        Self {
            comparison_pending: matches,
            comparison_latched: matches,
            ..Self::post_boot()
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

    /// LALU dffsr SUKO 0→1 capture. On TALU↑ leg-swaps applies the AO2222 gate-prop
    /// pulse-width filter; off-TALU evaluations use the boolean rising-edge rule.
    pub(in crate::ppu) fn detect_suko_edge(
        &mut self,
        legs: InterruptFlags,
        talu_rising: bool,
    ) -> bool {
        let prev = self.legs_was_high;
        self.legs_was_high = legs;

        let rising = legs - prev;
        let surviving = prev & legs;

        if rising.is_empty() {
            return false;
        }
        if !surviving.is_empty() {
            return false;
        }
        if !talu_rising {
            return true;
        }
        let falling = prev - legs;
        if falling.is_empty() {
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

/// Gate-prop arrival time of each SUKO source leg at the AO2222 inputs, in ps from
/// the triggering TALU↑.
#[derive(Copy, Clone)]
struct LegArrival {
    rising_ps: u16,
    falling_ps: u16,
}

/// LYC arm via ROPO.dff17 (TALU-clocked, 1 stage).
const LYC_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 874,
    falling_ps: 874,
};
/// Mode 1 arm via NYPE + POPU.dffr + PARU.not_x1 (rising slower than falling — PMOS skew).
const MODE_1_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 2_822,
    falling_ps: 2_300,
};
/// Mode 0 arm via NYPE + POPU + PARU + TOLU.not_x1 + TARU.AND2.
const MODE_0_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 4_038,
    falling_ps: 4_038,
};
/// Mode 2 arm via NYPE + POPU + PARU + TOLU.not_x1 + TAPA.AND2.
const MODE_2_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 3_970,
    falling_ps: 3_970,
};

/// LALU dffsr minimum captured SUKO low-pulse width. Cases 1 (1,802 ps) and 4
/// (1,524 ps) bracket the empirical threshold.
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
        unreachable!("arrival(): non-single-leg flags 0x{:02X}", leg.bits());
    }
}
