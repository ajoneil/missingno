mod fetcher;
mod fine_scroll;
mod frame_phase;
mod oam_scan;
mod pixel_output;
mod shifters;
mod sprite_fetch;
mod window;

pub use fetcher::{FetcherStep, FetcherTick};
pub use frame_phase::FramePhase;
pub use sprite_fetch::SpriteFetchPhase;

use core::fmt;

use crate::game_boy::ppu::{
    PipelineRegisters, VideoControl,
    memory::{Oam, Vram},
    screen::Screen,
};

use fetcher::TileFetcher;
use fine_scroll::FineScroll;
use oam_scan::{OamScanner, SpriteStore};
use shifters::{BgShifter, ObjShifter};
use sprite_fetch::{SpriteFetch, SpriteState};

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

pub(super) const SCANLINE_TOTAL_DOTS: u32 = 456;
/// Bit mask for XUGU NAND5 decode: PX bits 0+1+2+5+7 = 1+2+4+32+128 = 167.
/// WODU = AND2(!FEPO, !XUGU). XUGU is low (WODU fires) when all five bits set.
const XUGU_MASK: u8 = 0b1010_0111; // bits 0,1,2,5,7
/// Dot at which the RUTU line-end signal fires (LX=113 × 4 dots/M-cycle = 452).
/// This clocks the LY register and triggers line-end processing.
pub(super) const RUTU_LINE_END_DOT: u32 = SCANLINE_TOTAL_DOTS - 4;
/// Pixel pipeline rendering phase, modeling the XYMU (rendering latch)
/// and WODU (hblank gate) hardware signals on page 21.
///
/// On hardware, WODU fires combinationally when the pixel counter reaches
/// 167, then VOGA latches WODU on the next even phase to clear XYMU.
/// The STAT mode 0 interrupt condition (TARU) uses WODU directly, so it
/// sees HBlank one phase before XYMU clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPhase {
    /// Not drawing — before Mode 3 starts or after line-end reset.
    /// On line 0, the OAM scan runs with `LineStart` render phase (BESU
    /// is never set on line 0). STAT reads mode 0 for dots 0-3, then
    /// mode 2 from dot 4 onward (matching TCAGBD section 8.11.1).
    LineStart,
    /// Mode 2: BESU set, OAM scanner active. ACYL_SCANNINGp drives
    /// STAT register mode bit 1. Set by CATU_LINE_ENDp at dot 1
    /// for lines 1+, cleared by AVAP when the scan completes.
    /// Line 0 skips this phase (BESU never set on first line).
    OamScan,
    /// Mode 3: XYMU set, fetcher running. Covers the entire rendering
    /// period from AVAP (scan done) through WODU (PX≥167). During
    /// startup, the NYKA/PORY/PYGO cascade DFFs propagate while the
    /// pixel clock waits for POKY (bg_shifter.loaded) to latch.
    Drawing,
    /// WODU fired (XUGU decode + !FEPO): STAT sees mode=0 via TARU,
    /// pixel clock stops, VRAM/OAM unlocked. VOGA captures on next
    /// even phase, clearing XYMU and WUSA via WEGO. Lasts 1 dot.
    DrawingComplete,
    /// Mode 0 (HBlank): XYMU cleared via VOGA latch. Rendering fully stopped.
    /// Hardware: XYMU clear, WODU set.
    HorizontalBlank,
}

/// Within-phase snapshot of signals that are both read and written during
/// `mode3_odd`. On hardware, combinational logic within a phase reads DFF
/// outputs from before the clock edge. This struct captures those values
/// at the top of `mode3_odd` before any sequential mutations within the
/// same phase.
struct OddPhaseInputs {
    /// RYDY value from the previous phase boundary. TYFA, SEKO, and SUZU
    /// all read this (modeling state_old.RYDY) rather than the live value.
    rydy: bool,
    /// Pixel counter value before SACU increment. NUKO (window trigger
    /// comparator) reads pix_count DFF Q-outputs combinationally — the
    /// pre-clock value, before the SACU edge advances the counter.
    pixel_counter: u8,
}

pub struct PipelineSnapshot {
    pub pixel_counter: u8,
    pub render_phase: RenderPhase,
    pub bg_low: u8,
    pub bg_high: u8,
    pub bg_loaded: bool,
    pub obj_low: u8,
    pub obj_high: u8,
    pub obj_palette: u8,
    pub obj_priority: u8,
    pub sprite_fetch_phase: Option<SpriteFetchPhase>,
    pub sprite_tile_data: Option<(u8, u8)>,
    pub lcd_x: u8,
    pub fetcher_step: FetcherStep,
    pub fetcher_tick: FetcherTick,
    pub rydy: bool,
    pub wusa: bool,
    pub pova: bool,
    pub pygo: bool,
    pub wx_triggered: bool,
}

pub struct Rendering {
    pub(super) screen: Screen,
    window_line_counter: u8,
    /// After LCD enable, the first line's Mode 2 doesn't begin at dot 0.
    /// The STAT mode bits read as 0 until Mode 2 actually starts.
    pub(super) lcd_turning_on: bool,
    /// Pixel pipeline phase — models XYMU (rendering latch) and WODU
    /// (hblank gate). See `RenderPhase` for hardware signal mapping.
    pub(super) render_phase: RenderPhase,
    /// Sprites on this line, stored as hardware register file entries.
    sprites: SpriteStore,
    /// OAM scanner — active during Mode 2, consumed when scan completes.
    scanner: Option<OamScanner>,
    /// Whether the window has been rendered on this line.
    window_rendered: bool,
    /// Background pixel shift register (page 32).
    bg_shifter: BgShifter,
    /// Sprite pixel shift register (pages 33-34).
    obj_shifter: ObjShifter,
    /// Background/window tile fetcher.
    fetcher: TileFetcher,
    /// NYKA_FETCH_DONEp_evn: DFF17, latches on ALET (EVEN edge).
    /// Goes high when the first BG tile fetch completes (LYRY fires).
    /// Reset by NAFY (window mode trigger) and at scanline boundaries.
    nyka: bool,
    /// PORY_FETCH_DONEp_odd: DFF17, latches on MYVO (ODD edge).
    /// Captures NYKA one half-phase after NYKA goes high.
    /// Reset by NAFY (window mode trigger) and at scanline boundaries.
    pory: bool,
    /// PYGO_FETCH_DONEp_evn: DFF17, latches on ALET (EVEN edge).
    /// Captures PORY one half-phase after PORY goes high.
    /// Reset at scanline boundaries (XYMU_RENDERINGn).
    pygo: bool,
    /// Fine scroll counter and pixel clock gate (ROXY). Gates the pixel
    /// clock for SCX & 7 dots at the start of each line.
    fine_scroll: FineScroll,
    /// RYDY NOR latch — window hit signal. When high, gates TYFA
    /// (via SOCY_WIN_HITn = not1(TOMU_WIN_HITp)), freezing both the
    /// fine counter (PECU via ROXO) and pixel counter (SACU via SEGU)
    /// during a window fetch stall. SET by the TOMU commit block (from
    /// `rydy_set_pending`), CLEAR by SUZU (window fetch complete).
    ///
    /// The TOMU DFF delay is modeled in two stages: `OddPhaseInputs`
    /// snapshots `self.rydy` at the top of `mode3_odd`, then the TOMU
    /// commit block propagates `rydy_set_pending` into `self.rydy`
    /// AFTER the snapshot. This gives a 2-dot pipeline: SET on dot N,
    /// snapshot sees false on dot N+1, snapshot sees true on dot N+2.
    rydy: bool,
    /// TOMU DFF staging for RYDY SET. Models the 1-DFF delay between the
    /// RYDY NOR latch output and TOMU's Q output that TYFA reads. When
    /// check_window_trigger fires, it writes this staging field instead of
    /// self.rydy directly. The staging field is committed to self.rydy
    /// AFTER the OddPhaseInputs snapshot is taken (but before any other
    /// processing), giving a 2-dot pipeline: SET on dot N, staging=true;
    /// dot N+1 snapshot sees self.rydy=false, then staging commits
    /// self.rydy=true; dot N+2 snapshot sees self.rydy=true.
    ///
    /// Only the SET path uses staging. SUZU's clear of self.rydy writes
    /// directly (1-dot pipeline for clear), matching hardware where the
    /// NOR latch RESET propagates through TOMU on the same edge.
    rydy_set_pending: bool,
    /// Hardware pixel counter (XEHO-SYBE, page 21). Counts from 0 when
    /// the pixel clock starts after startup. Drives WODU (hblank gate)
    /// at PX=167. Not reset on window trigger — PX is a monotonic
    /// per-line counter.
    pixel_counter: u8,
    /// WUSA NOR latch — LCD clock gate (page 24). SET by XAJO
    /// (AND2 of pixel counter bits 0 and 3, first at PX=9). CLEAR
    /// by WEGO (= OR2(VID_RST, VOGA)). Gates TOBA (LCD clock pin).
    wusa: bool,
    /// VOGA DFF17 — hblank pipeline register (page 21). Clocked on
    /// even phases (ALET). Captures WODU from the previous odd phase.
    /// Feeds WEGO = OR2(VID_RST, VOGA), which clears both WUSA and
    /// XYMU (rendering latch). Reset by TADY (line reset).
    voga: bool,
    /// POVA_FINE_MATCH_TRIGp — rising-edge trigger on the fine scroll
    /// match signal. Computed on even phases as AND2(PUXA, !NYZE).
    /// Generates one extra LCD clock pulse via SEMU = OR2(TOBA, POVA),
    /// providing the 160th LCD clock edge before WUSA opens.
    pova: bool,
    /// LCD shift register write position. Driven by SEMU edges:
    /// POVA provides lcd_x=0 (the 160th clock edge, before WUSA opens),
    /// then TOBA provides lcd_x=1–159 (PX=9–167, after WUSA opens).
    /// Replaces the `pixel_counter - 8` approximation with hardware-
    /// accurate SEMU-edge counting.
    lcd_x: u8,
    /// Sprite fetch lifecycle — Idle or Fetching.
    sprite_state: SpriteState,
    /// Window reactivation zero pixel (DMG only). Set when WX re-matches
    /// while the window is active with specific fetcher/FIFO conditions.
    /// Causes the next pixel output to use bg_color=0 without popping
    /// the BG shifter. The OBJ shifter is popped normally.
    window_zero_pixel: bool,
    /// WX comparator suppression latch. Models the hardware behavior where
    /// the RYDY latch prevents the WX comparator (PYCO) from re-firing
    /// after the window has already triggered on this scanline. Cleared
    /// when WX is written mid-scanline, allowing reactivation with a new
    /// WX value.
    wx_triggered: bool,
    /// Last observed WX output value, used to detect mid-scanline WX changes
    /// that should clear the wx_triggered latch.
    last_wx_value: u8,
    /// Cached WX value for the NUKO comparator. On hardware, NUKO reads
    /// the DFF8 slave output, which lags the master by one clock edge.
    /// Updated unconditionally at the end of every mode3_odd from the
    /// live DFF output. check_window_trigger reads this instead of the
    /// live register, providing a 1-dot lag on mid-scanline WX writes.
    nuko_wx: u8,
    /// WUVU_ABxxEFxx: 2-dot toggle DFF, clocked every ODD edge.
    /// Reset to false on LCD-on only; free-runs across scanlines.
    wuvu: bool,
    /// BYBA_SCAN_DONEp_odd: captures scanner-done on XUPY rising edges.
    byba: bool,
    /// DOBA_SCAN_DONEp_evn: captures BYBA on every EVEN edge.
    doba: bool,
}

impl Rendering {
    pub(super) fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            window_line_counter: 0,
            lcd_turning_on: false,
            render_phase: RenderPhase::LineStart,
            sprites: SpriteStore::new(),
            scanner: Some(OamScanner::new()),
            window_rendered: false,
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            nyka: false,
            pory: false,
            pygo: false,
            fine_scroll: FineScroll::new(),
            rydy: false,
            rydy_set_pending: false,
            pixel_counter: 0,
            wusa: false,
            voga: false,
            pova: false,
            lcd_x: 0,
            sprite_state: SpriteState::Idle,
            window_zero_pixel: false,
            wx_triggered: false,
            last_wx_value: 0xFF,
            nuko_wx: 0xFF,
            wuvu: false,
            byba: false,
            doba: false,
        }
    }

    fn new_lcd_on() -> Self {
        Rendering {
            screen: Screen::new(),
            window_line_counter: 0,
            lcd_turning_on: true,
            render_phase: RenderPhase::LineStart,
            sprites: SpriteStore::new(),
            scanner: Some(OamScanner::new()),
            window_rendered: false,
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            nyka: false,
            pory: false,
            pygo: false,
            fine_scroll: FineScroll::new(),
            rydy: false,
            rydy_set_pending: false,
            pixel_counter: 0,
            wusa: false,
            voga: false,
            pova: false,
            lcd_x: 0,
            sprite_state: SpriteState::Idle,
            window_zero_pixel: false,
            wx_triggered: false,
            last_wx_value: 0xFF,
            nuko_wx: 0xFF,
            wuvu: false,
            byba: false,
            doba: false,
        }
    }

    fn mode(&self, video: &VideoControl) -> Mode {
        match self.render_phase {
            RenderPhase::Drawing => Mode::Drawing,
            RenderPhase::OamScan => Mode::OamScan,
            RenderPhase::LineStart if self.scanner.is_some() && video.dot() >= 1 => Mode::OamScan,
            _ => Mode::HorizontalBlank,
        }
    }

    /// Mode as seen by the STAT register (ACYL/XYMU/POPU-derived).
    /// Scanning maps to mode 2 via the BESU/ACYL signal path.
    ///
    /// No look-aheads needed: CPU bus reads/writes execute after
    /// both phases, so AVAP (ODD) and VOGA (EVEN) have already
    /// fired and updated render_phase before stat_mode() is called.
    fn stat_mode(&self, video: &VideoControl) -> Mode {
        match self.render_phase {
            RenderPhase::DrawingComplete | RenderPhase::HorizontalBlank => Mode::HorizontalBlank,
            RenderPhase::Drawing => Mode::Drawing,
            RenderPhase::OamScan => Mode::OamScan,
            RenderPhase::LineStart if self.scanner.is_some() && video.dot() >= 4 => Mode::OamScan,
            RenderPhase::LineStart => Mode::HorizontalBlank,
        }
    }

    /// Whether the mode 2 STAT interrupt condition is active.
    fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        // On hardware, lines 1+ get an early Mode 2 pre-trigger at clock 0
        // from the previous HBlank pre-setting mode_for_interrupt. Line 0
        // has no previous HBlank, so Mode 2 STAT fires at clock 4 instead.
        self.mode(video) == Mode::OamScan && (video.ly() != 0 || video.dot() >= 4)
    }

    pub(super) fn scanner_oam_address(&self) -> Option<u8> {
        self.scanner.as_ref().map(|s| s.oam_address())
    }

    pub fn pipeline_state(&self) -> PipelineSnapshot {
        let (bg_low, bg_high, bg_loaded) = self.bg_shifter.registers();
        let (obj_low, obj_high, obj_palette, obj_priority) = self.obj_shifter.registers();
        let (sprite_fetch_phase, sprite_tile_data) = match &self.sprite_state {
            SpriteState::Fetching(sf) => (Some(sf.phase), Some(sf.tile_data())),
            SpriteState::Idle => (None, None),
        };
        PipelineSnapshot {
            pixel_counter: self.pixel_counter,
            render_phase: self.render_phase,
            bg_low,
            bg_high,
            bg_loaded,
            obj_low,
            obj_high,
            obj_palette,
            obj_priority,
            sprite_fetch_phase,
            sprite_tile_data,
            lcd_x: self.lcd_x,
            fetcher_step: self.fetcher.step,
            fetcher_tick: self.fetcher.tick,
            rydy: self.rydy,
            wusa: self.wusa,
            pova: self.pova,
            pygo: self.pygo,
            wx_triggered: self.wx_triggered,
        }
    }

    fn oam_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::OamScan | RenderPhase::Drawing | RenderPhase::DrawingComplete
        )
    }

    fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp. XYMU stays set through
        // DrawingComplete (WODU dot); VOGA clears it on the next even phase.
        matches!(
            self.render_phase,
            RenderPhase::Drawing | RenderPhase::DrawingComplete
        )
    }

    fn oam_write_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::OamScan | RenderPhase::Drawing | RenderPhase::DrawingComplete
        )
    }

    fn vram_write_locked(&self) -> bool {
        // Hardware: XYMU gates reads and writes identically via XANE/SERE/SOHY.
        matches!(
            self.render_phase,
            RenderPhase::Drawing | RenderPhase::DrawingComplete
        )
    }

    /// DELTA_EVEN half-cycle: setup phase (runs after DELTA_ODD).
    ///
    /// On hardware, DELTA_EVEN handles fetcher control signals (NYKA,
    /// POKY), mode transitions (VOGA/WEGO clearing XYMU), fine scroll
    /// match (PUXA), and window WX match (PYCO).
    pub(super) fn half_even(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // DOBA_SCAN_DONEp_evn: captures BYBA_old on every EVEN edge (ALET clock).
        self.doba = self.byba;

        // CATU_LINE_ENDp fires at phase_lx=2 (dot 1), setting the
        // BESU_SCAN_DONEn NOR latch → RenderPhase::OamScan.
        // BESU is never set on line 0 (hardware special case).
        if video.dot() == 1 && video.ly() != 0 {
            self.render_phase = RenderPhase::OamScan;
        }

        if self.scanner.is_some() {
            // Mode 2: OAM scan uses M-cycle sub-phases, not simple
            // EVEN/ODD. Full scan processing deferred to half_odd
            // for step 1 behavior preservation.
            return;
        }

        // VOGA DFF17 (DELTA_EVEN, clocked on ALET). Captures WODU from
        // this dot's preceding ODD phase. DrawingComplete means WODU
        // fired this dot.
        if self.render_phase == RenderPhase::DrawingComplete {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA). Clears both WUSA (LCD clock gate)
        // and XYMU (rendering latch). VID_RST is handled separately in
        // reset_scanline; here we model the VOGA path.
        if self.voga {
            self.wusa = false;
            self.render_phase = RenderPhase::HorizontalBlank;
        }

        // Mode 3 EVEN-phase processing
        if self.render_phase == RenderPhase::Drawing {
            self.mode3_even(regs, video, oam, vram);
        }
    }

    /// DELTA_ODD half-cycle: output phase.
    ///
    /// On hardware, DELTA_ODD handles pixel counter increment,
    /// fine counter increment, pipe shift, and sprite X matching.
    pub(super) fn half_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // WUVU_ABxxEFxx: toggle DFF, unconditional on every ODD edge.
        self.wuvu = !self.wuvu;
        let xupy_rising = self.wuvu;

        // BYBA_SCAN_DONEp_odd: capture FETO_old on XUPY rising edge.
        // scanner.is_none() means FETO was high from the previous phase.
        let feto_old = self.scanner.is_none();
        if xupy_rising {
            self.byba = feto_old;
        }

        // AVAP: combinational scan-done trigger.
        // Fires for one half-phase when BYBA has captured but DOBA has not.
        let avap = self.byba && !self.doba;

        if let Some(ref mut scanner) = self.scanner {
            // Mode 2: OAM scan — process one entry every 2 dots
            scanner.scan_next_entry(video.ly(), &mut self.sprites, regs, oam);
            if scanner.done() {
                // FETO_SCAN_DONE — scan complete. The scanner is consumed,
                // making feto_old == true for subsequent half_odd() calls.
                // The actual Mode 3 transition happens when AVAP fires
                // (after BYBA captures FETO_old on the next XUPY rising edge).
                self.scanner = None;
            }
        } else if avap {
            // AVAP fires: Mode 2 → Mode 3 transition.
            // Sets XYMU (rendering latch), clears BESU (scan flag), resets
            // the BG fetcher (NYXU). The fetcher begins advancing on the
            // same dot's EVEN phase (mode3_even).
            self.render_phase = RenderPhase::Drawing;
            self.lcd_turning_on = false;
            // Seed NUKO's WX cache from the live DFF8 output at Mode 3
            // entry. The DFF8 slave has been stable since before Mode 3,
            // so the live output is the correct initial value. Without
            // this, the first dot of Mode 3 would compare against 0xFF
            // (from reset), causing a 1-dot-late trigger for WX values
            // that should match on the first dot.
            self.nuko_wx = regs.window.x_plus_7.output();
            // With ODD-before-EVEN ordering, this dot's half_even runs
            // next and sees render_phase == Drawing, so mode3_even
            // advances the fetcher naturally on the AVAP dot. No
            // explicit pre-advance needed.
        } else {
            // Mode 3 (drawing) — pixel output phase
            if self.render_phase == RenderPhase::Drawing {
                self.mode3_odd(regs, video, oam, vram);
            }

            // WODU hblank gate (DELTA_ODD). XUGU = NAND5(PX0,PX1,PX2,PX5,PX7)
            // decodes PX=167 (bits 0+1+2+5+7 all set). WODU = AND2(!FEPO, !XUGU).
            // WODU fires combinationally when the pixel counter has all five
            // XUGU bits set and no sprite match (FEPO) is active.
            //
            // TARU (STAT mode 0 interrupt) uses WODU directly on the same
            // phase — DrawingComplete models this. VOGA latches WODU on the
            // next EVEN phase, clearing XYMU (handled in half_even).
            let xugu = self.pixel_counter & XUGU_MASK == XUGU_MASK;
            let fepo = matches!(self.sprite_state, SpriteState::Fetching(_));
            let wodu = self.render_phase == RenderPhase::Drawing && xugu && !fepo;
            if wodu {
                self.render_phase = RenderPhase::DrawingComplete;
            }
        }
    }

    /// Reset per-line state at the scanline boundary. Called by
    /// `Ppu::tcycle_odd` when `advance_dot` signals a new scanline.
    pub(super) fn reset_scanline(&mut self) {
        self.render_phase = RenderPhase::LineStart;
        if self.window_rendered {
            self.window_line_counter += 1;
        }
        self.sprites = SpriteStore::new();
        self.scanner = Some(OamScanner::new());
        self.window_rendered = false;
        self.bg_shifter = BgShifter::new();
        self.obj_shifter = ObjShifter::new();
        self.fetcher = TileFetcher::new();
        self.nyka = false;
        self.pory = false;
        self.pygo = false;
        self.fine_scroll = FineScroll::new();
        self.rydy = false;
        self.rydy_set_pending = false;
        self.pixel_counter = 0;
        self.wusa = false;
        self.voga = false;
        self.pova = false;
        self.lcd_x = 0;
        self.sprite_state = SpriteState::Idle;
        self.window_zero_pixel = false;
        self.wx_triggered = false;
        self.last_wx_value = 0xFF;
        self.nuko_wx = 0xFF;
        // WUVU free-runs across scanlines (no reset). BYBA and DOBA are
        // cleared by LINE_RST at the scanline boundary.
        self.byba = false;
        self.doba = false;
    }

    /// DELTA_EVEN Mode 3 processing.
    ///
    /// Fetcher advances (phase_tfetch EVEN half), cascade DFFs (NYKA,
    /// PYGO), and fine scroll match (PUXA) fire on DELTA_EVEN.
    fn mode3_even(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        let sprite_data_fetch = matches!(
            self.sprite_state,
            SpriteState::Fetching(SpriteFetch {
                phase: SpriteFetchPhase::FetchingData,
                ..
            })
        );

        // BG fetcher advances on the even phase (LEBO clock), gated
        // only by sprite data fetch. LEBO = NAND2(ALET, MOCE).
        if !sprite_data_fetch {
            self.fetcher.advance(
                self.pixel_counter,
                self.window_line_counter,
                regs,
                video,
                vram,
            );
        }

        // LYRY: combinational gate, high when the fetcher has completed
        // its current tile fetch (step == Idle). Captured here immediately
        // after fetcher.advance() and before TAVE resets the fetcher to
        // GetTile — once TAVE fires, the fetcher is no longer Idle.
        let lyry = self.fetcher.step == FetcherStep::Idle;

        // SUZU: window fetch completion. When the fetcher reaches Idle
        // while RYDY is active, load the window tile into the BG pipe
        // and clear the window hit signal. Co-located with the fetcher
        // advance (both on EVEN) to preserve 0-delay relative timing.
        //
        // Hardware fires SUZU 2 half-phases after fetcher step 5 via
        // the NYKA/PORY cascade. The emulator uses a 0-delay
        // simplification (checking fetcher.step directly instead of
        // routing through the cascade). This is safe because the
        // simplification was already in place when checks lived in
        // mode3_odd -- the only change is which phase both run on.
        //
        // self.rydy reflects the current dot's ODD phase changes
        // (TOMU commit runs in mode3_odd before mode3_even), so the
        // value read here is equivalent to inputs.rydy in the old
        // mode3_odd location.
        if self.rydy && self.fetcher.step == FetcherStep::Idle {
            self.fetcher.load_into(&mut self.bg_shifter);
            self.rydy = false;
        }

        // Sprite wait exit: when the BG fetcher reaches Idle during
        // sprite wait (WaitingForFetcher) and the shifter is non-empty,
        // transition to sprite data fetch. Co-located with the fetcher
        // advance to preserve 0-delay relative timing.
        //
        // The transition sets the phase to FetchingData. The first
        // sprite fetch advance fires on the next ODD phase (sprite
        // data fetch runs on ODD in mode3_odd). This is a phase
        // change from the old code where both the exit check and the
        // first sf.advance() ran on the same ODD -- now exit is on
        // EVEN and first advance is on next ODD. However, this is
        // actually more correct: on hardware, the sprite fetch clock
        // (VONU/TOBU) is separate from the BG fetcher clock (LEBO).
        // Sprite wait exit uses PYGO (cascade DFF output) instead of
        // POKY (bg_shifter.loaded). Both go high on the same EVEN phase
        // and remain high for the rest of the scanline.
        if let SpriteState::Fetching(ref mut sf) = self.sprite_state
            && sf.phase == SpriteFetchPhase::WaitingForFetcher
            && self.fetcher.step == FetcherStep::Idle
            && self.pygo
        {
            sf.phase = SpriteFetchPhase::FetchingData;
            // The first sprite fetch step fires immediately on the
            // same dot as the wait exit. This preserves the old
            // timing where both the exit check and first sf.advance()
            // ran on the same phase.
            sf.advance(regs, oam, vram);
        }

        // --- Cascade DFF propagation (EVEN edge: NYKA and PYGO) ---
        //
        // Hardware chain: LYRY -> NYKA -> PORY -> PYGO -> POKY
        // NYKA and PYGO are clocked on ALET (EVEN rising edge).
        // Each DFF captures its data input from the previous half-phase.
        let old_pory = self.pory;

        // NYKA captures LYRY (fetcher step == Idle). The `lyry` local
        // was captured after fetcher.advance() but before TAVE resets
        // the fetcher, so it directly observes the hardware signal.
        // NYKA latches high and stays high until reset by NAFY (window
        // trigger) or scanline end.
        if lyry && !self.nyka {
            self.nyka = true;
        }

        // PYGO captures old PORY (from the preceding ODD phase).
        // PYGO latches high and stays high until scanline end.
        if old_pory && !self.pygo {
            self.pygo = true;
        }

        // Fine scroll match fires on DELTA_EVEN (PUXA_SCX_FINE_MATCH_evn).
        // Active only after startup (POKY latched). POVA = AND2(PUXA,
        // !NYZE) fires on the rising edge of the match — stored for
        // the next odd phase to compute SEMU = OR2(TOBA, POVA).
        // POVA gate uses PYGO (cascade DFF output) instead of POKY
        // (bg_shifter.loaded). Both go high on the same EVEN phase.
        let roxy_gating = !self.fine_scroll.pixel_clock_active();
        self.pova = if self.pygo {
            self.fine_scroll
                .check_scroll_match(regs.background_viewport.x.output())
        } else {
            false
        };

        // POVA pixel output: the first SEMU rising edge on this line.
        // On hardware, SEMU = OR2(TOBA, POVA). At line start, SEMU=0
        // (TOBA=0 because WUSA not yet open). POVA fires -> SEMU rises
        // -> LCD captures pre-shift pipe MSBs. After this, ROXY=0
        // prevents subsequent POVA re-fires from producing new SEMU
        // edges (TOBA keeps SEMU high at tile boundaries). The
        // roxy_gating check models this: ROXY is still Gating (high)
        // only on the first fire.
        if self.pova && roxy_gating {
            pixel_output::semu_pixel_out(
                &self.bg_shifter,
                &self.obj_shifter,
                &mut self.lcd_x,
                &mut self.window_zero_pixel,
                &mut self.screen,
                regs,
                video,
            );
        }
    }

    /// DELTA_ODD Mode 3 pixel pipeline processing.
    fn mode3_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // Phase-boundary snapshot: capture pre-edge values of signals
        // that are both read and written within this half-phase. All
        // combinational logic (TYFA, SEKO, SUZU, NUKO) reads from
        // `inputs`; all mutations go to `self`.
        let inputs = OddPhaseInputs {
            rydy: self.rydy,
            pixel_counter: self.pixel_counter,
        };

        // TOMU DFF commit: propagate staged RYDY SET into the live field.
        // This runs AFTER the snapshot (so this dot's TYFA sees the old
        // value) but BEFORE everything else. The NEXT dot's snapshot will
        // see the committed value -- giving the 2-dot pipeline:
        //   Dot N: check_window_trigger sets rydy_set_pending = true
        //   Dot N+1: snapshot captures self.rydy = false (old); commit sets self.rydy = true
        //   Dot N+2: snapshot captures self.rydy = true; TYFA sees it
        if self.rydy_set_pending {
            self.rydy = true;
            self.rydy_set_pending = false;
        }

        // PORY captures old NYKA (ODD edge, MYVO clock).
        // Part of the NYKA -> PORY -> PYGO -> POKY startup cascade.
        let old_nyka = self.nyka;
        if old_nyka && !self.pory {
            self.pory = true;
        }

        // TAVE one-shot preload: AND4(rendering, !POKY, NYKA, PORY).
        // Fires on the same ODD phase that PORY goes high, because NYKA
        // was already latched on the preceding EVEN. The !PYGO guard
        // models !POKY -- PYGO hasn't captured yet (it fires on the next
        // EVEN), so !self.pygo is true at TAVE time. Once PYGO fires,
        // !self.pygo permanently disables TAVE, matching hardware where
        // POKY disables SUVU/TAVE.
        if self.nyka && self.pory && !self.pygo {
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // Fine scroll match already processed in mode3_even (DELTA_EVEN).

        match self.sprite_state {
            SpriteState::Fetching(ref mut sf) => {
                match sf.phase {
                    SpriteFetchPhase::WaitingForFetcher => {
                        // BG fetcher advances on EVEN (mode3_even).
                        // Wait exit check is in mode3_even.
                    }
                    SpriteFetchPhase::FetchingData => {
                        // BG fetcher is frozen. Advance the sprite data pipeline.
                        let done = sf.advance(regs, oam, vram);
                        if done {
                            sf.merge_into(&mut self.obj_shifter, oam);
                            sf.phase = SpriteFetchPhase::Done;
                        }
                    }
                    SpriteFetchPhase::Done => {
                        // Data-pin pixel overwrite (sfetch-done dot).
                        //
                        // No SEMU edge fires during sprite fetch (SACU
                        // frozen → TOBA=0), but the data pins (REMY/RAVO)
                        // update combinationally after sprite merge.
                        // Overwrite the last SEMU-written position with
                        // the merged pixel data (data-pin model).
                        pixel_output::sprite_overwrite_pixel_out(
                            &self.bg_shifter,
                            &self.obj_shifter,
                            self.lcd_x,
                            &mut self.window_zero_pixel,
                            &mut self.screen,
                            regs,
                            video,
                        );
                        self.sprite_state = SpriteState::Idle;

                        // Re-evaluate FEPO: check if another sprite matches at
                        // the same pixel_counter. On hardware, FEPO is the OR
                        // of all 10 store comparators — when sfetch_done clears
                        // the fetched sprite's store_x to 0xFF, FEPO immediately
                        // re-evaluates against the still-frozen pix_count. If
                        // another sprite matches, a new fetch begins without any
                        // pixel counter advancement. This chains all same-X
                        // sprite fetches back-to-back.
                        self.check_sprite_trigger(regs);
                    }
                }
            }
            SpriteState::Idle => {
                // TYFA_CLKPIPE (page 21) = AND3(SOCY, POKY, VYBO).
                //   SOCY = NOT(TOMU_WIN_HITp) — old-RYDY inverted
                //   POKY = preload done latch (our `pygo`)
                //   VYBO = NOR3(FEPO_old, WODU_old, MYVO) — sprite match and
                //     hblank gate from previous phase. Both are structurally
                //     guaranteed false here: we're in SpriteState::Idle (no FEPO)
                //     and RenderPhase::Drawing (no WODU).
                //
                // TOMU DFF delay: TYFA reads state_old.RYDY (the pre-edge
                // value captured in `inputs`). Writes to self.rydy by SUZU
                // or check_window_trigger don't affect this dot's TYFA.
                let tyfa = !inputs.rydy && self.pygo;

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
                }

                // Pixel counter increment (SACU clock). On hardware, SACU
                // clocks the counter and pixel output on the same edge. The
                // counter's Q output updates first; pixel output reads the
                // post-increment value. Placed before pixel output to model this.
                if sacu {
                    self.pixel_counter += 1;
                }

                // XAJO: AND2(PX bit 0, PX bit 3). Sets the WUSA NOR latch,
                // opening the LCD clock gate. First fires at PX=9 (0b1001).
                // Subsequent fires (PX=11, 13, 15, 25...) are no-ops since
                // WUSA is already set (NOR latch semantics).
                if !self.wusa && (self.pixel_counter & 0b1001 == 0b1001) {
                    self.wusa = true;
                }

                // TOBA = AND2(WUSA, SACU_CLKPIPE) — the gated LCD clock.
                // On hardware, TOBA clocks the 159-stage LCD shift register,
                // firing from PX=9 through PX=167 (159 clock edges).
                let toba = self.wusa && sacu;

                // SEMU_LCD_CLOCK = OR2(TOBA, POVA). TOBA provides
                // lcd_x=1–159 (PX=9–167). POVA (lcd_x=0) is handled in
                // mode3_even where it outputs pre-shift pipe MSBs gated
                // by the ROXY one-shot.
                if toba {
                    pixel_output::semu_pixel_out(
                        &self.bg_shifter,
                        &self.obj_shifter,
                        &mut self.lcd_x,
                        &mut self.window_zero_pixel,
                        &mut self.screen,
                        regs,
                        video,
                    );
                } else if tyfa {
                    // Consume window_zero_pixel during pre-visible TYFA
                    // cycles (fine scroll gating, pre-WUSA). On hardware,
                    // the data pins update on every TYFA edge — the window
                    // zero pixel is consumed even when SACU/TOBA don't fire.
                    self.window_zero_pixel = false;
                }

                // Sprite trigger check.
                self.check_sprite_trigger(regs);

                // BG fetcher advances on EVEN (mode3_even).
                // SUZU check is in mode3_even.

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
        window::check_window_trigger(
            inputs.rydy,
            &mut self.rydy_set_pending,
            &mut self.fetcher,
            &mut self.nyka,
            &mut self.pory,
            &mut self.bg_shifter,
            &mut self.fine_scroll,
            &mut self.window_zero_pixel,
            &mut self.wx_triggered,
            &mut self.window_rendered,
            inputs.pixel_counter,
            &mut self.last_wx_value,
            self.nuko_wx,
            self.pygo,
            regs,
            video,
        );

        // Update NUKO's WX input from the live DFF8 output. Placed
        // unconditionally at the end of mode3_odd so the cache tracks
        // the DFF output even during sprite fetch. On hardware, the
        // DFF8 slave captures on every clock edge regardless of XYMU
        // or sprite fetch state.
        self.nuko_wx = regs.window.x_plus_7.output();
    }

    /// Check if a sprite should start fetching at the current pixel position.
    /// Scans all store slots in parallel, matching the hardware's 10
    /// independent X comparators. The lowest-indexed matching slot wins.
    fn check_sprite_trigger(&mut self, regs: &PipelineRegisters) {
        if !regs.control.sprites_enabled() {
            return;
        }

        let match_x = self.pixel_counter;

        for i in 0..self.sprites.count as usize {
            if self.sprites.fetched & (1 << i) != 0 {
                continue; // Already fetched — reset flag is set
            }

            let entry = &self.sprites.entries[i];

            if entry.x != match_x {
                continue; // X doesn't match current pixel counter
            }

            if entry.x >= 168 {
                // Off-screen right — mark as fetched so we don't check again
                self.sprites.fetched |= 1 << i;
                continue;
            }

            // Match found — trigger sprite fetch, mark slot as fetched
            self.sprites.fetched |= 1 << i;
            self.sprite_state = SpriteState::Fetching(SpriteFetch::new(*entry));
            break; // Only one sprite fetch at a time
        }
    }
}
