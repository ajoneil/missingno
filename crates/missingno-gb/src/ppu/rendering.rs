pub use super::draw::sprite_fetch::SpriteFetchPhase;

use core::fmt;

use crate::dma::OamBusOwner;
use crate::ppu::{
    PipelineRegisters, PixelOutput, VideoControl,
    memory::{Oam, Vram},
    types::sprites::SpriteId,
};

use super::draw::fetch_cascade::FetchCascade;
use super::draw::fetcher::TileFetcher;
use super::draw::fine_scroll::FineScroll;
use super::draw::hblank_pipeline::HblankPipeline;
use super::draw::lcd_control::LcdControl;
use super::draw::pixel_counter::PixelCounter;
use super::draw::pixel_output;
use super::draw::shifters::{BgShifter, ObjShifter};
use super::draw::sprite_fetch::{SpriteFetch, SpriteState};
use super::draw::sprite_trigger::SpriteTrigger;
use super::draw::window_control::WindowControl;
use super::scan::scanner::SpriteScanner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    HorizontalBlank = 0,
    VerticalBlank = 1,
    OamScan = 2,
    Drawing = 3,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::HorizontalBlank => write!(f, "HBlank"),
            Mode::VerticalBlank => write!(f, "VBlank"),
            Mode::OamScan => write!(f, "OAM Scan"),
            Mode::Drawing => write!(f, "Drawing"),
        }
    }
}

pub struct SpriteStoreSnapshot {
    pub count: u8,
    pub fetched: u16,
    pub entries: Vec<SpriteStoreEntrySnapshot>,
}

pub struct SpriteStoreEntrySnapshot {
    pub oam_index: u8,
    pub line_offset: u8,
    pub x: u8,
    pub fetched: bool,
}

/// gbtrace `ppu_internal` snapshot. Field names match the gbtrace spec.
pub struct PpuTraceSnapshot {
    pub sprite_x: [u8; 10],
    pub sprite_id: [u8; 10],
    pub sprite_attr: [u8; 10],
    pub bgw_fifo_a: u8,
    pub bgw_fifo_b: u8,
    pub spr_fifo_a: u8,
    pub spr_fifo_b: u8,
    pub pal_pipe: u8,
    pub tfetch_state: u8,
    /// 0 when no sprite fetch active.
    pub sfetch_state: u8,
    pub tile_temp_a: u8,
    pub tile_temp_b: u8,
    pub pix_count: u8,
    pub sprite_count: u8,
    pub scan_count: u8,
    pub rendering: bool,
    pub win_mode: bool,
    pub frame_num: u16,
}

/// Debugger pipeline snapshot, serialised by the headless debugger JSON API.
pub struct PipelineSnapshot {
    pub pixel_counter: u8,
    /// Mode 3 active (XYMU; inverted polarity).
    pub rendering_active: bool,
    pub bg_low: u8,
    pub bg_high: u8,
    pub obj_low: u8,
    pub obj_high: u8,
    pub obj_palette: u8,
    pub obj_priority: u8,
    pub sprite_fetch_phase: Option<SpriteFetchPhase>,
    pub sprite_tile_data: Option<(u8, u8)>,
    pub lcd_x: u8,
    pub fetch_counter: u8,
    /// Window-hit latch (RYDY).
    pub window_hit: bool,
    /// LCD pixel-emit gate (WUSA).
    pub pixel_gate: bool,
    /// Fine-scroll match for the cp_pad strobe (POVA).
    pub fine_scroll_match: bool,
    /// Fetcher-idle stage 3 (PYGO).
    pub fetcher_idle_stage_3: bool,
    /// Fetcher-ready output (POKY).
    pub fetcher_ready: bool,
    pub wx_triggered: bool,
    /// Video clock divider (WUVU).
    pub video_clock: bool,
    /// Scan-done flag (BYBA, dffr, XUPY-clocked).
    pub scan_done: bool,
    /// Prior-scan-done (DOBA, dffr, ALET-clocked).
    pub scan_done_prev: bool,
}

pub struct Rendering {
    /// FEPO → WODU → VOGA → WEGO → clears XYMU.
    hblank: HblankPipeline,
    /// Scan counter, BESU latch, BYBA/DOBA pipeline, sprite store.
    scan: SpriteScanner,
    bg_shifter: BgShifter,
    obj_shifter: ObjShifter,
    fetcher: TileFetcher,
    /// LYRY → NYKA → PORY → PYGO → POKY.
    cascade: FetchCascade,
    /// Fine-scroll counter + ROXY pixel-clock gate.
    fine_scroll: FineScroll,
    /// RYDY latch, WX comparator, window line counter.
    window: WindowControl,
    /// `!FEPO && !WODU && !RYDY && POKY`; snapshotted at end of `mode3_rising`,
    /// consumed in `mode3_pixel_pipeline`. RYDY is sampled before `tick_rising`
    /// so same-dot RYDY↑ doesn't reach the snapshot (models SYLO/TOMU/SOCY → SACU delay).
    tyfa: bool,
    pixel_counter: PixelCounter,
    /// WUSA gating, POVA trigger, LCD pixel push.
    lcd: LcdControl,
    sprite_state: SpriteState,
    /// TEKY → SOBU → SUDA → RYCE → TAKA.
    sprite_trigger: SpriteTrigger,
    /// PANY drain-detector slip carry-over: NUKO=1 lands while SEKO would fire (count==7),
    /// splitting PANY's high pulse — RYFA captures the second half, slipping SEKO→TEVO→NYXU by 1 dot.
    pany_slip_pending: bool,
    /// Window trigger (MOSU) fired on the prior rise via the deferred-completion path
    /// (LCDC.5 restore drops XOFO while NUNU=1); consumed on the following fall to hold
    /// the BG fetch counter at 0 via NYXU's reset pulse.
    pending_window_trigger: bool,
    /// WODU early pulse: at the advance onto terminal PX the shallow XANO decode settles
    /// before the deeper FEPO comparator, so WODU glitches high for the settling window.
    /// One-shot — set at the advance fall, cleared on the next rise — so the fall SUKO eval
    /// catches it but off-edge reads (the STAT-write glitch) see settled WODU.
    terminal_wodu_pulse: bool,
}

impl Rendering {
    pub(super) fn new() -> Self {
        Rendering {
            hblank: HblankPipeline::new(),
            scan: SpriteScanner::new(),
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            cascade: FetchCascade::new(),
            fine_scroll: FineScroll::new(),
            window: WindowControl::new(),
            tyfa: false,
            pixel_counter: PixelCounter::new(),
            lcd: LcdControl::new(),
            sprite_state: SpriteState::Idle,
            sprite_trigger: SpriteTrigger::new(),
            pany_slip_pending: false,
            pending_window_trigger: false,
            terminal_wodu_pulse: false,
        }
    }

    pub(super) fn post_boot() -> Self {
        Rendering {
            hblank: HblankPipeline::post_boot(),
            scan: SpriteScanner::post_boot(),
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::post_boot(),
            cascade: FetchCascade::new(),
            fine_scroll: FineScroll::new(),
            window: WindowControl::new(),
            tyfa: false,
            pixel_counter: PixelCounter::post_boot(),
            lcd: LcdControl::post_boot(),
            sprite_state: SpriteState::Idle,
            sprite_trigger: SpriteTrigger::new(),
            pany_slip_pending: false,
            pending_window_trigger: false,
            terminal_wodu_pulse: false,
        }
    }

    /// VID_RST deassertion releases the scan counter alongside the rest of the pipeline.
    pub(super) fn start_scanning(&mut self) {
        self.scan.start_scanning();
    }

    /// XYMU rendering latch; `true` during Mode 3 (opposite polarity to spec's active-low XYMU).
    pub(super) fn rendering_active(&self) -> bool {
        self.hblank.rendering_active()
    }

    /// WODU = AND2(XUGU, !FEPO); combinational, doesn't depend on XYMU.
    pub(super) fn end_of_line_signal(&self, sprites_enabled: bool) -> bool {
        HblankPipeline::compute_end_of_line(
            self.pixel_counter.terminal(),
            self.fepo(sprites_enabled),
        )
    }

    /// Early WODU↑ pulse at the advance onto terminal PX (XANO settles before FEPO); ORed
    /// into the Mode-0 STAT leg. One-shot, cleared on the next rise.
    pub(super) fn terminal_wodu_pulse(&self) -> bool {
        self.terminal_wodu_pulse
    }

    /// LCD-enable first line — no prior scanline boundary, so RUTU is suppressed.
    fn is_first_line(&self) -> bool {
        !self.scan.scan_capture_armed()
    }

    /// TAPA_INT_OAM active. RUTU is suppressed on the LCD-enable first line.
    pub(super) fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        if self.is_first_line() {
            return false;
        }
        video.line_end_active()
    }

    pub(super) fn scanner_oam_address(&self) -> Option<u8> {
        self.scan.oam_address()
    }

    pub(super) fn scan_counter_entry(&self) -> u8 {
        self.scan.scan_counter_entry()
    }

    pub(super) fn scan_mode2_active(&self) -> bool {
        self.scan.mode2_active()
    }

    pub(super) fn lcd_pushing_active(&self) -> bool {
        self.lcd.pixel_gate()
    }

    pub(super) fn sprite_store_snapshot(&self) -> SpriteStoreSnapshot {
        let sprites = &self.scan.sprites_ref();
        SpriteStoreSnapshot {
            count: sprites.count,
            fetched: sprites.fetched,
            entries: (0..sprites.count as usize)
                .map(|i| {
                    let e = &sprites.entries[i];
                    SpriteStoreEntrySnapshot {
                        oam_index: e.oam_index,
                        line_offset: e.line_offset,
                        x: e.x,
                        fetched: sprites.fetched & (1 << i) != 0,
                    }
                })
                .collect(),
        }
    }

    pub(super) fn trace_snapshot(&self, oam: &Oam) -> PpuTraceSnapshot {
        let sprites = self.scan.sprites_ref();
        let mut sprite_x = [0u8; 10];
        let mut sprite_id = [0u8; 10];
        let mut sprite_attr = [0u8; 10];
        for i in 0..sprites.count as usize {
            let entry = &sprites.entries[i];
            sprite_x[i] = entry.x;
            let oam_sprite = oam.sprite(SpriteId(entry.oam_index));
            sprite_id[i] = oam_sprite.tile.0;
            sprite_attr[i] = oam_sprite.attributes.0;
        }

        let (bg_low, bg_high) = self.bg_shifter.registers();
        let (obj_low, obj_high, obj_palette, _obj_priority) = self.obj_shifter.registers();

        let sfetch_state = match &self.sprite_state {
            SpriteState::Fetching(sf) => sf.fetch_counter(),
            SpriteState::Idle => 0,
        };

        PpuTraceSnapshot {
            sprite_x,
            sprite_id,
            sprite_attr,
            bgw_fifo_a: bg_low,
            bgw_fifo_b: bg_high,
            spr_fifo_a: obj_low,
            spr_fifo_b: obj_high,
            pal_pipe: obj_palette,
            tfetch_state: self.fetcher.fetch_counter,
            sfetch_state,
            tile_temp_a: self.fetcher.tile_data_low(),
            tile_temp_b: self.fetcher.tile_data_high(),
            pix_count: self.pixel_counter.value(),
            sprite_count: sprites.count,
            scan_count: self.scan.scan_counter_entry(),
            rendering: self.hblank.rendering_active(),
            win_mode: self.window.window_rendered(),
            frame_num: 0,
        }
    }

    pub fn pipeline_state(
        &self,
        video: &VideoControl,
        regs: &PipelineRegisters,
    ) -> PipelineSnapshot {
        let (bg_low, bg_high) = self.bg_shifter.registers();
        let (obj_low, obj_high, obj_palette, obj_priority) = self.obj_shifter.registers();
        let (sprite_fetch_phase, sprite_tile_data) = match &self.sprite_state {
            SpriteState::Fetching(sf) => {
                (Some(SpriteFetchPhase::FetchingData), Some(sf.tile_data()))
            }
            SpriteState::Idle => (None, None),
        };
        PipelineSnapshot {
            pixel_counter: self.pixel_counter.value(),
            rendering_active: self.hblank.rendering_active(),
            bg_low,
            bg_high,
            obj_low,
            obj_high,
            obj_palette,
            obj_priority,
            sprite_fetch_phase,
            sprite_tile_data,
            lcd_x: self.lcd.lcd_x(),
            fetch_counter: self.fetcher.fetch_counter,
            window_hit: self.window.rydy(),
            pixel_gate: self.lcd.pixel_gate(),
            fine_scroll_match: self.lcd.fine_scroll_match(),
            fetcher_idle_stage_3: self.cascade.pygo(),
            fetcher_ready: self.cascade.poky(),
            wx_triggered: self.window.wx_triggered(regs),
            video_clock: video.scan_clock(),
            scan_done: self.scan.scan_done_flag(),
            scan_done_prev: self.scan.scan_done_prev(),
        }
    }

    pub(super) fn oam_locked(&self) -> bool {
        // OAM blocked by ACYL (BESU) or XYMU; also by scan_capture_pending (RUTU set, BESU not yet asserted).
        self.scan.mode2_active()
            || self.hblank.rendering_active()
            || self.scan.scan_capture_pending()
    }

    pub(super) fn vram_locked(&self) -> bool {
        self.hblank.rendering_active()
    }

    pub(super) fn oam_write_locked(&self) -> bool {
        // AJUJ = NOR3(dma_run, mode2, mode3) — write-permit override during the AVAP-cascade window.
        !self.hblank.ajuj_pulse() && (self.scan.mode2_active() || self.hblank.rendering_active())
    }

    pub(super) fn vram_write_locked(&self) -> bool {
        !self.hblank.ajuj_pulse() && self.hblank.rendering_active()
    }

    /// ALET rising: ALET-clocked DFFs capture (NYKA, PYGO, VOGA); XUPY-derived logic and combinational signals settle.
    pub(super) fn on_ppu_clock_rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &Vram,
    ) -> Option<PixelOutput> {
        // Terminal WODU pulse is a fall-edge transient; clear it so rise / off-edge reads see settled WODU.
        self.terminal_wodu_pulse = false;

        // REJO re-evaluates on every PPU rise (vblank↑ etc.); SARY captures only on TALU↑ (in fall).
        self.window.update_rejo_on_rise(video);

        // BYBA/AVAP have moved to on_ppu_clock_fall; here ALET-clocked DFFs and AJUJ close fire.
        self.hblank.tick_ajuj_pulse_on_rise();

        if self.scan.scanning() {
            return None;
        }

        // Capture XYMU before commit_end_of_line_on_rise() may clear it.
        let was_rendering = self.hblank.rendering_active();

        if was_rendering {
            self.mode3_rising(regs, video, oam, oam_bus, vram);
            // WODU is combinational on XANO/!FEPO. Re-evaluate post-WUTY so a same-rise
            // FEPO drop at a terminal pix latches VOGA without waiting for the next fall.
            let xano = self.pixel_counter.terminal();
            let fepo = self.fepo(regs.control.sprites_enabled());
            self.hblank.latch_end_of_line(xano, fepo);
        }

        // VOGA.q captures on this rise; WEGO clears XYMU.
        // `end_of_line` flags VOGA's just-committed transition — LCD pushes screen_x=159 on this dot.
        let end_of_line = self.hblank.commit_end_of_line_on_rise();

        let post_shift_pixel =
            pixel_output::resolve_current_pixel(&self.bg_shifter, &self.obj_shifter, regs);
        let pixel = self.lcd.on_ppu_clock_rise(
            self.hblank.end_of_line_latched(),
            end_of_line,
            post_shift_pixel,
        );

        // Mode 3 exit: clear fetch cascade and fine-scroll on XYMU↑.
        if was_rendering && !self.hblank.rendering_active() {
            self.cascade.reset();
            self.fine_scroll = FineScroll::new();
        }

        pixel
    }

    /// CATU runs every XUPY cycle regardless of POPU so the DFF advances across the 153→0 boundary.
    /// CATU's capture edge is the ATEJ pulse; ATEJ drives TADY low which async-resets PX bits
    /// (and VOGA, scan counter — shared `h_reset_n` net). PX reset rides this edge rather than
    /// firing synchronously in `reset_scanline`, matching the measured 1-dot delay between
    /// RUTU.q↑ and WODU↓.
    pub(super) fn tick_scan_capture(&mut self, video: &VideoControl) {
        let atej_rising = self.scan.tick_scan_capture(video.scan_clock(), video.ly());
        if atej_rising {
            self.pixel_counter.reset();
        }
    }

    /// ALET falling: MYVO-clocked DFFs capture (PORY); LEBO advances BG fetch counter; SACU drives CLKPIPE.
    pub(super) fn on_ppu_clock_fall(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        scan_clock_rising: bool,
        talu_rising: bool,
    ) -> Option<PixelOutput> {
        // SARY captures wy_match on TALU↑ (hclk); REJO re-evaluates every PPU fall for vblank↓.
        self.window.tick_wy_match_falling(regs, video, talu_rising);

        // Snapshot before AVAP reaction sets XYMU; the rise→rise gap models the 1-dot AVAP→LAXU delay.
        let was_rendering = self.hblank.rendering_active();

        // BYBA/AVAP co-locate on this XUPY-rising fall.
        let scan = self
            .scan
            .advance_scan(scan_clock_rising, video.ly(), regs, oam, oam_bus);
        if scan.avap {
            // Mode 3 begins on AVAP-fall; AJUJ pulse asserts alongside mode3↑ for write-permit.
            self.hblank.pulse_ajuj_on_avap_fall();
            self.window.init_nuko_wx(regs.window.x.output());
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // Mode 3 pixel output runs in two sub-phases: fetcher DFFs (MYVO-clocked, depth 16-22ge),
        // then pixel pipeline (SACU-driven, depth 63.8ge).
        // mode3_advance_fetcher is gated on was_rendering so the AVAP-reaction rise leaves LAXU at 0.
        if self.hblank.rendering_active() {
            // PORY may clear RYDY during advance_fetcher; SEKO and window restart need pre-PORY RYDY.
            // Pixel counter is only advanced by SACU in mode3_pixel_pipeline.
            let rydy_before_pory = self.window.rydy();
            let pixel_counter_before_sacu = self.pixel_counter.value();

            // MOSU↑ arming runs before mode3_advance_fetcher so the counter=0 VRAM read sees
            // fetching_window=true. When MOSU↑ fires, advance_fetcher is gated out for this dot.
            let poky_for_window = self.cascade.poky();
            // Pre-CUPA FEPO snapshot — matches `mode3_rising`'s read; gates PYCO via the VYBO halt path.
            let fepo_for_window = self.fepo(regs.sprites_enabled_pre_cupa);
            let mosu_fired = self.window.tick_falling(
                &mut self.fetcher,
                &mut self.cascade,
                &mut self.fine_scroll,
                pixel_counter_before_sacu,
                poky_for_window,
                fepo_for_window,
                regs,
            );

            // SUZU is a TEVO OR3 input alongside SEKO/TAVE; drives NYXU low (LOZE holds BG shifter).
            // `pending_window_trigger` carries the deferred-completion MOSU from the prior rise.
            let deferred_window_trigger = self.pending_window_trigger;
            self.pending_window_trigger = false;
            let load_window_pulse = if was_rendering && !mosu_fired && !deferred_window_trigger {
                self.mode3_advance_fetcher()
            } else {
                false
            };
            // MOSU is also a direct NYXU input; the pulse holds the BG shifter on this dot.
            let advance_nyxu_pulse = mosu_fired || deferred_window_trigger || load_window_pulse;
            self.mode3_pixel_pipeline(
                regs,
                rydy_before_pory,
                advance_nyxu_pulse,
                pixel_counter_before_sacu,
            )
        } else {
            None
        }
    }

    pub(super) fn reset_scanline(&mut self, scanline: u8) {
        self.hblank.reset();
        self.scan.reset();
        self.scan.arm_scan_capture();
        self.bg_shifter = BgShifter::new();
        self.obj_shifter = ObjShifter::new();
        self.fetcher.reset_scanline();
        self.cascade.reset();
        self.fine_scroll = FineScroll::new();
        self.window.reset_scanline();

        self.tyfa = false;
        self.pany_slip_pending = false;
        // pixel_counter is async-reset by the ATEJ pulse — fires from `tick_scan_capture`
        // when CATU.q rises, ~1 dot after RUTU.q rises here.
        self.lcd.reset(scanline);
        self.sprite_state = SpriteState::Idle;
        // SECA's ATEJ arm re-asserts TAKA at each scanline boundary; SOBU/SUDA free-run.
        self.sprite_trigger.arm_at_line_end();
    }

    /// Frame boundary at LY=0: window line counter and the per-scanline reset for line 0.
    pub(super) fn reset_frame(&mut self) {
        self.window.reset_frame();
        self.reset_scanline(0);
    }

    /// ALET rising: fetcher VRAM reads, cascade DFFs (NYKA, PYGO), POKY, TYFA, SABE, PUXA.
    fn mode3_rising(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &Vram,
    ) {
        // SOBU's ALET-rising DFF capture wins the TEKY→SOBU race vs CUPA's transparent-latch path —
        // SOBU sees the pre-write LCDC.1 value, so FEPO here uses pre-CUPA sprites_enabled.
        let mut fepo_pre_cupa = self.fepo(regs.sprites_enabled_pre_cupa);

        // LYRY = fetch_counter >= 5 (combinational). Counter only increments on rising.
        let bg_fetch_done = self.fetcher.bg_fetch_done();

        // BG fetcher counter=0/2/4 VRAM reads. Counter saturates at 5 during sprite fetch (MOCE=0
        // freezes LEBO) — no explicit !taka() gate needed here.
        self.fetcher.advance_falling(
            self.pixel_counter.value(),
            self.fine_scroll.pixel_clock_active(),
            self.window.window_line_counter(),
            self.window.wx_triggered(regs),
            regs,
            video,
            vram,
        );

        // Cascade advance runs before tick_rising so POKY's just-set value reaches the window's PYCO gate.
        self.cascade.advance_cascade(bg_fetch_done);

        // SOCY's gate chain is too slow to suppress the same-dot in-flight SACU↑;
        // sample RYDY before tick_rising so the TYFA snapshot sees the pre-MOSU value.
        let rydy_pre_mosu = self.window.rydy();

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
            match self.sprite_state {
                SpriteState::Fetching(ref mut sf) => {
                    let slot_index = sf.slot_index;
                    let done = sf.advance(regs, oam, oam_bus, vram);
                    if done {
                        let (s1y, s1x) = sf.stage1_capture();
                        sf.merge_into(&mut self.obj_shifter);
                        self.sprite_state = SpriteState::Idle;
                        self.sprite_trigger.clear_fetch_running();
                        // Per-slot fetched-flag captures at WUTY↑ (fetch completion); FEPO drops for this slot.
                        self.scan.sprites_mut().fetched |= 1 << slot_index;
                        // The fetch latched (tile-index, attribute) into the shared Stage-1 dlatches.
                        self.scan.set_stage1_held(s1y, s1x);
                        fepo_pre_cupa = self.fepo(regs.control.sprites_enabled());
                    }
                }
                SpriteState::Idle => {}
            }
        }

        // TEKY = AND4(FEPO, !RYDY, LYRY, !TAKA).
        let teky = fepo_pre_cupa
            && !self.window.rydy()
            && bg_fetch_done
            && !self.sprite_trigger.fetch_running();
        let ryce = self.sprite_trigger.tick_trigger_on_rise(teky);

        if ryce {
            self.start_sprite_fetch();
        }

        // Post-CUPA FEPO drives TYFA's combinational AND (CUPA→AROR→FEPO settles well before SACU).
        let fepo_post_cupa = self.fepo(regs.control.sprites_enabled());

        // TYFA = AND3(SOCY, POKY, VYBO). VYBO = NOR3(FEPO_old, WODU_old, MYVO).
        // rydy_pre_mosu is the pre-MOSU value so in-flight pre-window SACU fires on MOSU↑.
        self.tyfa = !fepo_post_cupa
            && !self.pixel_counter.terminal()
            && !rydy_pre_mosu
            && self.cascade.poky();

        // POHU = (count == SCX & 7); ROXO captures POHU into PUXA on the falling edge.
        self.fine_scroll
            .compare_falling(regs.background_viewport.x.output());
    }

    /// MYVO-clocked DFFs: SUDA, PORY, BG fetch counter (LEBO). Runs before the pixel pipeline
    /// (depth ~16-22 ge vs SACU at ~63.8 ge).
    fn mode3_advance_fetcher(&mut self) -> bool {
        self.sprite_trigger.tick_trigger_on_fall();

        // Counter saturates at 5 so it stays at 5 during sprite fetch without a !taka gate.
        self.fetcher.advance_rising();
        self.cascade.capture_pory();

        // PORY clears RYDY via the NOR3(PUKU, PORY, VID_RST) reset arm.
        // SUZU = AND2(!RYDY_new, SOVY): one-half-cycle pulse on RYDY 1→0; triggers TEVO.
        let load_window_pulse = self
            .window
            .release_window_hit_on_fetcher_reset(self.cascade.pory());
        if load_window_pulse {
            // SUZU → TEVO → NYXU: load window tile, reset fine counter.
            self.fetcher.load_into(&mut self.bg_shifter);
            self.fine_scroll.reset_counter();
        }

        // TAVE one-shot preload: fires when NYKA+PORY have risen but POKY hasn't latched PYGO yet.
        if self.cascade.nyka() && self.cascade.pory() && !self.cascade.poky() {
            self.fetcher.load_into(&mut self.bg_shifter);
            self.fine_scroll.reset_counter();
            // VEKU's TAVE arm clears TAKA carry-over from the prior scanline.
            self.sprite_trigger.clear_fetch_running();
        }

        load_window_pulse
    }

    /// SACU/CLKPIPE domain (depth ~63.8 ge); runs against settled fetcher state.
    /// Handles TYFA consumption, PUXA/POVA, pixel shifts, SEKO tile reload, LCD output, NUKO window trigger.
    fn mode3_pixel_pipeline(
        &mut self,
        regs: &PipelineRegisters,
        rydy_before_pory: bool,
        advance_nyxu_pulse: bool,
        pixel_counter_before_sacu: u8,
    ) -> Option<PixelOutput> {
        // FEPO before the pixel advance, for the terminal WODU pulse (FEPO settles after XANO).
        let pre_advance_fepo = self.fepo(regs.control.sprites_enabled());

        // TYFA snapshot from the prior rise; captures pre-MOSU RYDY so in-flight pre-window SACU fires on MOSU↑.
        let tyfa = self.tyfa;
        self.tyfa = false;

        // PUXA via ROXO. Using prior-rise TYFA carries the correct cascade-propagated POKY value.
        let fine_scroll_match = if tyfa {
            self.fine_scroll.capture_rising()
        } else {
            false
        };

        // SACU = TYFA && ROXY-released. VYBO = NOR3(MYVO, FEPO, WODU); TAKA freezes SACU only
        // indirectly via FEPO=1 on the unfetched per-slot flag.
        let sacu = tyfa && self.fine_scroll.pixel_clock_active();

        // PANY drain-detector slip: NUKO=1 lands when SEKO would fire (count==7), truncating
        // PANY's high pulse — RYFA captures the second half, slipping SEKO→TEVO→NYXU by 1 dot.
        let proposed_seko = self.fine_scroll.count == 7 && !rydy_before_pory;
        let window_x_hit = self.window.window_x_reached(pixel_counter_before_sacu);
        let pany_slip_now = proposed_seko && window_x_hit;
        let raw_seko_fire = (proposed_seko && !pany_slip_now) || self.pany_slip_pending;
        self.pany_slip_pending = pany_slip_now;

        // SEKO drain-detector freeze during sprite-fetch FEPO-held window: FEPO=1 → VYBO=0 →
        // SACU=0 → SEGU stuck at 1 → RYFA frozen → RENE.D = RYFA holds → SEKO = NOR2(RENE, RYFA)
        // holds at its pre-freeze value (0 in normal BG cadence). The collapsed `raw_seko_fire`
        // formula doesn't model the cascade DFFs explicitly, so we override to 0 during the
        // freeze. Zero NYXU pulses across 30 TAKA-high windows confirmed by gate-level FST.
        let fepo_held =
            self.sprite_trigger.fetch_running() && self.fepo(regs.control.sprites_enabled());
        let seko_fire = if fepo_held { false } else { raw_seko_fire };

        let nyxu_pulse = seko_fire || advance_nyxu_pulse;

        let pixel = pixel_output::resolve_current_pixel(&self.bg_shifter, &self.obj_shifter, regs);

        if seko_fire {
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        if sacu {
            // NYXU pulse holds the BG shifter via LOZE; OBJ shifter is not LOZE-gated.
            if !nyxu_pulse {
                self.bg_shifter.shift();
            }
            self.obj_shifter.shift();
            self.pixel_counter.advance();
        }

        // WODU sampled on the post-advance XANO/FEPO so OAM-X=167 sprites are visible on the same edge.
        let post_advance_fepo = self.fepo(regs.control.sprites_enabled());
        self.hblank
            .latch_end_of_line(self.pixel_counter.terminal(), post_advance_fepo);

        // Early WODU pulse: the post-advance XANO terminal decode settles before the FEPO
        // comparator, so an advance onto terminal PX pulses WODU before a terminal sprite's FEPO.
        self.terminal_wodu_pulse = self.pixel_counter.terminal() && !pre_advance_fepo;

        let (_toba, pixel_out) =
            self.lcd
                .on_ppu_clock_fall(sacu, pixel, fine_scroll_match, self.pixel_counter.value());

        if tyfa {
            self.fine_scroll.tick();
        }

        if seko_fire {
            self.fine_scroll.reset_counter();
        }

        self.window.update_nuko_wx(regs.window.x.output());

        pixel_out
    }

    /// FEPO: any unfetched sprite's X matches the pixel counter. Feeds VYBO/XENA/TEKY.
    /// Collapses XYLO/AROR/per-sprite-decoders/FOVE/FEFY into one loop; off-screen X≥168 excluded.
    fn fepo(&self, sprites_enabled: bool) -> bool {
        if !sprites_enabled {
            return false;
        }

        let match_x = self.pixel_counter.value();
        let sprites = self.scan.sprites_ref();
        for i in 0..sprites.count as usize {
            if sprites.fetched & (1 << i) != 0 {
                continue;
            }
            if sprites.entries[i].x == match_x && sprites.entries[i].x < 168 {
                return true;
            }
        }
        false
    }

    /// The per-slot fetched-flag is set at fetch completion (WUTY↑), not here, so FEPO stays
    /// high through the 6-dot fetch window (freezing SACU via VYBO).
    fn start_sprite_fetch(&mut self) {
        let match_x = self.pixel_counter.value();
        let sprites = self.scan.sprites_mut();

        for i in 0..sprites.count as usize {
            if sprites.fetched & (1 << i) != 0 {
                continue;
            }
            let entry = &sprites.entries[i];
            if entry.x == match_x && entry.x < 168 {
                self.sprite_state =
                    SpriteState::Fetching(SpriteFetch::new_fetching(*entry, i as u8));
                break;
            }
        }
    }
}
