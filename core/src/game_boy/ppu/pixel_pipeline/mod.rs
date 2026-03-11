mod fetcher;
mod fine_scroll;
mod frame_phase;
mod lcd_shift_register;
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
    palette::PaletteIndex,
    screen::Screen,
};

use fetcher::TileFetcher;
use fine_scroll::FineScroll;
use lcd_shift_register::LcdShiftRegister;
use oam_scan::{OamScanner, SpriteStore};
use pixel_output::PixelOutputSnapshot;
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
/// Total dots for line 0 after LCD enable. The hardware's WUVU/VENA phase
/// offset on initial enable shortens the first scanline by 2 dots.
pub(super) const FIRST_LINE_TOTAL_DOTS: u32 = 454;
/// Bit mask for XUGU NAND5 decode: PX bits 0+1+2+5+7 = 1+2+4+32+128 = 167.
/// WODU = AND2(!FEPO, !XUGU). XUGU is low (WODU fires) when all five bits set.
const XUGU_MASK: u8 = 0b1010_0111; // bits 0,1,2,5,7
/// Dot at which the RUTU line-end signal fires (LX=113 × 4 dots/M-cycle = 452).
/// This clocks the LY register and triggers line-end processing.
pub(super) const RUTU_LINE_END_DOT: u32 = SCANLINE_TOTAL_DOTS - 4;
/// RUTU dot for the first line after LCD enable. Same as normal lines —
/// RUTU fires at dot 452 regardless of line length. The post-RUTU period
/// is 2 dots (vs normal 4) because FIRST_LINE_TOTAL_DOTS is 454.
pub(super) const FIRST_LINE_RUTU_DOT: u32 = RUTU_LINE_END_DOT;
/// Pixel pipeline rendering phase, modeling the XYMU (rendering latch)
/// and WODU (hblank gate) hardware signals on page 21.
///
/// On hardware, WODU fires combinationally when the pixel counter reaches
/// 167, then VOGA latches WODU on the next falling phase to clear XYMU.
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
    /// for lines 1+ (in half_falling), cleared by AVAP when the scan
    /// completes. Line 0 skips this phase (BESU never set on first
    /// line; scanner runs under LineStart instead).
    OamScan,
    /// Mode 3: XYMU set, fetcher running. Covers the entire rendering
    /// period from AVAP (scan done) through WODU (PX≥167). During
    /// startup, the NYKA/PORY/PYGO cascade DFFs propagate while the
    /// pixel clock waits for POKY (bg_shifter.loaded) to latch.
    Drawing,
    /// WODU fired (XUGU decode + !FEPO): STAT sees mode=0 via TARU,
    /// pixel clock stops, VRAM/OAM unlocked. VOGA captures on next
    /// falling phase, clearing XYMU and WUSA via WEGO. Lasts 1 dot.
    DrawingComplete,
    /// Mode 0 (HBlank): XYMU cleared via VOGA latch. Rendering fully stopped.
    /// Hardware: XYMU clear, WODU set.
    HorizontalBlank,
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
    pub poky: bool,
    pub wx_triggered: bool,
    pub wuvu: bool,
    pub byba: bool,
    pub doba: bool,
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
    /// OAM scanner (YFEL-FONY counter). Always present — on hardware
    /// the counter is never destroyed, just reset at line boundaries.
    scanner: OamScanner,
    /// BESU scanning latch. Set when OAM scan starts (dot 1 for lines 1+,
    /// dot 0 for line 0), cleared by AVAP when scan completes. Gates
    /// ACYL_SCANNINGp which drives STAT mode bits and OAM bus ownership.
    scanning: bool,
    /// Whether the window has been rendered on this line.
    window_rendered: bool,
    /// Background pixel shift register (page 32).
    bg_shifter: BgShifter,
    /// Sprite pixel shift register (pages 33-34).
    obj_shifter: ObjShifter,
    /// Background/window tile fetcher.
    fetcher: TileFetcher,
    /// LYRY previous-phase latch. Models `reg_old.LYRY` (= `reg_old.phase_tfetch >= 10`)
    /// for the NYKA DFF17 input. On hardware, NYKA reads the previous falling
    /// phase's LYRY value, not the current one. This 1-phase delay adds 1 dot
    /// to pipeline priming, matching the hardware cascade timing.
    lyry_prev: bool,
    /// NYKA_FETCH_DONEp_evn: DFF17, latches on ALET (falling edge).
    /// Goes high when the first BG tile fetch completes (LYRY fires).
    /// Reset by NAFY (window mode trigger) and at scanline boundaries.
    nyka: bool,
    /// PORY_FETCH_DONEp_odd: DFF17, latches on MYVO (rising edge).
    /// Captures NYKA one half-phase after NYKA goes high.
    /// Reset by NAFY (window mode trigger) and at scanline boundaries.
    pory: bool,
    /// PYGO_FETCH_DONEp_evn: DFF17, latches on ALET (falling edge).
    /// Captures PORY one half-phase after PORY goes high.
    /// Reset at scanline boundaries (XYMU_RENDERINGn).
    pygo: bool,
    /// POKY NOR latch — captures PYGO on falling edge. TYFA reads this
    /// instead of PYGO directly, adding 1 dot of cascade delay to the
    /// pixel clock enable. Reset at scanline boundaries.
    poky: bool,
    /// Fine scroll counter and pixel clock gate (ROXY). Gates the pixel
    /// clock for SCX & 7 dots at the start of each line.
    fine_scroll: FineScroll,
    /// RYDY NOR latch — window hit signal. When high, gates TYFA
    /// (via SOCY_WIN_HITn = not1(TOMU_WIN_HITp)), freezing both the
    /// fine counter (PECU via ROXO) and pixel counter (SACU via SEGU)
    /// during a window fetch stall. SET directly by check_window_trigger,
    /// CLEAR by PORY (NYKA/PORY cascade after fetcher completes).
    ///
    /// 1-dot delay: check_window_trigger sets self.rydy at the end of
    /// mode3_rising, AFTER the RisingPhaseInputs snapshot. The snapshot on
    /// the NEXT dot sees rydy=true, giving 1-dot NUKO-to-TYFA latency.
    rydy: bool,
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
    /// falling phases (ALET). Captures WODU from the previous rising phase.
    /// Feeds WEGO = OR2(VID_RST, VOGA), which clears both WUSA and
    /// XYMU (rendering latch). Reset by TADY (line reset).
    voga: bool,
    /// POVA_FINE_MATCH_TRIGp — rising-edge trigger on the fine scroll
    /// match signal. Computed on rising phases as AND2(PUXA, !NYZE).
    /// Generates one extra LCD clock pulse via SEMU = OR2(TOBA, POVA),
    /// providing the 160th LCD clock edge before WUSA opens.
    pova: bool,
    /// TYFA result computed in falling phase, consumed by rising phase. TYFA
    /// is combinational in hardware (falling phase), but downstream SACU is
    /// combinational in the rising phase. This bridge carries the
    /// falling-phase TYFA result to rising-phase SACU.
    tyfa_bridge: bool,
    /// LCD shift register — 159-stage pixel buffer between the pixel
    /// mux and the Screen. Replaces direct framebuffer writes.
    lcd_shift_register: LcdShiftRegister,
    /// LCD data pin latch (REMY/RAVO qp_ext_old model). On hardware,
    /// the LCD data pins are combinational from the pipe MSBs, but the
    /// LCD captures `qp_ext_old()` — the previous half-cycle's value.
    /// This buffer holds the resolved pixel from the previous SACU edge.
    /// TOBA shifts this buffered value into the LCD shift register,
    /// giving a 1-dot lag: TOBA at PX=N outputs PX=(N-1)'s pixel.
    lcd_data_latch: PaletteIndex,
    /// Pixel output register snapshot — captured at the end of mode3_rising.
    /// Models the TOBA qp_ext_old mechanism: pixel output on dot N+1 uses
    /// palette/LCDC values snapshotted at the end of dot N's rising phase,
    /// making CPU writes between rising and falling invisible to the current
    /// dot's pixel output.
    pixel_snapshot: PixelOutputSnapshot,
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
    /// Updated unconditionally at the end of every mode3_rising from the
    /// live DFF output. check_window_trigger reads this instead of the
    /// live register, providing a 1-dot lag on mid-scanline WX writes.
    nuko_wx: u8,
    /// WUVU_ABxxEFxx: 2-dot toggle DFF, clocked every rising edge.
    /// Reset to false on LCD-on only; free-runs across scanlines.
    wuvu: bool,
    /// BYBA_SCAN_DONEp_odd: captures scanner-done on XUPY rising edges.
    byba: bool,
    /// DOBA_SCAN_DONEp_evn: captures BYBA on every falling edge.
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
            scanner: OamScanner::new(),
            scanning: true,
            window_rendered: false,
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            lyry_prev: false,
            nyka: false,
            pory: false,
            pygo: false,
            poky: false,
            fine_scroll: FineScroll::new(),
            rydy: false,

            pixel_counter: 0,
            wusa: false,
            voga: false,
            pova: false,
            tyfa_bridge: false,
            lcd_shift_register: LcdShiftRegister::new(),
            lcd_data_latch: PaletteIndex(0),
            pixel_snapshot: PixelOutputSnapshot {
                bgp: 0xFC,
                obp0: 0xFF,
                obp1: 0xFF,
                bg_window_enabled: true,
                sprites_enabled: false,
            },
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
            scanner: OamScanner::new(),
            scanning: false,
            window_rendered: false,
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            lyry_prev: false,
            nyka: false,
            pory: false,
            pygo: false,
            poky: false,
            fine_scroll: FineScroll::new(),
            rydy: false,

            pixel_counter: 0,
            wusa: false,
            voga: false,
            pova: false,
            tyfa_bridge: false,
            lcd_shift_register: LcdShiftRegister::new(),
            lcd_data_latch: PaletteIndex(0),
            pixel_snapshot: PixelOutputSnapshot {
                bgp: 0xFC,
                obp0: 0xFF,
                obp1: 0xFF,
                bg_window_enabled: true,
                sprites_enabled: false,
            },
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
            RenderPhase::LineStart if self.scanning && video.dot() >= 1 => Mode::OamScan,
            _ => Mode::HorizontalBlank,
        }
    }

    /// Mode as seen by the STAT register (ACYL/XYMU/POPU-derived).
    /// Scanning maps to mode 2 via the BESU/ACYL signal path.
    ///
    /// No look-aheads needed: CPU bus reads/writes execute after
    /// both phases, so AVAP (rising) and VOGA (falling) have already
    /// fired and updated render_phase before stat_mode() is called.
    fn stat_mode(&self, video: &VideoControl) -> Mode {
        match self.render_phase {
            RenderPhase::DrawingComplete | RenderPhase::HorizontalBlank => Mode::HorizontalBlank,
            RenderPhase::Drawing => Mode::Drawing,
            RenderPhase::OamScan => Mode::OamScan,
            RenderPhase::LineStart if self.scanning && video.dot() >= 4 => Mode::OamScan,
            RenderPhase::LineStart => Mode::HorizontalBlank,
        }
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
    fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        let ly = video.ly();
        let dot = video.dot();

        if ly == 0 {
            // Line 0: no TAPA pulse. Mode 2 interrupt at dot 4 when
            // OamScan mode activates (LineStart -> scanning && dot >= 4).
            dot == 4
        } else {
            // Lines 1-143: TAPA pulse for dots 0-3.
            dot <= 3
        }
    }

    pub(super) fn scanner_oam_address(&self) -> Option<u8> {
        if self.scanning {
            Some(self.scanner.oam_address())
        } else {
            None
        }
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
            lcd_x: self.lcd_shift_register.count(),
            fetcher_step: self.fetcher.step,
            fetcher_tick: self.fetcher.tick,
            rydy: self.rydy,
            wusa: self.wusa,
            pova: self.pova,
            pygo: self.pygo,
            poky: self.poky,
            wx_triggered: self.wx_triggered,
            wuvu: self.wuvu,
            byba: self.byba,
            doba: self.doba,
        }
    }

    fn oam_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::LineStart
                | RenderPhase::OamScan
                | RenderPhase::Drawing
                | RenderPhase::DrawingComplete
        )
    }

    fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp. XYMU stays set through
        // DrawingComplete (WODU dot); VOGA clears it on the next falling phase.
        matches!(
            self.render_phase,
            RenderPhase::Drawing | RenderPhase::DrawingComplete
        )
    }

    fn oam_write_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::LineStart
                | RenderPhase::OamScan
                | RenderPhase::Drawing
                | RenderPhase::DrawingComplete
        )
    }

    fn vram_write_locked(&self) -> bool {
        // Hardware: XYMU gates reads and writes identically via XANE/SERE/SOHY.
        matches!(
            self.render_phase,
            RenderPhase::Drawing | RenderPhase::DrawingComplete
        )
    }

    /// Falling edge half-cycle: setup phase (runs after rising edge).
    ///
    /// On hardware, the falling edge handles fetcher control signals (NYKA,
    /// POKY), mode transitions (VOGA/WEGO clearing XYMU), fine scroll
    /// match (PUXA), and window WX match (PYCO).
    pub(super) fn half_falling(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // DOBA_SCAN_DONEp_evn: captures BYBA_old on every falling edge (ALET clock).
        self.doba = self.byba;

        if self.scanning {
            // Mode 2: OAM scan uses M-cycle sub-phases, not simple
            // falling/rising. Full scan processing deferred to half_rising
            // for step 1 behavior preservation.
            return;
        }

        // VOGA DFF17 (DELTA_EVEN, clocked on ALET). Captures WODU from
        // this dot's preceding rising phase. DrawingComplete means WODU
        // fired this dot.
        if self.render_phase == RenderPhase::DrawingComplete {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA). Clears both WUSA (LCD clock gate)
        // and XYMU (rendering latch). VID_RST is handled separately in
        // reset_scanline; here we model the VOGA path.
        if self.voga {
            self.wusa = false;
            if self.render_phase == RenderPhase::DrawingComplete {
                // LCD NOR latch provides the 160th pixel: the data pins'
                // current value at latch time. This is the lcd_data_latch
                // (last resolved pixel from the final SACU edge, PX=167).
                self.lcd_shift_register.shift_in(self.lcd_data_latch);
                // LCD_LATCH (PIN_55): transfer shift register to column drivers.
                self.lcd_shift_register.latch_to_screen(&mut self.screen);
            }
            self.render_phase = RenderPhase::HorizontalBlank;
        }

        // Mode 3 falling-phase processing
        if self.render_phase == RenderPhase::Drawing {
            self.mode3_falling(regs, video, oam, vram);
        }
    }

    /// Rising edge half-cycle: output phase.
    ///
    /// On hardware, the rising edge handles pixel counter increment,
    /// fine counter increment, pipe shift, and sprite X matching.
    pub(super) fn half_rising(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        // CATU_LINE_ENDp: at dot 1 for lines 1+, CATU fires (phase_lx=2,
        // LINE_RSTn released), setting BESU (scan latch) and resetting
        // the scan counter. Line 0 already has scanning=true from reset_scanline.
        if video.dot() == 1 && video.ly() != 0 && !self.scanning {
            self.render_phase = RenderPhase::OamScan;
            self.scanning = true;
            self.scanner.reset();
        }

        // WUVU_ABxxEFxx: toggle DFF, unconditional on every rising edge.
        self.wuvu = !self.wuvu;
        let xupy_rising = self.wuvu;

        // FETO_SCAN_DONE: combinational AND4 of scan counter bits 0,1,2,5.
        // Fires when counter reaches 39 (0b100111), before entry 39's
        // comparison completes. On hardware this is a wire, not a latch.
        let feto = self.scanner.scan_done();

        // BYBA_SCAN_DONEp_odd: capture FETO on XUPY rising edge.
        if xupy_rising {
            self.byba = feto;
        }

        // AVAP: combinational scan-done trigger.
        // Fires for one half-phase when BYBA has captured but DOBA has not.
        let avap = self.byba && !self.doba;

        // OAM scan: GAVA and COTA fire on the same sub-phase (A/E of
        // XUPY). At dot granularity, one tick compares the current
        // entry and increments the counter. Gated on XUPY rising
        // (2-dot period). FETO only freezes the counter, not the
        // comparison — entry 39 is still compared.
        if self.scanning && xupy_rising {
            self.scanner.tick(video.ly(), &mut self.sprites, regs, oam);
        }

        if avap && self.scanning && !self.lcd_turning_on {
            // AVAP fires: Mode 2 → Mode 3 transition.
            // Clears BESU (scan flag), sets XYMU (rendering latch),
            // resets the BG fetcher (NYXU). The fetcher begins
            // advancing on the same dot's falling phase (mode3_falling).
            self.scanning = false;
            self.render_phase = RenderPhase::Drawing;
            self.lcd_turning_on = false;
            // Seed NUKO's WX cache from the live DFF8 output at Mode 3
            // entry. The DFF8 slave has been stable since before Mode 3,
            // so the live output is the correct initial value. Without
            // this, the first dot of Mode 3 would compare against 0xFF
            // (from reset), causing a 1-dot-late trigger for WX values
            // that should match on the first dot.
            self.nuko_wx = regs.window.x_plus_7.output();
            // With rising-before-falling ordering, this dot's half_falling runs
            // next and sees render_phase == Drawing, so mode3_falling
            // advances the fetcher naturally on the AVAP dot. No
            // explicit pre-advance needed.
        } else if self.lcd_turning_on && video.dot() == 80 {
            // LCD turn-on: Mode 0 → Mode 3 transition. Hardware transitions
            // directly to Mode 3, skipping the OAM scan. Mode 3 starts at
            // approximately dot 80, the same as normal scanlines. The video
            // clock divider (WUVU/VENA) comes out of LCD-enable reset at a
            // misaligned phase, adding ~8 dots of delay beyond the naive
            // 18 NOP × 4 = 72 calculation from Mooneye lcdon_timing-GS.
            self.render_phase = RenderPhase::Drawing;
            self.lcd_turning_on = false;
            self.nuko_wx = regs.window.x_plus_7.output();
        }

        // Mode 3 (drawing) — pixel output phase.
        // Runs when in Drawing phase and not during a mode transition dot.
        if self.render_phase == RenderPhase::Drawing {
            self.mode3_rising(regs, video, oam, vram);
        }

        // WODU hblank gate (DELTA_ODD). XUGU = NAND5(PX0,PX1,PX2,PX5,PX7)
        // decodes PX=167 (bits 0+1+2+5+7 all set). WODU = AND2(!FEPO, !XUGU).
        // WODU fires combinationally when the pixel counter has all five
        // XUGU bits set and no sprite match (FEPO) is active.
        //
        // TARU (STAT mode 0 interrupt) uses WODU directly on the same
        // phase — DrawingComplete models this. VOGA latches WODU on the
        // next falling phase, clearing XYMU (handled in half_falling).
        let xugu = self.pixel_counter & XUGU_MASK == XUGU_MASK;
        let fepo = matches!(self.sprite_state, SpriteState::Fetching(_));
        let wodu = self.render_phase == RenderPhase::Drawing && xugu && !fepo;
        if wodu {
            self.render_phase = RenderPhase::DrawingComplete;
        }
    }

    /// Reset per-line state at the scanline boundary. Called by
    /// `Ppu::tcycle_rising` when `advance_dot` signals a new scanline.
    pub(super) fn reset_scanline(&mut self, scanline: u8) {
        self.render_phase = RenderPhase::LineStart;
        if self.window_rendered {
            self.window_line_counter += 1;
        }
        self.sprites = SpriteStore::new();
        self.scanner.reset();
        if scanline == 0 {
            // Line 0: BESU set at dot 0 (boot ROM / post-boot init
            // sets the LCD on mid-line, so the full scan runs from dot 0).
            self.scanning = true;
        } else {
            // Lines 1+: BESU deferred to dot 1 (CATU_LINE_ENDp fires
            // at phase_lx=2, releasing LINE_RSTn and setting BESU).
            self.scanning = false;
        }
        self.window_rendered = false;
        self.bg_shifter = BgShifter::new();
        self.obj_shifter = ObjShifter::new();
        self.fetcher = TileFetcher::new();
        self.lyry_prev = false;
        self.nyka = false;
        self.pory = false;
        self.pygo = false;
        self.poky = false;
        self.fine_scroll = FineScroll::new();
        self.rydy = false;

        self.pixel_counter = 0;
        self.wusa = false;
        self.voga = false;
        self.pova = false;
        self.tyfa_bridge = false;
        self.lcd_shift_register.reset(scanline);
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
            && self.lyry_prev
            && self.pygo
        {
            sf.phase = SpriteFetchPhase::FetchingData;
            // The first sprite fetch step fires immediately on the
            // same dot as the wait exit. This preserves the old
            // timing where both the exit check and first sf.advance()
            // ran on the same phase.
            sf.advance(regs, oam, vram);
        }

        // --- Cascade DFF propagation (falling edge: NYKA) ---
        //
        // Hardware chain: LYRY -> NYKA -> PORY -> PYGO -> POKY
        // NYKA is a DFF17 clocked on ALET (falling edge of master clock).
        // DFF17 reads state_old -- the PREVIOUS falling phase's LYRY value.

        // NYKA captures lyry_prev (the previous phase's LYRY), not the
        // current lyry. This models the DFF17 state_old read.
        if self.lyry_prev && !self.nyka {
            self.nyka = true;
        }

        // Update lyry_prev for next falling phase.
        self.lyry_prev = lyry;

        // PYGO captures PORY on falling edge (ALET clock). On hardware,
        // PYGO is DFF17 clocked by ALET_xBxDxFxH. PORY was latched
        // on the preceding rising (MYVO clock), so PYGO reads state_old.PORY.
        if self.pory && !self.pygo {
            self.pygo = true;
        }

        // POKY NOR latch fires on falling, reading the just-updated PYGO.
        // Zero propagation delay from PYGO to POKY within this falling
        // phase, matching hardware NOR latch behavior.
        if self.pygo && !self.poky {
            self.poky = true;
        }

        // TYFA = AND3(SOCY, POKY, VYBO). Compute in falling, store for rising.
        // SOCY = NOT(RYDY): self.rydy was set in the preceding rising by
        // check_window_trigger and is stable during falling (rising signal,
        // constant during falling phases per GateBoy).
        // VYBO: structurally guaranteed — SpriteState::Idle means no FEPO,
        // and we're in Drawing (no WODU). During sprite fetch, TYFA=0.
        self.tyfa_bridge = match self.sprite_state {
            SpriteState::Idle => !self.rydy && self.poky,
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
            rydy: self.rydy,
            pixel_counter: self.pixel_counter,
        };

        // PORY captures old NYKA (rising edge, MYVO clock).
        // Part of the NYKA -> PORY -> PYGO -> POKY startup cascade.
        let old_nyka = self.nyka;
        if old_nyka && !self.pory {
            self.pory = true;
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
        if self.pory && self.rydy {
            self.rydy = false;

            // SUZU → TEVO → NYXU: load window tile data into pipe.
            self.fetcher.load_into(&mut self.bg_shifter);

            // TEVO → PASO: reset fine counter.
            self.fine_scroll.reset_counter();

            // REMY/RAVO combinational update: data pins reflect the
            // newly loaded window tile data immediately.
            self.lcd_data_latch = pixel_output::resolve_current_pixel(
                &self.bg_shifter,
                &self.obj_shifter,
                &mut self.window_zero_pixel,
                &self.pixel_snapshot,
            );
        }

        // TAVE one-shot preload: AND4(rendering, !POKY, NYKA, PORY).
        // Fires on the same rising phase that PORY goes high, because NYKA
        // was already latched on the preceding falling edge. The !PYGO guard
        // models !POKY -- PYGO is captured below (after TAVE), so
        // !self.pygo is still true at TAVE time. Once PYGO fires,
        // !self.pygo permanently disables TAVE, matching hardware where
        // POKY disables SUVU/TAVE.
        if self.nyka && self.pory && !self.pygo {
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // Forward-compute TYFA from cascade DFF state.
        // PORY was just latched above. PYGO and POKY propagate on falling,
        // but we can forward-compute their would-be values: if PORY is now
        // true, PYGO would go true on the next falling, and POKY would follow.
        let forward_pygo = self.pygo || self.pory;
        let forward_poky = self.poky || forward_pygo;
        let forward_tyfa =
            !self.rydy && forward_poky && matches!(self.sprite_state, SpriteState::Idle);

        // PUXA capture: ROXO fires when TYFA is active. Forward-compute
        // TYFA so the capture sees the same-dot cascade result.
        self.pova = if forward_tyfa {
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
                            &mut self.lcd_data_latch,
                            &mut self.window_zero_pixel,
                            &self.pixel_snapshot,
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
                        self.check_sprite_trigger();
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
                    self.lyry_prev = false;
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
                //
                // RYDY suppresses TOBA indirectly via TYFA→SEGU→SACU:
                // RYDY→SOCY→TYFA=0→SACU stuck. No explicit RYDY gate
                // exists on TOBA in hardware.
                let toba = self.wusa && sacu;

                // LCD data pin lag model (REMY/RAVO qp_ext_old).
                //
                // On hardware, the LCD data pins (REMY/RAVO) are combinational
                // from the pipe MSBs, but the LCD captures qp_ext_old — the
                // previous half-cycle's pin state. TOBA shifts the BUFFERED
                // pixel (from the previous SACU edge) into the LCD register,
                // then the latch updates to the current pipe state.
                //
                // This gives a 1-dot offset: TOBA at PX=9 outputs PX=8's
                // pixel, TOBA at PX=10 outputs PX=9's pixel, etc. Total:
                // 159 TOBA edges output pixels for PX=8–166. The 160th pixel
                // (PX=167) is captured by the NOR latch at end-of-line.
                if toba {
                    self.lcd_shift_register.shift_in(self.lcd_data_latch);
                }

                // Update the LCD data latch with the current pipe state.
                // On hardware, REMY/RAVO are combinational from pipe MSBs
                // — they update every phase, not just on TYFA or SACU
                // edges. During the RYDY stall, pipe content changes when
                // window tile data loads (SEKO/TEVO), and the data pins
                // reflect this immediately. The first post-stall TOBA
                // captures window data via qp_ext_old.
                self.lcd_data_latch = pixel_output::resolve_current_pixel(
                    &self.bg_shifter,
                    &self.obj_shifter,
                    &mut self.window_zero_pixel,
                    &self.pixel_snapshot,
                );

                if !toba && self.tyfa_bridge {
                    // Consume window_zero_pixel during pre-visible TYFA
                    // cycles (fine scroll gating, pre-WUSA). On hardware,
                    // the data pins update on every TYFA edge — the window
                    // zero pixel is consumed even when SACU/TOBA don't fire.
                    self.window_zero_pixel = false;
                }

                // Sprite trigger check.
                self.check_sprite_trigger();

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
        window::check_window_trigger(
            inputs.rydy,
            &mut self.rydy,
            &mut self.fetcher,
            &mut self.nyka,
            &mut self.pory,
            &mut self.lyry_prev,
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
        // unconditionally at the end of mode3_rising so the cache tracks
        // the DFF output even during sprite fetch. On hardware, the
        // DFF8 slave captures on every clock edge regardless of XYMU
        // or sprite fetch state.
        self.nuko_wx = regs.window.x_plus_7.output();

        // Capture pixel output snapshot for the NEXT dot's mode3_rising.
        // Models TOBA qp_ext_old: pixel output reads the previous rising
        // phase's register values, not the current ones. DriveBus writes
        // that land after this capture (between rising and falling) will
        // only be visible two dots later.
        self.pixel_snapshot = PixelOutputSnapshot::capture(regs);
    }

    /// Check if a sprite should start fetching at the current pixel position.
    /// Scans all store slots in parallel, matching the hardware's 10
    /// independent X comparators. The lowest-indexed matching slot wins.
    fn check_sprite_trigger(&mut self) {
        if !self.pixel_snapshot.sprites_enabled {
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
