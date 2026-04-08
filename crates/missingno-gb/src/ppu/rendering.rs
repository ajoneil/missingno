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

/// Within-phase snapshot of signals that are both read and written during
/// `mode3_rising`. On hardware, combinational logic within a phase reads DFF
/// outputs from before the clock edge (rising edge). This struct captures
/// those values at the top of `mode3_rising` before any sequential mutations
/// within the same phase.
struct RisingPhaseInputs {
    /// RYDY value from the previous phase boundary. TYFA, SEKO, and SUZU
    /// all read this (modeling state_old.RYDY) rather than the live value.
    rydy: bool,
    /// Pixel counter value before SACU increment. NUKO (window trigger
    /// comparator) reads pix_count DFF Q-outputs combinationally — the
    /// pre-clock value, before the SACU edge advances the counter.
    pixel_counter: u8,
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

/// Values bridged between half-phases. The pipeline alternates:
/// fall() produces FallingToRising (tyfa), rise() consumes it.
/// rise() produces RisingConsumed as a sentinel, fall() ignores it.
///
/// TEKY is computed directly in mode3_falling (where SOBU captures it
/// on the TAVA clock edge), so it no longer needs bridging from rising.
/// This also ensures FEPO/TEKY see the current LCDC sprites_enabled
/// value (LCDC writes land before ppu.fall() in execute.rs).
enum PhaseBridge {
    /// Produced by mode3_falling, consumed by mode3_rising.
    FallingToRising {
        /// TYFA_CLKPIPE_evn: pixel clock enable.
        /// AND(!FEPO, !WODU, !RYDY, POKY).
        tyfa: bool,
    },
    /// Sentinel written by mode3_rising after consuming FallingToRising.
    /// Prevents stale tyfa from being re-consumed on the next rising.
    RisingConsumed,
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
    /// Combinational values crossing the half-phase boundary. Alternates
    /// between FallingToRising (tyfa) and RisingConsumed.
    phase_bridge: PhaseBridge,
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
            phase_bridge: PhaseBridge::RisingConsumed,
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

    /// WODU: combinational hblank gate. AND3(XYMU, XUGU, !FEPO).
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

    /// Falling edge (DELTA_EVEN): setup phase.
    ///
    /// On hardware, the falling edge handles XUPY-derived logic (DOBA,
    /// scan-counter), fetcher control signals (NYKA, POKY), mode
    /// transitions (VOGA/WEGO clearing XYMU), fine scroll match (PUXA),
    /// and window WX match (PYCO). AVAP and CATU moved to rise().
    ///
    /// XUPY derives from WUVU, which is clocked by XOTA rising (H→A
    /// boundary). The XOTA divider toggle runs in Ppu::rise(), before
    /// this Falling-phase method, so video.xupy() reflects the
    /// post-toggle state here.
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

        // Sprite scanner falling edge: counter tick, DOBA capture.
        self.scan.fall(xupy_rising, video.ly(), regs, oam);

        if self.scan.scanning() {
            // Mode 2: fetcher/VOGA/WEGO logic suppressed during scanning.
            return None;
        }

        // Hblank pipeline: if settle_alet() already ran this dot,
        // returns cached values. Otherwise computes fresh (e.g.
        // Mode 2→3 transition where scanning was active during settle).
        let wodu = self.hblank.fall(self.lcd.xugu());

        // lcd.fall() receives current-dot wodu for last_pixel (the final
        // pixel push happens on the dot WODU fires, not one dot later).
        let pixel = self.lcd.fall(self.hblank.voga(), wodu);

        if self.hblank.xymu_before_settle() {
            // Use xymu_before_settle: on the dot VOGA fires, settle_alet()
            // already cleared XYMU, but mode3_falling still needs to run
            // for the final fetcher/TYFA work.
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

    /// Rising edge (DELTA_ODD): output phase.
    ///
    /// On hardware, the rising edge handles BYBA capture, AVAP evaluation,
    /// pixel counter increment, fine counter increment, pipe shift, and
    /// sprite X matching. CATU pipeline is advanced separately by
    /// `tick_catu()` (unconditional, not gated by VBlank).
    pub(super) fn rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) -> Option<PixelOutput> {
        // Sprite scanner rising edge: BYBA captures FETO, AVAP evaluated.
        // CATU was already advanced by tick_catu().
        let xupy_rising = video.xupy();
        let scan = self.scan.rise(xupy_rising, video.ly(), regs, oam);

        // React to scan signals.
        // AVAP fires identically on normal lines and the LCD-on first line —
        // the scan counter runs to 39 independent of BESU (scanning latch).
        if scan.avap {
            self.hblank.set_xymu();
            self.window.init_nuko_wx(regs.window.x_plus_7.output());
            // NYXU fires: load stale tile_temp into BG pipe (LOZE -> DFF22 SET/RST).
            // On hardware, tile_temp retains data from the previous line's last
            // BG fetch. TileFetcher's tile_data_low/tile_data_high model tile_temp.
            self.fetcher.load_into(&mut self.bg_shifter);
            // NYXU reset overrides LEBO on this ODD phase — the counter stays
            // at 0 until the next rise. Suppress the advance_rising that will
            // run later this same dot in mode3_rising.
            self.fetcher.nyxu_reset_active = true;
        }

        // SARY/REJO: sample the WY==LY latch every dot in all modes.
        // On hardware, SARY is clocked by TALU (rising-edge-derived).
        // Placing it before the xymu gate ensures mode 2 dots sample it.
        self.window.sample_wy_match(regs, video);

        // Mode 3 (drawing) — pixel output phase.
        // Runs when XYMU is set (rendering active).
        if self.hblank.xymu() {
            self.mode3_rising(regs, video, oam, vram)
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

        self.phase_bridge = PhaseBridge::RisingConsumed;
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

    /// Falling edge Mode 3 processing.
    ///
    /// Fetcher VRAM reads (counter doesn't increment on fall — LEBO fires
    /// on ODD/rise only), cascade DFFs (NYKA, PYGO), NOR latches (POKY),
    /// combinational signals (TYFA bridge), and fine scroll match (PUXA)
    /// fire on the falling edge.
    fn mode3_falling(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        _oam: &Oam,
        vram: &Vram,
        palette_changed: bool,
    ) {
        // FEPO evaluated before any falling-phase mutations. Feeds VYBO
        // (TYFA suppression) and is latched into hblank for next dot's wodu().
        let fepo = self.fepo(regs);

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
        self.phase_bridge = PhaseBridge::FallingToRising { tyfa };

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

    /// Rising edge Mode 3 pixel pipeline processing.
    fn mode3_rising(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) -> Option<PixelOutput> {
        let mut pixel_out: Option<PixelOutput> = None;

        // Consume TYFA from falling phase. On the first rising phase
        // after AVAP (Mode 2→3), no falling has run yet — TYFA is false
        // (pixel clock not yet enabled).
        let tyfa = match self.phase_bridge {
            PhaseBridge::FallingToRising { tyfa } => tyfa,
            PhaseBridge::RisingConsumed => false,
        };
        // Mark bridge as consumed. TEKY is computed directly in
        // mode3_falling (where SOBU captures it), so no data is bridged
        // from rising to falling.
        self.phase_bridge = PhaseBridge::RisingConsumed;

        // SUDA DFF: captures SOBU on LAPE rising edge (depth 6).
        self.sprite_trigger.rise();

        // Phase-boundary snapshot: capture pre-edge values of signals
        // that are both read and written within this half-phase. All
        // combinational logic (TYFA, SEKO, SUZU, NUKO) reads from
        // `inputs`; all mutations go to `self`.
        let inputs = RisingPhaseInputs {
            rydy: self.window.rydy(),
            pixel_counter: self.lcd.pixel_counter(),
        };

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

        // Track whether sprite fetch just completed this dot. On hardware,
        // FEPO_old (previous dot's FEPO) gates TYFA — even after sfetch_done
        // clears TAKA, FEPO_old keeps TYFA suppressed for one more dot because
        // FEPO was still true on the preceding dot (sprite store X isn't cleared
        // until sfetch_done). In missingno, we model this by not running the
        // pixel pipeline on the sfetch_done dot — TAKA clears but the pipeline
        // waits until the next dot when TYFA is recomputed with FEPO=false.
        let mut sfetch_done_this_dot = false;

        if self.sprite_trigger.taka() {
            // Sprite fetch active: advance sprite data pipeline.
            match self.sprite_state {
                SpriteState::Fetching(ref mut sf) => {
                    let done = sf.advance(regs, oam, vram);
                    if done {
                        // WUTY fires on the rising phase of counter=5 (the
                        // same dot as the tile data HIGH read). On hardware,
                        // sprite pixel merge (RACA latch) and TAKA clear
                        // happen on the same dot — no separate "done" phase.
                        sf.merge_into(&mut self.obj_shifter, oam);

                        // Data-pin pixel overwrite: REMY/RAVO update
                        // combinationally after sprite merge. Overwrite the
                        // last SEMU-written position with merged pixel data.
                        pixel_output::sprite_overwrite_data_latch(
                            &self.bg_shifter,
                            &self.obj_shifter,
                            self.lcd.data_latch_mut(),
                            self.window.window_zero_pixel_mut(),
                            regs,
                        );
                        self.sprite_state = SpriteState::Idle;
                        // VEKU clears TAKA — sprite fetch complete.
                        // Do NOT fall through to the pixel pipeline on this
                        // dot. On hardware, FEPO_old (from the previous dot,
                        // when FEPO was still true) suppresses TYFA for one
                        // more dot after sfetch_done. The pixel pipeline will
                        // resume on the next dot when TYFA is recomputed.
                        self.sprite_trigger.clear_taka();
                        sfetch_done_this_dot = true;
                    }
                }
                SpriteState::Idle => {
                    // TAKA set but no sprite fetching yet — RYCE just fired
                    // on the falling phase and start_sprite_fetch set up the
                    // fetch. The first advance will happen on the next rising.
                }
            }
        }

        if !self.sprite_trigger.taka() && !sfetch_done_this_dot {
            // Normal pixel pipeline — no sprite fetch active.

            // SACU_CLKPIPE = pixel clock edge, derived from TYFA and ROXY.
            // SEGU = NOT(TYFA). SACU = OR2(SEGU, ROXY) through toggle.
            // Net: SACU fires when TYFA is high AND ROXY is done (fine
            // scroll complete). Drives pipe shift registers and pixel counter.
            let sacu = tyfa && self.fine_scroll.pixel_clock_active();

            // Hardware within-tick ordering for DFF22 shift register cells:
            // 1. Synchronous shift (SACU clock edge)
            // 2. Async parallel load (LOZE SET/RST — overwrites shift)
            // 3. Pixel output reads final state
            if sacu {
                self.bg_shifter.shift();
                self.obj_shifter.shift();
            }

            // RYFA DFF captures (count==7 && !RYDY) on each dot.
            // SEKO is the rising-edge detector on RYFA — it fires one dot
            // after count reaches 7. Reading count HERE (before tick)
            // naturally models this one-dot DFF delay. PANY gates RYFA
            // on !RYDY (window hit blocks tile boundary detection).
            let seko_fire = self.fine_scroll.count == 7 && !inputs.rydy;

            // SEKO → TEVO → NYXU: pipe reload (async). LOZE SET/RST
            // overwrites the shift result on the same tick — the load
            // naturally wins because the shift already fired above
            // (matching DFF22 behavior).
            if seko_fire {
                self.fetcher.load_into(&mut self.bg_shifter);
                // SEKO resets the fetcher counter (TEVO -> LOVY/LAXU/TYFO
                // reset), which drives LYRY low combinationally (phase < 10).
            }

            // LCD Control (page 24): pixel counter, XAJO, TOBA, shift
            // register, data latch — all internal to the block. We
            // provide SACU, the resolved pixel, and POVA.
            let pixel = pixel_output::resolve_current_pixel(
                &self.bg_shifter,
                &self.obj_shifter,
                self.window.window_zero_pixel_mut(),
                regs,
            );
            let (toba, pix) = self.lcd.rise(sacu, pixel, pova);
            pixel_out = pix;

            if !toba && tyfa {
                // Consume window_zero_pixel during pre-visible TYFA
                // cycles (fine scroll gating, pre-WUSA). On hardware,
                // the data pins update on every TYFA edge — the window
                // zero pixel is consumed even when SACU/TOBA don't fire.
                self.window.consume_window_zero_pixel();
            }

            // BG fetcher advances on falling (mode3_falling).
            // SUZU (window fetch completion) is triggered by PORY in mode3_rising.

            // PECU (fine counter clock) derives from ROXO, which derives
            // from TYFA. Fine scroll ticks whenever the pixel clock is
            // enabled, regardless of ROXY (fine scroll itself).
            if tyfa {
                self.fine_scroll.tick();
            }

            // TEVO → PASO: when SEKO fired this dot, reset the fine
            // counter to 0. Placed after tick() because tick() self-stops
            // at 7 (ROZE gate) — PASO then clears the stopped counter.
            if seko_fire {
                self.fine_scroll.reset_counter();
            }
        }

        // NUKO (combinational WX comparator) reads pre-SACU
        // pixel_counter (inputs.pixel_counter). On hardware, NUKO
        // reads pix_count DFF Q-outputs combinationally; PYCO
        // captures on the same ROCO edge that SACU increments
        // pix_count. The pygo parameter gates the comparison
        // (PYCO requires ROCO, which requires POKY). Placed
        // outside the sprite_state match because NUKO is combinational
        // — it fires regardless of sprite fetch state. During sprite
        // fetch, pixel_counter is frozen, so the match just re-checks
        // the same value each dot.
        // XOFO: combinational gate — when WIN_EN is low, resets PYNU
        // (wx_triggered), allowing re-triggering when WIN_EN goes high.
        // Placed before check_trigger so a fresh trigger can fire on the
        // same dot that WIN_EN transitions high.
        self.window.apply_xofo(regs.control.window_enabled());

        let pygo = self.cascade.pygo();
        self.window.check_trigger(
            inputs.rydy,
            &mut self.fetcher,
            &mut self.cascade,
            &mut self.fine_scroll,
            inputs.pixel_counter,
            pygo,
            regs,
            video,
        );

        // Update NUKO's WX input from the live DFF8 output. Placed
        // unconditionally at the end of mode3_rising so the cache tracks
        // the DFF output even during sprite fetch. On hardware, the
        // DFF8 slave captures on every clock edge regardless of XYMU
        // or sprite fetch state.
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
