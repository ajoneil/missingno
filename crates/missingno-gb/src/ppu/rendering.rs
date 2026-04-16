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
    pub byba: bool,
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
    /// LCD Control block (die page 24): pixel X counter, LCD clock
    /// gating (WUSA), POVA trigger, LCD shift register, data latch.
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

    pub(super) fn xymu(&self) -> bool {
        self.hblank.xymu()
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
        self.hblank.wodu(self.lcd.xugu())
    }

    /// Pre-CPU-read settling: VOGA captures WODU, XYMU clears.
    /// Called after PPU rise, before CPU bus read. On hardware, ALET
    /// falls at F->G before BUKE opens at G-H.
    pub(super) fn settle_alet(&mut self) {
        if self.scan.scanning() {
            return; // Mode 2: no hblank logic
        }
        self.hblank.settle_alet(self.lcd.xugu());
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
            pix_count: self.lcd.pixel_counter(),
            sprite_count: sprites.count,
            scan_count: self.scan.scan_counter_entry(),
            rendering: self.hblank.xymu(),
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
            pixel_counter: self.lcd.pixel_counter(),
            xymu: self.hblank.xymu(),
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
            byba: self.scan.byba(),
            doba: self.scan.doba(),
        }
    }

    pub(super) fn oam_locked(&self) -> bool {
        // Hardware: OAM blocked by ACYL (BESU-driven) or XYMU (rendering).
        // Also blocked when CATU is pending (RUTU set but not yet consumed) —
        // on hardware, the scan machinery gates the OAM bus as soon as the
        // scanline boundary fires, before BESU formally asserts.
        self.scan.besu() || self.hblank.xymu() || self.scan.catu_pending()
    }

    pub(super) fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp.
        self.hblank.xymu()
    }

    pub(super) fn oam_write_locked(&self) -> bool {
        // On DMG, reads and writes are gated identically.
        self.oam_locked()
    }

    pub(super) fn vram_write_locked(&self) -> bool {
        // On DMG, reads and writes are gated identically.
        self.vram_locked()
    }

    /// Falling edge (master clock falls → alet rises): setup phase.
    ///
    /// Alet-clocked DFFs capture here: NYKA, PYGO (cascade), VOGA
    /// (hblank). Also handles XUPY-derived logic (DOBA, scan-counter),
    /// NOR latches (POKY), combinational signals (TYFA bridge), fine
    /// scroll match (PUXA), and window WX match (PYCO).
    ///
    /// XUPY derives from WUVU, which is clocked by XOTA rising.
    /// The XOTA divider toggle runs in Ppu::rise(), before this method,
    /// so video.xupy() reflects the post-toggle state here.
    pub(super) fn fall(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
        palette_changed: bool,
    ) -> Option<PixelOutput> {
        // XUPY rising edge detection: the XOTA divider toggle (in
        // Ppu::rise()) ran before this, so xupy()==true means WUVU
        // just went low→high.
        let xupy_rising = video.xupy();

        // Sprite scanner falling edge: counter tick, BYBA/DOBA capture,
        // AVAP evaluation (all XUPY/alet-clocked, fire in fall).
        self.scan.fall(xupy_rising, video.ly(), regs, oam);

        // AVAP reaction: XYMU NOR latch responds immediately to AVAP.
        // Mode 3 init (fetcher preload, NYXU reset, window WX cache)
        // fires in the same fall() so Mode 3 is ready for the next rise().
        if self.scan.avap_pending() {
            self.hblank.set_xymu();
            self.window.init_nuko_wx(regs.window.x_plus_7.output());
            self.fetcher.load_into(&mut self.bg_shifter);
            self.fetcher.nyxu_reset_active = true;
        }

        if self.scan.scanning() {
            // Mode 2: fetcher/VOGA/WEGO logic suppressed during scanning.
            return None;
        }

        // Capture XYMU before hblank.fall() may clear it via VOGA/WEGO.
        let was_rendering = self.hblank.xymu();

        // Hblank pipeline: VOGA captures WODU on alet rising (= fall).
        // WEGO clears XYMU. This is the primary Mode 3→0 path.
        let wodu = self.hblank.fall(self.lcd.xugu());

        // lcd.fall() receives current-dot wodu for last_pixel (the final
        // pixel push happens on the dot WODU fires, not one dot later).
        let pixel = self.lcd.fall(self.hblank.voga(), wodu);

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

    /// Rising edge (master clock rises → alet falls): output phase.
    ///
    /// Myvo-clocked DFFs capture here: PORY (cascade). BG fetch counter
    /// advances (LEBO fires when alet falls). CLKPIPE fires (SACU
    /// rising edge, late in the dot). Handles BYBA capture, AVAP
    /// evaluation, pixel counter increment, fine counter increment,
    /// pipe shift, sprite X matching, and pixel output. CATU pipeline
    /// is advanced separately by `tick_catu()` (unconditional).
    pub(super) fn rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
    ) -> Option<PixelOutput> {
        // CATU processing (scanning start) is back in tick_catu (fall)
        // with 1-dot propagation delay — matching dmg-sim data showing
        // CATU and first scan advance simultaneous, 1 dot after RUTU.
        let xupy_rising = video.xupy();
        let scan = self.scan.rise(xupy_rising, video.ly(), regs, oam);

        // AVAP reaction (XYMU set, fetcher preload, NYXU reset, window
        // WX init) now fires in fall() when AVAP is detected.

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
        if self.hblank.xymu() {
            // Snapshot pre-step-2 RYDY for SEKO and window check_trigger.
            // PORY may clear RYDY during mode3_advance_fetcher, but SEKO
            // and the window reactivation path need the pre-PORY value.
            // Pixel counter is unchanged by mode3_advance_fetcher (only
            // SACU increments it, which happens in mode3_pixel_pipeline).
            let rydy_before_pory = self.window.rydy();
            let pixel_counter_before_sacu = self.lcd.pixel_counter();
            self.mode3_advance_fetcher(regs);
            self.mode3_pixel_pipeline(
                regs,
                video,
                rydy_before_pory,
                pixel_counter_before_sacu,
            )
        } else {
            None
        }
    }

    /// Reset per-line state at the scanline boundary. Called by
    /// `Ppu::rise()` when `tick_dot` signals a new scanline.
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

    /// Falling edge Mode 3 processing (master falls → alet rises).
    ///
    /// Fetcher VRAM reads (counter doesn't increment on fall — LEBO fires
    /// on rise only), cascade DFFs (NYKA, PYGO), NOR latches (POKY),
    /// combinational signals (TYFA), sprite fetch counter advance (SABE),
    /// and fine scroll match (PUXA) fire on the falling edge.
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
                self.lcd.pixel_counter(),
                self.window.window_line_counter(),
                regs,
                video,
                vram,
            );

            self.cascade.fall(lyry);
        }

        // Sprite fetch counter advance (SABE clock). On hardware, SABE =
        // NAND2(LAPE, TAME) fires when alet rises (= master falls = this
        // falling phase). Placed BEFORE the TEKY/RYCE block so a newly
        // initiated sprite fetch doesn't advance on its first dot (matching
        // hardware where SABE needs one alet cycle after TAKA sets).
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
        let ryce = self.sprite_trigger.fall(teky);

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
        let tyfa = !fepo && !self.lcd.xugu() && !self.window.rydy() && self.cascade.poky();
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
    /// Runs first within rise(), before the pixel pipeline. Myvo-clocked
    /// DFFs capture here: SUDA (sprite trigger), PORY (cascade), BG fetch
    /// counter (LEBO). NOR latch responses (RYDY clear via PORY) and the
    /// TAVE one-shot preload also fire here.
    ///
    /// On hardware, these signals settle at depth 16-22 ge — well before
    /// CLKPIPE fires at depth 63.8 ge. Separating them models the
    /// hardware's actual signal domains: fetcher DFFs settle first, then
    /// the pixel pipeline evaluates against the settled state.
    fn mode3_advance_fetcher(&mut self, regs: &PipelineRegisters) {
        // SUDA DFF: captures SOBU on LAPE rising edge (depth 6).
        self.sprite_trigger.rise();

        // BG fetcher rising-edge advance: counter increment (LEBO clock).
        // Paused during sprite fetch (TAKA gates fetcher counter).
        if !self.sprite_trigger.taka() {
            self.fetcher.advance_rising();
            self.cascade.rise();
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
    /// Runs second within rise(), after fetcher DFFs have settled. SACU
    /// (the pixel clock) fires at depth 63.8 ge — significantly later
    /// than the myvo-clocked DFFs. This method evaluates against the
    /// settled fetcher state from `mode3_advance_fetcher`.
    ///
    /// Handles: TYFA consumption, PUXA/POVA fine scroll match, pixel
    /// shift registers, SEKO tile reload, LCD output, fine scroll
    /// counter, and NUKO window trigger. Sprite fetch advance now
    /// happens in mode3_falling (SABE clock, alet-rise edge).
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
        // alet-rise edge). When TAKA is true, the pixel pipeline is frozen.
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
            let (toba, pix) = self.lcd.rise(sacu, pixel, pova);
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

        let match_x = self.lcd.pixel_counter();
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
        let match_x = self.lcd.pixel_counter();
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
