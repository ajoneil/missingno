//! Master-clock rise/fall entry points and their per-edge work.

use crate::dma::OamBusOwner;

use super::memory::Vram;
use super::{Ppu, PpuTickResult, screen};

impl Ppu {
    /// Snapshot LCDC.1 (XYLO) BEFORE the CPU's staged bus write applies
    /// this rise. The captured value is consumed in mode3_rising to
    /// model the SOBU vs CUPA gate-delay race.
    pub fn snapshot_pre_cupa_lcdc(&mut self) {
        self.registers.sprites_enabled_pre_cupa = self.registers.control.sprites_enabled();
    }

    /// Master clock rise — PPU clock (ALET) rises. ALET-clocked DFFs
    /// capture: NYKA, LYZU, PYGO, RENE, DOBA, NOPA, VOGA.
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

    /// Master clock fall — PPU clock (ALET) falls. XOTA rises here,
    /// toggling the divider chain (WUVU/VENA/TALU); MYVO-clocked DFFs
    /// (PORY) capture; SACU fires and drives pixel output.
    pub fn on_master_clock_fall(
        &mut self,
        is_mcycle: bool,
        oam_bus: OamBusOwner,
    ) -> PpuTickResult {
        let mut result = PpuTickResult::default();

        // XODO↓ collapses to this fall; subsequent tick_dot is WUVU's
        // first toggle.
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

        // tick_dot toggles WUVU; the returned previous WUVU.Q value
        // determines this fall's XUPY edge (XUPY = WUVU.Q).
        let xupy_rising = !self.video.tick_dot();

        self.advance_dividers(&mut result);
        self.registers.tick_on_master_clock_fall(self.besu());
        self.run_ppu_clock_fall(oam_bus, xupy_rising, &mut result);

        result
    }

    /// VID_RST deasserts at XOTA rising (= our fall). Toggle DFFs
    /// async-reset to q=0; the divider cascade then ramps WUVU then
    /// VENA. The first RUTU-capturing edge is VENA's first rise.
    pub(super) fn initialize_lcd_on(&mut self) {
        self.video.vid_rst();
        // ROPO is not VID_RST-reset; PALY is combinational so recompute
        // here. ROPO latches this value at the first TALU rising edge.
        self.video.update_ly_comparison();

        self.pixel_pipeline = Some(super::Rendering::new());
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.start_scanning();
        }

        // STAT line and its edge detector reach steady state together
        // when VID_RST deasserts — sync to avoid a spurious first edge.
        let stat_line = self.stat_line();
        self.video.stat.set_line_was_high(stat_line);
    }

    /// VENA rising/falling drives scanline-boundary handling and frame
    /// completion (new_frame / request_vblank / reset_frame).
    fn advance_dividers(&mut self, result: &mut PpuTickResult) {
        if !self.video.dividers.half_mcycle_fell() {
            return;
        }

        let vena_was = self.video.dividers.tick_mcycle();
        let vena_now = self.video.dividers.mcycle();
        let popu_was = self.video.vblank();

        let mut scanline_boundary = false;
        if !vena_was && vena_now {
            // VENA↑ = TALU↑. ROPO captures pre-reset PALY (4-stage
            // capture beats 6-stage MYTA→LY-reset). NYPE captures
            // POPU/MYTA and LX advances.
            self.video.update_ly_comparison();
            self.video.stat.latch_comparison();
            self.video.on_lx_counter_clock_rise();
            self.video.update_ly_comparison();
        }
        if vena_was && !vena_now {
            // VENA↓ = SONO↑ = TALU↓. RUTU captures SANU; LY advances.
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

    /// PPU clock falling work: pixel emit + CATU pipeline.
    fn run_ppu_clock_fall(
        &mut self,
        oam_bus: OamBusOwner,
        xupy_rising: bool,
        result: &mut PpuTickResult,
    ) {
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel = rendering.on_ppu_clock_fall(
                &self.registers,
                &self.video,
                &self.oam,
                oam_bus,
                xupy_rising,
            );
            if result.pixel.is_some() {
                self.registers.palettes.note_bg_pixel_emit();
            }
        }

        // CATU runs AFTER on_ppu_clock_fall so advance_scan reads
        // pre-tick_catu state. On a scanline-boundary +1 fall,
        // advance_scan sees scanning=false; tick_catu then captures
        // CATU, sets scanning=true and counter=0.
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.tick_catu(&self.video);
        }
    }

    /// LCD off (or just disabled): tear down the pipeline on the next
    /// M-cycle, hold counters in VID_RST.
    fn handle_lcd_off(&mut self, is_mcycle: bool, mut result: PpuTickResult) -> PpuTickResult {
        if !is_mcycle {
            return result;
        }
        if self.pixel_pipeline.is_some() {
            self.pixel_pipeline = None;
            self.registers.clear_latches();
            result.lcd_disabled = true;
        }
        // Hardware holds counters at 0 continuously while LCD is off;
        // we reset each M-cycle to match. comparison_latched is not
        // updated — the comparison clock stops with the PPU.
        self.video.vid_rst();
        result
    }
}
