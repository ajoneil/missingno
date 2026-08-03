//! Master-clock rise/fall entry points.

use crate::dma::OamBusOwner;

use super::{Ppu, PpuModel, PpuTickResult, TileSelGlitch, screen};

impl<P: PpuModel> Ppu<P> {
    /// Snapshot LCDC.1 (XYLO) before the CPU's staged bus write applies, for the SOBU/CUPA race in mode3_rising.
    pub fn snapshot_pre_cupa_lcdc(&mut self) {
        self.registers.sprites_enabled_pre_cupa = self.registers.control.sprites_enabled();
    }

    /// ALET rises; ALET-clocked DFFs capture (NYKA, LYZU, PYGO, RENE, DOBA, NOPA, VOGA).
    pub fn on_master_clock_rise(
        &mut self,
        vram: &P::Vram,
        oam_bus: OamBusOwner,
    ) -> PpuTickResult<P::Pixel> {
        // A caller reaches this only on a dot the span did not defer, so what
        // earlier deferrals owe is settled here, before anything reads the
        // counters.
        self.sync_span();

        if !self.control().video_enabled() {
            return PpuTickResult::default();
        }

        if self.span.asleep() {
            return PpuTickResult::default();
        }

        let mut result = PpuTickResult::default();

        self.registers.palettes.clear_capture_coincident_old();

        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel = rendering.on_ppu_clock_rise(
                &self.model,
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
        mcycle_last_fall: bool,
        oam_bus: OamBusOwner,
    ) -> PpuTickResult<P::Pixel> {
        let mut result = PpuTickResult::default();

        // A caller reaches this only on a dot the span did not defer, so what
        // earlier deferrals owe is settled here, before anything reads the
        // counters.
        self.sync_span();

        // XYMU's dot-fall crossing stage captures the pre-edge value: the
        // AVAP-fall set races it and reaches the CRAM lock a dot late. A dot the
        // span slept left XYMU low, so only a live dot has to resolve it.
        self.drawing_fall_stage =
            !self.span.asleep() && self.mode() == super::rendering::Mode::Drawing;

        // XODO↓ collapses to this fall; subsequent tick_dot is WUVU's first toggle.
        if self.lcd_on_init_pending {
            self.initialize_lcd_on();
            self.lcd_on_init_pending = false;
        }

        if !self.control().video_enabled() {
            self.span.invalidate();
            return self.handle_lcd_off(is_mcycle, result);
        }
        if self.pixel_pipeline.is_none() {
            self.span.invalidate();
            return result;
        }

        // XUPY = WUVU.Q; tick_dot returns previous WUVU.Q so scan_clock_rising = !was.
        let scan_clock_rising = !self.video.tick_dot();

        let talu_rising = self.advance_dividers(&mut result);

        if self.span_sleepable() {
            self.span.arm(self.video.dots_to_line_end());
            self.sleep_dot();
            return result;
        }
        self.span.wake();

        self.run_fall(
            is_mcycle,
            mcycle_last_fall,
            oam_bus,
            scan_clock_rising,
            talu_rising,
            &mut result,
        );

        // The register crossings have captured on this M-boundary fall, so
        // every write since the last one has reached the IRQ block.
        if is_mcycle && mcycle_last_fall {
            self.span.note_crossings_captured();
        }
        self.span
            .settle(self.video.stat.ly_eq_lyc(), self.video.vblank());

        result
    }

    /// Rendering and the scan chain are both parked on a sleeping dot, so the
    /// STAT mode bits are POPU alone.
    fn sleep_dot(&mut self) {
        self.span.sleep(if self.video.vblank() {
            super::rendering::Mode::VerticalBlank
        } else {
            super::rendering::Mode::HorizontalBlank
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn run_fall(
        &mut self,
        is_mcycle: bool,
        mcycle_last_fall: bool,
        oam_bus: OamBusOwner,
        scan_clock_rising: bool,
        talu_rising: bool,
        result: &mut PpuTickResult<P::Pixel>,
    ) {
        self.registers.tick_on_master_clock_fall(
            self.mode2_active(),
            P::BGP_WRITE_RACE,
            P::OBP_WRITE_RACE,
        );
        self.model.tile_sel_glitch_mut().tick();
        self.run_ppu_clock_fall(
            oam_bus,
            scan_clock_rising,
            talu_rising,
            mcycle_last_fall,
            result,
        );
        // The FF45→IRQ-block crossing captures on its resolved edge; the synced
        // LYC lands in the next TALU↑. The DS double-capture (this last-fall and
        // the standalone M-boundary fall) is safe: FF45 is write-stable across
        // the intra-M-cycle falls, so both reads see the same cell value.
        if P::LYC_CROSSING.is_synced() && mcycle_last_fall {
            let ly = self.video.ly();
            self.video
                .stat
                .capture_synced_lyc(ly, self.model.stat_shadow_mut());
        }
        let conditions = self.stat_conditions();
        let edge = if P::STAT_ENABLES_CROSSING.is_synced() {
            // M-boundary fall: the FF41 synchroniser captures here, racing this
            // fall's condition edges (ROPO captured pre-edge PALY above). The
            // WY/WX/LCDC.5/LCDC.2 crossing ticks inside `on_ppu_clock_fall` at
            // the M-cycle's last PPU fall instead.
            self.video.stat.eval_synced(
                conditions,
                talu_rising,
                is_mcycle,
                self.model.stat_shadow_mut(),
            )
        } else {
            self.video.stat.eval_conditions(conditions, talu_rising)
        };
        if edge {
            result.request_stat = true;
        }
    }

    /// VID_RST deasserts at XOTA rising (= our fall); dividers reset, WUVU then VENA ramp.
    pub(super) fn initialize_lcd_on(&mut self) {
        self.span.invalidate();
        self.video.vid_rst();
        // ROPO is not VID_RST-reset; PALY is combinational so recompute now.
        self.video.update_ly_comparison(self.model.stat_shadow());

        self.pixel_pipeline = Some(super::Rendering::new());
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            rendering.start_scanning();
            rendering.capture_register_sync(&self.registers);
        }

        // Prime the LALU baselines to avoid a spurious first edge on VID_RST deassertion.
        let legs = self.stat_legs();
        let conditions = self.stat_conditions();
        self.video.stat.prime_baselines(legs, conditions);
    }

    /// Returns `true` if a TALU↑ DFF capture happened in this fall (drives the SUKO
    /// pulse-width filter regime in the caller).
    fn advance_dividers(&mut self, result: &mut PpuTickResult<P::Pixel>) -> bool {
        if !self.video.dividers.half_mcycle_fell() {
            return false;
        }

        let vena_was = self.video.dividers.tick_mcycle();
        let vena_now = self.video.dividers.mcycle();
        let popu_was = self.video.vblank();

        let mut scanline_boundary = false;
        let talu_rising = !vena_was && vena_now;
        if talu_rising {
            // VENA↑ = TALU↑: ROPO captures PALY; NYPE captures POPU/MYTA; LX advances.
            self.video.update_ly_comparison(self.model.stat_shadow());
            self.video.stat.latch_comparison();
            self.video.on_lx_counter_clock_rise();
            self.video.update_ly_comparison(self.model.stat_shadow());
        }
        if vena_was && !vena_now {
            // VENA↓ = SONO↑ = TALU↓: RUTU captures SANU; LY advances.
            scanline_boundary = self.video.on_lx_counter_clock_fall();
            self.video.update_ly_comparison(self.model.stat_shadow());
        }

        if scanline_boundary && let Some(rendering) = self.pixel_pipeline.as_mut() {
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

        talu_rising
    }

    fn run_ppu_clock_fall(
        &mut self,
        oam_bus: OamBusOwner,
        scan_clock_rising: bool,
        talu_rising: bool,
        mcycle_last_fall: bool,
        result: &mut PpuTickResult<P::Pixel>,
    ) {
        if let Some(rendering) = self.pixel_pipeline.as_mut() {
            result.pixel = rendering.on_ppu_clock_fall(
                &self.model,
                &self.registers,
                &self.video,
                &self.oam,
                oam_bus,
                scan_clock_rising,
                talu_rising,
                mcycle_last_fall,
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

    fn handle_lcd_off(
        &mut self,
        is_mcycle: bool,
        mut result: PpuTickResult<P::Pixel>,
    ) -> PpuTickResult<P::Pixel> {
        if !is_mcycle {
            return result;
        }
        if self.pixel_pipeline.is_some() {
            self.pixel_pipeline = None;
            self.registers.clear_latches();
            self.model.tile_sel_glitch_mut().clear();
            result.lcd_disabled = true;
        }
        // Hardware holds counters at 0 while LCD is off; comparison_latched freezes (clock stops).
        self.video.vid_rst();
        // The CPU-clocked register synchroniser keeps capturing with the
        // LCD off; the LYC leg stays live on frozen ROPO.
        if self.capture_register_sync_standalone() {
            result.request_stat = true;
        }
        result
    }
}
