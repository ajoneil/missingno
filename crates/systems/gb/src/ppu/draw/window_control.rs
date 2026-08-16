use crate::ppu::types::sprites::SpriteSize;
use crate::ppu::{DffBit, DffLatch, NorLatch, PipelineRegisters, PpuModel, VideoControl};

/// WY/WX/LCDC.5/LCDC.2 captured on the PPU side of the CGB register-file
/// crossing (the write M-cycle's last PPU fall). Consumers split pre/post-capture
/// by call order within that fall — SARY reads before the capture, XOFO / the
/// NUKO WX slave / the scan Y-comparator after — so no pending/output DFF pair is
/// needed. DMG reads the live cells and never builds this.
#[derive(Clone, Copy)]
struct CrossedWindowRegisters {
    wy: u8,
    wx: u8,
    enabled: bool,
    sprite_size: SpriteSize,
}

impl CrossedWindowRegisters {
    fn new() -> Self {
        CrossedWindowRegisters {
            wy: 0,
            wx: 0,
            enabled: false,
            sprite_size: SpriteSize::Single,
        }
    }
}

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// WY-match SARY/REJO/REPU, WX-match capture chain (PYCO → NUNU → PYNU → NOPA), RYDY/PUKU
/// NOR-latch, and the WAZY/VYNO window-line counter clocked by `wy_clk = NOT(PYNU)`.
///
/// Each DFF captures on its hardware-correct edge:
/// - SARY captures `wy_match` on master rise (hclk rising).
/// - PYCO captures NUKO on PPU rise (ROCO is TYFA/SEGU-derived).
/// - NOPA captures PYNU on PPU rise.
/// - NUNU captures PYCO on PPU fall (MEHE).
/// - PYNU nor_latch: S=NUNU, R=XOFO; re-evaluated on both edges.
/// - REJO nor_latch: S=SARY.q, R=REPU (vblank); re-evaluated on both edges.
/// - NUNY = AND2(PYNU, NOPA_n). MOSU↑ fires on NUNY 0→1.
pub(in crate::ppu) struct WindowControl {
    /// Window-hit (RYDY nor3 + PUKU feedback). Set on NUNY rise; cleared by PORY during cascade restart.
    window_hit: NorLatch,
    /// PYCO: captures NUKO on PPU rise (ROCO rising, gated by POKY=1).
    wx_match_capture_1: DffLatch,
    /// NUNU: captures PYCO on PPU fall (MEHE rising) — one half-dot after PYCO.
    wx_match_capture_2: DffLatch,
    /// Level-sensitive PYNU: sets when NUNU=1 with XOFO=0; clears when XOFO=1.
    window_armed: NorLatch,
    /// Captures PYNU on PPU rise; NOPA_n drives NUNY's AND2 low gate.
    window_mode: DffLatch,
    /// Previous-dot NUNY for MOSU rising-edge detection.
    prev_window_trigger_pulse: bool,
    /// Window has rendered at least one pixel on the current line (WAZY-equivalent flag).
    window_rendered: bool,
    /// WX as the NUKO comparator sees it: the register's DFF8 slave output
    /// (lags the master by one ALET edge).
    match_wx: u8,
    /// WAZY → VYNO ripple, clocked by PYNU 1→0 transitions during rendering.
    window_line_counter: u8,
    /// SOVY: MYVO-clocked DFF delaying RYDY; SUZU = AND2(!RYDY, SOVY).
    delayed_window_hit: DffBit,
    /// SARY: hclk-clocked DFF sampling `wy_match = LCDC.5 ∧ (LY == WY)`.
    wy_match_sample: DffLatch,
    /// REJO WY-match frame latch. Set by SARY.q; reset by REPU = vblank (mode1).
    wy_match_frame: NorLatch,
    /// REJO.q as NUKO's fall-phase consumer (PANY) sees it: sampled before this fall's
    /// hclk/SARY→REJO update, since the NUKO decode precedes the late hclk edge.
    wy_match_frame_at_capture: bool,
    /// CGB WY/WX/LCDC.5/LCDC.2 as the window decode, trigger chain, and scan
    /// Y-comparator see them: register cells cross into the PPU domain at the
    /// write M-cycle's last PPU fall (the STAT register file's sibling
    /// crossing). Unused on DMG (the consumers read the cells live).
    synced: CrossedWindowRegisters,
    /// POPU's output at the previous TALU capture = its pre-edge value at this
    /// one (POPU only toggles on capture-co-located falls). REPU gates the CGB
    /// SARY input: captures up to and including the vblank-exit one take 0, so
    /// the first post-exit capture commits the frame's WY match.
    vblank_at_last_capture: bool,
}

impl WindowControl {
    pub(in crate::ppu) fn new() -> Self {
        WindowControl {
            window_hit: NorLatch::new(false),
            wx_match_capture_1: DffLatch::new(0),
            wx_match_capture_2: DffLatch::new(0),
            window_armed: NorLatch::new(false),
            window_mode: DffLatch::new(0),
            prev_window_trigger_pulse: false,
            window_rendered: false,
            match_wx: 0xFF,
            window_line_counter: 0,
            delayed_window_hit: DffBit::new(false, false),
            wy_match_sample: DffLatch::new(0),
            wy_match_frame: NorLatch::new(false),
            wy_match_frame_at_capture: false,
            synced: CrossedWindowRegisters::new(),
            vblank_at_last_capture: true,
        }
    }

    pub(in crate::ppu) fn capture_register_sync(
        &mut self,
        wy: u8,
        wx: u8,
        enabled: bool,
        sprite_size: SpriteSize,
    ) {
        self.synced = CrossedWindowRegisters {
            wy,
            wx,
            enabled,
            sprite_size,
        };
    }

    pub(in crate::ppu) fn synced_sprite_size(&self) -> SpriteSize {
        self.synced.sprite_size
    }

    /// Seed NUKO's WX slave at the AVAP-fall Mode-3 entry.
    pub(in crate::ppu) fn init_match_wx(&mut self, wx: u8) {
        self.match_wx = wx;
    }

    /// Advance NUKO's WX slave one ALET edge behind the register master.
    pub(in crate::ppu) fn update_match_wx(&mut self, wx: u8, synced: bool) {
        self.match_wx = if synced { self.synced.wx } else { wx };
    }

    /// SARY's D input: `LCDC.5 ∧ (LY == WY)`, read live on the DMG and through
    /// the M-boundary crossing on the CGB, where REPU also gates it.
    fn wy_match(&self, regs: &PipelineRegisters, video: &VideoControl, synced: bool) -> bool {
        if synced {
            !self.vblank_at_last_capture && self.synced.enabled && video.ly() == self.synced.wy
        } else {
            regs.control.window_enabled() && video.ly() == regs.window.y
        }
    }

    /// REJO's fall-phase view (PANY) already holds the latch output this fall's
    /// copy would rewrite.
    pub(in crate::ppu) fn wy_match_frame_settled(&self) -> bool {
        self.wy_match_frame_at_capture == self.wy_match_frame.output()
    }

    /// SARY already holds the match its next TALU↑ capture would write, with
    /// REPU's POPU copy current — so both the capture and REJO's update rewrite
    /// what is there. LY advances inside a span, so this is the WY-match ender.
    pub(in crate::ppu) fn wy_match_settled(
        &self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        synced: bool,
    ) -> bool {
        (self.wy_match_sample.output() != 0) == self.wy_match(regs, video, synced)
            && self.vblank_at_last_capture == video.vblank()
            && self.wy_match_sample.pending().is_none()
    }

    /// SARY's TALU↑ capture.
    fn capture_wy_match(&mut self, regs: &PipelineRegisters, video: &VideoControl, synced: bool) {
        let wy_match = self.wy_match(regs, video, synced);
        self.vblank_at_last_capture = video.vblank();
        self.wy_match_sample.write(if wy_match { 1 } else { 0 });
        self.wy_match_sample.tick();
    }

    /// REJO: set by SARY.q, reset by REPU (vblank).
    fn update_wy_match_frame(&mut self, video: &VideoControl) {
        if video.vblank() {
            self.wy_match_frame.clear();
        } else if self.wy_match_sample.output() != 0 {
            self.wy_match_frame.set();
        }
    }

    /// REJO re-evaluates against current SARY + vblank on every PPU rise (handles vblank↑).
    /// SARY itself only captures on TALU↑ — see `tick_wy_match_falling`.
    pub(in crate::ppu) fn update_wy_match_frame_on_rise(&mut self, video: &VideoControl) {
        self.update_wy_match_frame(video);
    }

    /// TALU↑ (hclk rising) lands on a PPU fall in the emulator's clock model. SARY captures
    /// wy_match on that edge; REJO re-evaluates on every fall to handle vblank↓.
    pub(in crate::ppu) fn tick_wy_match_falling(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        lx_clock_rising: bool,
        register_sync: bool,
    ) {
        self.wy_match_frame_at_capture = self.wy_match_frame.output();
        if lx_clock_rising {
            self.capture_wy_match(regs, video, register_sync);
        }
        self.update_wy_match_frame(video);
    }

    /// PORY's RYDY reset arm, then SUZU = AND2(!RYDY, SOVY): true on any RYDY 1→0 —
    /// PORY's release or the XOFO abort — triggering TEVO's load-window pulse.
    pub(in crate::ppu) fn release_window_hit_on_fetcher_reset(
        &mut self,
        fetcher_reset: bool,
    ) -> bool {
        if fetcher_reset {
            self.window_hit.clear();
        }
        self.delayed_window_hit.output() && !self.window_hit.output()
    }

    /// SOVY captures RYDY on MYVO; free-runs even when NAFY gates the fetcher advance.
    pub(in crate::ppu) fn tick_delayed_window_hit(&mut self) {
        self.delayed_window_hit.write(self.window_hit.output());
        self.delayed_window_hit.tick();
    }

    /// NUKO = AND2(REJO, PX == WX).
    fn compute_wx_match(&self, pixel_counter: u8, wy_match_frame: bool) -> bool {
        wy_match_frame && pixel_counter == self.match_wx
    }

    /// PYCO captures NUKO on ROCO↑ (ALET-phase, one half-dot before NUNU's
    /// MEHE capture). PYCO holds when FEPO=1 or POKY=0: VYBO/TYFA halt ROCO.
    /// On CGB, XOFO's reset reach dominates the capture (r-dominant dffr).
    pub(in crate::ppu) fn capture_wx_match_on_pixel_clock<P: PpuModel>(
        &mut self,
        pixel_counter: u8,
        fetcher_ready: bool,
        sprite_x_match: bool,
        regs: &PipelineRegisters,
    ) {
        if P::ENABLE_QUALIFIED_WINDOW_HIT
            && self.compute_window_arm_reset(regs, P::WINDOW_CROSSING.is_synced())
        {
            self.wx_match_capture_1.write_immediate(0);
            return;
        }
        let wx_match = self.compute_wx_match(pixel_counter, self.wy_match_frame.output());
        if fetcher_ready && !sprite_x_match {
            self.wx_match_capture_1.write(if wx_match { 1 } else { 0 });
            self.wx_match_capture_1.tick();
        }
    }

    /// NUNY = AND2(PYNU, NOPA_n).
    fn window_trigger_pulse(&self) -> bool {
        self.window_armed.output() && self.window_mode.output() == 0
    }

    /// Live NUKO (pixel_counter == WX). Two netlist consumers: PYCO (this chain) and PANY
    /// (drain-detector input). PANY's tile-boundary high window is where a same-dot hit lands
    /// as the cascade slip.
    pub(in crate::ppu) fn window_x_reached(&self, pixel_counter: u8) -> bool {
        self.compute_wx_match(pixel_counter, self.wy_match_frame_at_capture)
    }

    /// XOFO during rendering simplifies to NOT(LCDC.5) — read live on the
    /// DMG, through the M-boundary crossing on the CGB.
    fn compute_window_arm_reset(&self, regs: &PipelineRegisters, synced: bool) -> bool {
        if synced {
            !self.synced.enabled
        } else {
            !regs.control.window_enabled()
        }
    }

    /// PPU rise: NOPA captures prior-fall PYNU; PYNU re-evaluates; MOSU↑ fires if NUNY rises.
    /// Catches the deferred-completion case (LCDC.5 restore drops XOFO while NUNU=1 from prior fall).
    pub(in crate::ppu) fn tick_rising<P: PpuModel>(
        &mut self,
        fetcher: &mut TileFetcher<P>,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        regs: &PipelineRegisters,
    ) -> bool {
        // NOPA captures BEFORE the PYNU update so it observes PYNU's prior-fall value.
        self.window_mode
            .write(if self.window_armed.output() { 1 } else { 0 });
        self.window_mode.tick();

        self.update_window_armed_and_check_trigger(regs, fetcher, cascade, fine_scroll)
    }

    /// PPU fall: NUNU captures PYCO on MEHE↑ (= NOT(ALET)), one half-dot after
    /// PYCO's ROCO capture on the rise.
    pub(in crate::ppu) fn tick_falling<P: PpuModel>(
        &mut self,
        fetcher: &mut TileFetcher<P>,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        regs: &PipelineRegisters,
    ) -> bool {
        self.wx_match_capture_2
            .write(self.wx_match_capture_1.output());
        self.wx_match_capture_2.tick();

        self.update_window_armed_and_check_trigger(regs, fetcher, cascade, fine_scroll)
    }

    /// PYNU/NUNY/MOSU update. Runs on every edge since PYNU is combinational on NUNU/XOFO.
    fn update_window_armed_and_check_trigger<P: PpuModel>(
        &mut self,
        regs: &PipelineRegisters,
        fetcher: &mut TileFetcher<P>,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
    ) -> bool {
        let window_arm_reset = self.compute_window_arm_reset(regs, P::WINDOW_CROSSING.is_synced());
        let prev_window_armed = self.window_armed.output();

        if window_arm_reset {
            self.window_armed.clear();
            if P::ENABLE_QUALIFIED_WINDOW_HIT {
                // CGB extends XOFO's reset reach into the capture chain: a hit
                // landing while LCDC.5=0 cannot wait armed for a re-enable
                // (DMG keeps PYCO/NUNU propagating and fires the deferred
                // completion; CGB does not).
                self.window_hit.clear();
                self.wx_match_capture_1.write_immediate(0);
                self.wx_match_capture_2.write_immediate(0);
            }
        } else if self.wx_match_capture_2.output() != 0 {
            self.window_armed.set();
        }

        let window_trigger_pulse = self.window_trigger_pulse();
        let window_triggered = window_trigger_pulse && !self.prev_window_trigger_pulse;
        self.prev_window_trigger_pulse = window_trigger_pulse;

        // WAZY ticks on PYNU 1→0 (mid-mode-3 LCDC.5↓ or end-of-mode-3 ATEJ↑).
        if prev_window_armed && !self.window_armed.output() && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }

        if window_triggered {
            fine_scroll.reset_for_window();
            self.window_hit.set();
            fetcher.reset_for_window();
            cascade.reset_window();
            self.window_rendered = true;
        }

        window_triggered
    }

    pub(in crate::ppu) fn reset_frame(&mut self) {
        self.window_line_counter = 0;
        self.window_rendered = false;
    }

    /// Models ATEJ↑'s XOFO pulse on PYNU: clear briefly, re-set from NUNU carryover, NOPA captures.
    /// The CGB's extended XOFO reach clears PYCO/NUNU too — the right-edge NUNU=1 carryover dies,
    /// so the cascade re-fires fresh each line where the DMG's stays armed.
    pub(in crate::ppu) fn reset_scanline(&mut self, arm_reset_reaches_capture_chain: bool) {
        self.window_hit.clear();
        self.delayed_window_hit = DffBit::new(false, false);
        if self.window_armed.output() && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }
        self.window_armed.clear();
        if arm_reset_reaches_capture_chain {
            self.wx_match_capture_1.write_immediate(0);
            self.wx_match_capture_2.write_immediate(0);
        }
        if self.wx_match_capture_2.output() != 0 {
            self.window_armed.set();
        }
        self.window_mode
            .write(if self.window_armed.output() { 1 } else { 0 });
        self.window_mode.tick();
        self.prev_window_trigger_pulse = self.window_trigger_pulse();
        self.match_wx = 0xFF;
    }

    /// RYDY.
    pub(in crate::ppu) fn window_hit(&self) -> bool {
        self.window_hit.output()
    }

    pub(in crate::ppu) fn wx_triggered(&self, regs: &PipelineRegisters, synced: bool) -> bool {
        self.window_armed.output() && !self.compute_window_arm_reset(regs, synced)
    }

    pub(in crate::ppu) fn window_rendered(&self) -> bool {
        self.window_rendered
    }

    pub(crate) fn window_line_counter(&self) -> u8 {
        self.window_line_counter
    }

    pub(crate) fn set_window_line_counter(&mut self, value: u8) {
        self.window_line_counter = value;
    }
}
