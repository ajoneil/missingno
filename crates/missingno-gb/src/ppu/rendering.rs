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
    /// window line counter.
    window: WindowControl,
    /// TYFA (pixel clock enable): `!FEPO && !WODU && !RYDY && POKY`,
    /// snapshotted at end of `mode3_rising` and consumed in
    /// `mode3_pixel_pipeline`. RYDY is sampled BEFORE the rise's
    /// `WindowControl::tick_rising` so a same-dot RYDY↑ doesn't reach the
    /// snapshot — modelling the SYLO/TOMU/SOCY → TYFA/SEGU/SACU gate
    /// chain delay. The in-flight pre-window SACU fires on the MOSU↑ dot
    /// on both the normal cascade (MOSU↑ on fall) and the deferred
    /// cascade (MOSU↑ on rise).
    tyfa: bool,
    /// Pixel X position counter (PX). Advances on SACU; feeds WODU
    /// via `terminal()` for the Mode 3→0 transition.
    pixel_counter: PixelCounter,
    /// LCD Control block: LCD clock gating (WUSA), POVA trigger, and
    /// pixel push to the LCD glass.
    lcd: LcdControl,
    /// Sprite fetch lifecycle — Idle or Fetching.
    sprite_state: SpriteState,
    /// Sprite fetch trigger pipeline: TEKY → SOBU → SUDA → RYCE → TAKA.
    /// See `sprite_trigger.rs` for clock domain and race pair documentation.
    sprite_trigger: SpriteTrigger,
    /// PANY drain-detector slip carry-over. Set when NUKO=1 lands during
    /// the dot where SEKO would fire (fine_scroll.count == 7), splitting
    /// PANY's high pulse across the SEGU capture edge — RYFA captures
    /// the second half, slipping the SEKO → TEVO → NYXU/wx_clk cascade
    /// by 1 dot. The slipped fire happens on the next dot; both the BG
    /// shifter parallel-load and the window-tile-X counter increment
    /// land 1 dot late.
    pany_slip_pending: bool,
    /// MOSU fired on the prior PPU rise (deferred-completion path —
    /// LCDC.5 restore drops XOFO while NUNU=1 from the prior fall).
    /// NYXU's reset pulse holds the BG fetch counter at 0 across the
    /// following fall — consumed in `on_ppu_clock_fall` to gate out
    /// `mode3_advance_fetcher` on the post-MOSU dot.
    mosu_fired_rising: bool,
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
            mosu_fired_rising: false,
        }
    }

    /// Boot-ROM-handoff rendering state: hblank pipeline carries VOGA
    /// latched from prior Mode 3; scan counter at terminal 39 (frozen)
    /// with BYBA/DOBA latched and `catu_enabled` released; BG fetch
    /// counter at terminal 5 (frozen); pixel counter at terminal 167
    /// residual; LCD push counter at the prior line's 160-pixel terminal
    /// value with scanline=143. All other blocks share `Rendering::new()`'s
    /// power-on defaults.
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
            mosu_fired_rising: false,
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

    /// WODU: combinational hblank gate. AND2(XUGU, !FEPO).
    /// On hardware, WODU is purely combinational — it does not
    /// depend on XYMU. During HBlank, WODU stays high (PX frozen
    /// at 167, FEPO=0), which is correct for CLKPIPE freeze and
    /// STAT mode readback.
    pub(super) fn wodu(&self, sprites_enabled: bool) -> bool {
        HblankPipeline::wodu(self.pixel_counter.terminal(), self.fepo(sprites_enabled))
    }

    /// Whether this is the LCD-enable first line (no prior scanline boundary).
    fn is_first_line(&self) -> bool {
        !self.scan.catu_enabled()
    }

    /// Whether the TAPA_INT_OAM signal is active.
    ///
    /// On hardware, TAPA = AND(TOLU_VBLANKn, SELA), where SELA derives from
    /// RUTU_LINE_ENDp — a 4-dot pulse (one TALU cycle) at each scanline
    /// boundary. POPU gating at the call site handles the VBlank delay
    /// on normal line 0.
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

    pub(super) fn scan_besu(&self) -> bool {
        self.scan.besu()
    }

    pub(super) fn lcd_pushing_active(&self) -> bool {
        self.lcd.wusa()
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
            wx_triggered: self.window.wx_triggered(regs),
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
        // WYJA write-enable: AJUJ = NOR3(dma_run, mode2, mode3). The
        // §10.5.6 AJUJ pulse is an explicit write-permit override active
        // for the 2,100 ps window during the AVAP cascade.
        !self.hblank.ajuj_pulse() && (self.scan.besu() || self.hblank.rendering_active())
    }

    pub(super) fn vram_write_locked(&self) -> bool {
        // SERE/XEDU tri-state enables gate VRAM writes on mode3 only.
        // §10.5.6 AJUJ pulse overrides during the AVAP-cascade window.
        !self.hblank.ajuj_pulse() && self.hblank.rendering_active()
    }

    /// Master-clock rising edge (= ALET rising): setup phase dispatcher.
    ///
    /// ALET-clocked DFFs capture here: NYKA, PYGO (cascade), VOGA
    /// (hblank). Also handles XUPY-derived logic (DOBA, scan-counter),
    /// NOR latches (POKY), combinational signals (TYFA bridge), fine
    /// scroll match (PUXA), and window WX match (PYCO).
    ///
    /// XUPY derives from WUVU, which is clocked by XOTA rising. The
    /// XOTA divider toggle runs in the preceding master-clock-falling
    /// phase, so video.xupy() reflects the post-toggle state here.
    pub(super) fn on_ppu_clock_rise(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &Vram,
    ) -> Option<PixelOutput> {
        // Scanner advance (BYBA capture, AVAP detection + reaction)
        // moved to on_ppu_clock_fall — BYBA is XUPY-clocked and
        // captures on alet falling, opposite edge to alet-clocked
        // DFFs (NYKA, DOBA).

        // §10.5.6 AJUJ pulse close: the write-permit window opened on
        // the prior AVAP-fall closes at this rise.
        self.hblank.tick_ajuj_pulse_on_rise();

        if self.scan.scanning() {
            // Mode 2: fetcher/VOGA/WEGO logic suppressed during scanning.
            return None;
        }

        // Capture XYMU before tick_voga_on_rise() may clear it via VOGA/WEGO.
        let was_rendering = self.hblank.rendering_active();

        // Hblank pipeline: WODU was sampled on the prior fall (after
        // PixelCounter::advance); VOGA.q captures here on ALET rising
        // and WEGO clears XYMU combinationally. `wodu` reports whether
        // VOGA just committed from pending — used by the LCD to push
        // screen_x=159 once per scanline.
        let wodu = self.hblank.tick_voga_on_rise();

        // WODU rise push emits screen_x=159 — captures the post-fall-shift
        // shifter MSB at the WODU dot (the bg shifter has already shifted,
        // gated by the NYXU-pulse-hold path).
        let post_shift_pixel =
            pixel_output::resolve_current_pixel(&self.bg_shifter, &self.obj_shifter, regs);
        let pixel = self
            .lcd
            .on_ppu_clock_rise(self.hblank.voga(), wodu, post_shift_pixel);

        if was_rendering {
            self.mode3_rising(regs, video, oam, oam_bus, vram);
        }

        // XYMU↑ (Mode 3 exit): mode3 = NOT(XYMU.q) falls, async-resetting
        // the BG fetch cascade DFFs (NYKA/PORY via nafy outside-Mode-3
        // steady-state; PYGO via r_n=mode3; POKY via NOR-latch R from
        // LOBY=NOT(mode3)) and the fine-scroll counter (RYKU/ROGA/RUBU
        // via PASO=NOR2(PAHA, TEVO) where PAHA=NOT(mode3)=1; ROXY held
        // at 1/Gating by PAHA).
        if was_rendering && !self.hblank.rendering_active() {
            self.cascade.reset();
            self.fine_scroll = FineScroll::new();
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

    /// Master-clock falling edge (= ALET falling): output phase
    /// dispatcher.
    ///
    /// MYVO-clocked DFFs capture here: PORY (cascade). BG fetch counter
    /// advances (LEBO fires on this edge — LEBO = NAND(ALET, MOCE)).
    /// CLKPIPE fires (SACU rising edge: SACU rising = ALET falling,
    /// delayed by the VYBO/TYFA/SEGU chain). Handles BYBA
    /// capture, AVAP evaluation, pixel counter increment, fine counter
    /// increment, pipe shift, sprite X matching, and pixel output. CATU
    /// pipeline is advanced separately by `tick_catu()` (unconditional).
    pub(super) fn on_ppu_clock_fall(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        xupy_rising: bool,
    ) -> Option<PixelOutput> {
        // Snapshot xymu BEFORE the AVAP reaction can set it. This gates
        // the fetcher advance so the first LAXU toggle occurs on the
        // NEXT rise — matching hardware's 1-dot AVAP→LAXU delay. The
        // natural rise→rise gap plays the role previously filled by
        // the nyxu_reset_active hold.
        let was_rendering = self.hblank.rendering_active();

        // Scanner advance: counter tick + BYBA capture + AVAP detection +
        // reaction all co-locate on this XUPY-rising fall. BYBA is
        // XUPY-clocked and captures on alet falling (= this edge);
        // AVAP propagates combinationally from BYBA/DOBA, and XYMU set,
        // fetcher preload, and window WX init fire here so Mode 3 init
        // aligns with hardware's AVAP-rising edge.
        let scan = self
            .scan
            .advance_scan(xupy_rising, video.ly(), regs, oam, oam_bus);
        if scan.avap {
            // Mode 3 begins on AVAP-fall per spec §7.1. The §10.5.6 AJUJ
            // permit pulse is asserted alongside mode3↑ — it represents
            // the 2,100 ps write-permit window between BESU.q↓ and the
            // buffered mode3 net↑, consumed by oam_write_locked /
            // vram_write_locked at the mid-CUPA sample.
            self.hblank.pulse_ajuj_on_avap_fall();
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

            // MOSU↑ arming runs before mode3_advance_fetcher so the
            // counter=0 falling VRAM read sees fetching_window=true and
            // AMUV/VEVY select the window tilemap. When MOSU↑ fires,
            // mode3_advance_fetcher is gated out for this dot — mirrors
            // AVAP's `was_rendering` gate. NYXU's reset hold keeps the
            // fetcher counter at 0 through the next ALET edge so the
            // first falling-edge VRAM read fires at counter=0 (window
            // tile-index) before the counter=2/4 data reads use it.
            let poky_for_window = self.cascade.poky();
            let taka_for_window = self.sprite_trigger.taka();
            let mosu_fired = self.window.tick_falling(
                &mut self.fetcher,
                &mut self.cascade,
                &mut self.fine_scroll,
                pixel_counter_before_sacu,
                poky_for_window,
                taka_for_window,
                regs,
            );

            // SUZU is one of TEVO's OR3 inputs (alongside SEKO and TAVE) and
            // drives NYXU low — holding the BG shifter via LOZE on its dot.
            // Surface whether SUZU fired in mode3_advance_fetcher so the
            // BG-shift NYXU gate in mode3_pixel_pipeline can include it.
            // `mosu_fired_rising` carries a deferred-completion MOSU from
            // the prior rise — NYXU's reset hold straddles this fall, so
            // mode3_advance_fetcher is gated out on this dot too.
            let mosu_fired_rising = self.mosu_fired_rising;
            self.mosu_fired_rising = false;
            let suzu_fired = if was_rendering && !mosu_fired && !mosu_fired_rising {
                self.mode3_advance_fetcher()
            } else {
                false
            };
            // MOSU is also a direct NYXU input. When MOSU fires,
            // mode3_advance_fetcher is skipped, but the MOSU pulse itself
            // holds the BG shifter on this dot.
            let advance_nyxu_pulse = mosu_fired || mosu_fired_rising || suzu_fired;
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
        self.pany_slip_pending = false;
        self.pixel_counter.reset();
        self.lcd.reset(scanline);
        self.sprite_state = SpriteState::Idle;
        // ATEJ arm of SECA = NOR3(RYCE, ROSY, ATEJ): the line-end
        // pulse re-asserts TAKA at every scanline boundary. SOBU/SUDA
        // are free-running DFFs (no reset). TAKA's next clear is by
        // VEKU's TAVE arm at AVAP+5.998 in the startup pipeline.
        self.sprite_trigger.set_taka();
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

    /// Mode 3 processing on the master-clock rising edge (= ALET rising).
    ///
    /// Fetcher VRAM reads (counter doesn't increment here — LEBO fires
    /// on the falling edge only), cascade DFFs (NYKA, PYGO), NOR latches
    /// (POKY), combinational signals (TYFA), sprite fetch counter
    /// advance (SABE), and fine scroll match (PUXA) fire on this edge.
    fn mode3_rising(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &Vram,
    ) {
        // Pre-CUPA FEPO drives the TEKY → SOBU gate-delay race: SOBU's
        // alet-rising DFF capture wins by ~14 ns over CUPA's transparent-
        // latch propagation through XYLO, so SOBU sees the pre-write
        // LCDC.1 value.
        let mut fepo_pre_cupa = self.fepo(regs.sprites_enabled_pre_cupa);

        // LYRY: combinational on fetch_counter (>= 5). The counter only
        // increments on rising (LEBO clock), so the value here reflects
        // the preceding rise — matching hardware's NYKA capturing
        // reg_old.LYRY on ALET (falling edge).
        let lyry = self.fetcher.lyry();

        // BG fetcher counter=0/2/4 VRAM reads. Counter increments on the
        // companion fall via mode3_advance_fetcher; this rise drives
        // the per-counter VRAM activity, reading fetching_window from
        // the prior fall's PYNU via wx_triggered.
        //
        // BG fetcher and cascade DFFs continue to advance during sprite
        // fetch — only SACU/CLKPIPE freezes (gated separately via TYFA
        // suppression). The counter saturates at 5 (LYRY=1, MOCE=0
        // freezes LEBO) which produces the observable "tfetch=5
        // throughout sprite fetch" behaviour without an explicit !taka()
        // gate here.
        self.fetcher.advance_falling(
            self.pixel_counter.value(),
            self.fine_scroll.pixel_clock_active(),
            self.window.window_line_counter(),
            self.window.wx_triggered(regs),
            regs,
            video,
            vram,
        );

        // Cascade advance: NYKA captures LYRY, PYGO captures PORY, POKY
        // settles. Runs before tick_rising so POKY's just-set value is
        // visible to the window's PYCO gate.
        self.cascade.advance_cascade(lyry);

        // SOCY's gate-chain (SYLO/TOMU/SOCY → TYFA/SEGU/SACU) is too slow
        // to suppress the same-dot in-flight SACU↑ when RYDY rises mid-dot.
        // Sample RYDY before tick_rising so the TYFA snapshot below sees
        // the pre-MOSU value on both edges — symmetric for normal cascade
        // (MOSU↑ on fall) and deferred cascade (MOSU↑ on rise).
        let rydy_pre_mosu = self.window.rydy();

        // Window cascade rise tick: NOPA captures prior-fall PYNU (ALET
        // rising), then PYNU's level-sensitive nor_latch re-evaluates.
        // The deferred-completion path can fire MOSU↑ on this edge when
        // the LCDC.5 restore CUPA drops XOFO while NUNU=1 is held from
        // the prior fall — NUNY rises on the same edge as PYNU.
        // Carries to the following fall to gate `mode3_advance_fetcher`
        // (NYXU reset hold across the post-MOSU dot).
        self.mosu_fired_rising = self.window.tick_rising(
            &mut self.fetcher,
            &mut self.cascade,
            &mut self.fine_scroll,
            regs,
            video,
        );

        // Sprite fetch counter advance (SABE clock). On hardware, SABE =
        // NAND2(LAPE, TAME) fires when ALET rises (= master clock fall =
        // PPU clock rise = this method's edge). Placed BEFORE the
        // TEKY/RYCE block so a newly initiated sprite fetch doesn't
        // advance on its first dot (matching hardware where SABE needs
        // one ALET cycle after TAKA sets).
        if self.sprite_trigger.taka() {
            match self.sprite_state {
                SpriteState::Fetching(ref mut sf) => {
                    let slot_index = sf.slot_index;
                    let done = sf.advance(regs, oam, oam_bus, vram);
                    if done {
                        sf.merge_into(&mut self.obj_shifter);
                        self.sprite_state = SpriteState::Idle;
                        self.sprite_trigger.clear_taka();
                        // Per-slot fetched-flag DFF captures HIGH at WUTY↑
                        // (spec §6.9 line 1730). Until this point FEPO=1
                        // holds the trigger frozen; setting the flag drops
                        // FEPO for this slot, allowing SACU to resume on
                        // the next combinational evaluation.
                        self.scan.sprites_mut().fetched |= 1 << slot_index;
                        // Recompute FEPO with the now-fetched slot for any
                        // next-iteration TEKY trigger on this rise.
                        fepo_pre_cupa = self.fepo(regs.control.sprites_enabled());
                    }
                }
                SpriteState::Idle => {}
            }
        }

        // TEKY: combinational sprite fetch request.
        //
        // Hardware: TEKY = AND4(FEPO, TUKU, LYRY, !TAKA), where TUKU =
        // NOT(RYDY) collapses the SYLO/TOMU/TUKU triple-inversion.
        let teky = fepo_pre_cupa && !self.window.rydy() && lyry && !self.sprite_trigger.taka();
        let ryce = self.sprite_trigger.capture_sobu(teky);

        if ryce {
            // Find and mark the matching sprite entry, start the fetch.
            self.start_sprite_fetch();
        }

        // Post-CUPA FEPO drives TYFA's combinational AND. CUPA's
        // transparent-latch propagation through XYLO → AROR → FEPO
        // completes within ~90 ps of CUPA↑, well before the next
        // alet-falling SACU pulse evaluation on this dot.
        let fepo_post_cupa = self.fepo(regs.control.sprites_enabled());

        // TYFA = AND3(SOCY, POKY, VYBO). Snapshotted at end of master-
        // clock rise; consumed in mode3_pixel_pipeline on master-clock
        // fall. SOCY = !RYDY; `rydy_pre_mosu` (sampled before tick_rising)
        // is the pre-MOSU value so the in-flight pre-window SACU fires on
        // the MOSU↑ dot before RYDY's effect reaches SACU through the
        // SYLO/TOMU/SOCY → TYFA/SEGU/SACU gate chain. VYBO = NOR3(
        // FEPO_old, WODU_old, MYVO); the !pixel_counter.terminal()
        // factor encodes !WODU_old.
        self.tyfa = !fepo_post_cupa
            && !self.pixel_counter.terminal()
            && !rydy_pre_mosu
            && self.cascade.poky();

        // POHU: combinational comparator, count == SCX & 7.
        // On hardware, POHU is combinational and ROXO captures into PUXA
        // on the falling edge. The count value is from the preceding rising
        // (reg_old), matching hardware.
        self.fine_scroll
            .compare_falling(regs.background_viewport.x.output());
    }

    /// Mode 3 fetcher-DFF advance (MYVO-clocked domain).
    ///
    /// Runs first within `on_ppu_clock_fall()`, before the pixel pipeline.
    /// MYVO-clocked DFFs capture here: SUDA (sprite trigger), PORY
    /// (cascade), BG fetch counter (LEBO). NOR latch responses (RYDY
    /// clear via PORY) and the TAVE one-shot preload also fire here.
    ///
    /// On hardware, these signals settle at depth 16-22 ge — well before
    /// CLKPIPE fires at depth 63.8 ge. Separating them models the
    /// hardware's actual signal domains: fetcher DFFs settle first, then
    /// the pixel pipeline evaluates against the settled state.
    fn mode3_advance_fetcher(&mut self) -> bool {
        // SUDA DFF: captures SOBU on LAPE rising edge (depth 6).
        self.sprite_trigger.capture_suda();

        // BG fetcher rising-edge advance: counter increment (LEBO clock).
        // The counter saturates at 5 (advance_rising is a no-op there)
        // so during sprite fetch it stays at 5 without an explicit gate.
        self.fetcher.advance_rising();
        self.cascade.capture_pory();

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
        let suzu_fired = self.window.clear_rydy_on_pory(self.cascade.pory());
        if suzu_fired {
            // SUZU → TEVO → NYXU: load window tile data into pipe.
            self.fetcher.load_into(&mut self.bg_shifter);

            // TEVO → PASO: reset fine counter.
            self.fine_scroll.reset_counter();
        }

        // TAVE one-shot preload: TAVE = NOT(SUVU); SUVU = NAND4(NYKA,
        // PORY, ROMO, mode3) where ROMO = NOT(POKY). Fires during the
        // cascade walk after NYKA and PORY have risen but before POKY
        // has captured PYGO via the NOR-latch.
        if self.cascade.nyka() && self.cascade.pory() && !self.cascade.poky() {
            self.fetcher.load_into(&mut self.bg_shifter);
            // TAVE → TEVO → PASO: reset fine counter. On hardware, TEVO
            // drives PASO which resets the fine counter on every pipe load
            // (TAVE, SEKO, SUZU).
            self.fine_scroll.reset_counter();
            // TAVE arm of VEKU = NOR2(WUTY, TAVE): clears TAKA carry-over
            // from the prior scanline so the sprite trigger can re-arm.
            self.sprite_trigger.clear_taka();
        }

        suzu_fired
    }

    /// Mode 3 pixel pipeline (CLKPIPE / SACU domain).
    ///
    /// Runs second within `on_ppu_clock_fall()`, after fetcher DFFs have settled.
    /// SACU (the pixel clock) fires at depth 63.8 ge — significantly later
    /// than the MYVO-clocked DFFs. This method evaluates against the
    /// settled fetcher state from `mode3_advance_fetcher`.
    ///
    /// Handles: TYFA consumption, PUXA/POVA fine scroll match, pixel
    /// shift registers, SEKO tile reload, LCD output, fine scroll
    /// counter, and NUKO window trigger. Sprite fetch advance now
    /// happens in mode3_rising (SABE clock, PPU-clock-rise edge).
    fn mode3_pixel_pipeline(
        &mut self,
        regs: &PipelineRegisters,
        rydy_before_pory: bool,
        advance_nyxu_pulse: bool,
        pixel_counter_before_sacu: u8,
    ) -> Option<PixelOutput> {
        // Consume TYFA from the rise-side snapshot. The snapshot
        // captures RYDY's pre-MOSU-set value, so the in-flight
        // pre-window SACU fires on the MOSU↑ dot before RYDY's
        // effect propagates (matching hardware's sub-dot SACU/SOCY
        // race).
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

        // VYBO = NOR3(MYVO, FEPO, WODU) — SACU is gated by FEPO/WODU
        // (and the MYVO clock phase), NOT by TAKA. TAKA freezes SACU
        // only indirectly: during sprite fetch the per-slot fetched-flag
        // is unset (captured at WUTY↑, spec §6.9 line 1730), so FEPO=1
        // for the unfetched sprite at sprite_x. When mid-fetch LCDC.1=0
        // drops FEPO via the AROR combinational chain (spec §6.10 line
        // 1858), SACU resumes inside the 6-dot fetch window.
        let sacu = tyfa && self.fine_scroll.pixel_clock_active();

        // PANY drain-detector slip: when NUKO=1 lands during the dot
        // where SEKO would naturally fire, NUKO truncates PANY's
        // high pulse; RYFA captures the second (later) half on the
        // next SEGU edge, slipping the SEKO → TEVO → NYXU/wx_clk
        // cascade by 1 dot. Both the BG-shifter parallel-load and
        // the window-tile-X counter increment land 1 dot late.
        let proposed_seko = self.fine_scroll.count == 7 && !rydy_before_pory;
        let nuko_now = self.window.nuko(pixel_counter_before_sacu);
        let pany_slip_now = proposed_seko && nuko_now;
        let seko_fire = (proposed_seko && !pany_slip_now) || self.pany_slip_pending;
        self.pany_slip_pending = pany_slip_now;

        let nyxu_pulse = seko_fire || advance_nyxu_pulse;

        // BG-fetcher state-change gate during sprite fetch with FEPO held.
        // Spec describes the BG fetcher counter cycling normally during
        // sprite fetch after a brief NYXU=0 pulse (~0.499 dots, spec §6.1
        // line 1201). In our model, firing `fetcher.load_into` (which
        // resets fetch_counter to 0 and parallel-loads bg_shifter) and
        // `fine_scroll.reset_counter` during sprite fetch produces
        // observable divergence from hardware reference images in
        // mealybug LCDC.{2,3,4,6}/SCX/SCY mid-Mode-3 tests and mooneye
        // sprite-intr tests. The root cause is downstream of these state
        // changes — likely in BG-fetcher VRAM-read timing or tile_temp
        // capture during the new tile cycle that occurs during sprite
        // fetch. Until that audit is complete, gate these specific
        // operations on `!fepo_held` (sprite-fetch-with-FEPO=1) so they
        // fire at the previous steady-state cadence. Everything else in
        // this method runs per dot per hardware.
        let fepo_held = self.sprite_trigger.taka() && self.fepo(regs.control.sprites_enabled());

        let pixel = pixel_output::resolve_current_pixel(&self.bg_shifter, &self.obj_shifter, regs);

        if seko_fire && !fepo_held {
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // NYXU pulse holds the BG shifter via LOZE async set/reset;
        // the concurrent SACU edge is overridden, so no shift fires
        // on the pulse dot. The OBJ shifter is not LOZE-gated — uses
        // sprite_onN / xefy parallel-load — so shifts on SACU normally.
        if sacu && !nyxu_pulse {
            self.bg_shifter.shift();
        }
        if sacu {
            self.obj_shifter.shift();
        }
        if sacu {
            self.pixel_counter.advance();
        }

        // WODU↑ is sampled on this fall — combinational on the
        // post-advance XANO (pixel_counter.terminal()) and the
        // post-advance FEPO (re-evaluated against the updated
        // pixel_counter so OAM-X=167 sprites are visible to WODU on
        // the same edge XANO becomes true, per spec §8.2's
        // FEPO→WODU combinational path).
        let wodu_fepo = self.fepo(regs.control.sprites_enabled());
        self.hblank
            .evaluate_wodu_on_fall(self.pixel_counter.terminal(), wodu_fepo);

        let (_toba, pixel_out) =
            self.lcd
                .on_ppu_clock_fall(sacu, pixel, pova, self.pixel_counter.value());

        if tyfa {
            self.fine_scroll.tick();
        }

        if seko_fire && !fepo_held {
            self.fine_scroll.reset_counter();
        }

        self.window.update_nuko_wx(regs.window.x_plus_7.output());

        pixel_out
    }

    /// FEPO: sprite X priority aggregate. True when any unfetched
    /// sprite's stored X matches the current pixel counter.
    ///
    /// Collapses the cascade `XYLO → AROR → 10 per-sprite NAND3
    /// decoders (dego/dydu/dyka/efyl/egom/xage/ybez/ydug/ygem/yloz)
    /// → FOVE/FEFY NAND5 → FEPO = OR2(FOVE, FEFY)`: `sprites_enabled()`
    /// carries the XYLO/AROR gate, the unfetched-loop carries the
    /// per-sprite decoders, and any-match carries OR2(FOVE, FEFY).
    /// The 16 SACU-clocked DFFSRs that latch per-sprite match state
    /// are recomputed combinationally each call; the 1-dot FEPO→WODU
    /// propagation is modelled by `HblankPipeline::fepo`. Off-screen
    /// X≥168 sprites are excluded (pixel_counter maxes at 167).
    ///
    /// Feeds VYBO (CLKPIPE freeze), XENA (WODU hblank gate), TEKY
    /// (sprite-fetch trigger). Pixel-MUX XYLO counterpart:
    /// `draw::pixel_output::resolve_pixel`.
    fn fepo(&self, sprites_enabled: bool) -> bool {
        if !sprites_enabled {
            return false; // AROR = AND(XYLO, AZEM). XYLO=0 forces AROR=0 → FEPO=0.
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
    ///
    /// The per-slot fetched-flag is NOT set here — hardware's per-slot
    /// DFF captures HIGH at WUTY↑ (fetch completion, spec §6.9 line
    /// 1730), not at fetch start. The flag is set in `mode3_rising`'s
    /// fetch-completion branch once `SpriteFetch::advance` returns
    /// `done`. This lets FEPO stay HIGH for the fetched slot through
    /// the fetch window (freezing SACU via VYBO), and lets the AROR
    /// combinational chain drop FEPO mid-fetch on a LCDC.1=0 write
    /// (spec §6.4.1 VYBO = NOR3(MYVO, FEPO, WODU); no TAKA term).
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
