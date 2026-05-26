//! Master-clock rise/fall entry points.

use crate::dma::OamBusOwner;

use super::memory::Vram;
use super::{Ppu, PpuTickResult, screen};

impl Ppu {
    /// Snapshot LCDC.1 (XYLO) before the CPU's staged bus write applies, for the SOBU/CUPA race in mode3_rising.
    pub fn snapshot_pre_cupa_lcdc(&mut self) {
        self.registers.sprites_enabled_pre_cupa = self.registers.control.sprites_enabled();
    }

    /// ALET rises; ALET-clocked DFFs capture (NYKA, LYZU, PYGO, RENE, DOBA, NOPA, VOGA).
    pub fn on_master_clock_rise(&mut self, vram: &Vram, oam_bus: OamBusOwner) -> PpuTickResult {
        let mut result = PpuTickResult::default();

        if !self.control().video_enabled() {
            return result;
        }

        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel = rendering.on_ppu_clock_rise(
                &self.registers,
                &self.video,
                &self.oam,
                oam_bus,
                vram,
            );
        }

        result
    }

    /// ALET falls; XOTA rises, toggling WUVU/VENA/TALU; MYVO-clocked DFFs capture; SACU drives pixel output.
    pub fn on_master_clock_fall(
        &mut self,
        is_mcycle: bool,
        oam_bus: OamBusOwner,
    ) -> PpuTickResult {
        let mut result = PpuTickResult::default();

        // XODO↓ collapses to this fall; subsequent tick_dot is WUVU's first toggle.
        if self.lcd_on_init_pending {
            self.initialize_lcd_on();
            self.lcd_on_init_pending = false;
        }

        if !self.control().video_enabled() {
            return self.handle_lcd_off(is_mcycle, result);
        }
        if self.pixel_pipeline.is_none() {
            return result;
        }

        // XUPY = WUVU.Q; tick_dot returns previous WUVU.Q so scan_clock_rising = !was.
        let scan_clock_rising = !self.video.tick_dot();

        // Capture pre-advance vblank for the TOLU-lagged Mode 0 / Mode 2 leg evaluations.
        // POPU.q → PARU is 2 gate stages (fast); POPU.q → TOLU → TARU / TAPA is 4 stages (slow).
        let pre_advance_vblank = self.video.vblank_or_holdover();

        self.advance_dividers(&mut result);
        // Snapshot with Mode 1 leg already updated (live vblank) but Mode 0 / Mode 2 legs
        // still seeing pre-advance vblank — models the 1-gate TOLU lag.
        let post_fast = self.stat_legs_with_slow_vblank(pre_advance_vblank);

        self.registers.tick_on_master_clock_fall(self.mode2_active());
        self.run_ppu_clock_fall(oam_bus, scan_clock_rising, &mut result);
        // Final snapshot after TOLU has settled and CATU may have driven the slow Mode 0 drop.
        let final_legs = self.stat_legs();
        if self.control().video_enabled()
            && self.video.stat.detect_two_phase_edge(post_fast, final_legs)
        {
            result.request_stat = true;
        }

        result
    }

    /// VID_RST deasserts at XOTA rising (= our fall); dividers reset, WUVU then VENA ramp.
    pub(super) fn initialize_lcd_on(&mut self) {
        self.video.vid_rst();
        // ROPO is not VID_RST-reset; PALY is combinational so recompute now.
        self.video.update_ly_comparison();

        self.pixel_pipeline = Some(super::Rendering::new());
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.start_scanning();
        }

        // Prime LALU per-leg baseline to avoid a spurious first edge on VID_RST deassertion.
        let legs = self.stat_legs();
        self.video.stat.prime_legs(legs);
    }

    fn advance_dividers(&mut self, result: &mut PpuTickResult) {
        if !self.video.dividers.half_mcycle_fell() {
            return;
        }

        let vena_was = self.video.dividers.tick_mcycle();
        let vena_now = self.video.dividers.mcycle();
        let popu_was = self.video.vblank();

        let mut scanline_boundary = false;
        if !vena_was && vena_now {
            // VENA↑ = TALU↑: ROPO captures PALY; NYPE captures POPU/MYTA; LX advances.
            self.video.update_ly_comparison();
            self.video.stat.latch_comparison();
            self.video.on_lx_counter_clock_rise();
            self.video.update_ly_comparison();
        }
        if vena_was && !vena_now {
            // VENA↓ = SONO↑ = TALU↓: RUTU captures SANU; LY advances.
            scanline_boundary = self.video.on_lx_counter_clock_fall();
            self.video.update_ly_comparison();
        }

        if scanline_boundary
            && let Some(rendering) = self.pixel_pipeline.as_mut()
        {
            let ly = self.video.ly();
            if ly == screen::NUM_SCANLINES {
                self.frame_number = self.frame_number.wrapping_add(1);
                result.new_frame = true;
            } else if self.video.ly_hardware() == 0 {
                rendering.reset_frame();
            } else if self.video.ly() < 144 {
                rendering.reset_scanline(ly);
            }
        }

        // POPU↑ → VYPU → LOPE: VBlank IF.
        if self.video.vblank() && !popu_was {
            result.request_vblank = true;
        }
    }

    fn run_ppu_clock_fall(
        &mut self,
        oam_bus: OamBusOwner,
        scan_clock_rising: bool,
        result: &mut PpuTickResult,
    ) {
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel = rendering.on_ppu_clock_fall(
                &self.registers,
                &self.video,
                &self.oam,
                oam_bus,
                scan_clock_rising,
            );
            if result.pixel.is_some() {
                self.registers.palettes.note_bg_pixel_emit();
            }
        }

        // CATU runs after advance_scan so advance_scan reads pre-tick_scan_capture state.
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.tick_scan_capture(&self.video);
        }
    }

    fn handle_lcd_off(&mut self, is_mcycle: bool, mut result: PpuTickResult) -> PpuTickResult {
        if !is_mcycle {
            return result;
        }
        if self.pixel_pipeline.is_some() {
            self.pixel_pipeline = None;
            self.registers.clear_latches();
            result.lcd_disabled = true;
        }
        // Hardware holds counters at 0 while LCD is off; comparison_latched freezes (clock stops).
        self.video.vid_rst();
        result
    }
}
