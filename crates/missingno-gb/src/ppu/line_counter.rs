//! §2.1 + §2.2 line counters composed per hardware cascade.
//!
//! `LineCounter` composes `LineCounterX` (§2.1 scanline dot position; LX
//! 7-bit ripple clocked by TALU) and `LineCounterY` (§2.2 scanline index;
//! LY 8-bit ripple clocked by RUTU when X completes a line). The cascade
//! matches hardware: LX's RUTU pulse clocks LY; no other coupling.
//!
//! Two-level LY vocabulary: `y.value()` is the hardware-internal counter
//! (0-153); `y.value_register()` is the CPU-visible $FF44 value (MYTA-
//! smoothed to 0 during the frame-end transition).
//!
//! Hardware reference: spec §2.1 (LX), §2.2 (LY/POPU/MYTA).

use crate::ppu::line_end_pipeline::NypeEdge;

pub struct LineCounter {
    pub x: LineCounterX,
    pub y: LineCounterY,
}

pub struct LineCounterX {
    pub(in crate::ppu) value: u8,
    pub(in crate::ppu) line_end_detected: bool,
    pub(in crate::ppu) line_end_active: bool,
}

pub struct LineCounterY {
    pub(in crate::ppu) value: u8,
    pub(in crate::ppu) vblank: bool,
    pub(in crate::ppu) popu_holdover: bool,
    pub(in crate::ppu) frame_end_reset: bool,
}

impl LineCounter {
    /// LX counter clock rising — orchestrates POPU/MYTA capture per
    /// NYPE edge from the LineEndPipeline, plus LX advance + SANU
    /// decode. `nype_edge` distinguishes rising (POPU fires) from
    /// falling (MYTA fires) per the NYPE dual-edge distribution.
    pub(in crate::ppu) fn on_lx_counter_clock_rise(&mut self, nype_edge: NypeEdge) {
        match nype_edge {
            NypeEdge::Rising => self.y.capture_popu(),
            NypeEdge::Falling => self.y.capture_myta(),
            NypeEdge::None => {}
        }
        self.x.advance();
        self.x.detect_line_end();
    }

    /// LX counter clock falling — atomic scanline boundary. Hardware-
    /// atomic ordering: RUTU fires → LX resets → LY advances/wraps →
    /// POPU holdover armed on wrap. Returns whether the boundary fired
    /// (for the orchestrator's §7.3 NYPE feed).
    pub(in crate::ppu) fn on_lx_counter_clock_fall(&mut self) -> bool {
        if self.x.line_end_detected {
            self.x.fire_rutu_and_reset();
            let wrap_occurred = self.y.advance_or_wrap();
            self.y.update_popu_holdover(wrap_occurred);
            true
        } else {
            false
        }
    }

    // Pass-through accessors — external callers read line state without
    // reaching through x/y directly.
    pub(in crate::ppu) fn ly(&self) -> u8 {
        self.y.value_register()
    }
    pub(in crate::ppu) fn ly_hardware(&self) -> u8 {
        self.y.value
    }
    pub(in crate::ppu) fn vblank(&self) -> bool {
        self.y.vblank
    }
    pub(in crate::ppu) fn popu_active(&self) -> bool {
        self.y.popu_active()
    }
    pub(in crate::ppu) fn popu_holdover(&self) -> bool {
        self.y.popu_holdover
    }
    pub(in crate::ppu) fn line_end_active(&self) -> bool {
        self.x.line_end_active
    }
    pub(in crate::ppu) fn dot_position(&self) -> u8 {
        self.x.value
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.x.vid_rst();
        self.y.vid_rst();
    }
}

impl LineCounterX {
    /// Advance LX on LX counter clock rising. Suppressed during RUTU
    /// pulse — MUDE async-resets LX at the same TALU falling as RUTU.
    pub(in crate::ppu) fn advance(&mut self) {
        if !self.line_end_active {
            self.value += 1;
        }
        self.line_end_active = false;
    }

    /// SANU combinational LX=113 decode; cached for RUTU to consume on
    /// the next LX counter clock falling.
    pub(in crate::ppu) fn detect_line_end(&mut self) {
        self.line_end_detected = self.value == 113;
    }

    /// RUTU fires on LX counter clock falling when SANU is high: line-
    /// end pulse active, LX async-resets to 0, SANU cleared.
    pub(in crate::ppu) fn fire_rutu_and_reset(&mut self) {
        self.line_end_detected = false;
        self.value = 0;
        self.line_end_active = true;
    }

    pub(in crate::ppu) fn value(&self) -> u8 {
        self.value
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.value = 0;
        self.line_end_detected = false;
        self.line_end_active = false;
    }
}

impl LineCounterY {
    /// LY ripple counter advance or 153→0 wrap. On wrap the MYTA-held
    /// window clears (frame_end_reset=false) and POPU goes low. Returns
    /// true if the wrap occurred.
    pub(in crate::ppu) fn advance_or_wrap(&mut self) -> bool {
        if self.value >= 153 {
            self.value = 0;
            self.frame_end_reset = false;
            self.vblank = false;
            true
        } else {
            self.value += 1;
            false
        }
    }

    /// POPU VBlank capture — fires on NYPE rising edge. Caller gates
    /// on NypeEdge::Rising.
    pub(in crate::ppu) fn capture_popu(&mut self) {
        self.vblank = self.value >= 144;
    }

    /// MYTA FRAME_END capture — fires on NYPE falling edge (nype_n
    /// rising), one TALU period after POPU's capture edge. Caller
    /// gates on NypeEdge::Falling.
    ///
    /// Sets `frame_end_reset` (register smoothing for LY=0).
    pub(in crate::ppu) fn capture_myta(&mut self) {
        if self.value == 153 {
            self.frame_end_reset = true;
        }
    }

    /// POPU holdover: extends VBlank by one dot past the 153→0 wrap
    /// (modelling the NYPE→POPU DFF propagation delay). Armed only on
    /// the wrap path; cleared on the next XOTA edge by `tick_dot`.
    pub(in crate::ppu) fn update_popu_holdover(&mut self, wrap_occurred: bool) {
        if wrap_occurred {
            self.popu_holdover = true;
        }
    }

    /// Clear the POPU holdover flag. Called on each XOTA edge (tick_dot).
    pub(in crate::ppu) fn clear_popu_holdover(&mut self) {
        self.popu_holdover = false;
    }

    /// Hardware-internal LY (0-153). Use for hardware-level checks.
    pub(in crate::ppu) fn value(&self) -> u8 {
        self.value
    }

    /// CPU-visible LY value for $FF44. On line 153, MYTA's async-reset
    /// path drives LAMA low, making LY read as 0 while the internal
    /// counter is still 153. Two-level vocabulary partition per §2.7.
    pub(in crate::ppu) fn value_register(&self) -> u8 {
        if self.frame_end_reset { 0 } else { self.value }
    }

    pub(in crate::ppu) fn popu_active(&self) -> bool {
        self.vblank || self.popu_holdover
    }

    pub(in crate::ppu) fn write_ly(&mut self, value: u8) {
        self.value = value;
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.value = 0;
        self.vblank = false;
        self.popu_holdover = false;
        self.frame_end_reset = false;
    }
}
