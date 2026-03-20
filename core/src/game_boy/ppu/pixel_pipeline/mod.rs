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
mod sprite_trigger;
mod window_control;

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
use sprite_trigger::SpriteTrigger;
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
    pub phase_tfetch: u8,
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
    /// Latched WODU from the current dot's falling phase. Used by the
    /// TYFA gate (VYBO = NOR3(FEPO_old, WODU_old, MYVO)) to suppress
    /// the pixel clock after PX=167. VOGA captures live WODU directly.
    wodu_latch: bool,
    /// TYFA_CLKPIPE_evn: AND3(SOCY_WIN_HITn, POKY, VYBO_CLKPIPE).
    /// Combinational pixel clock enable, active on falling (EVEN) phases
    /// only. Read by SACU on the next rising phase.
    tyfa: bool,
    /// LCD Control block (die page 24): pixel X counter, LCD clock
    /// gating (WUSA), POVA trigger, LCD shift register, data latch.
    lcd: LcdControl,
    /// Sprite fetch lifecycle — Idle or Fetching.
    sprite_state: SpriteState,
    /// Sprite fetch trigger pipeline: TEKY → SOBU → SUDA → RYCE → TAKA.
    /// See `sprite_trigger.rs` for clock domain and race pair documentation.
    sprite_trigger: SpriteTrigger,
    /// Latched FEPO value for `wodu()` and TYFA computation.
    fepo_latch: bool,
    /// FEPO from the start of the previous rising phase (before pixel_counter
    /// increments). Used to compute TEKY with `_old` input semantics.
    fepo_old: bool,
    /// TEKY: combinational sprite fetch request, computed during rising from
    /// live FEPO/RYDY/LYRY/TAKA. Bridged to the next falling phase where
    /// `sprite_trigger.fall(teky_latch)` captures it into SOBU via TAVA clock.
    teky_latch: bool,
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
            wodu_latch: false,
            tyfa: false,
            lcd: LcdControl::new(),
            sprite_state: SpriteState::Idle,
            sprite_trigger: SpriteTrigger::new(),
            fepo_latch: false,
            fepo_old: false,
            teky_latch: false,
        }
    }

    /// Pre-set scanning active for LCD-on. Models VID_RST deassertion
    /// releasing the scan counter simultaneously with the rest of the
    /// pipeline.
    pub(super) fn start_scanning(&mut self) {
        self.scan.start_scanning();
    }

    /// VOGA latch: true from the dot WODU fires through the rest of HBlank.
    pub(super) fn voga(&self) -> bool {
        self.voga
    }

    /// WODU: combinational hblank gate. AND3(XYMU, XUGU, !FEPO).
    /// On hardware, WODU is not a latch — it's valid whenever its
    /// inputs are valid. TARU (STAT mode 0) reads WODU directly.
    pub(super) fn wodu(&self) -> bool {
        self.xymu && self.lcd.xugu() && !self.fepo_latch
    }

    pub(super) fn mode(&self, _video: &VideoControl) -> Mode {
        if self.scan.besu() {
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
        video.rutu_active()
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
            xymu: self.xymu,
            bg_low,
            bg_high,
            obj_low,
            obj_high,
            obj_palette,
            obj_priority,
            sprite_fetch_phase,
            sprite_tile_data,
            lcd_x: self.lcd.lcd_x(),
            phase_tfetch: self.fetcher.phase_tfetch,
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
        self.scan.besu() || self.xymu
    }

    pub(super) fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp.
        self.xymu
    }

    pub(super) fn oam_write_locked(&self) -> bool {
        // Hardware: OAM writes blocked by ACYL (BESU-driven) or XYMU (rendering).
        self.scan.besu() || self.xymu
    }

    pub(super) fn vram_write_locked(&self) -> bool {
        // Hardware: XYMU gates reads and writes identically via XANE/SERE/SOHY.
        self.xymu
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
    ) {
        // XUPY rising edge detection: the XOTA divider toggle (in
        // Ppu::rise()) ran before this, so xupy()==true means WUVU
        // just went low→high.
        let xupy_rising = video.xupy();

        // Sprite scanner falling edge: counter tick, DOBA capture.
        self.scan.fall(xupy_rising, video.ly(), regs, oam);

        if self.scan.scanning() {
            // Mode 2: fetcher/VOGA/WEGO logic suppressed during scanning.
            return;
        }

        // VOGA DFF17 (DELTA_EVEN, clocked on ALET). Captures WODU_old
        // — the WODU value from the previous half-phase. Since fall()
        // runs after rise(), self.wodu() here reflects the state after
        // the most recent rise() — this IS WODU_old for the falling edge.
        let wodu = self.wodu();

        // wodu_latch maintained for TYFA gating in mode3_falling().
        // Updated AFTER VOGA capture so VOGA sees current wodu (= WODU_old)
        // and mode3_falling sees the value from this dot.
        self.wodu_latch = wodu;

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
    /// On hardware, the rising edge handles BYBA capture, AVAP evaluation,
    /// pixel counter increment, fine counter increment, pipe shift, and
    /// sprite X matching.
    pub(super) fn rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // Sprite scanner rising edge: BYBA captures FETO, AVAP evaluated,
        // CATU scan-start fires.
        let xupy_rising = video.xupy();
        let scan = self.scan.rise(xupy_rising, video.ly());

        // React to scan signals.
        // AVAP fires identically on normal lines and the LCD-on first line —
        // the scan counter runs to 39 independent of BESU (scanning latch).
        if scan.avap {
            self.xymu = true;
            self.window.init_nuko_wx(regs.window.x_plus_7.output());
        }

        // Mode 3 (drawing) — pixel output phase.
        // Runs when XYMU is set (rendering active).
        if self.xymu {
            self.mode3_rising(regs, video, oam, vram);
        }
    }

    /// Reset per-line state at the scanline boundary. Called by
    /// `Ppu::rise()` when `tick_xota` signals a new scanline.
    pub(super) fn reset_scanline(&mut self, scanline: u8) {
        self.xymu = false;
        self.scan.reset();
        self.scan.enable_catu();
        self.bg_shifter = BgShifter::new();
        self.obj_shifter = ObjShifter::new();
        self.fetcher = TileFetcher::new();
        self.cascade.reset();
        self.fine_scroll = FineScroll::new();
        self.window.reset_scanline();

        self.voga = false;
        self.wodu_latch = false;
        self.tyfa = false;
        self.lcd.reset(scanline);
        self.sprite_state = SpriteState::Idle;
        self.sprite_trigger.reset();
        self.fepo_latch = false;
        self.fepo_old = false;
        self.teky_latch = false;
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
        _oam: &Oam,
        vram: &Vram,
    ) {
        // FEPO_old feeds VYBO for TYFA suppression. Captured before any
        // falling-phase mutations.
        let fepo_old = self.fepo(regs);

        // BG fetcher falling-edge advance: VRAM reads + counter increment.
        // LEBO = NAND2(ALET, MOCE) — no dependency on TAKA or TEXY.
        // The BG fetcher counter keeps ticking unconditionally during sprite
        // fetch. On hardware, VRAM bus ownership switches (TEXY gates sprite
        // addresses onto the bus), but the counter itself runs freely.
        self.fetcher.advance_falling(
            self.lcd.pixel_counter(),
            self.window.window_line_counter(),
            regs,
            video,
            vram,
        );

        // LYRY: combinational gate, high when the fetcher has completed
        // its current tile fetch (step == Idle). Captured here immediately
        // after fetcher.advance() and before TAVE resets the fetcher to
        // GetTile — once TAVE fires, the fetcher is no longer Idle.
        let lyry = self.fetcher.lyry();

        self.cascade.fall(lyry);

        // Sprite trigger pipeline: SOBU captures TEKY on TAVA falling
        // edge (depth 7, after ALET at 5). RYCE edge-detects SOBU rising.
        let ryce = self.sprite_trigger.fall(self.teky_latch);

        if ryce {
            // Find and mark the matching sprite entry, start the fetch.
            self.start_sprite_fetch(regs);
        }

        // Latch FEPO_old for wodu() and TYFA computation.
        self.fepo_latch = fepo_old;

        // TYFA = AND3(SOCY, POKY, VYBO). Compute in falling, store for rising.
        // SOCY = NOT(RYDY). VYBO = NOR3(FEPO_old, WODU_old, MYVO).
        // FEPO_old suppresses TYFA during sprite X match (before and during
        // fetch). When wodu_latch is true (WODU fired on previous dot), TYFA
        // must be suppressed to stop the pixel clock at PX=167.
        self.tyfa = !fepo_old && !self.wodu_latch && !self.window.rydy() && self.cascade.poky();

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
        // SUDA DFF: captures SOBU on LAPE rising edge (depth 6).
        self.sprite_trigger.rise();

        // TEKY is an `_odd` signal — it only updates during rising (odd)
        // half-phases. On hardware, TEKY's inputs (FEPO, LYRY, TAKA, RYDY)
        // are combinational and settle within the same phase. GateBoy uses
        // `_old` values because its simulation model can't represent
        // intra-phase settling, but the race pair analysis (13-gate
        // differential, ~65ns vs ~120ns half-cycle) shows the path settles
        // within a half T-cycle. Use live values to match hardware's
        // combinational settling.
        let fepo_now = self.fepo(regs);
        self.teky_latch = fepo_now && !self.window.rydy() && self.fetcher.lyry() && !self.sprite_trigger.taka();
        self.fepo_old = fepo_now;

        // Phase-boundary snapshot: capture pre-edge values of signals
        // that are both read and written within this half-phase. All
        // combinational logic (TYFA, SEKO, SUZU, NUKO) reads from
        // `inputs`; all mutations go to `self`.
        let inputs = RisingPhaseInputs {
            rydy: self.window.rydy(),
            pixel_counter: self.lcd.pixel_counter(),
        };

        // BG fetcher rising-edge advance: counter increment only.
        // LEBO has no TAKA dependency — the BG counter runs freely
        // during sprite fetch, same as the falling advance above.
        self.fetcher.advance_rising();

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
        // capture PORY until the next falling phase. Use tyfa
        // (computed at the end of the previous falling phase) which has
        // the correct cascade-propagated POKY value.
        let pova = if self.tyfa {
            self.fine_scroll.capture_rising()
        } else {
            false
        };

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
                        // Fall through to the pixel pipeline below so
                        // SACU can fire on this same dot. On hardware,
                        // SACU is combinational — when TAKA clears and
                        // TYFA is true, the pixel clock fires immediately.
                        self.sprite_trigger.clear_taka();
                    }
                }
                SpriteState::Idle => {
                    // TAKA set but no sprite fetching yet — RYCE just fired
                    // on the falling phase and start_sprite_fetch set up the
                    // fetch. The first advance will happen on the next rising.
                }
            }
        }

        if !self.sprite_trigger.taka() {
            // Normal pixel pipeline — no sprite fetch active.

            // TYFA was computed in falling phase and bridged. SACU is
            // computed here in rising — hardware-correct phase for SACU.
            let tyfa = self.tyfa;

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
            let toba = self.lcd.rise(sacu, pixel, pova);

            if !toba && self.tyfa {
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

    /// TEKY = AND4(FEPO, !WIN_HIT, LYRY, !TAKA). Combinational signal
    /// that indicates a sprite fetch should start. Checked each falling
    /// phase; SOBU captures the result.
    fn teky(&self, regs: &PipelineRegisters) -> bool {
        self.fepo(regs)
            && !self.window.rydy()   // TUKU_WIN_HITn = NOT(RYDY)
            && self.fetcher.lyry()   // LYRY_BFETCH_DONEp
            && !self.sprite_trigger.taka() // SOWO = NOT(TAKA)
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
