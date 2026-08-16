//! Mode 3's pixel pipeline as one object: the per-edge composition of the
//! fetcher, cascade, window, sprite and LCD blocks, and the scanline resets
//! that arm them.
//!
//! The mode-3 datapath is in `mode3`, the CPU-facing OAM/VRAM access
//! predicates in `locks`, and the debugger/trace faces in `snapshot`.

mod locks;
mod mode3;
mod snapshot;

pub use snapshot::{
    PipelineSnapshot, PpuTraceSnapshot, SpriteFetchPhase, SpriteStoreEntrySnapshot,
    SpriteStoreSnapshot,
};

use core::fmt;

use crate::dma::OamBusOwner;
use crate::ppu::{DrawnPixel, PipelineRegisters, PpuModel, VideoControl, memory::Oam};

use super::draw::fetch_cascade::FetchCascade;
use super::draw::fetcher::TileFetcher;
use super::draw::fine_scroll::FineScroll;
use super::draw::hblank_pipeline::HblankPipeline;
use super::draw::lcd_control::LcdControl;
use super::draw::pixel_counter::PixelCounter;
use super::draw::pixel_output;
use super::draw::shifters::BgShifter;
use super::draw::sprite_fetch::SpriteState;
use super::draw::sprite_trigger::SpriteTrigger;
use super::draw::window_control::WindowControl;
use super::scan::scanner::{ScanSignals, SpriteScanner};

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

pub struct Rendering<P: PpuModel> {
    /// FEPO → WODU → VOGA → WEGO → clears XYMU.
    hblank: HblankPipeline,
    /// Scan counter, BESU latch, BYBA/DOBA pipeline, sprite store.
    scan: SpriteScanner,
    bg_shifter: BgShifter<P::BgCell>,
    obj_fifo: P::ObjFifo,
    fetcher: TileFetcher<P>,
    /// LYRY → NYKA → PORY → PYGO → POKY.
    cascade: FetchCascade,
    /// Fine-scroll counter + ROXY pixel-clock gate.
    fine_scroll: FineScroll,
    /// CGB register-file crossing copy of FF43 read by the fine-scroll match
    /// decode (POHU); captured at the write M-cycle's last PPU fall. Unused on DMG.
    synced_scx: u8,
    /// RYDY latch, WX comparator, window line counter.
    window: WindowControl,
    /// TYFA = `!FEPO && !WODU && !RYDY && POKY`; snapshotted at end of `mode3_rising`,
    /// consumed in `mode3_pixel_pipeline`. RYDY is sampled before `tick_rising`
    /// so same-dot RYDY↑ doesn't reach the snapshot (models SYLO/TOMU/SOCY → SACU delay).
    pipe_clock_enable: bool,
    pixel_counter: PixelCounter,
    /// WUSA gating, POVA trigger, LCD pixel push.
    lcd: LcdControl,
    sprite_state: SpriteState,
    /// TEKY → SOBU → SUDA → RYCE → TAKA.
    sprite_trigger: SpriteTrigger,
    /// PANY drain-detector slip carry-over: NUKO=1 lands while SEKO would fire (count==7),
    /// splitting PANY's high pulse — RYFA captures the second half, slipping SEKO→TEVO→NYXU by 1 dot.
    drain_slip_pending: bool,
    /// Window trigger (MOSU) fired on the prior rise via the deferred-completion path
    /// (LCDC.5 restore drops XOFO while NUNU=1); consumed on the following fall to hold
    /// the BG fetch counter at 0 via NYXU's reset pulse.
    pending_window_trigger: bool,
    /// WODU early pulse: at the advance onto terminal PX the shallow XANO decode settles
    /// before the deeper FEPO comparator, so WODU glitches high for the settling window.
    /// One-shot — set at the advance fall, cleared on the next rise — so the fall SUKO eval
    /// catches it but off-edge reads (the STAT-write glitch) see settled WODU.
    terminal_end_of_line_pulse: bool,
    /// MOSU fired at terminal PX this line. The CGB mode-0 STAT leg follows XUGU from
    /// here to line end — terminal sprite fetches no longer mask it. Unused on DMG.
    terminal_restart: bool,
}

impl<P: PpuModel> Rendering<P> {
    pub(crate) fn window_line_counter(&self) -> u8 {
        self.window.window_line_counter()
    }

    /// Seed the internal window line counter from a save state.
    pub(crate) fn restore_window_line_counter(&mut self, value: u8) {
        self.window.set_window_line_counter(value);
    }

    pub(super) fn new() -> Self {
        Rendering {
            hblank: HblankPipeline::new(),
            scan: SpriteScanner::new(),
            bg_shifter: BgShifter::new(),
            obj_fifo: P::ObjFifo::default(),
            fetcher: TileFetcher::new(),
            cascade: FetchCascade::new(),
            fine_scroll: FineScroll::new(),
            synced_scx: 0,
            window: WindowControl::new(),
            pipe_clock_enable: false,
            pixel_counter: PixelCounter::new(),
            lcd: LcdControl::new(),
            sprite_state: SpriteState::Idle,
            sprite_trigger: SpriteTrigger::new(),
            drain_slip_pending: false,
            pending_window_trigger: false,
            terminal_end_of_line_pulse: false,
            terminal_restart: false,
        }
    }

    pub(super) fn post_boot() -> Self {
        Rendering {
            hblank: HblankPipeline::post_boot(),
            scan: SpriteScanner::post_boot(),
            bg_shifter: BgShifter::new(),
            obj_fifo: P::ObjFifo::default(),
            fetcher: TileFetcher::post_boot(),
            cascade: FetchCascade::new(),
            fine_scroll: FineScroll::new(),
            synced_scx: 0,
            window: WindowControl::new(),
            pipe_clock_enable: false,
            pixel_counter: PixelCounter::post_boot(),
            lcd: LcdControl::post_boot(),
            sprite_state: SpriteState::Idle,
            sprite_trigger: SpriteTrigger::new(),
            drain_slip_pending: false,
            pending_window_trigger: false,
            terminal_end_of_line_pulse: false,
            terminal_restart: false,
        }
    }

    /// VID_RST deassertion releases the scan counter alongside the rest of the pipeline.
    pub(super) fn start_scanning(&mut self) {
        self.scan.start_scanning();
    }

    /// Whether `P` crosses the window register file into the pixel pipeline on a
    /// named M-cycle edge — the CGB case. DMG reads the cells live.
    fn window_synced() -> bool {
        P::WINDOW_CROSSING.is_synced()
    }

    /// CGB register crossing into the window decode and scan comparator; the
    /// capture edge is the write M-cycle's last PPU fall.
    pub(super) fn capture_register_sync(&mut self, regs: &PipelineRegisters) {
        self.window.capture_register_sync(
            regs.window.y,
            regs.window.x.output(),
            regs.control.window_enabled(),
            regs.control.sprite_size(),
        );
        if P::SCX_CROSSING.is_synced() {
            self.synced_scx = regs.background_viewport.x.output();
        }
    }

    /// Nothing on the pixel side can move on this dot: XYMU is low so the whole
    /// mode-3 datapath is gated out, the OAM scan chain is parked, and both
    /// window latches hold the values their next capture would rewrite.
    pub(super) fn span_inert(&self, regs: &PipelineRegisters, video: &VideoControl) -> bool {
        !self.hblank.rendering_active()
            && self.scan.chain_idle()
            && self.window.wy_match_frame_settled()
            && self
                .window
                .wy_match_settled(regs, video, Self::window_synced())
    }

    /// XYMU rendering latch; `true` during Mode 3 (opposite polarity to spec's active-low XYMU).
    pub(super) fn rendering_active(&self) -> bool {
        self.hblank.rendering_active()
    }

    /// WODU = AND2(XUGU, !FEPO); combinational, doesn't depend on XYMU.
    pub(super) fn end_of_line_signal(&self, sprites_enabled: bool) -> bool {
        HblankPipeline::compute_end_of_line(
            self.pixel_counter.terminal(),
            self.sprite_x_match(sprites_enabled),
        )
    }

    /// Early WODU↑ pulse at the advance onto terminal PX (XANO settles before FEPO); ORed
    /// into the Mode-0 STAT leg. One-shot, cleared on the next rise.
    pub(super) fn terminal_end_of_line_pulse(&self) -> bool {
        self.terminal_end_of_line_pulse
    }

    /// MOSU fired at terminal PX this line (CGB mode-0 STAT leg follows XUGU from there).
    pub(super) fn terminal_restart(&self) -> bool {
        self.terminal_restart
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

    /// ALET rising: ALET-clocked DFFs capture (NYKA, PYGO, VOGA); XUPY-derived logic and combinational signals settle.
    pub(super) fn on_ppu_clock_rise(
        &mut self,
        model: &P,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &P::Vram,
    ) -> Option<DrawnPixel<P::Pixel>> {
        // Terminal WODU pulse is a fall-edge transient; clear it so rise / off-edge reads see settled WODU.
        self.terminal_end_of_line_pulse = false;

        // REJO re-evaluates on every PPU rise (vblank↑ etc.); SARY captures only on TALU↑ (in fall).
        self.window.update_wy_match_frame_on_rise(video);

        // BYBA/AVAP have moved to on_ppu_clock_fall; here ALET-clocked DFFs and AJUJ close fire.
        self.hblank.tick_access_permit_on_rise();

        if self.scan.scanning() {
            return None;
        }

        // Capture XYMU before commit_end_of_line_on_rise() may clear it.
        let was_rendering = self.hblank.rendering_active();

        if was_rendering {
            self.mode3_rising(model, regs, video, oam, oam_bus, vram);
            // WODU is combinational on XANO/!FEPO. Re-evaluate post-WUTY so a same-rise
            // FEPO drop at a terminal pix latches VOGA without waiting for the next fall.
            let terminal_count = self.pixel_counter.terminal();
            let sprite_x_match = self.sprite_x_match(regs.control.sprites_enabled());
            self.hblank
                .latch_end_of_line(terminal_count, sprite_x_match);
        }

        // VOGA.q captures on this rise; WEGO clears XYMU.
        // `end_of_line` flags VOGA's just-committed transition — LCD pushes screen_x=159 on this dot.
        let end_of_line = if !P::WINDOW_RESTART_MASKS_MODE3_END || !self.window.window_hit() {
            self.hblank.commit_end_of_line_on_rise()
        } else {
            false
        };

        let bg_shifter = &self.bg_shifter;
        let obj_fifo = &self.obj_fifo;
        let resolve_pixel =
            || model.resolve(&pixel_output::current_mux::<P>(bg_shifter, obj_fifo), regs);
        let pixel = self.lcd.on_ppu_clock_rise(
            self.hblank.end_of_line_latched(),
            end_of_line,
            resolve_pixel,
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
        let line_end_pulse_rising = self.scan.tick_scan_capture(video.scan_clock(), video.ly());
        if line_end_pulse_rising {
            self.pixel_counter.reset();
        }
    }

    /// ALET falling: MYVO-clocked DFFs capture (PORY); LEBO advances BG fetch counter; SACU drives CLKPIPE.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn on_ppu_clock_fall(
        &mut self,
        model: &P,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        oam_bus: OamBusOwner,
        scan_clock_rising: bool,
        lx_clock_rising: bool,
        mcycle_last_fall: bool,
    ) -> Option<DrawnPixel<P::Pixel>> {
        // SARY captures wy_match on TALU↑ (hclk); REJO re-evaluates every PPU fall for vblank↓.
        self.window
            .tick_wy_match_falling(regs, video, lx_clock_rising, Self::window_synced());

        // The M-cycle's last PPU fall: the WY/WX/LCDC.5/LCDC.2 crossing's
        // capture edge (the boundary fall at single speed; the T2 fall in
        // double speed when T3's edge carries no PPU fall). SARY's coinciding
        // TALU↑ capture reads the pre-tick output (DFF chain); XOFO, the NUKO
        // slave, and the scan comparator read the post-tick output this same
        // fall.
        if Self::window_synced() && mcycle_last_fall {
            self.capture_register_sync(regs);
        }

        // Snapshot before AVAP reaction sets XYMU; the rise→rise gap models the 1-dot AVAP→LAXU delay.
        let was_rendering = self.hblank.rendering_active();

        // CGB: the scan Y-comparator's XYMO view is the live bit OR the
        // register-crossing copy — a grow reaches GOVU live, a shrink waits
        // for the crossing capture.
        let scan_sprite_height = if Self::window_synced() {
            regs.control
                .sprite_size()
                .height()
                .max(self.window.synced_sprite_size().height())
        } else {
            regs.control.sprite_size().height()
        };

        // BYBA/AVAP co-locate on this XUPY-rising fall.
        let scan = if scan_clock_rising {
            self.scan
                .advance_scan(video.ly(), scan_sprite_height, oam, oam_bus)
        } else {
            ScanSignals::HELD
        };
        if scan.scan_complete {
            // Mode 3 begins on AVAP-fall; AJUJ pulse asserts alongside mode3↑ for write-permit.
            self.hblank.pulse_access_permit_on_scan_complete();
            self.window.init_match_wx(regs.window.x.output());
            self.fetcher.load_into(&mut self.bg_shifter);
        }

        // Mode 3 pixel output runs in two sub-phases: fetcher DFFs (MYVO-clocked, depth 16-22ge),
        // then pixel pipeline (SACU-driven, depth 63.8ge).
        // mode3_advance_fetcher is gated on was_rendering so the AVAP-reaction rise leaves LAXU at 0.
        if self.hblank.rendering_active() {
            // PORY may clear RYDY during advance_fetcher; SEKO and window restart need pre-PORY RYDY.
            // Pixel counter is only advanced by SACU in mode3_pixel_pipeline.
            let window_hit_before_fetcher_advance = self.window.window_hit();
            let pixel_counter_before_shift = self.pixel_counter.value();

            // MOSU↑ arming runs before mode3_advance_fetcher so the counter=0 VRAM read sees
            // fetching_window=true. When MOSU↑ fires, advance_fetcher is gated out for this dot.
            let window_triggered = self.window.tick_falling(
                &mut self.fetcher,
                &mut self.cascade,
                &mut self.fine_scroll,
                regs,
            );

            // SUZU is a TEVO OR3 input alongside SEKO/TAVE; drives NYXU low (LOZE holds BG shifter).
            // `pending_window_trigger` carries the deferred-completion MOSU from the prior rise.
            let deferred_window_trigger = self.pending_window_trigger;
            self.pending_window_trigger = false;
            let load_window_pulse =
                if was_rendering && !window_triggered && !deferred_window_trigger {
                    self.mode3_advance_fetcher()
                } else {
                    false
                };
            // MOSU is also a direct NYXU input; the pulse holds the BG shifter on this dot.
            let window_restart_reset_pulse =
                window_triggered || deferred_window_trigger || load_window_pulse;
            self.window.tick_delayed_window_hit();
            let px = self.mode3_pixel_pipeline(
                model,
                regs,
                window_hit_before_fetcher_advance,
                window_restart_reset_pulse,
                pixel_counter_before_shift,
            );
            // A restart landing at terminal PX disconnects FEPO's WODU mask from the
            // CGB mode-0 STAT leg for the rest of the line (the IRQ tree follows XUGU).
            if window_triggered && self.pixel_counter.terminal() {
                self.terminal_restart = true;
            }
            px
        } else {
            None
        }
    }

    pub(super) fn reset_scanline(&mut self, scanline: u8) {
        self.hblank.reset();
        self.scan.reset();
        self.scan.arm_scan_capture();
        self.bg_shifter = BgShifter::new();
        self.obj_fifo = P::ObjFifo::default();
        self.fetcher.reset_scanline();
        self.cascade.reset();
        self.fine_scroll = FineScroll::new();
        self.window.reset_scanline(P::ENABLE_QUALIFIED_WINDOW_HIT);

        self.pipe_clock_enable = false;
        self.drain_slip_pending = false;
        self.terminal_restart = false;
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

    /// Arm the BG set glitch (mid-fetch LCDC.4 SET, from the register write path).
    /// The substituted byte is a sprite high-plane fetch on the tile-data bus, so
    /// the glitch only manifests when a sprite is scanned onto this line; the
    /// fetcher then latches it only when the SET is observed at counter 1.
    pub(in crate::ppu) fn arm_bg_set_glitch(&mut self) {
        if self.scan.sprites_ref().count > 0 {
            self.fetcher.arm_set_glitch();
        }
    }

    /// A mid-fetch LCDC.4 CLEAR snapshots the bus into the glitch source at the
    /// next completed BG fetch (the BG tile as of the TILE_SEL reset).
    pub(in crate::ppu) fn arm_bg_glitch_capture(&mut self) {
        self.fetcher.arm_glitch_capture();
    }

    pub(in crate::ppu) fn sprite_on_line(&self, sprites_enabled: bool) -> bool {
        sprites_enabled && self.scan.sprites_ref().count > 0
    }
}
