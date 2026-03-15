mod fetch_cascade;
mod fetcher;
mod fine_scroll;
mod lcd_control;
mod lcd_shift_register;
mod oam_scan;
mod pixel_output;
mod shifters;
mod sprite_fetch;
mod sprite_scanner;
mod window_control;

pub use fetcher::FetcherStep;
pub use sprite_fetch::SpriteFetchPhase;

use core::fmt;

use crate::game_boy::ppu::{
    PipelineRegisters, VideoControl,
    memory::{Oam, Vram},
    screen::Screen,
};

use fetch_cascade::FetchCascade;
use fetcher::TileFetcher;
use fine_scroll::FineScroll;
use lcd_control::LcdControl;
use shifters::{BgShifter, ObjShifter};
use sprite_fetch::{SpriteFetch, SpriteState};
use sprite_scanner::SpriteScanner;
use window_control::WindowControl;

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

pub struct PipelineSnapshot {
    pub pixel_counter: u8,
    /// XYMU rendering latch (page 21). True = Mode 3 rendering active.
    pub xymu: bool,
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
    pub(super) screen: Screen,
    /// XYMU rendering latch (page 21). SET by AVAP (scan done, Mode 2→3).
    /// CLEAR by WEGO = OR2(VID_RST, VOGA). When set, the fetcher and pixel
    /// pipeline are active (Mode 3). WODU is computed combinationally from
    /// XUGU and !FEPO while XYMU is set.
    pub(super) xymu: bool,
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
    /// VOGA DFF17 — hblank pipeline register (page 21). Clocked on
    /// falling phases (ALET). Captures WODU from the previous rising phase.
    /// Feeds WEGO = OR2(VID_RST, VOGA), which clears both WUSA and
    /// XYMU (rendering latch). Reset by TADY (line reset).
    voga: bool,
    /// TYFA result computed in falling phase, consumed by rising phase. TYFA
    /// is combinational in hardware (falling phase), but downstream SACU is
    /// combinational in the rising phase. This bridge carries the
    /// falling-phase TYFA result to rising-phase SACU.
    tyfa_bridge: bool,
    /// LCD Control block (die page 24): pixel X counter, LCD clock
    /// gating (WUSA), POVA trigger, LCD shift register, data latch.
    lcd: LcdControl,
    /// Sprite fetch lifecycle — Idle or Fetching.
    sprite_state: SpriteState,
}

impl Rendering {
    pub(super) fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            xymu: false,
            scan: SpriteScanner::new(),
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            cascade: FetchCascade::new(),
            fine_scroll: FineScroll::new(),
            window: WindowControl::new(),
            voga: false,
            tyfa_bridge: false,
            lcd: LcdControl::new(),
            sprite_state: SpriteState::Idle,
        }
    }

    /// Set the scan counter's initial entry value. Used for LCD-on
    /// initialization where the counter starts ahead of zero to compensate
    /// for the wasted XUPY tick that scan_started provides on normal lines.
    pub(super) fn set_scan_counter_entry(&mut self, entry: u8) {
        self.scan.set_counter_entry(entry);
    }

    /// WODU: combinational hblank gate. AND3(XYMU, XUGU, !FEPO).
    /// On hardware, WODU is not a latch — it's valid whenever its
    /// inputs are valid. TARU (STAT mode 0) reads WODU directly.
    fn wodu(&self) -> bool {
        let fepo = matches!(self.sprite_state, SpriteState::Fetching(_));
        self.xymu && self.lcd.xugu() && !fepo
    }

    pub(super) fn mode(&self, _video: &VideoControl) -> Mode {
        if self.scan.scanning() {
            Mode::OamScan
        } else if self.xymu && !self.wodu() {
            Mode::Drawing
        } else {
            Mode::HorizontalBlank
        }
    }

    /// Mode as seen by the STAT register. Hardware STAT mode bits are
    /// combinational — no pipeline or latch between the mode signals
    /// and the CPU data bus.
    pub(super) fn stat_mode(&self, video: &VideoControl) -> Mode {
        self.mode(video)
    }

    /// Whether the TAPA_INT_OAM signal is active.
    ///
    /// On hardware, TAPA derives from RUTU_LINE_ENDp — a DFF that pulses
    /// high for dots 0-3 at each line boundary, gated by NOT_VBLANK.
    /// TAPA is independent of ACYL/BESU (the scanning latch that drives
    /// the STAT register mode bits). It fires *before* ACYL activates.
    ///
    /// Line 0 has no RUTU pulse (suppressed by first_line). The mode 2
    /// interrupt on line 0 fires at dot 4 through a separate path.
    pub(super) fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        let ly = video.ly();

        if ly == 0 {
            // Line 0: no TAPA pulse. Mode 2 interrupt fires at LX=1
            // phase 0 (dot 4), when OamScan mode activates.
            video.lx == 1 && video.talu() && !video.wuvu
        } else {
            // Lines 1-143: TAPA pulse during LX=0 (dots 0-3).
            video.lx == 0
        }
    }

    pub(super) fn scanner_oam_address(&self) -> Option<u8> {
        self.scan.oam_address()
    }

    pub fn pipeline_state(&self, video: &VideoControl) -> PipelineSnapshot {
        let (bg_low, bg_high, bg_loaded) = self.bg_shifter.registers();
        let (obj_low, obj_high, obj_palette, obj_priority) = self.obj_shifter.registers();
        let (sprite_fetch_phase, sprite_tile_data) = match &self.sprite_state {
            SpriteState::Fetching(sf) => (Some(sf.phase), Some(sf.tile_data())),
            SpriteState::Idle => (None, None),
        };
        PipelineSnapshot {
            pixel_counter: self.lcd.pixel_counter(),
            xymu: self.xymu,
            bg_low,
            bg_high,
            bg_loaded,
            obj_low,
            obj_high,
            obj_palette,
            obj_priority,
            sprite_fetch_phase,
            sprite_tile_data,
            lcd_x: self.lcd.lcd_x(),
            fetcher_step: self.fetcher.step,
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
        // Hardware: OAM blocked by ACYL (scanning) or XYMU (rendering).
        self.scan.scanning() || self.xymu
    }

    pub(super) fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp.
        self.xymu
    }

    pub(super) fn oam_write_locked(&self) -> bool {
        // Hardware: OAM writes blocked by ACYL (scanning) or XYMU (rendering).
        self.scan.scanning() || self.xymu
    }

    pub(super) fn vram_write_locked(&self) -> bool {
        // Hardware: XYMU gates reads and writes identically via XANE/SERE/SOHY.
        self.xymu
    }

    /// Falling edge (DELTA_EVEN): setup phase.
    ///
    /// On hardware, the falling edge handles XUPY-derived logic (BYBA,
    /// CATU, scan-counter, AVAP mode transitions), fetcher control signals
    /// (NYKA, POKY), mode transitions (VOGA/WEGO clearing XYMU), fine
    /// scroll match (PUXA), and window WX match (PYCO).
    ///
    /// XUPY derives from WUVU, which is clocked by XOTA rising (= our
    /// falling phase). tick_xota() runs before tcycle_falling() in the
    /// executor, so video.xupy() reflects the post-toggle state here.
    pub(super) fn fall(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // XUPY rising edge detection: since tick_xota() toggled WUVU
        // before this function, xupy()==true means WUVU just went low→high.
        let xupy_rising = video.xupy();

        // Sprite scanner falling edge: counter tick, scan-start, BYBA, AVAP.
        let scan = self
            .scan
            .fall(xupy_rising, video.lx, video.wuvu, video.ly(), regs, oam);

        // React to scan signals.
        // AVAP fires identically on normal lines and the LCD-on first line —
        // the scan counter runs to 39 independent of BESU (scanning latch).
        if scan.avap {
            self.xymu = true;
            self.window.init_nuko_wx(regs.window.x_plus_7.output());
        }

        if self.scan.scanning() {
            // Mode 2: fetcher/VOGA/WEGO logic suppressed during scanning.
            return;
        }

        // VOGA DFF17 (DELTA_EVEN, clocked on ALET). Captures WODU
        // combinationally. XYMU is still set at this point (VOGA
        // hasn't cleared it yet), so wodu() is valid to sample.
        let wodu = self.wodu();
        if wodu {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA). Clears both WUSA (LCD clock gate)
        // and XYMU (rendering latch). VID_RST is handled separately in
        // reset_scanline; here we model the VOGA path.
        self.lcd.fall(self.voga, wodu, &mut self.screen);
        if self.voga {
            self.xymu = false;
        }

        // Mode 3 falling-phase processing
        if self.xymu {
            self.mode3_falling(regs, video, oam, vram);
        }
    }

    /// Rising edge (DELTA_ODD): output phase.
    ///
    /// On hardware, the rising edge handles DOBA capture, pixel counter
    /// increment, fine counter increment, pipe shift, and sprite X matching.
    pub(super) fn rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // Sprite scanner rising edge: DOBA captures BYBA.
        self.scan.rise();

        // Mode 3 (drawing) — pixel output phase.
        // Runs when XYMU is set (rendering active).
        if self.xymu {
            self.mode3_rising(regs, video, oam, vram);
        }
    }

    /// Reset per-line state at the scanline boundary. Called by
    /// `Ppu::tcycle_rising` when `advance_dot` signals a new scanline.
    pub(super) fn reset_scanline(&mut self, scanline: u8) {
        self.xymu = false;
        self.scan.reset();
        self.bg_shifter = BgShifter::new();
        self.obj_shifter = ObjShifter::new();
        self.fetcher = TileFetcher::new();
        self.cascade.reset();
        self.fine_scroll = FineScroll::new();
        self.window.reset_scanline();

        self.voga = false;
        self.tyfa_bridge = false;
        self.lcd.reset(scanline);
        self.sprite_state = SpriteState::Idle;
        // BYBA, DOBA, and WUVU are handled by scan.reset() above.
        // WUVU free-runs (no reset) — lives on VideoControl.
    }

    /// Reset for a new frame (VBlank → Active Display transition at LY=0).
    /// Resets the screen buffer and window line counter, then performs the
    /// standard per-scanline reset for line 0. On hardware, the circuits
    /// persist through VBlank — this models the frame-boundary resets that
    /// individual blocks perform, not struct destruction/recreation.
    pub(super) fn reset_frame(&mut self) {
        self.screen = Screen::new();
        self.window.reset_frame();
        self.reset_scanline(0);
    }

    /// Falling edge Mode 3 processing.
    ///
    /// Fetcher advances (phase_tfetch falling half), cascade DFFs (NYKA,
    /// PYGO), NOR latches (POKY), combinational signals (TYFA bridge),
    /// and fine scroll match (PUXA) fire on the falling edge.
    fn mode3_falling(
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

        // BG fetcher advances on the falling phase (LEBO clock), gated
        // only by sprite data fetch. LEBO = NAND2(ALET, MOCE).
        if !sprite_data_fetch {
            self.fetcher.advance(
                self.lcd.pixel_counter(),
                self.window.window_line_counter(),
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

        // Sprite wait exit: when the BG fetcher reaches Idle during
        // sprite wait (WaitingForFetcher) and the shifter is non-empty,
        // transition to sprite data fetch. Co-located with the fetcher
        // advance to preserve 0-delay relative timing.
        //
        // The transition sets the phase to FetchingData. The first
        // sprite fetch advance fires on the next rising phase (sprite
        // data fetch runs on rising in mode3_rising). This is a phase
        // change from the old code where both the exit check and the
        // first sf.advance() ran on the same rising -- now exit is on
        // falling and first advance is on next rising. However, this is
        // actually more correct: on hardware, the sprite fetch clock
        // (VONU/TOBU) is separate from the BG fetcher clock (LEBO).
        // Sprite wait exit uses PYGO (cascade DFF output) instead of
        // POKY (bg_shifter.loaded). Both go high on the same falling phase
        // and remain high for the rest of the scanline.
        if let SpriteState::Fetching(ref mut sf) = self.sprite_state
            && sf.phase == SpriteFetchPhase::WaitingForFetcher
            && self.cascade.lyry_prev()
            && self.cascade.pygo()
        {
            sf.phase = SpriteFetchPhase::FetchingData;
            // The first sprite fetch step fires immediately on the
            // same dot as the wait exit. This preserves the old
            // timing where both the exit check and first sf.advance()
            // ran on the same phase.
            sf.advance(regs, oam, vram);
        }

        self.cascade.fall(lyry);

        // TYFA = AND3(SOCY, POKY, VYBO). Compute in falling, store for rising.
        // SOCY = NOT(RYDY): self.rydy was set in the preceding rising by
        // check_window_trigger and is stable during falling (rising signal,
        // constant during falling phases per GateBoy).
        // VYBO: structurally guaranteed — SpriteState::Idle means no FEPO,
        // and we're in Drawing (no WODU). During sprite fetch, TYFA=0.
        self.tyfa_bridge = match self.sprite_state {
            SpriteState::Idle => !self.window.rydy() && self.cascade.poky(),
            _ => false,
        };

        // POHU: combinational comparator, count == SCX & 7.
        // On hardware, POHU is combinational and ROXO captures into PUXA
        // on the falling edge. The count value is from the preceding rising
        // (reg_old), matching hardware.
        self.fine_scroll
            .compare_falling(regs.background_viewport.x.output());
    }

    /// Rising edge Mode 3 pixel pipeline processing.
    fn mode3_rising(
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
        let inputs = RisingPhaseInputs {
            rydy: self.window.rydy(),
            pixel_counter: self.lcd.pixel_counter(),
        };

        self.cascade.rise();

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
        }

        // PUXA capture: ROXO fires when TYFA is active. TYFA is
        // combinational (AND3(SOCY, POKY, VYBO)), but POKY only updates
        // on the falling edge — PORY just latched above, but PYGO won't
        // capture PORY until the next falling phase. Use tyfa_bridge
        // (computed at the end of the previous falling phase) which has
        // the correct cascade-propagated POKY value.
        let pova = if self.tyfa_bridge {
            self.fine_scroll.capture_rising()
        } else {
            false
        };

        match self.sprite_state {
            SpriteState::Fetching(ref mut sf) => {
                match sf.phase {
                    SpriteFetchPhase::WaitingForFetcher => {
                        // BG fetcher advances on falling (mode3_falling).
                        // Wait exit check is in mode3_falling.
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
                        pixel_output::sprite_overwrite_data_latch(
                            &self.bg_shifter,
                            &self.obj_shifter,
                            self.lcd.data_latch_mut(),
                            self.window.window_zero_pixel_mut(),
                            regs,
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
                // TYFA was computed in falling phase and bridged. SACU is
                // computed here in rising — hardware-correct phase for SACU.
                let tyfa = self.tyfa_bridge;

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
                    // Clear lyry_prev so the next falling phase sees the reset
                    // state — otherwise a sprite triggered at an X%8==0 boundary
                    // would see stale lyry_prev=true and exit wait immediately.
                    self.cascade.clear_lyry();
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
                let toba = self.lcd.rise(sacu, pixel, pova);

                if !toba && self.tyfa_bridge {
                    // Consume window_zero_pixel during pre-visible TYFA
                    // cycles (fine scroll gating, pre-WUSA). On hardware,
                    // the data pins update on every TYFA edge — the window
                    // zero pixel is consumed even when SACU/TOBA don't fire.
                    self.window.consume_window_zero_pixel();
                }

                // Sprite trigger check.
                self.check_sprite_trigger(regs);

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
        let pygo = self.cascade.pygo();
        self.window.check_trigger(
            inputs.rydy,
            &mut self.fetcher,
            &mut self.cascade,
            &self.bg_shifter,
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
    }

    /// Check if a sprite should start fetching at the current pixel position.
    /// Scans all store slots in parallel, matching the hardware's 10
    /// independent X comparators. The lowest-indexed matching slot wins.
    fn check_sprite_trigger(&mut self, regs: &PipelineRegisters) {
        if !regs.control.sprites_enabled() {
            return;
        }

        let match_x = self.lcd.pixel_counter();

        let sprites = self.scan.sprites_mut();
        for i in 0..sprites.count as usize {
            if sprites.fetched & (1 << i) != 0 {
                continue; // Already fetched — reset flag is set
            }

            let entry = &sprites.entries[i];

            if entry.x != match_x {
                continue; // X doesn't match current pixel counter
            }

            if entry.x >= 168 {
                // Off-screen right — mark as fetched so we don't check again
                sprites.fetched |= 1 << i;
                continue;
            }

            // Match found — trigger sprite fetch, mark slot as fetched
            sprites.fetched |= 1 << i;
            self.sprite_state = SpriteState::Fetching(SpriteFetch::new(*entry));
            break; // Only one sprite fetch at a time
        }
    }
}
