pub use super::draw::sprite_fetch::SpriteFetchPhase;

use core::fmt;

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

/// Snapshot of all PPU internal state for gbtrace output.
/// Field names and semantics match the gbtrace spec's `ppu_internal` group.
pub struct PpuTraceSnapshot {
    /// Sprite store: 10 entries × (x, tile_index, attributes).
    /// `oamN_x` = X position, `oamN_id` = tile index from OAM byte 2,
    /// `oamN_attr` = attribute flags from OAM byte 3.
    /// Slots beyond `sprite_count` are zeroed.
    pub sprite_x: [u8; 10],
    pub sprite_id: [u8; 10],
    pub sprite_attr: [u8; 10],
    /// Pixel FIFO shift registers.
    pub bgw_fifo_a: u8,
    pub bgw_fifo_b: u8,
    pub spr_fifo_a: u8,
    pub spr_fifo_b: u8,
    pub pal_pipe: u8,
    /// Background tile fetcher counter (0-11 internal, mapped to 3-bit
    /// hardware counter by dividing by 2).
    pub tfetch_state: u8,
    /// Sprite fetcher counter (0-5). 0 when no sprite fetch active.
    pub sfetch_state: u8,
    /// Tile data temporary latches from the fetcher.
    pub tile_temp_a: u8,
    pub tile_temp_b: u8,
    /// Pixel counter (0-167).
    pub pix_count: u8,
    /// Number of sprites found during OAM scan (0-10).
    pub sprite_count: u8,
    /// OAM scan counter entry (0-39).
    pub scan_count: u8,
    /// XYMU rendering latch — true during Mode 3.
    pub rendering: bool,
    /// Window mode latch — true when window is being rendered.
    pub win_mode: bool,
    /// Frame counter (wrapping u16, incremented each VBlank).
    pub frame_num: u16,
}

/// Debug snapshot of pipeline state. Consumed by `headless::pipeline_state`
/// in the `missingno` crate, which emits field names verbatim as JSON keys
/// for the debugger HTTP API.
///
/// **Field names frozen as de-facto JSON API** — renames here break the
/// external debugger UI. Gate-level field names (XYMU, BYBA, DOBA, RYDY,
/// WUSA, POVA, PYGO, POKY, WUVU) are preserved for that reason. Internal
/// fields on `Rendering`, `HblankPipeline`, `SpriteScanner` etc. may
/// rename; the mapping happens in `Rendering::pipeline_state()`.
pub struct PipelineSnapshot {
    pub pixel_counter: u8,
    /// XYMU rendering latch (page 21). True = Mode 3 rendering active.
    pub xymu: bool,
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
    pub rydy: bool,
    pub wusa: bool,
    pub pova: bool,
    pub pygo: bool,
    pub poky: bool,
    pub wx_triggered: bool,
    pub wuvu: bool,
    /// Scan-done flag. BYBA (dffr, clocked by XUPY).
    pub byba: bool,
    /// Scan-done flag from the previous XUPY cycle. DOBA (dffr, ALET).
    pub doba: bool,
}

pub struct Rendering {
    /// Hblank pipeline: FEPO → WODU → VOGA → WEGO → clears XYMU.
    /// See `hblank_pipeline.rs` for clock domain and race pair documentation.
    hblank: HblankPipeline,
    /// Sprite scanner — scan counter, scanning latch, BYBA/DOBA pipeline,
    /// and the sprite store that bridges Mode 2 and Mode 3.
    scan: SpriteScanner,
    /// Background pixel shift register (page 32).
    bg_shifter: BgShifter,
    /// Sprite pixel shift register (pages 33-34).
    obj_shifter: ObjShifter,
    /// Background/window tile fetcher.
    fetcher: TileFetcher,
    /// Fetch-done cascade: LYRY → NYKA → PORY → PYGO → POKY DFF chain.
    /// Propagates fetcher-idle through pipeline delay stages.
    cascade: FetchCascade,
    /// Fine scroll counter and pixel clock gate (ROXY). Gates the pixel
    /// clock for SCX & 7 dots at the start of each line.
    fine_scroll: FineScroll,
    /// Window control block (die page 27): RYDY latch, WX comparator,
    /// window line counter, window zero pixel.
    window: WindowControl,
    /// TYFA (pixel clock enable): AND(!FEPO, !WODU, !RYDY, POKY).
    /// Computed in mode3_falling, consumed in mode3_rising.
    tyfa: bool,
    /// Pixel X position counter (PX; spec §2.5). Advances on SACU; feeds
    /// WODU via `terminal()` for the Mode 3→0 transition.
    pixel_counter: PixelCounter,
    /// LCD Control block (die page 24): LCD clock gating (WUSA),
    /// POVA trigger, LCD shift register, data latch.
    lcd: LcdControl,
    /// Sprite fetch lifecycle — Idle or Fetching.
    sprite_state: SpriteState,
    /// Sprite fetch trigger pipeline: TEKY → SOBU → SUDA → RYCE → TAKA.
    /// See `sprite_trigger.rs` for clock domain and race pair documentation.
    sprite_trigger: SpriteTrigger,
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
        }
    }

    /// Pre-set scanning active for LCD-on. Models VID_RST deassertion
    /// releasing the scan counter simultaneously with the rest of the
    /// pipeline.
    pub(super) fn start_scanning(&mut self) {
        self.scan.start_scanning();
    }

    /// Rendering-mode latch. XYMU (nor_latch); `true` during Mode 3.
    /// Polarity is opposite-sign from spec XYMU (which is active-low).
    pub(super) fn rendering_active(&self) -> bool {
        self.hblank.rendering_active()
    }

    /// ACYL signal: OAM scanning active (BESU-driven).
    /// Used by Ppu::mode() for independent NOR-gate mode bit computation.
    pub(super) fn is_scanning(&self) -> bool {
        self.scan.besu()
    }

    /// VOGA latch: true from the dot WODU fires through the rest of HBlank.
    pub(super) fn voga(&self) -> bool {
        self.hblank.voga()
    }

    /// WODU: combinational hblank gate. AND2(XUGU, !FEPO).
    /// On hardware, WODU is purely combinational — it does not
    /// depend on XYMU. During HBlank, WODU stays high (PX frozen
    /// at 167, FEPO=0), which is correct for CLKPIPE freeze and
    /// STAT mode readback.
    pub(super) fn wodu(&self) -> bool {
        self.hblank.wodu(self.pixel_counter.terminal())
    }

    /// Whether this is the LCD-enable first line (no prior scanline boundary).
    fn is_first_line(&self) -> bool {
        !self.scan.catu_enabled()
    }

    /// Whether the TAPA_INT_OAM signal is active.
    ///
    /// On hardware, TAPA = AND(TOLU_VBLANKn, SELA), where SELA derives from
    /// RUTU_LINE_ENDp — a 2-dot pulse at each scanline boundary. POPU
    /// gating at the call site handles the VBlank delay on normal line 0.
    ///
    /// On the LCD-enable first line, RUTU is suppressed (no scanline
    /// boundary has occurred), so TAPA never fires.
    pub(super) fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        if self.is_first_line() {
            return false;
        }
        video.line_end_active()
    }

    pub(super) fn scanner_oam_address(&self) -> Option<u8> {
        self.scan.oam_address()
    }

    /// Current OAM scan counter entry (0-39).
    pub(super) fn scan_counter_entry(&self) -> u8 {
        self.scan.scan_counter_entry()
    }

    /// Snapshot of the sprite store for debugging.
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

    /// Snapshot of all PPU internal state for gbtrace output.
    pub(super) fn trace_snapshot(&self, oam: &Oam) -> PpuTraceSnapshot {
        let sprites = self.scan.sprites_ref();
        let mut sprite_x = [0u8; 10];
        let mut sprite_id = [0u8; 10];
        let mut sprite_attr = [0u8; 10];
        for i in 0..sprites.count as usize {
            let entry = &sprites.entries[i];
            sprite_x[i] = entry.x;
            // Look up tile index (byte 2) and attributes (byte 3) from OAM.
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
            frame_num: 0, // Set by Ppu::trace_snapshot()
        }
    }

    pub fn pipeline_state(&self, video: &VideoControl) -> PipelineSnapshot {
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
            xymu: self.hblank.rendering_active(),
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
            rydy: self.window.rydy(),
            wusa: self.lcd.wusa(),
            pova: self.lcd.pova(),
            pygo: self.cascade.pygo(),
            poky: self.cascade.poky(),
            wx_triggered: self.window.wx_triggered(),
            wuvu: video.xupy(),
            byba: self.scan.scan_done_flag(),
            doba: self.scan.scan_done_prev(),
        }
    }

    pub(super) fn oam_locked(&self) -> bool {
        // Hardware: OAM blocked by ACYL (BESU-driven) or XYMU (rendering).
        // Also blocked when CATU is pending (RUTU set but not yet consumed) —
        // on hardware, the scan machinery gates the OAM bus as soon as the
        // scanline boundary fires, before BESU formally asserts.
        self.scan.besu() || self.hblank.rendering_active() || self.scan.catu_pending()
    }

    pub(super) fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp.
        self.hblank.rendering_active()
    }

    pub(super) fn oam_write_locked(&self) -> bool {
        // On DMG, reads and writes are gated identically.
        self.oam_locked()
    }

    pub(super) fn vram_write_locked(&self) -> bool {
        // On DMG, reads and writes are gated identically.
        self.vram_locked()
    }

    /// PPU clock rise (master-clock fall; gate: ALET rising): setup
    /// phase dispatcher.
    ///
    /// ALET-clocked DFFs capture here: NYKA, PYGO (cascade), VOGA
    /// (hblank). Also handles XUPY-derived logic (DOBA, scan-counter),
    /// NOR latches (POKY), combinational signals (TYFA bridge), fine
    /// scroll match (PUXA), and window WX match (PYCO).
    ///
    /// XUPY derives from WUVU, which is clocked by XOTA rising.
    /// The XOTA divider toggle runs in the preceding PPU-clock-fall
    /// phase, so video.xupy() reflects the post-toggle state here.
    pub(super) fn on_ppu_clock_rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
        palette_changed: bool,
    ) -> Option<PixelOutput> {
        // XUPY rising edge detection: the XOTA divider toggle (in the
        // preceding PPU-clock-fall phase) ran before this, so xupy()==true
        // means WUVU just went low→high.
        let xupy_rising = video.xupy();

        // Sprite scanner advance: counter tick, BYBA/DOBA capture,
        // AVAP evaluation (BYBA/DOBA-combinational, fires here).
        // AVAP reaction (XYMU set, fetcher load_into, BESU/scanning
        // clear) is deferred to the following PPU-clock-fall phase so
        // Mode 3 init co-occurs with AVAP's rising edge, which follows
        // BYBA's XUPY-rising capture at the PPU-clock-fall boundary.
        self.scan.advance_scan(xupy_rising, video.ly(), regs, oam);

        if self.scan.scanning() {
            // Mode 2: fetcher/VOGA/WEGO logic suppressed during scanning.
            return None;
        }

        // Capture XYMU before capture_voga() may clear it via VOGA/WEGO.
        let was_rendering = self.hblank.rendering_active();

        // Hblank pipeline: VOGA captures WODU on PPU clock rise (ALET rising).
        // WEGO clears XYMU. This is the primary Mode 3→0 path.
        let wodu = self.hblank.capture_voga(self.pixel_counter.terminal());

        // lcd fall receives current-dot wodu for last_pixel (the final
        // pixel push happens on the dot WODU fires, not one dot later).
        let pixel = self.lcd.on_ppu_clock_rise(self.hblank.voga(), wodu);

        if was_rendering {
            self.mode3_falling(regs, video, oam, vram, palette_changed);
        }

        pixel
    }

    /// Advance the CATU DFF — runs every dot regardless of VBlank.
    ///
    /// On hardware, CATU evaluates every XUPY cycle regardless of POPU.
    /// This must be called unconditionally so the DFF can advance during
    /// the 153->0 frame boundary while POPU is still high. When CATU
    /// fires, it sets the scanning latch (BESU) and resets the counter,
    /// priming the scanner for the new line.
    pub(super) fn tick_catu(&mut self, video: &VideoControl) {
        self.scan.tick_catu(video.xupy(), video.ly());
    }

    /// PPU clock fall (master-clock rise; gate: ALET falling): output
    /// phase dispatcher.
    ///
    /// MYVO-clocked DFFs capture here: PORY (cascade). BG fetch counter
    /// advances (LEBO fires on the PPU-clock-fall edge — LEBO =
    /// NAND(ALET, MOCE)). CLKPIPE fires (SACU rising edge, late in the
    /// dot). Handles BYBA capture, AVAP evaluation, pixel counter
    /// increment, fine counter increment, pipe shift, sprite X matching,
    /// and pixel output. CATU pipeline is advanced separately by
    /// `tick_catu()` (unconditional).
    pub(super) fn on_ppu_clock_fall(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
    ) -> Option<PixelOutput> {
        // WEGO pipeline stage: captures VOGA on this master-clock
        // edge, one edge after VOGA's own capture at the preceding
        // fall(). WEGO drives XYMU's set input, so rendering_active
        // clears here when VOGA is set. Defers the Mode 3→0 register
        // transition by one edge relative to VOGA capture.
        self.hblank.propagate_wego();

        let xupy_rising = video.xupy();

        // Snapshot xymu BEFORE the AVAP reaction can set it. This gates
        // the fetcher advance so the first LAXU toggle occurs on the
        // NEXT rise — matching hardware's 1-dot AVAP→LAXU delay (§5.3,
        // Q.A). The natural rise→rise gap plays the role previously
        // filled by the nyxu_reset_active hold.
        let was_rendering = self.hblank.rendering_active();

        // Scanner consumes avap_pending from the preceding advance_scan(),
        // clearing scanning/besu when AVAP fires.
        let scan = self
            .scan
            .apply_pending_avap(xupy_rising, video.ly(), regs, oam);

        // AVAP reaction: BYBA captures on XUPY rising, which follows
        // ALET falling in the divider chain (= PPU clock fall = this
        // method's edge). AVAP propagates combinationally from the
        // BYBA/DOBA/BALU scan-done detector. XYMU set, fetcher preload,
        // and window WX init fire here so Mode 3 init aligns with
        // hardware's AVAP-rising edge.
        if scan.avap {
            self.hblank.begin_rendering();
            self.window.init_nuko_wx(regs.window.x_plus_7.output());
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // SARY/REJO: sample the WY==LY latch every dot in all modes.
        // On hardware, SARY is clocked by TALU (rising-edge-derived).
        // Placing it before the xymu gate ensures mode 2 dots sample it.
        self.window.sample_wy_match(regs, video);

        // Mode 3 (drawing) — pixel output phase.
        // Runs when XYMU is set (rendering active).
        //
        // Two sub-phases model the hardware's signal domains:
        // 1. Fetcher DFF advance (myvo-clocked, depth 16-22 ge)
        // 2. Pixel pipeline (CLKPIPE/SACU-driven, depth 63.8 ge)
        //
        // mode3_advance_fetcher is gated on was_rendering: on the
        // AVAP-reaction rise, xymu was just set and the counter must
        // remain at 0 (LAXU reset still asserted through NYXU pulse).
        // The first advance fires on the next rise.
        if self.hblank.rendering_active() {
            // Snapshot pre-step-2 RYDY for SEKO and window check_trigger.
            // PORY may clear RYDY during mode3_advance_fetcher, but SEKO
            // and the window reactivation path need the pre-PORY value.
            // Pixel counter is unchanged by mode3_advance_fetcher (only
            // SACU increments it, which happens in mode3_pixel_pipeline).
            let rydy_before_pory = self.window.rydy();
            let pixel_counter_before_sacu = self.pixel_counter.value();
            if was_rendering {
                self.mode3_advance_fetcher(regs);
            }
            self.mode3_pixel_pipeline(regs, video, rydy_before_pory, pixel_counter_before_sacu)
        } else {
            None
        }
    }

    /// Reset per-line state at the scanline boundary. Called by
    /// `Ppu::on_master_clock_fall()` when `tick_dot` signals a new scanline.
    pub(super) fn reset_scanline(&mut self, scanline: u8) {
        self.hblank.reset();
        self.scan.reset();
        self.scan.enable_catu();
        self.bg_shifter = BgShifter::new();
        self.obj_shifter = ObjShifter::new();
        self.fetcher.reset_scanline();
        self.cascade.reset();
        self.fine_scroll = FineScroll::new();
        self.window.reset_scanline();

        self.tyfa = false;
        self.pixel_counter.reset();
        self.lcd.reset(scanline);
        self.sprite_state = SpriteState::Idle;
        self.sprite_trigger.reset();
        // BYBA, DOBA, and WUVU are handled by scan.reset() above.
        // WUVU free-runs (no reset) — lives on VideoControl.
    }

    /// Reset for a new frame (VBlank → Active Display transition at LY=0).
    /// Resets the screen buffer and window line counter, then performs the
    /// standard per-scanline reset for line 0. On hardware, the circuits
    /// persist through VBlank — this models the frame-boundary resets that
    /// individual blocks perform, not struct destruction/recreation.
    pub(super) fn reset_frame(&mut self) {
        self.window.reset_frame();
        self.reset_scanline(0);
    }

    /// Mode 3 processing on the PPU-clock-rise phase (master clock
    /// falls; gate: ALET rising).
    ///
    /// Fetcher VRAM reads (counter doesn't increment here — LEBO fires
    /// on PPU clock fall only), cascade DFFs (NYKA, PYGO), NOR latches
    /// (POKY), combinational signals (TYFA), sprite fetch counter
    /// advance (SABE), and fine scroll match (PUXA) fire on this edge.
    fn mode3_falling(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
        palette_changed: bool,
    ) {
        // FEPO evaluated before any falling-phase mutations. Feeds VYBO
        // (TYFA suppression) and is latched into hblank for next dot's wodu().
        let mut fepo = self.fepo(regs);

        // LYRY: combinational on fetch_counter (>= 5). The counter only
        // increments on rising (LEBO clock), so the value here reflects
        // the preceding rise — matching hardware's NYKA capturing
        // reg_old.LYRY on ALET (falling edge).
        let lyry = self.fetcher.lyry();

        // BG fetcher pauses during sprite fetch. On hardware, TAKA gates
        // the fetcher counter via LEBO — tfetch stays idle (5) throughout
        // sprite fetch, as confirmed by GateBoy traces showing
        // tfetch_state=05 for the entire duration of sfetch_state cycling.
        if !self.sprite_trigger.taka() {
            self.fetcher.advance_falling(
                self.pixel_counter.value(),
                self.window.window_line_counter(),
                regs,
                video,
                vram,
            );

            self.cascade.advance_cascade(lyry);
        }

        // Sprite fetch counter advance (SABE clock). On hardware, SABE =
        // NAND2(LAPE, TAME) fires when ALET rises (= master clock fall =
        // PPU clock rise = this method's edge). Placed BEFORE the
        // TEKY/RYCE block so a newly initiated sprite fetch doesn't
        // advance on its first dot (matching hardware where SABE needs
        // one ALET cycle after TAKA sets).
        if self.sprite_trigger.taka() {
            match self.sprite_state {
                SpriteState::Fetching(ref mut sf) => {
                    let done = sf.advance(regs, oam, vram);
                    if done {
                        sf.merge_into(&mut self.obj_shifter, oam);
                        // Data-pin pixel overwrite: REMY/RAVO update
                        // combinationally after sprite merge.
                        pixel_output::sprite_overwrite_data_latch(
                            &self.bg_shifter,
                            &self.obj_shifter,
                            self.lcd.data_latch_mut(),
                            self.window.window_zero_pixel_mut(),
                            regs,
                        );
                        self.sprite_state = SpriteState::Idle;
                        self.sprite_trigger.clear_taka();
                        // Recompute FEPO: the sprite is now marked as fetched,
                        // so FEPO goes false. On hardware, FEPO is purely
                        // combinational — TYFA sees the updated value
                        // immediately, allowing CLKPIPE to resume on the
                        // next rising edge.
                        fepo = self.fepo(regs);
                    }
                }
                SpriteState::Idle => {}
            }
        }

        // TEKY: combinational sprite fetch request.
        //
        // Before POKY fires (during the initial tile fetch pipeline startup),
        // TEKY is suppressed. On hardware, the TAVE→NYXU→MOCE combinational
        // path kills LYRY before TEKY can fire during the initial BG fetch
        // cascade. The cascade propagation (LYRY → NYKA → PORY → TAVE →
        // NYXU → LYRY=false) completes within 2 dots, but TEKY would fire
        // on the falling phase where LYRY first goes true — before TAVE
        // has fired on the next rising. By suppressing TEKY until POKY fires
        // (the end of the cascade), we ensure the second BG fetch completes
        // before sprite triggers can fire.
        //
        // After POKY fires (TAVE permanently disabled, normal rendering),
        // TEKY uses current LYRY directly — sprite triggers fire immediately
        // when a BG fetch completes.
        let lyry_for_teky = lyry && self.cascade.poky();
        let teky = fepo && !self.window.rydy() && lyry_for_teky && !self.sprite_trigger.taka();
        let ryce = self.sprite_trigger.capture_sobu(teky);

        if ryce {
            // Find and mark the matching sprite entry, start the fetch.
            self.start_sprite_fetch(regs);
        }

        // Latch FEPO for next dot's wodu() evaluation.
        self.hblank.latch_fepo(fepo);

        // TYFA = AND3(SOCY, POKY, VYBO). Bridge to rising phase for SACU.
        // SOCY = NOT(RYDY). VYBO = NOR3(FEPO_old, WODU_old, MYVO).
        //
        // LogicBoy uses `state_old.pix_count != 167` instead of `!WODU_old`.
        // At this point (falling, after rising incremented), pixel_counter
        // holds the post-increment value — which will be `state_old` on the
        // NEXT dot when TYFA is consumed. This avoids a one-dot delay through
        // the WODU storage path that caused pix_count to overshoot to 168.
        let tyfa =
            !fepo && !self.pixel_counter.terminal() && !self.window.rydy() && self.cascade.poky();
        self.tyfa = tyfa;

        // POHU: combinational comparator, count == SCX & 7.
        // On hardware, POHU is combinational and ROXO captures into PUXA
        // on the falling edge. The count value is from the preceding rising
        // (reg_old), matching hardware.
        self.fine_scroll
            .compare_falling(regs.background_viewport.x.output());

        // REMY/RAVO combinational refresh: if a palette write resolved
        // this dot, re-resolve the current pixel with the new palette
        // values. On hardware, REMY/RAVO is combinational and immediately
        // reflects palette changes — no pipeline delay.
        if palette_changed {
            self.lcd.set_data_latch(pixel_output::resolve_current_pixel(
                &self.bg_shifter,
                &self.obj_shifter,
                self.window.window_zero_pixel_mut(),
                regs,
            ));
        }
    }

    /// Rising edge Mode 3 — fetcher DFF advance (myvo-clocked domain).
    ///
    /// Runs first within on_ppu_clock_fall(), before the pixel pipeline.
    /// MYVO-clocked DFFs capture here: SUDA (sprite trigger), PORY
    /// (cascade), BG fetch counter (LEBO). NOR latch responses (RYDY
    /// clear via PORY) and the TAVE one-shot preload also fire here.
    ///
    /// On hardware, these signals settle at depth 16-22 ge — well before
    /// CLKPIPE fires at depth 63.8 ge. Separating them models the
    /// hardware's actual signal domains: fetcher DFFs settle first, then
    /// the pixel pipeline evaluates against the settled state.
    fn mode3_advance_fetcher(&mut self, regs: &PipelineRegisters) {
        // SUDA DFF: captures SOBU on LAPE rising edge (depth 6).
        self.sprite_trigger.capture_suda();

        // BG fetcher rising-edge advance: counter increment (LEBO clock).
        // Paused during sprite fetch (TAKA gates fetcher counter).
        if !self.sprite_trigger.taka() {
            self.fetcher.advance_rising();
            self.cascade.capture_pory();
        }

        // PORY clears RYDY: on hardware, PORY is a reset input to the
        // RYDY NOR latch (NOR3(PUKU, PORY, VID_RST)). When PORY goes
        // high, RYDY clears on the same half-cycle. The NYKA→PORY
        // cascade adds 1 dot of delay between the fetcher reaching Idle
        // (LYRY) and RYDY clearing, matching the hardware cascade timing.
        //
        // SUZU falling-edge detector: AND2(!RYDY_new, SOVY). SOVY holds
        // the pre-clear RYDY value (captured on falling). SUZU fires for
        // exactly one half-cycle when RYDY transitions 1→0, triggering
        // TEVO (pipe load + fine counter reset).
        if self.window.clear_rydy_on_pory(self.cascade.pory()) {
            // SUZU → TEVO → NYXU: load window tile data into pipe.
            self.fetcher.load_into(&mut self.bg_shifter);

            // TEVO → PASO: reset fine counter.
            self.fine_scroll.reset_counter();

            // REMY/RAVO combinational update: data pins reflect the
            // newly loaded window tile data immediately.
            self.lcd.set_data_latch(pixel_output::resolve_current_pixel(
                &self.bg_shifter,
                &self.obj_shifter,
                self.window.window_zero_pixel_mut(),
                regs,
            ));
        }

        // TAVE one-shot preload: AND4(rendering, !POKY, NYKA, PORY).
        // Fires on the same rising phase that PORY goes high, because NYKA
        // was already latched on the preceding falling edge. The !PYGO guard
        // models !POKY -- PYGO is captured below (after TAVE), so
        // !self.pygo is still true at TAVE time. Once PYGO fires,
        // !self.pygo permanently disables TAVE, matching hardware where
        // POKY disables SUVU/TAVE.
        if self.cascade.nyka() && self.cascade.pory() && !self.cascade.pygo() {
            self.fetcher.load_into(&mut self.bg_shifter);
            // TAVE → TEVO → PASO: reset fine counter. On hardware, TEVO
            // drives PASO which resets the fine counter on every pipe load
            // (TAVE, SEKO, SUZU). The SUZU path already has this reset
            // (line above); SEKO has it in the pixel pipeline section. TAVE
            // was missing it, causing the first tile after the preload to
            // be 7 pixels instead of 8.
            self.fine_scroll.reset_counter();
        }
    }

    /// Rising edge Mode 3 — pixel pipeline (CLKPIPE / SACU domain).
    ///
    /// Runs second within on_ppu_clock_fall(), after fetcher DFFs have settled.
    /// SACU (the pixel clock) fires at depth 63.8 ge — significantly later
    /// than the MYVO-clocked DFFs. This method evaluates against the
    /// settled fetcher state from `mode3_advance_fetcher`.
    ///
    /// Handles: TYFA consumption, PUXA/POVA fine scroll match, pixel
    /// shift registers, SEKO tile reload, LCD output, fine scroll
    /// counter, and NUKO window trigger. Sprite fetch advance now
    /// happens in mode3_falling (SABE clock, PPU-clock-rise edge).
    fn mode3_pixel_pipeline(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        rydy_before_pory: bool,
        pixel_counter_before_sacu: u8,
    ) -> Option<PixelOutput> {
        let mut pixel_out: Option<PixelOutput> = None;

        // Consume TYFA from falling phase. On the first rising phase
        // after AVAP (Mode 2→3), no falling has run yet — TYFA is false
        // (pixel clock not yet enabled).
        let tyfa = self.tyfa;
        self.tyfa = false;

        // PUXA capture: ROXO fires when TYFA is active. TYFA is
        // combinational (AND3(SOCY, POKY, VYBO)), but POKY only updates
        // on the falling edge — PORY just latched above, but PYGO won't
        // capture PORY until the next falling phase. Use tyfa
        // (computed at the end of the previous falling phase) which has
        // the correct cascade-propagated POKY value.
        let pova = if tyfa {
            self.fine_scroll.capture_rising()
        } else {
            false
        };

        // Sprite fetch counter now advances in mode3_falling (SABE clock,
        // PPU-clock-rise edge). When TAKA is true, the pixel pipeline is frozen.
        // After sprite fetch completes in falling, TAKA clears and FEPO is
        // recomputed, so TYFA correctly enables the pixel clock on the next
        // rising edge.

        if !self.sprite_trigger.taka() {
            let sacu = tyfa && self.fine_scroll.pixel_clock_active();

            if sacu {
                self.bg_shifter.shift();
                self.obj_shifter.shift();
            }

            let seko_fire = self.fine_scroll.count == 7 && !rydy_before_pory;

            if seko_fire {
                self.fetcher.load_into(&mut self.bg_shifter);
            }

            let pixel = pixel_output::resolve_current_pixel(
                &self.bg_shifter,
                &self.obj_shifter,
                self.window.window_zero_pixel_mut(),
                regs,
            );
            if sacu {
                self.pixel_counter.advance();
            }
            let (toba, pix) =
                self.lcd
                    .on_ppu_clock_fall(sacu, pixel, pova, self.pixel_counter.value());
            pixel_out = pix;

            if !toba && tyfa {
                self.window.consume_window_zero_pixel();
            }

            if tyfa {
                self.fine_scroll.tick();
            }

            if seko_fire {
                self.fine_scroll.reset_counter();
            }
        }

        self.window.apply_xofo(regs.control.window_enabled());

        let pygo = self.cascade.pygo();
        self.window.check_trigger(
            rydy_before_pory,
            &mut self.fetcher,
            &mut self.cascade,
            &mut self.fine_scroll,
            pixel_counter_before_sacu,
            pygo,
            regs,
            video,
        );

        self.window.update_nuko_wx(regs.window.x_plus_7.output());

        pixel_out
    }

    /// FEPO: combinational OR of all unfetched sprite store X comparators,
    /// gated by AROR (sprites_enabled). True when any unfetched sprite
    /// matches the current pixel counter.
    fn fepo(&self, regs: &PipelineRegisters) -> bool {
        if !regs.control.sprites_enabled() {
            return false; // AROR = AND(RENDERING, XYLO). XYLO off -> FEPO low.
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

    /// Start sprite fetch for the first matching unfetched sprite.
    /// Called when RYCE fires (SOBU rising edge detected).
    fn start_sprite_fetch(&mut self, _regs: &PipelineRegisters) {
        let match_x = self.pixel_counter.value();
        let sprites = self.scan.sprites_mut();

        for i in 0..sprites.count as usize {
            if sprites.fetched & (1 << i) != 0 {
                continue;
            }
            let entry = &sprites.entries[i];
            if entry.x == match_x && entry.x < 168 {
                sprites.fetched |= 1 << i;
                self.sprite_state = SpriteState::Fetching(SpriteFetch::new_fetching(*entry));
                break;
            }
        }
    }
}
