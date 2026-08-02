use super::dff::DffLatch;
use super::types::control::{Control, ControlFlags};
use super::types::palette::Palettes;
use super::types::sprites::SpriteSize;
use super::types::tiles::TileAddressMode;

#[derive(Hash)]
pub struct BackgroundViewportPosition {
    pub x: DffLatch,
    pub y: DffLatch,
}

#[derive(Hash)]
pub struct Window {
    pub y: u8,
    pub x: DffLatch,
}

/// OLD-value overlay for an LCDC bit that transitioned mid-Mode-3. Arms with the
/// pre-write value on a CPU write site and holds it for `hold` falls so the
/// BG/OBJ resolve still sees OLD, then clears. The base hold of 1 covers the
/// same fall's tick; CGB's clock-domain write lag (e.g. VYXE/RAJY) adds one more.
#[derive(Default, Hash)]
pub(in crate::ppu) struct OldOverlay {
    value: Option<bool>,
    hold: u8,
}

impl OldOverlay {
    fn arm(&mut self, old: bool, new: bool, extra_hold: u8) {
        if old != new {
            self.value = Some(old);
            self.hold = 1 + extra_hold;
        }
    }

    fn tick(&mut self) {
        if self.hold > 0 {
            self.hold -= 1;
        } else {
            self.value = None;
        }
    }

    /// No OLD value is being held — the overlay has fully cleared.
    fn settled(&self) -> bool {
        self.value.is_none() && self.hold == 0
    }

    fn resolve(&self, live: bool) -> bool {
        self.value.unwrap_or(live)
    }

    fn clear(&mut self) {
        self.value = None;
        self.hold = 0;
    }
}

/// The TILE_SEL reset glitch cell, behind a [`PpuModel`] associated type: an
/// LCDC.4-clearing write reaches the tile-data addressing at the crossing-capture
/// dot, so a bitplane read on that dot returns the tile index byte instead of
/// VRAM data. Live for one dot. The CGB owns the real cell; the DMG a ZST `()`.
///
/// [`PpuModel`]: super::PpuModel
pub trait TileSelGlitch {
    /// Arm from an LCDC.4-clearing write; it goes active on the next tick.
    fn arm(&mut self);
    /// Per-fall advance: the armed pending value becomes this dot's active value.
    fn tick(&mut self);
    /// Whether a bitplane read this dot substitutes the tile index byte.
    fn active(&self) -> bool;
    /// LCD-off freeze/clear.
    fn clear(&mut self);
}

impl TileSelGlitch for () {
    fn arm(&mut self) {}
    fn tick(&mut self) {}
    fn active(&self) -> bool {
        false
    }
    fn clear(&mut self) {}
}

/// CPU → pixel pipeline register file (DFF bank). DFF8/DFF9 write-conflict behaviour during Mode 3 is specific to this group.
#[derive(Hash)]
pub struct PipelineRegisters {
    pub control: Control,
    /// DFF9 latch for full LCDC byte. `write_immediate`-only (no delayed LCDC
    /// commit path), so `control` commits the next fall — this is what keeps it
    /// in lock-step with `tile_map_select` when that crossing is also immediate.
    pub control_latch: DffLatch,
    /// The LCDC byte the tile-map-select fetch samples. DMG tracks `control`
    /// combinationally; the CGB latches a mid-Mode-3 LCDC write onto its own
    /// clock so the map-select change reaches the fetch the crossing's falls late.
    pub(in crate::ppu) tile_map_select: DffLatch,
    /// The LCDC byte the BG tile-data fetch samples (LCDC.4). DMG tracks `control`
    /// combinationally; the CGB latches a mid-Mode-3 LCDC write onto its own clock
    /// so the tile-data-select change reaches the fetch the crossing's falls late.
    pub(in crate::ppu) tile_data_select: DffLatch,
    /// The LCDC byte the sprite fetch samples for OBJ size (LCDC.2). DMG tracks
    /// `control` combinationally; the CGB latches a mid-Mode-3 LCDC write onto its
    /// own clock so the size change reaches the fetch reads the crossing's falls late.
    pub(in crate::ppu) obj_size_select: DffLatch,
    pub background_viewport: BackgroundViewportPosition,
    pub window: Window,
    pub palettes: Palettes,
    /// VYXE OLD-overlay for mid-Mode-3 LCDC.0 transitions.
    pub(in crate::ppu) bg_window_enabled_overlay: OldOverlay,
    /// XYLO popper-side OLD-overlay for mid-Mode-3 LCDC.1 transitions.
    /// Sprite-fetch trigger chain sees live XYLO, not this overlay.
    pub(in crate::ppu) sprites_enabled_overlay: OldOverlay,
    /// LCDC.1 snapshot taken at start of rise() before staged write applies; consumed by FEPO-for-TEKY (SOBU/CUPA race).
    pub(in crate::ppu) sprites_enabled_pre_cupa: bool,
    /// Falls remaining in which a CPU register write's staged value may still be
    /// resolving through a DFF crossing or OLD-overlay hold. Armed at each write,
    /// decremented per fall; the per-fall latch ticks run only while it is > 0,
    /// since every gated cell is fed solely by CPU writes and is otherwise idle.
    pub(in crate::ppu) register_write_settle: u8,
}

/// Falls to hold the settle window open for any staged register write short of
/// the HALT-wake park: the deepest crossing commits in 3 falls (OBJ-size on the
/// CGB), plus one fall for the BGP NURA overlay / OLD-overlay to clear after.
const REGISTER_CROSSING_SETTLE_FALLS: u8 = 4;

/// Falls to hold the window open for a BGP write parked in a HALT-wake handler:
/// up to 6 park falls, then one fall to commit the un-parked DFF write and one
/// more to clear the NURA overlay.
const HALT_WAKE_BGP_SETTLE_FALLS: u8 = 8;

impl PipelineRegisters {
    /// Arm the settle window for a register write whose staged value crosses or
    /// holds for at most [`REGISTER_CROSSING_SETTLE_FALLS`] falls.
    pub(in crate::ppu) fn arm_register_write_settle(&mut self) {
        self.register_write_settle = self
            .register_write_settle
            .max(REGISTER_CROSSING_SETTLE_FALLS);
    }

    /// Arm the settle window for a BGP write parked in a HALT-wake handler, which
    /// resolves over the longer [`HALT_WAKE_BGP_SETTLE_FALLS`] window.
    pub(in crate::ppu) fn arm_halt_wake_bgp_settle(&mut self) {
        self.register_write_settle = self.register_write_settle.max(HALT_WAKE_BGP_SETTLE_FALLS);
    }

    /// Per-fall work: tick palette/DFF9 latches, run the BESU↑ edge detector, then advance
    /// OLD-overlay shadows. Order matters — pipeline consumers read `reg_old` before any tick fires.
    ///
    /// The register latches are fed only by CPU writes, so their ticks run only
    /// while the settle window is open; the BESU↑ recovery edge detector is
    /// PPU-internal and runs every fall.
    pub fn tick_on_master_clock_fall(
        &mut self,
        mode2_active: bool,
        bgp_write_race: bool,
        obp_write_race: bool,
    ) {
        if self.register_write_settle > 0 {
            self.register_write_settle -= 1;

            self.palettes.tick_background(bgp_write_race);
            self.palettes.tick_sprites(obp_write_race);

            self.background_viewport.x.tick();
            self.background_viewport.y.tick();
            self.window.x.tick();
            if self.control_latch.tick() {
                self.control =
                    Control::new(ControlFlags::from_bits_retain(self.control_latch.output));
            }
            self.tile_map_select.tick();
            self.tile_data_select.tick();
            self.obj_size_select.tick();

            self.bg_window_enabled_overlay.tick();
            self.sprites_enabled_overlay.tick();
        } else {
            debug_assert!(
                self.no_gated_pending(),
                "settle window closed with a register latch still holding a staged write"
            );
        }

        self.palettes.tick_mode2_active(mode2_active);
    }

    /// Every CPU-write-fed latch has committed and cleared — nothing a skipped
    /// tick would have advanced. Backs the settle-window skip-path assertion.
    fn no_gated_pending(&self) -> bool {
        self.background_viewport.x.pending().is_none()
            && self.background_viewport.y.pending().is_none()
            && self.window.x.pending().is_none()
            && self.control_latch.pending().is_none()
            && self.tile_map_select.pending().is_none()
            && self.tile_data_select.pending().is_none()
            && self.obj_size_select.pending().is_none()
            && self.palettes.no_pending_writes()
            && self.bg_window_enabled_overlay.settled()
            && self.sprites_enabled_overlay.settled()
    }

    /// Freeze latches at their current output (LCD off).
    pub fn clear_latches(&mut self) {
        self.palettes.background.clear();
        self.palettes.sprite0.clear();
        self.palettes.sprite1.clear();
        self.palettes.clear_background_overlay();
        self.background_viewport.x.clear();
        self.background_viewport.y.clear();
        self.window.x.clear();
        self.control_latch.clear();
        self.tile_map_select.clear();
        self.tile_data_select.clear();
        self.obj_size_select.clear();
        self.bg_window_enabled_overlay.clear();
        self.sprites_enabled_overlay.clear();
        self.register_write_settle = 0;
    }

    /// The LCDC byte the tile-map-select fetch samples — the live byte on DMG,
    /// the crossing-lagged byte on CGB.
    pub fn tile_map_select_byte(&self) -> u8 {
        self.tile_map_select.output()
    }

    /// Apply an LCDC write to the tile-map-select view: immediate on DMG
    /// (`falls` = 0), or `falls` falls late on the CGB clock-domain crossing.
    pub fn write_tile_map_select(&mut self, value: u8, falls: u8) {
        self.tile_map_select.write_crossing(value, falls);
    }

    /// The tile-data addressing mode the BG fetch samples — the live LCDC.4 on
    /// DMG, the crossing-lagged bit on CGB.
    pub fn tile_data_address_mode(&self) -> TileAddressMode {
        Self::latch_control(&self.tile_data_select).tile_address_mode()
    }

    /// Apply an LCDC write to the tile-data-select view: immediate on DMG
    /// (`falls` = 0), or `falls` falls late on the CGB clock-domain crossing.
    pub fn write_tile_data_select(&mut self, value: u8, falls: u8) {
        self.tile_data_select.write_crossing(value, falls);
    }

    /// The OBJ size the sprite fetch samples — the live LCDC.2 on DMG, the
    /// crossing-lagged bit on CGB (it reaches the c2/c4 reads the crossing's
    /// falls late, so a mid-fetch size change splits the two bitplanes).
    pub fn obj_size_for_fetch(&self) -> SpriteSize {
        Self::latch_control(&self.obj_size_select).sprite_size()
    }

    /// Apply an LCDC write to the obj-size-select view: immediate on DMG
    /// (`falls` = 0), or `falls` falls late on the CGB clock-domain crossing.
    pub fn write_obj_size_select(&mut self, value: u8, falls: u8) {
        self.obj_size_select.write_crossing(value, falls);
    }

    /// Decode a crossing-lagged LCDC latch byte into a `Control` view.
    fn latch_control(latch: &DffLatch) -> Control {
        Control::new(ControlFlags::from_bits_retain(latch.output()))
    }

    /// VYXE state for the BG plane gate (RAJY/TADE), with OLD-overlay applied.
    pub fn bg_window_enabled_for_resolve(&self) -> bool {
        self.bg_window_enabled_overlay
            .resolve(self.control.background_and_window_enabled())
    }

    /// Capture pre-write VYXE if LCDC.0 transitions during Mode 3. `extra_hold`
    /// holds OLD one fall longer for the CGB clock-domain write lag.
    pub fn arm_bg_window_enabled_shadow(
        &mut self,
        old_value: bool,
        new_value: bool,
        extra_hold: u8,
    ) {
        self.bg_window_enabled_overlay
            .arm(old_value, new_value, extra_hold);
    }

    /// XYLO state for the OBJ-mux popper, with OLD-overlay applied. Sprite-fetch trigger does NOT use this.
    pub fn sprites_enabled_for_resolve(&self) -> bool {
        self.sprites_enabled_overlay
            .resolve(self.control.sprites_enabled())
    }

    /// Capture pre-write XYLO if LCDC.1 transitions during Mode 3. `extra_hold`
    /// holds OLD longer for the CGB clock-domain write lag — one fall on the CGB
    /// (the XYLO crossing), zero on DMG.
    pub fn arm_sprites_enabled_shadow(&mut self, old_value: bool, new_value: bool, extra_hold: u8) {
        self.sprites_enabled_overlay
            .arm(old_value, new_value, extra_hold);
    }
}
