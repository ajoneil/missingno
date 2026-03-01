mod fetcher;
mod fine_scroll;
mod frame_phase;
mod oam_scan;
mod pixel_output;
mod shifters;
mod sprite_fetch;
mod window;

pub use frame_phase::FramePhase;
pub use sprite_fetch::SpriteFetchPhase;

use core::fmt;

use crate::game_boy::ppu::{
    PipelineRegisters, VideoControl,
    memory::{Oam, Vram},
    screen::Screen,
};

use fetcher::{FetcherStep, StartupFetch, TileFetcher};
use fine_scroll::{FineScroll, WindowHit};
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
/// Hardware pixel counter value at which WODU fires (hblank gate).
/// XUGU = NAND5(PX0, PX1, PX2, PX5, PX7) decodes 128+32+4+2+1 = 167.
const WODU_PIXEL_COUNT: u8 = 167;
/// First pixel counter value that produces a visible LCD pixel.
/// On hardware, the LCD X coordinate is `pix_count - 8`. Pixels at
/// PX 0–7 shift the first tile's data through the pipe invisibly.
const FIRST_VISIBLE_PIXEL: u8 = 8;
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
    /// startup, the `StartupFetch` cascade gates the pixel clock until
    /// the first tile fetch completes and POKY latches.
    Drawing,
    /// WODU fired (PX≥167, no sprite match): STAT sees mode=0 via TARU,
    /// pixel clock stops, VRAM/OAM unlocked. XYMU clears next dot.
    /// Hardware: XYMU set, WODU set. Lasts 1 dot.
    DrawingComplete,
    /// Mode 0 (HBlank): XYMU cleared via VOGA latch. Rendering fully stopped.
    /// Hardware: XYMU clear, WODU set.
    HorizontalBlank,
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
    /// Tracks the two startup tile fetches at the beginning of mode 3.
    /// Hardware performs one BG tile fetch (6 dots) before any
    /// pixels shift out. `None` once startup is complete.
    startup_fetch: Option<StartupFetch>,
    /// Fine scroll counter and pixel clock gate (ROXY). Gates the pixel
    /// clock for SCX & 7 dots at the start of each line.
    fine_scroll: FineScroll,
    /// RYDY NOR latch — window hit signal. Gates TYFA, freezing both
    /// the fine counter (PECU via ROXO) and pixel counter (SACU via SEGU)
    /// during a window fetch stall.
    window_hit: WindowHit,
    /// Hardware pixel counter (XEHO-SYBE, page 21). Counts from 0 when
    /// the pixel clock starts after startup. Drives WODU (hblank gate)
    /// at PX=167. Not reset on window trigger — PX is a monotonic
    /// per-line counter.
    pixel_counter: u8,
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
            startup_fetch: Some(StartupFetch::FirstTile),
            fine_scroll: FineScroll::new(),
            window_hit: WindowHit::Inactive,
            pixel_counter: 0,
            sprite_state: SpriteState::Idle,
            window_zero_pixel: false,
            wx_triggered: false,
            last_wx_value: 0xFF,
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
            startup_fetch: Some(StartupFetch::FirstTile),
            fine_scroll: FineScroll::new(),
            window_hit: WindowHit::Inactive,
            pixel_counter: 0,
            sprite_state: SpriteState::Idle,
            window_zero_pixel: false,
            wx_triggered: false,
            last_wx_value: 0xFF,
        }
    }

    fn mode(&self) -> Mode {
        match self.render_phase {
            RenderPhase::Drawing => Mode::Drawing,
            RenderPhase::OamScan => Mode::OamScan,
            _ if self.scanner.is_some() => Mode::OamScan,
            _ => Mode::HorizontalBlank,
        }
    }

    /// Mode as seen by the STAT register (ACYL/XYMU/POPU-derived).
    /// Scanning maps to mode 2 via the BESU/ACYL signal path.
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
        self.mode() == Mode::OamScan && (video.ly() != 0 || video.dot() >= 4)
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
        }
    }

    fn oam_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::OamScan | RenderPhase::Drawing
        )
    }

    fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp, cleared when WODU fires.
        matches!(self.render_phase, RenderPhase::Drawing)
    }

    fn oam_write_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::OamScan | RenderPhase::Drawing
        )
    }

    fn vram_write_locked(&self) -> bool {
        // Hardware: XYMU gates reads and writes identically via XANE/SERE/SOHY.
        matches!(self.render_phase, RenderPhase::Drawing)
    }

    /// DELTA_EVEN half-cycle: setup phase.
    ///
    /// On hardware, DELTA_EVEN handles fetcher control signals (NYKA,
    /// POKY), mode transitions (VOGA/WEGO clearing XYMU), fine scroll
    /// match (PUXA), and window WX match (PYCO).
    pub(super) fn half_even(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) {
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

        // VOGA latch (DELTA_EVEN). On hardware, VOGA captures WODU on the
        // even phase following the odd phase when WODU fired. This cascades
        // through WEGO to clear XYMU (rendering).
        if self.render_phase == RenderPhase::DrawingComplete {
            self.render_phase = RenderPhase::HorizontalBlank;
        }

        // Mode 3 EVEN-phase processing
        if self.render_phase == RenderPhase::Drawing {
            self.mode3_even(regs, video, vram);
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
        if let Some(ref mut scanner) = self.scanner {
            // Mode 2: OAM scan — process one entry every 2 dots
            scanner.scan_next_entry(video.ly(), &mut self.sprites, regs, oam);
            if scanner.done() {
                // FETO_SCAN_DONE — scan complete, begin Mode 2→3 transition.
                self.scanner = None;
                self.lcd_turning_on = false;
                // AVAP: scan complete, rendering active. StartupFetch
                // gates pixel output until the LYRY→NYKA→PORY→POKY
                // cascade completes. Fetcher's first advance comes from
                // mode3_even on the next DELTA_EVEN (LEBO clock is
                // EVEN-only on hardware).
                self.render_phase = RenderPhase::Drawing;
            }
        } else {
            // Mode 3 (drawing) — pixel output phase
            if self.render_phase == RenderPhase::Drawing {
                self.mode3_odd(regs, video, oam, vram);
            }

            // WODU hblank gate (DELTA_ODD). On hardware, WODU fires
            // combinationally on the ODD phase when pix_count reaches
            // 167 and no sprite match is active. TARU (STAT mode 0
            // interrupt condition) uses WODU directly on the same
            // phase. VOGA latches WODU on the next EVEN phase,
            // clearing XYMU (handled in half_even).
            if self.render_phase == RenderPhase::Drawing
                && self.pixel_counter >= WODU_PIXEL_COUNT
                && !matches!(self.sprite_state, SpriteState::Fetching(_))
            {
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
        self.startup_fetch = Some(StartupFetch::FirstTile);
        self.fine_scroll = FineScroll::new();
        self.window_hit = WindowHit::Inactive;
        self.pixel_counter = 0;
        self.sprite_state = SpriteState::Idle;
        self.window_zero_pixel = false;
        self.wx_triggered = false;
        self.last_wx_value = 0xFF;
    }

    /// DELTA_EVEN Mode 3 processing.
    ///
    /// Fetcher advances (phase_tfetch EVEN half), cascade DFFs (NYKA,
    /// POKY), fine scroll match (PUXA), and window WX match (PYCO)
    /// all fire on DELTA_EVEN.
    fn mode3_even(&mut self, regs: &PipelineRegisters, video: &VideoControl, vram: &Vram) {
        // Startup cascade DFF captures on DELTA_EVEN. Each arm reads
        // state set by the *previous* DELTA_EVEN or DELTA_ODD — the
        // DFF capture delay is explicit in the state machine.
        match self.startup_fetch {
            Some(StartupFetch::LyryFired) => {
                // NYKA captures LYRY on this DELTA_EVEN.
                self.startup_fetch = Some(StartupFetch::NykaFired);
            }
            Some(StartupFetch::PoryFired) => {
                // POKY latches from PORY — enables pixel clock.
                self.startup_fetch = None;
            }
            _ => {}
        }

        // During startup, the fetcher advances on DELTA_EVEN (LEBO clock).
        // After startup, the fetcher advances only on DELTA_ODD (line 952).
        // The BG fetcher is frozen during sprite data fetch (FetchingData).
        let sprite_data_fetch = matches!(
            self.sprite_state,
            SpriteState::Fetching(SpriteFetch {
                phase: SpriteFetchPhase::FetchingData,
                ..
            })
        );
        if !sprite_data_fetch && self.startup_fetch.is_some() {
            self.fetcher.advance(
                self.pixel_counter,
                self.window_line_counter,
                regs,
                video,
                vram,
            );
        }

        // TAVE preload: when the startup fetch first reaches Idle
        // (GetTileDataHigh complete), load the pipe immediately. This is
        // the one-shot preload trigger (TAVE on hardware) — it fires once
        // during startup and never again after POKY latches.
        if self.startup_fetch == Some(StartupFetch::FirstTile)
            && self.fetcher.step == FetcherStep::Idle
            && self.bg_shifter.is_empty()
        {
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // LYRY fires combinationally when the first tile fetch fills
        // the BG shifter. This is a combinational signal, not a DFF —
        // it fires in the same DELTA_EVEN as the advance that fills
        // the shifter.
        if self.startup_fetch == Some(StartupFetch::FirstTile) && !self.bg_shifter.is_empty() {
            self.startup_fetch = Some(StartupFetch::LyryFired);
        }

        // Fine scroll match fires on DELTA_EVEN (PUXA_SCX_FINE_MATCH_evn).
        // No fine scroll processing during startup fetch.
        if self.startup_fetch.is_none() {
            self.fine_scroll
                .check_scroll_match(regs.background_viewport.x.output());
        }

        // Window WX match fires on DELTA_EVEN (PYCO_WIN_MATCHp).
        // Active during both startup fetch and normal rendering.
        window::check_window_trigger(
            &mut self.window_hit,
            &mut self.fetcher,
            &mut self.startup_fetch,
            &mut self.bg_shifter,
            &mut self.obj_shifter,
            &mut self.fine_scroll,
            &mut self.window_zero_pixel,
            &mut self.wx_triggered,
            &mut self.window_rendered,
            self.pixel_counter,
            &mut self.last_wx_value,
            regs,
            video,
        );
    }

    /// DELTA_ODD Mode 3 pixel pipeline processing.
    fn mode3_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // PORY captures NYKA on DELTA_ODD. This is the only cascade
        // transition that fires on the odd phase — it must happen
        // regardless of whether pixel processing runs this tick.
        if self.startup_fetch == Some(StartupFetch::NykaFired) {
            self.startup_fetch = Some(StartupFetch::PoryFired);
        }

        // Fine scroll match already processed in mode3_even (DELTA_EVEN).

        match self.sprite_state {
            SpriteState::Fetching(ref mut sf) => {
                match sf.phase {
                    SpriteFetchPhase::WaitingForFetcher => {
                        // The BG fetcher continues advancing during the wait.
                        // This is the hardware behavior: the fetcher keeps
                        // stepping through its enum states, doing real tile
                        // fetches that may load pixels into the shifter.
                        self.fetcher.advance(
                            self.pixel_counter,
                            self.window_line_counter,
                            regs,
                            video,
                            vram,
                        );

                        // Wait exits when BOTH conditions are met:
                        // 1. The fetcher has completed GetTileDataHigh (reached Idle)
                        // 2. The BG shifter is non-empty
                        // This is an AND condition — both must be true simultaneously.
                        let fetcher_past_data = self.fetcher.step == FetcherStep::Idle;
                        let wait_done = fetcher_past_data && !self.bg_shifter.is_empty();

                        if wait_done {
                            // Freeze the BG fetcher at its current position.
                            // It stays wherever the wait left it (typically Load)
                            // and resumes from there after the sprite data fetch.

                            // Transition to sprite data fetch. The first sprite
                            // fetch step happens on the same dot as the wait
                            // exit — the transition itself does not consume a dot.
                            let sf = match self.sprite_state {
                                SpriteState::Fetching(ref mut sf) => sf,
                                _ => unreachable!(),
                            };
                            sf.phase = SpriteFetchPhase::FetchingData;
                            sf.advance(regs, oam, vram);
                        }
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
                        // sfetch-done dot: pixel clock is still frozen, but on
                        // hardware pixel output is unconditional — it reads the
                        // pipe MSBs (now containing the newly-merged sprite data)
                        // and writes to the same screen position as the trigger
                        // dot, overwriting it. The pipes do NOT shift (FEPO
                        // blocks clkpipe_gate).
                        pixel_output::peek_pixel_out(
                            &self.bg_shifter,
                            &self.obj_shifter,
                            &self.fine_scroll,
                            self.pixel_counter,
                            &mut self.screen,
                            regs,
                            video,
                        );
                        self.sprite_state = SpriteState::Idle;
                    }
                }
            }
            SpriteState::Idle => {
                // Clearing → Inactive: on the tick after SUZU fires, the pixel
                // clock gate sees RYDY=0 and resumes normal operation.
                if self.window_hit == WindowHit::Clearing {
                    self.window_hit = WindowHit::Inactive;
                }

                // Hardware within-tick ordering for DFF22 shift register cells:
                // 1. Synchronous shift (SACU clock edge)
                // 2. Async parallel load (LOZE SET/RST — overwrites shift)
                // 3. Pixel output reads final state
                //
                // SACU clocks both the pipe shift registers and the pixel
                // counter. SACU = or2(SEGU, ROXY) — frozen when either
                // SEGU=1 (window hit active) or ROXY=1 (fine scroll not
                // done). Gate matches the pixel counter increment below.
                if self.window_hit == WindowHit::Inactive
                    && self.fine_scroll.pixel_clock_active()
                    && !self.bg_shifter.is_empty()
                {
                    self.bg_shifter.shift();
                    self.obj_shifter.shift();
                }

                // SUZU/MOSU: when the window fetch completes (fetcher reaches Idle
                // while RYDY is active), load the first window tile and clear the
                // window hit signal. This is the hardware's dedicated window tile
                // load path — independent of fine_count.
                if self.window_hit == WindowHit::Active && self.fetcher.step == FetcherStep::Idle {
                    self.fetcher.load_into(&mut self.bg_shifter);
                    self.window_hit = WindowHit::Clearing;
                }

                // RYFA DFF captures (count==7 && !nuko_wx_match) on each dot.
                // SEKO is the rising-edge detector on RYFA — it fires one dot
                // after count reaches 7. Reading count HERE (before tick)
                // naturally models this one-dot DFF delay. PANY gates RYFA
                // on !NUKO_WX_MATCHp (modeled by window_hit == Inactive).
                let seko_fire =
                    self.fine_scroll.count == 7 && self.window_hit == WindowHit::Inactive;

                // SEKO → TEVO → NYXU: pipe reload (async). LOZE SET/RST
                // overwrites the shift result on the same tick — the load
                // naturally wins because the shift already fired above
                // (matching DFF22 behavior).
                if seko_fire {
                    self.fetcher.load_into(&mut self.bg_shifter);
                }

                // Pixel counter increment. On hardware, SACU (pixel clock) is
                // gated by TYFA (window hit via SEGU), ROXY (fine scroll), and
                // FIFO readiness (shifter non-empty).
                if self.window_hit == WindowHit::Inactive
                    && self.fine_scroll.pixel_clock_active()
                    && !self.bg_shifter.is_empty()
                {
                    self.pixel_counter += 1;
                }

                // Pixel output. On hardware, the pixel clock uses
                // state_old.FEPO — on the trigger dot, state_old.FEPO=0,
                // so PX increments, the pipe shifts, and a pixel outputs.
                // FEPO only becomes 1 in state_new, freezing clocks on
                // subsequent dots. Running pixel output before the sprite
                // trigger check models this registered-signal timing.
                if self.window_hit == WindowHit::Inactive && !self.bg_shifter.is_empty() {
                    pixel_output::shift_pixel_out(
                        &self.bg_shifter,
                        &self.obj_shifter,
                        &self.fine_scroll,
                        self.pixel_counter,
                        &mut self.window_zero_pixel,
                        &mut self.screen,
                        regs,
                        video,
                    );
                }

                // Sprite trigger check.
                self.check_sprite_trigger(regs);

                if self.startup_fetch.is_none() {
                    self.fetcher.advance(
                        self.pixel_counter,
                        self.window_line_counter,
                        regs,
                        video,
                        vram,
                    );
                }

                // PECU (fine counter clock) derives from ROXO, which derives from
                // TYFA. TYFA is gated by RYDY (window hit).
                if self.startup_fetch.is_none() && self.window_hit == WindowHit::Inactive {
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
