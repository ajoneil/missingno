use crate::{
    Console, Model, ScreenBuffer, clock::Edge, cpu::mcycle::TCycle, interrupts::Interrupt, ppu,
};

impl<M: Model> Console<M> {
    /// PPU rising-edge advance and its interrupt readback: pixel output,
    /// VBlank IF, the STAT edge, and the CPU's interrupt-state refresh.
    pub(super) fn ppu_rise_edge(&mut self) -> (bool, Option<ppu::PixelOutput>) {
        let oam_bus = self.chassis.dma.oam_bus_owner();
        let ppu_result = self
            .chassis
            .ppu
            .on_master_clock_rise(&self.chassis.vram_bus.vram, oam_bus);
        if ppu_result.request_vblank {
            self.chassis
                .interrupts
                .request(Interrupt::VideoBetweenFrames);
        }
        let (new_screen, pixel) = self.apply_ppu_result(&ppu_result);
        if self.chassis.ppu.check_stat_edge() {
            self.chassis.interrupts.request(Interrupt::VideoStatus);
        }
        let triggered = self.chassis.interrupts.triggered();
        self.chassis.cpu.update_interrupt_state(triggered);
        (new_screen, pixel)
    }

    /// PPU falling-edge advance: divider chain, CATU, scanline boundaries,
    /// fetcher, DFF8/DFF9, LCD-off. The caller applies the returned result's
    /// IF requests and pixel output.
    pub(super) fn ppu_fall_edge(
        &mut self,
        is_mcycle_boundary: bool,
        tcycle: TCycle,
    ) -> ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel> {
        let oam_bus = self.chassis.dma.oam_bus_owner();
        // The M-cycle's last PPU fall, where the WY/WX/LCDC.5/LCDC.2 crossing
        // captures — resolved by the divider cell from the ratio.
        let mcycle_last_fall = self
            .chassis
            .clock
            .divider()
            .mcycle_last_fall(is_mcycle_boundary, tcycle.as_u8());
        self.chassis
            .ppu
            .on_master_clock_fall(is_mcycle_boundary, mcycle_last_fall, oam_bus)
    }

    /// Apply a PPU fall's outputs: VBlank/STAT IF requests and the pixel/screen
    /// commit. The `cpu_irq_ack1` re-assert is the caller's (it runs on every
    /// CPU fall, not only the dot's PPU fall).
    pub(super) fn apply_ppu_fall(
        &mut self,
        video_result: &ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        let double_speed = self.double_speed_active();
        // VBlank IF: POPU transitions happen on the fall since the divider
        // chain runs there.
        if video_result.request_vblank {
            self.chassis
                .interrupts
                .request_ppu_fall(Interrupt::VideoBetweenFrames, double_speed);
        }
        // STAT IF: the SUKO check folds into request_stat; cpu_irq_ack1_pulse
        // (LALU.r_n=0) absorbs same-M-cycle SUKO rises.
        if video_result.request_stat && !self.chassis.cpu.irq.cpu_irq_ack1_pulse {
            self.chassis
                .interrupts
                .request_ppu_fall(Interrupt::VideoStatus, double_speed);
        }
        self.apply_ppu_result(video_result)
    }

    /// Run the PPU edge a CPU edge carries (if any). The rise outputs pixel +
    /// VBlank/STAT-edge IF; the fall runs the divider chain and applies its
    /// outputs. Double speed places the master fall on the High arm's rise.
    pub(super) fn fire_dot_ppu(
        &mut self,
        ppu: Edge,
        is_mcycle_boundary: bool,
        tcycle: TCycle,
    ) -> (bool, Option<ppu::PixelOutput>) {
        match ppu {
            Edge::Rise => self.ppu_rise_edge(),
            Edge::Fall => {
                // A dot fall on a CPU rise (double speed only): an LY tick on
                // the read's own T2 rise sits 3 half-edges before the latch,
                // inside the mux ripple — stash LY_old for the latch's AND. A
                // tick earlier in the M (T0) has settled by the latch.
                let ripple_old =
                    if self.chassis.cpu_bus.read_address() == Some(0xFF44) && tcycle.as_u8() == 2 {
                        Some(self.read(0xFF44))
                    } else {
                        None
                    };
                let video_result = self.ppu_fall_edge(is_mcycle_boundary, tcycle);
                if let Some(old) = ripple_old
                    && self.read(0xFF44) != old
                {
                    self.model.note_ff44_ripple_old(Some(old));
                }
                self.apply_ppu_fall(&video_result)
            }
        }
    }

    /// Apply this fall's PPU result — pixel draw and VSYNC/LCD-off present.
    /// `None` on the double-speed CPU T-cycle that carries no PPU fall.
    pub(super) fn apply_fall_ppu_result(
        &mut self,
        video_result: Option<&ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        match video_result {
            Some(video_result) => self.apply_ppu_result(video_result),
            None => (false, None),
        }
    }

    /// Process a PPU tick: draw the pixel, present on VSYNC (only if
    /// MEDA has pulsed since LCD-on), blank on LCD-off. Returns
    /// `(new_screen, pixel)` — `new_screen` fires only on VSYNC, never
    /// on LCD-off blank.
    fn apply_ppu_result(
        &mut self,
        result: &ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        let trace_pixel = result.pixel.map(|pixel| {
            if pixel.x < ppu::screen::PIXELS_PER_LINE && pixel.y < ppu::screen::NUM_SCANLINES {
                self.chassis
                    .screen
                    .draw_pixel(pixel.x, pixel.y, pixel.color);
            }
            ppu::PixelOutput {
                x: pixel.x,
                y: pixel.y,
                shade: <M::Ppu as ppu::PpuModel>::trace_shade(pixel.color),
            }
        });
        if result.new_frame {
            if self.chassis.ppu.control().video_enabled() && self.chassis.ppu.vsync_committed() {
                self.chassis.screen.present();
                self.model.on_present(&self.chassis.screen);
            }
            return (true, trace_pixel);
        }
        if result.lcd_disabled {
            self.chassis.screen.blank();
            self.model.on_present(&self.chassis.screen);
        }
        (false, trace_pixel)
    }
}
