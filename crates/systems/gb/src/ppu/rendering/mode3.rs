//! The mode-3 datapath: the ALET-rising fetcher and trigger evaluation, the
//! MYVO-clocked fetcher advance, and the SACU-clocked pixel pipeline that
//! shifts, drains and pushes.

use super::Rendering;
use crate::dma::OamBusOwner;
use crate::ppu::{
    DrawnPixel, PipelineRegisters, PpuModel, TileSelGlitch, VideoControl, memory::Oam,
};

use crate::ppu::draw::pixel_output;
use crate::ppu::draw::sprite_fetch::{SpriteFetch, SpriteState};
use crate::ppu::scan::oam_scan::OFF_SCREEN_SPRITE_X;

impl<P: PpuModel> Rendering<P> {
    /// ALET rising: fetcher VRAM reads, cascade DFFs (NYKA, PYGO), POKY, TYFA, SABE, PUXA.
    pub(super) fn mode3_rising(
        &mut self,
        model: &P,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &P::Vram,
    ) {
        // SOBU's ALET-rising DFF capture wins the TEKY→SOBU race vs CUPA's transparent-latch path —
        // SOBU sees the pre-write LCDC.1 value, so FEPO here uses pre-CUPA sprites_enabled.
        let mut sprite_x_match_pre_write =
            self.sprite_x_match(regs.sprites_enabled_pre_write_strobe);

        // LYRY = fetch_counter >= 5 (combinational). Counter only increments on rising.
        let bg_fetch_done = self.fetcher.bg_fetch_done();

        // BG fetcher counter=0/2/4 VRAM reads. Counter saturates at 5 during sprite fetch (MOCE=0
        // freezes LEBO) — no explicit !taka() gate needed here.
        self.fetcher.advance_falling(
            self.pixel_counter.value(),
            self.fine_scroll.pixel_clock_active(),
            self.synced_scx,
            self.window.window_line_counter(),
            self.window.wx_triggered(regs, Self::window_synced()),
            regs,
            video,
            vram,
            model.tile_sel_glitch().active(),
        );

        // Cascade advance runs before tick_rising so POKY's just-set value reaches the window's PYCO gate.
        self.cascade.advance_cascade(bg_fetch_done);

        // SOCY's gate chain is too slow to suppress the same-dot in-flight SACU↑;
        // sample RYDY before tick_rising so the TYFA snapshot sees the pre-MOSU value.
        let window_hit_pre_trigger = self.window.window_hit();

        // Window rise tick: NOPA captures prior-fall PYNU, then PYNU re-evaluates.
        // Deferred-completion path can fire MOSU↑ here when LCDC.5 restore drops XOFO while NUNU=1.
        self.pending_window_trigger = self.window.tick_rising(
            &mut self.fetcher,
            &mut self.cascade,
            &mut self.fine_scroll,
            regs,
        );

        // SABE clock fires on ALET rising. Placed before the TEKY/RYCE block so a newly
        // initiated sprite fetch doesn't advance on its first dot.
        if self.sprite_trigger.fetch_running() {
            // The fetch samples obj-size live on DMG; on the CGB it reads the
            // crossing-lagged size, so a mid-fetch 8x8↔8x16 change splits the two
            // tile-data reads (counter-2 low / counter-4 high) across tile rows.
            let effective_sprite_size = regs.obj_size_for_fetch();
            match self.sprite_state {
                SpriteState::Fetching(ref mut sf) => {
                    let slot_index = sf.slot_index;
                    let sprite_fetch_counter = sf.fetch_counter();
                    let done = sf.advance(model, effective_sprite_size, oam, oam_bus, vram);
                    // The OBJ high fetch (counter 4) drives the sprite onto the
                    // tile-data bus and becomes the glitch source for both planes.
                    if sprite_fetch_counter == 4 {
                        let (low, high) = sf.tile_data();
                        self.fetcher.drive_bus_from_sprite(low, high);
                    }
                    if done {
                        let (s1y, s1x) = sf.stage1_capture();
                        sf.merge_into(model, &mut self.obj_fifo);
                        self.sprite_state = SpriteState::Idle;
                        self.sprite_trigger.clear_fetch_running();
                        // Per-slot fetched-flag captures at WUTY↑ (fetch completion); FEPO drops for this slot.
                        self.scan.sprites_mut().mark_fetched(slot_index);
                        // The fetch latched (tile-index, attribute) into the shared Stage-1 dlatches.
                        self.scan.set_stage1_held(s1y, s1x);
                        sprite_x_match_pre_write =
                            self.sprite_x_match(regs.control.sprites_enabled());
                    }
                }
                SpriteState::Idle => {}
            }
        }

        // PYCO captures NUKO on ROCO↑ (ALET-phase); NUNU's MEHE capture follows on the fall.
        // ROCO halts with the rest of the VYBO chain when WODU is high (PX terminal).
        let pixel_clock_running = self.cascade.pixel_data_ready() && !self.pixel_counter.terminal();
        self.window.capture_wx_match_on_pixel_clock::<P>(
            self.pixel_counter.value(),
            pixel_clock_running,
            self.sprite_x_match(regs.sprites_enabled_pre_write_strobe),
            regs,
        );

        // TEKY = AND4(FEPO, !RYDY, LYRY, !TAKA); RYCE = AND2(!SUDA, SOBU) one-shots it.
        let x_match_trigger = sprite_x_match_pre_write
            && !self.window.window_hit()
            && bg_fetch_done
            && !self.sprite_trigger.fetch_running();
        let fetch_request_fired = self.sprite_trigger.tick_trigger_on_rise(x_match_trigger);

        if fetch_request_fired {
            self.start_sprite_fetch();
        }

        // Post-CUPA FEPO drives TYFA's combinational AND (CUPA→AROR→FEPO settles well before SACU).
        // On the CGB an OBJ-disable lands on AROR/FEPO a crossing late, so it can't un-commit an
        // in-flight fetch: the SACU freeze holds the fetch's FEPO through to WUTY and the penalty
        // isn't cut short. DMG's combinational AROR releases the freeze at once.
        let obj_enable_for_freeze = regs.control.sprites_enabled()
            || (P::OBJ_ENABLE_CROSSING.write_delayed_falls() > 0
                && self.sprite_trigger.fetch_running());
        let sprite_x_match_post_write = self.sprite_x_match(obj_enable_for_freeze);

        // TYFA = AND3(SOCY, POKY, VYBO). VYBO = NOR3(FEPO_old, WODU_old, MYVO).
        // window_hit_pre_trigger is the pre-MOSU value so in-flight pre-window SACU fires on MOSU↑.
        self.pipe_clock_enable = !sprite_x_match_post_write
            && !self.pixel_counter.terminal()
            && !window_hit_pre_trigger
            && self.cascade.pixel_data_ready();

        // POHU = (count == SCX & 7); ROXO captures POHU into PUXA on the falling edge.
        // CGB reads FF43 through the register-file crossing.
        let scx = if P::SCX_CROSSING.is_synced() {
            self.synced_scx
        } else {
            regs.background_viewport.x.output()
        };
        self.fine_scroll.compare_falling(scx);
    }
    /// MYVO-clocked DFFs: SUDA, PORY, BG fetch counter (LEBO). Runs before the pixel pipeline
    /// (depth ~16-22 ge vs SACU at ~63.8 ge).
    pub(super) fn mode3_advance_fetcher(&mut self) -> bool {
        self.sprite_trigger.tick_trigger_on_fall();

        // Counter saturates at 5 so it stays at 5 during sprite fetch without a !taka gate.
        self.fetcher.advance_rising();
        self.cascade.capture_fetch_complete_stage_2();

        // PORY clears RYDY via the NOR3(PUKU, PORY, VID_RST) reset arm.
        // SUZU = AND2(!RYDY_new, SOVY): one-half-cycle pulse on RYDY 1→0; triggers TEVO.
        let load_window_pulse = self
            .window
            .release_window_hit_on_fetcher_reset(self.cascade.fetch_complete_stage_2());
        if load_window_pulse {
            // SUZU → TEVO → NYXU: load window tile, reset fine counter.
            self.fetcher.load_into(&mut self.bg_shifter);
            self.fine_scroll.reset_counter();
        }

        // TAVE one-shot preload: fires when NYKA+PORY have risen but POKY hasn't latched PYGO yet.
        if self.cascade.fetch_complete()
            && self.cascade.fetch_complete_stage_2()
            && !self.cascade.pixel_data_ready()
        {
            self.fetcher.load_into(&mut self.bg_shifter);
            self.fine_scroll.reset_counter();
            // VEKU's TAVE arm clears TAKA carry-over from the prior scanline.
            self.sprite_trigger.clear_fetch_running();
        }

        load_window_pulse
    }
    /// SACU/CLKPIPE domain (depth ~63.8 ge); runs against settled fetcher state.
    /// Handles TYFA consumption, PUXA/POVA, pixel shifts, SEKO tile reload, LCD output, NUKO window trigger.
    pub(super) fn mode3_pixel_pipeline(
        &mut self,
        model: &P,
        regs: &PipelineRegisters,
        window_hit_before_fetcher_advance: bool,
        window_restart_reset_pulse: bool,
        pixel_counter_before_shift: u8,
    ) -> Option<DrawnPixel<P::Pixel>> {
        // FEPO before the pixel advance, for the terminal WODU pulse (FEPO settles after XANO).
        let pre_advance_sprite_x_match = self.sprite_x_match(regs.control.sprites_enabled());

        // TYFA snapshot from the prior rise; captures pre-MOSU RYDY so in-flight pre-window SACU fires on MOSU↑.
        let pipe_clock_enable = self.pipe_clock_enable;
        self.pipe_clock_enable = false;

        // PUXA via ROXO. Using prior-rise TYFA carries the correct cascade-propagated POKY value.
        let fine_scroll_match = if pipe_clock_enable {
            self.fine_scroll.capture_rising()
        } else {
            false
        };

        // SACU = TYFA && ROXY-released. VYBO = NOR3(MYVO, FEPO, WODU); TAKA freezes SACU only
        // indirectly via FEPO=1 on the unfetched per-slot flag.
        let pipe_shift_clock = pipe_clock_enable && self.fine_scroll.pixel_clock_active();

        // PANY drain-detector slip: NUKO=1 lands when SEKO would fire (count==7), truncating
        // PANY's high pulse — RYFA captures the second half, slipping SEKO→TEVO→NYXU by 1 dot.
        // The CGB's NUKO→PANY coupling needs WIN_EN; the DMG's fires while armed-but-disabled.
        let proposed_tile_boundary =
            self.fine_scroll.count == 7 && !window_hit_before_fetcher_advance;
        let window_x_hit = self.window.window_x_reached(pixel_counter_before_shift)
            && (P::WINDOW_DRAIN_SLIP_WHILE_DISABLED || regs.control.window_enabled());
        let drain_slip_now = proposed_tile_boundary && window_x_hit;
        let raw_tile_boundary_fire =
            (proposed_tile_boundary && !drain_slip_now) || self.drain_slip_pending;
        self.drain_slip_pending = drain_slip_now;

        // SEKO drain-detector freeze during sprite-fetch FEPO-held window: FEPO=1 → VYBO=0 →
        // SACU=0 → SEGU stuck at 1 → RYFA frozen → RENE.D = RYFA holds → SEKO = NOR2(RENE, RYFA)
        // holds at its pre-freeze value (0 in normal BG cadence). The collapsed `raw_tile_boundary_fire`
        // formula doesn't model the cascade DFFs explicitly, so we override to 0 during the
        // freeze. Zero NYXU pulses across 30 TAKA-high windows confirmed by gate-level FST.
        let sprite_x_match_held = self.sprite_trigger.fetch_running()
            && self.sprite_x_match(regs.control.sprites_enabled());
        let tile_boundary_fire = if sprite_x_match_held {
            false
        } else {
            raw_tile_boundary_fire
        };

        let bg_counter_reset_pulse = tile_boundary_fire || window_restart_reset_pulse;

        let mux = pixel_output::current_mux::<P>(&self.bg_shifter, &self.obj_fifo);
        let pixel = model.resolve(&mux, regs);

        if tile_boundary_fire {
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        if pipe_shift_clock {
            // NYXU pulse holds the BG shifter via LOZE; OBJ shifter is not LOZE-gated.
            if !bg_counter_reset_pulse {
                self.bg_shifter.shift();
            }
            P::obj_shift(&mut self.obj_fifo);
            self.pixel_counter.advance();
        }

        // WODU sampled on the post-advance XANO/FEPO so OAM-X=167 sprites are visible on the same edge.
        let post_advance_sprite_x_match = self.sprite_x_match(regs.control.sprites_enabled());
        self.hblank
            .latch_end_of_line(self.pixel_counter.terminal(), post_advance_sprite_x_match);

        // Early WODU pulse: the post-advance XANO terminal decode settles before the FEPO
        // comparator, so an advance onto terminal PX pulses WODU before a terminal sprite's FEPO.
        self.terminal_end_of_line_pulse =
            self.pixel_counter.terminal() && !pre_advance_sprite_x_match;

        let (_pixel_emit, pixel_out) = self.lcd.on_ppu_clock_fall(
            pipe_shift_clock,
            pixel,
            fine_scroll_match,
            self.pixel_counter.value(),
        );

        if pipe_clock_enable {
            self.fine_scroll.tick();
        }

        if tile_boundary_fire {
            self.fine_scroll.reset_counter();
        }

        self.window
            .update_match_wx(regs.window.x.output(), Self::window_synced());

        pixel_out
    }
    /// FEPO: any unfetched sprite's X matches the pixel counter. Feeds VYBO/XENA/TEKY.
    /// Collapses XYLO/AROR/per-sprite-decoders/FOVE/FEFY into the store's
    /// precomputed comparator bank; off-screen X excluded.
    pub(super) fn sprite_x_match(&self, sprites_enabled: bool) -> bool {
        if !sprites_enabled && P::FETCH_TRIGGER_GATED_BY_OBJ_ENABLE {
            return false;
        }

        self.scan
            .sprites_ref()
            .matches_at(self.pixel_counter.value())
    }
    /// The per-slot fetched-flag is set at fetch completion (WUTY↑), not here, so FEPO stays
    /// high through the 6-dot fetch window (freezing SACU via VYBO).
    pub(super) fn start_sprite_fetch(&mut self) {
        let match_x = self.pixel_counter.value();
        let sprites = self.scan.sprites_mut();

        for i in 0..sprites.count as usize {
            if sprites.fetched & (1 << i) != 0 {
                continue;
            }
            let entry = &sprites.entries[i];
            if entry.x == match_x && entry.x < OFF_SCREEN_SPRITE_X {
                self.sprite_state =
                    SpriteState::Fetching(SpriteFetch::new_fetching(*entry, i as u8));
                break;
            }
        }
    }
}
