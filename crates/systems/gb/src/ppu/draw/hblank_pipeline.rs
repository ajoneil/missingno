//! FEPO → WODU → VOGA → WEGO → XYMU clear. Mode 3→0 termination path.

/// WODU = AND2(XUGU, !FEPO) (combinational); VOGA captures it on ALET rising; WEGO clears XYMU.
/// `rendering_active` is true during Mode 3 (opposite polarity to hardware's active-low XYMU).
pub(in crate::ppu) struct HblankPipeline {
    /// XYMU NOR-latch (inverted polarity).
    rendering_active: bool,
    /// VOGA DFF — latches when WODU first rises (combinational on XANO/!FEPO); reset by TADY.
    end_of_line_latched: bool,
    /// AJUJ permit pulse — ~2,100 ps window between BESU.q↓ and mode3 net↑ during the AVAP cascade.
    /// Asserted at AVAP-fall with mode3↑, deasserted at the next master-clock rise.
    ajuj_pulse: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            rendering_active: false,
            end_of_line_latched: false,
            ajuj_pulse: false,
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            rendering_active: false,
            end_of_line_latched: true,
            ajuj_pulse: false,
        }
    }

    /// WODU = AND2(!FEPO, XANO). Zero registered cells between FEPO and WODU on hardware.
    pub(in crate::ppu) fn compute_end_of_line(xano: bool, fepo: bool) -> bool {
        xano && !fepo
    }

    /// Latch VOGA when WODU first rises. FEPO→WODU is combinational; `fepo` must reflect
    /// any same-edge transitions (post-WUTY for rise-side, post-pix-advance for fall-side).
    pub(in crate::ppu) fn latch_end_of_line(&mut self, xano: bool, fepo: bool) -> bool {
        let end_of_line_now = Self::compute_end_of_line(xano, fepo);
        if end_of_line_now {
            self.end_of_line_latched = true;
        }
        end_of_line_now
    }

    /// Clear XYMU on the same-dot ALET-rising edge after VOGA latched.
    /// Returns true iff XYMU just cleared — LCD uses this to push screen_x=159.
    pub(in crate::ppu) fn commit_end_of_line_on_rise(&mut self) -> bool {
        if self.end_of_line_latched && self.rendering_active {
            self.rendering_active = false;
            true
        } else {
            false
        }
    }

    /// AVAP-fall: set XYMU.q (rendering_active=true) and assert the AJUJ permit pulse for write-locks.
    pub(in crate::ppu) fn pulse_ajuj_on_avap_fall(&mut self) {
        self.rendering_active = true;
        self.ajuj_pulse = true;
    }

    /// Close the AJUJ window on the next rise.
    pub(in crate::ppu) fn tick_ajuj_pulse_on_rise(&mut self) {
        self.ajuj_pulse = false;
    }

    /// Write-permit override consumed by oam/vram_write_locked.
    pub(in crate::ppu) fn ajuj_pulse(&self) -> bool {
        self.ajuj_pulse
    }

    pub(in crate::ppu) fn rendering_active(&self) -> bool {
        self.rendering_active
    }

    pub(in crate::ppu) fn end_of_line_latched(&self) -> bool {
        self.end_of_line_latched
    }

    pub(in crate::ppu) fn reset(&mut self) {
        self.rendering_active = false;
        self.end_of_line_latched = false;
        self.ajuj_pulse = false;
    }
}
