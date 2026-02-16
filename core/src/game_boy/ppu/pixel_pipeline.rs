use core::fmt;

use crate::game_boy::ppu::{
    Registers,
    memory::{Oam, Vram},
    palette::PaletteIndex,
    screen::{self, Screen},
};

use super::{
    sprites::{self, SpriteId, SpriteSize},
    tiles::{TileAddressMode, TileIndex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    BetweenFrames = 1,
    PreparingScanline = 2,
    DrawingPixels = 3,
    BetweenLines = 0,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::BetweenFrames => write!(f, "Between Frames"),
            Mode::PreparingScanline => write!(f, "Preparing Scanline"),
            Mode::DrawingPixels => write!(f, "Drawing Pixels"),
            Mode::BetweenLines => write!(f, "Between Scanlines"),
        }
    }
}

const SCANLINE_TOTAL_DOTS: u32 = 456;
const SCANLINE_PREPARING_DOTS: u32 = 80;
/// Pipeline priming begins 1 dot after Mode 2 ends. STAT reports Mode 3
/// from dot 80, pipeline priming runs at dots 81-83, then the fetcher/shifter
/// activate at dot 84.
const SCANLINE_RENDERING_DOTS: u32 = SCANLINE_PREPARING_DOTS + 1;
/// Dots of pipeline priming at the start of Mode 3, before the
/// fetcher begins. The position counter increments during these
/// dots for trigger evaluation, but no tile fetch occurs.
const PIPELINE_PRIMING_DOTS: u8 = 3;
/// On the first scanline after LCD turn-on, the pixel pipeline activates
/// 8 dots after Mode 2 ends (vs 1 dot on normal lines). The hardware's
/// first Mode 0 is correspondingly shorter.
const FIRST_SCANLINE_PIPELINE_DELAY: u32 = 8;
/// Hardware pixel counter value at which WODU fires (hblank gate).
/// XUGU = NAND5(PX0, PX1, PX2, PX5, PX7) decodes 128+32+4+2+1 = 167.
const WODU_PIXEL_COUNT: u8 = 167;
/// First pixel counter value that produces a visible LCD pixel.
/// On hardware, the LCD X coordinate is `pix_count - 8`. Pixels at
/// PX 0–7 shift the first tile's data through the pipe invisibly.
const FIRST_VISIBLE_PIXEL: u8 = 8;
const BETWEEN_FRAMES_DOTS: u32 = SCANLINE_TOTAL_DOTS * 10;
const MAX_SPRITES_PER_LINE: usize = 10;

// --- Pixel shift registers ---
//
// On hardware (pages 32-34), each pixel layer uses separate 8-bit shift
// registers for each bitplane. Tile data is loaded in parallel and shifted
// out one bit per dot. The 2-bit color index is only formed at the pixel
// mux (page 35) by combining the two bitplane outputs.

/// Background pixel shift register (page 32 on the die).
///
/// Two 8-bit shift registers, one per bitplane (BgwPipeA/BgwPipeB).
/// Loaded in parallel from a BG/window tile fetch, shifted out one
/// bit per dot. The "FIFO is empty" condition from Pan Docs corresponds
/// to `len == 0` (all bits have been shifted out).
struct BgShifter {
    low: u8,
    high: u8,
    len: u8,
}

impl BgShifter {
    fn new() -> Self {
        Self {
            low: 0,
            high: 0,
            len: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn len(&self) -> u8 {
        self.len
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    /// Parallel load from a tile fetch. On hardware, the DFF22 shift
    /// register cells use async SET/RST pins, so a load unconditionally
    /// overwrites the current contents (SEKO pre-load at tile boundaries).
    fn load(&mut self, low: u8, high: u8) {
        self.low = low;
        self.high = high;
        self.len = 8;
    }

    /// Shift out one pixel's bitplane values (MSB first, matching hardware).
    /// Returns (low_bit, high_bit) — the 2-bit color is `(high << 1) | low`.
    fn shift(&mut self) -> (u8, u8) {
        debug_assert!(self.len > 0);
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        self.low <<= 1;
        self.high <<= 1;
        self.len -= 1;
        (lo, hi)
    }
}

/// Sprite pixel shift register (pages 33-34 on the die).
///
/// Four parallel 8-bit shift registers matching the hardware:
/// - `low`/`high`: sprite bitplanes (SprPipeA/SprPipeB, page 33)
/// - `palette`: palette selection bit per pixel (PalPipe, page 34)
/// - `priority`: BG-over-OBJ priority bit per pixel (MaskPipe, page 26)
///
/// Unlike the BG shifter, sprites are merged into the OBJ shifter with
/// transparency-aware logic: only non-zero (opaque) sprite pixels
/// overwrite existing transparent slots. This implements DMG sprite
/// priority (lower X / lower OAM index wins).
struct ObjShifter {
    low: u8,
    high: u8,
    palette: u8,
    priority: u8,
    len: u8,
}

impl ObjShifter {
    fn new() -> Self {
        Self {
            low: 0,
            high: 0,
            palette: 0,
            priority: 0,
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.low = 0;
        self.high = 0;
        self.palette = 0;
        self.priority = 0;
        self.len = 0;
    }

    /// Shift out one pixel's data (MSB first). Returns None if empty.
    /// When non-empty, returns (low_bit, high_bit, palette_bit, priority_bit).
    fn shift(&mut self) -> Option<(u8, u8, u8, u8)> {
        if self.len == 0 {
            return None;
        }
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (self.palette >> 7) & 1;
        let pri = (self.priority >> 7) & 1;
        self.low <<= 1;
        self.high <<= 1;
        self.palette <<= 1;
        self.priority <<= 1;
        self.len -= 1;
        Some((lo, hi, pal, pri))
    }

    /// Merge sprite tile data into the shifter with transparency-aware
    /// logic. Only non-zero (opaque) sprite pixels overwrite existing
    /// transparent (color 0) slots.
    ///
    /// `sprite_low`/`sprite_high` are the raw bitplane bytes from the
    /// sprite tile fetch (already X-flipped if needed). `palette_bit`
    /// and `priority_bit` are uniform for all 8 pixels of this sprite.
    /// `pixels_clipped_left` is how many MSB pixels to skip (for sprites
    /// partially off the left edge). `bg_len` is the current BG shifter
    /// length, used to determine padding.
    fn merge(
        &mut self,
        sprite_low: u8,
        sprite_high: u8,
        palette_bit: u8,
        priority_bit: u8,
        pixels_clipped_left: u8,
        bg_len: u8,
    ) {
        // Ensure the shifter is long enough to hold all visible sprite pixels.
        let visible_pixels = 8 - pixels_clipped_left;
        let required_len = bg_len.max(visible_pixels);
        if self.len < required_len {
            // Pad with transparent pixels (zeros) — just extend the length.
            // The shift register bits are already 0 in the extended positions
            // because we shift left and the low bits are 0.
            self.len = required_len;
        }

        // Overlay sprite pixels. Only replace transparent (color 0) slots.
        // Work MSB-first, skipping clipped pixels.
        for i in pixels_clipped_left..8 {
            let bit_pos = 7 - i;
            let lo = (sprite_low >> bit_pos) & 1;
            let hi = (sprite_high >> bit_pos) & 1;
            let color = (hi << 1) | lo;
            if color == 0 {
                continue; // Transparent sprite pixel — don't overwrite
            }

            // Position in the shifter (0 = MSB = next to shift out)
            let shifter_bit = 7 - (i - pixels_clipped_left);
            let existing_lo = (self.low >> shifter_bit) & 1;
            let existing_hi = (self.high >> shifter_bit) & 1;
            let existing_color = (existing_hi << 1) | existing_lo;
            if existing_color != 0 {
                continue; // Existing opaque pixel wins (DMG priority)
            }

            // Write this sprite's pixel into the slot
            let mask = 1 << shifter_bit;
            self.low = (self.low & !mask) | (lo << shifter_bit);
            self.high = (self.high & !mask) | (hi << shifter_bit);
            self.palette = (self.palette & !mask) | (palette_bit << shifter_bit);
            self.priority = (self.priority & !mask) | (priority_bit << shifter_bit);
        }
    }
}

// --- Fine scroll (ROXY pixel clock gate) ---

/// Hardware fine scroll counter (RYKU/ROGA/RUBU) and pixel clock
/// gate (ROXY). The ROXY latch gates the pixel clock (SACU) until
/// the counter matches SCX & 7, implementing sub-tile fine scrolling.
struct FineScroll {
    /// 3-bit counter (0–7).
    count: u8,
    /// ROXY latch. true = pixel clock gated (scrolling not done).
    /// Clears when count == SCX & 7 (one-shot per line).
    gating: bool,
}

impl FineScroll {
    fn new() -> Self {
        Self {
            count: 0,
            gating: true,
        }
    }

    /// Whether the pixel clock is active (SACU ungated).
    fn pixel_clock_active(&self) -> bool {
        !self.gating
    }

    /// Advance the fine counter by one dot (PECU clock).
    fn tick(&mut self) {
        self.count = (self.count + 1) & 7;
    }

    /// Check and clear the gating latch if count matches SCX & 7.
    /// One-shot: once cleared, stays cleared for the rest of the line.
    fn check_scroll_match(&mut self, scx: u8) {
        if self.gating && self.count == (scx & 7) {
            self.gating = false;
        }
    }

    /// Reset for window trigger — counter resets, gating clears
    /// (window has no fine scroll).
    fn reset_for_window(&mut self) {
        self.count = 0;
        self.gating = false;
    }
}

// --- Background fetcher ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetcherStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
    Load,
}

/// Mode 3 starts with pipeline priming (3 dots) then one BG tile fetch
/// (6 dots) before any pixels shift out. During priming the position
/// counter increments for trigger evaluation but no tile fetch occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupFetch {
    /// Pipeline priming — position counter increments, trigger checks run,
    /// but the fetcher does not advance. Models the gap between Mode 3
    /// start and first fetcher activation.
    PipelinePriming { dots_remaining: u8 },
    /// Single tile fetch — loads the first tile into the BG shifter.
    /// When the shifter becomes non-empty, startup is complete and
    /// normal rendering begins on the next dot.
    FirstTile,
}

struct TileFetcher {
    step: FetcherStep,
    /// Sub-dot counter within the current step (0 or 1 for 2-dot steps).
    dot_in_step: u8,
    /// Tile X coordinate in the tilemap row (increments per tile fetched).
    tile_x: u8,
    /// Cached tile index from GetTile step.
    tile_index: u8,
    /// Cached low byte of tile row from GetTileDataLow step.
    tile_data_low: u8,
    /// Cached high byte of tile row from GetTileDataHigh step.
    tile_data_high: u8,
    /// Whether we're fetching from the window tilemap.
    fetching_window: bool,
}

impl TileFetcher {
    fn new() -> Self {
        Self {
            step: FetcherStep::GetTile,
            dot_in_step: 0,
            tile_x: 0,
            tile_index: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
        }
    }
}

// --- Sprite store ---

/// One entry in the hardware's 10-slot sprite store register file.
/// Written during Mode 2 OAM scan, read during Mode 3 sprite fetch.
#[derive(Clone, Copy)]
struct SpriteStoreEntry {
    /// OAM sprite number (0-39). The hardware stores this as a 6-bit
    /// value. Used during Mode 3 to look up tile index and attributes
    /// from OAM via the sprite fetcher.
    oam_index: u8,
    /// Which row of the sprite falls on this scanline (0-15).
    /// Pre-computed during Mode 2 so the sprite fetcher can generate
    /// a VRAM tile address without re-reading OAM Y position.
    line_offset: u8,
    /// X position (the raw x_plus_8 value from OAM byte 1).
    /// Compared against the pixel position counter by the X matchers
    /// during Mode 3.
    x: u8,
}

/// The hardware's 10-entry sprite store. Populated during Mode 2 OAM scan,
/// consumed during Mode 3 by the X matchers and sprite fetcher.
struct SpriteStore {
    entries: [SpriteStoreEntry; MAX_SPRITES_PER_LINE],
    /// Number of entries written during this line's OAM scan (0-10).
    count: u8,
}

impl SpriteStore {
    fn new() -> Self {
        Self {
            entries: [SpriteStoreEntry {
                oam_index: 0,
                line_offset: 0,
                x: 0,
            }; MAX_SPRITES_PER_LINE],
            count: 0,
        }
    }
}

// --- Sprite fetch ---

/// The two phases of a sprite fetch on real hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpriteFetchPhase {
    /// The BG fetcher continues advancing through its normal steps.
    /// The wait ends when the fetcher has completed GetTileDataHigh
    /// (reached Load) AND the BG shifter is non-empty — both conditions
    /// must be true simultaneously. The variable sprite penalty (0-5
    /// dots) emerges from how many fetcher steps this phase consumes.
    WaitingForFetcher,
    /// The BG fetcher is frozen at its current position. Sprite tile
    /// data is read through the SpriteStep state machine (6 dots total).
    FetchingData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpriteStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
}

struct SpriteFetch {
    /// The sprite store entry that triggered this fetch.
    entry: SpriteStoreEntry,
    phase: SpriteFetchPhase,
    step: SpriteStep,
    dot_in_step: u8,
    tile_data_low: u8,
    tile_data_high: u8,
}

// --- Line state ---

struct Line {
    number: u8,
    dots: u32,
    /// Sprites on this line, stored as hardware register file entries.
    sprites: SpriteStore,
    /// Index of the next sprite store entry to check for triggering.
    next_sprite: u8,
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
    /// Hardware pixel counter (XEHO-SYBE, page 21). Counts from 0 when
    /// the pixel clock starts after startup. Drives WODU (hblank gate)
    /// at PX=167. Not reset on window trigger — PX is a monotonic
    /// per-line counter.
    pixel_counter: u8,
    /// Active sprite fetch, if any.
    sprite_fetch: Option<SpriteFetch>,
    /// Dot at which the pixel pipeline begins executing in Mode 3.
    /// Normally SCANLINE_PREPARING_DOTS + 4 (84); on the first scanline
    /// after LCD turn-on, SCANLINE_PREPARING_DOTS + 11 (91).
    rendering_start_dot: u32,
}

impl Line {
    fn new(number: u8) -> Self {
        Line {
            number,
            dots: 0,
            sprites: SpriteStore::new(),
            next_sprite: 0,
            window_rendered: false,
            bg_shifter: BgShifter::new(),
            obj_shifter: ObjShifter::new(),
            fetcher: TileFetcher::new(),
            startup_fetch: Some(StartupFetch::PipelinePriming {
                dots_remaining: PIPELINE_PRIMING_DOTS,
            }),
            fine_scroll: FineScroll::new(),
            pixel_counter: 0,
            sprite_fetch: None,
            rendering_start_dot: SCANLINE_RENDERING_DOTS,
        }
    }

    fn find_sprites(&mut self, data: &Registers, oam: &Oam) {
        let sprite_size = data.control.sprite_size();
        let mut count = 0u8;
        for (oam_index, sprite) in oam.sprites().iter().enumerate() {
            if count as usize >= MAX_SPRITES_PER_LINE {
                break;
            }
            if sprite.position.on_line(self.number, sprite_size) {
                let line_offset =
                    (self.number as i16 + 16 - sprite.position.y_plus_16 as i16) as u8;
                self.sprites.entries[count as usize] = SpriteStoreEntry {
                    oam_index: oam_index as u8,
                    line_offset,
                    x: sprite.position.x_plus_8,
                };
                count += 1;
            }
        }
        self.sprites.count = count;

        // DMG priority: lower X position wins, ties broken by OAM order (stable sort)
        self.sprites.entries[..count as usize].sort_by_key(|entry| entry.x);

        self.fine_scroll = FineScroll::new();
    }
}

// --- Rendering ---

pub struct Rendering {
    screen: Screen,
    line: Line,
    window_line_counter: u8,
    /// After LCD enable, the first line's Mode 2 doesn't begin at dot 0.
    /// The STAT mode bits read as 0 until Mode 2 actually starts.
    lcd_turning_on: bool,
    /// Hardware rendering latch (XYMU, page 21). True = Mode 3 active.
    /// Gates VRAM/OAM access, pixel pipeline clocks.
    rendering: bool,
    /// Hardware HBlank gate (WODU, page 21). True = pixel counter reached
    /// 160 and no sprite match active. Pixel clock stops immediately;
    /// rendering latch clears on the next dot.
    hblank_gate: bool,
}

impl Rendering {
    fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            line: Line::new(0),
            window_line_counter: 0,
            lcd_turning_on: false,
            rendering: false,
            hblank_gate: false,
        }
    }

    fn new_lcd_on() -> Self {
        let mut line = Line::new(0);
        line.rendering_start_dot = SCANLINE_PREPARING_DOTS + FIRST_SCANLINE_PIPELINE_DELAY;
        Rendering {
            screen: Screen::new(),
            line,
            window_line_counter: 0,
            lcd_turning_on: true,
            rendering: false,
            hblank_gate: false,
        }
    }

    fn mode(&self) -> Mode {
        if self.rendering {
            Mode::DrawingPixels
        } else if self.line.dots < SCANLINE_PREPARING_DOTS {
            Mode::PreparingScanline
        } else {
            Mode::BetweenLines
        }
    }

    fn stat_mode(&self) -> Mode {
        self.interrupt_mode()
    }

    /// Mode for STAT interrupt edge detection. Mode 0 fires from
    /// WODU (hblank_gate) directly — one dot before XYMU clears.
    fn interrupt_mode(&self) -> Mode {
        if self.hblank_gate {
            Mode::BetweenLines
        } else if self.rendering {
            Mode::DrawingPixels
        } else if self.line.dots < SCANLINE_PREPARING_DOTS {
            Mode::PreparingScanline
        } else {
            Mode::BetweenLines
        }
    }

    /// Whether the mode 2 STAT interrupt condition is active.
    fn mode2_interrupt_active(&self) -> bool {
        // On hardware, lines 1+ get an early Mode 2 pre-trigger at clock 0
        // from the previous HBlank pre-setting mode_for_interrupt. Line 0
        // has no previous HBlank, so Mode 2 STAT fires at clock 4 instead.
        self.mode() == Mode::PreparingScanline && (self.line.number != 0 || self.line.dots >= 4)
    }

    fn oam_locked(&self) -> bool {
        self.rendering || self.line.dots < SCANLINE_PREPARING_DOTS
    }

    fn vram_locked(&self) -> bool {
        self.rendering
    }

    fn oam_write_locked(&self) -> bool {
        self.rendering || self.line.dots < SCANLINE_PREPARING_DOTS
    }

    fn vram_write_locked(&self) -> bool {
        self.rendering
    }

    /// Advance by one dot (T-cycle). Returns true when a full frame is complete.
    fn dot(&mut self, data: &Registers, oam: &Oam, vram: &Vram) -> bool {
        if self.line.dots == 0 {
            self.line.find_sprites(data, oam);
        }

        if self.line.dots < SCANLINE_PREPARING_DOTS {
            // Mode 2: OAM scan
            self.line.dots += 1;
            if self.line.dots == SCANLINE_PREPARING_DOTS {
                self.lcd_turning_on = false;
                self.rendering = true;
            }
        } else {
            // Clear rendering latch when hblank gate fires. On hardware,
            // WODU (PX=167) feeds VOGA/WEGO to clear XYMU with a 1-dot
            // delay. Since hblank_gate is set at the end of the dot where
            // PX=167, checking it here (start of next dot) gives the delay.
            if self.hblank_gate && self.rendering {
                self.rendering = false;
            }

            // Mode 3 (drawing) and Mode 0 (HBlank)
            if self.rendering && self.line.dots >= self.line.rendering_start_dot {
                self.dot_mode3(data, oam, vram);
            }

            // Set hblank_gate when pixel counter reaches WODU threshold
            // and no sprite fetch is active
            // (WODU = AND(XENA_STORE_MATCHn, XANO_PX167p))
            if self.rendering
                && self.line.pixel_counter >= WODU_PIXEL_COUNT
                && self.line.sprite_fetch.is_none()
            {
                self.hblank_gate = true;
            }

            self.line.dots += 1;

            if self.line.dots == SCANLINE_TOTAL_DOTS {
                self.rendering = false;
                self.hblank_gate = false;
                if self.line.window_rendered {
                    self.window_line_counter += 1;
                }
                self.line = Line::new(self.line.number + 1);

                if self.line.number == screen::NUM_SCANLINES {
                    return true;
                }
            }
        }

        false
    }

    /// One dot of Mode 3 pixel pipeline processing.
    fn dot_mode3(&mut self, data: &Registers, oam: &Oam, vram: &Vram) {
        if let Some(phase) = self.line.startup_fetch {
            match phase {
                StartupFetch::PipelinePriming { dots_remaining } => {
                    // Pipeline priming: trigger checks, but no fetcher advance.
                    self.check_window_trigger(data);
                    if self.line.sprite_fetch.is_none() {
                        self.check_sprite_trigger(data);
                    }
                    self.line.startup_fetch = if dots_remaining > 1 {
                        Some(StartupFetch::PipelinePriming {
                            dots_remaining: dots_remaining - 1,
                        })
                    } else {
                        Some(StartupFetch::FirstTile)
                    };
                }
                StartupFetch::FirstTile => {
                    self.advance_bg_fetcher(data, vram);
                    self.check_window_trigger(data);

                    // Startup fetch completes when the fetcher loads pixels
                    // into the previously-empty shifter (detected by shifter
                    // becoming non-empty after the fetcher advance).
                    if !self.line.bg_shifter.is_empty() {
                        self.line.startup_fetch = None;
                    }
                }
            }
            return;
        }

        // Update fine scroll gating before sprite/pixel checks. On hardware,
        // the ROXY latch clears combinationally when the fine counter matches
        // SCX & 7. SACU (pixel clock) and sprite matchers both see the
        // updated state on the same dot.
        if self.line.startup_fetch.is_none() {
            self.line
                .fine_scroll
                .check_scroll_match(data.background_viewport.x);
        }

        if let Some(ref mut sf) = self.line.sprite_fetch {
            match sf.phase {
                SpriteFetchPhase::WaitingForFetcher => {
                    // The BG fetcher continues advancing during the wait.
                    // This is the hardware behavior: the fetcher keeps
                    // stepping through its enum states, doing real tile
                    // fetches that may load pixels into the shifter.
                    self.advance_bg_fetcher(data, vram);

                    // Wait exits when BOTH conditions are met:
                    // 1. The fetcher has completed GetTileDataHigh (reached Load)
                    // 2. The BG shifter is non-empty
                    // This is an AND condition — both must be true simultaneously.
                    let fetcher_past_data = self.line.fetcher.step == FetcherStep::Load;
                    let wait_done = fetcher_past_data && !self.line.bg_shifter.is_empty();

                    if wait_done {
                        // Freeze the BG fetcher at its current position.
                        // It stays wherever the wait left it (typically Load)
                        // and resumes from there after the sprite data fetch.

                        // Transition to sprite data fetch. The first sprite
                        // fetch step happens on the same dot as the wait
                        // exit — the transition itself does not consume a dot.
                        let sf = self.line.sprite_fetch.as_mut().unwrap();
                        sf.phase = SpriteFetchPhase::FetchingData;
                        Self::advance_sprite_fetch(sf, data, oam, vram);
                    }
                }
                SpriteFetchPhase::FetchingData => {
                    // BG fetcher is frozen. Advance the sprite data pipeline.
                    Self::advance_sprite_fetch(sf, data, oam, vram);
                    if sf.step == SpriteStep::GetTileDataHigh && sf.dot_in_step == 2 {
                        Self::merge_sprite_into_obj_shifter(
                            sf,
                            oam,
                            &self.line.bg_shifter,
                            &mut self.line.obj_shifter,
                        );
                        self.line.sprite_fetch = None;
                    }
                }
            }
        } else {
            // Pixel counter increment (change D). On hardware, pix_count
            // increments on SACU (pixel clock), gated by POKY (after startup)
            // and ROXY (fine scroll). Sprite matching uses state_new.pix_count
            // (post-increment), so the increment must precede trigger checks.
            if self.line.fine_scroll.pixel_clock_active() {
                self.line.pixel_counter += 1;
            }

            // Sprite trigger check — now uses pixel_counter (change A).
            if self.line.sprite_fetch.is_none() {
                self.check_sprite_trigger(data);
            }

            // SEKO pre-load (change C). On hardware, when fine_count==7
            // the async SET/RST pipe load overwrites the shift register
            // contents before the clock-driven shift on the same dot.
            if self.line.fine_scroll.count == 7
                && self.line.fetcher.step == FetcherStep::Load
                && self.line.bg_shifter.len() == 1
            {
                self.load_bg_tile();
            }

            if !self.line.bg_shifter.is_empty() {
                self.shift_pixel_out(data);
            }

            self.advance_bg_fetcher(data, vram);
            self.line.fine_scroll.tick();
        }
    }

    // --- Address generation (pages 26-27) ---
    //
    // On the die, BG and window have separate address generators:
    //   Page 26 (BACKGROUND): tilemap coords from SCX, SCY, tile_x, LY
    //   Page 27 (WINDOW MAP LOOKUP): tilemap coords from tile_x, window_line_counter
    // Both feed into the shared VRAM interface (page 25).

    /// BG tilemap coordinate computation (page 26).
    /// Applies SCX/SCY scroll offsets and wraps at 32-tile boundaries.
    fn bg_tilemap_coords(&self, data: &Registers) -> (u8, u8) {
        let scx = data.background_viewport.x;
        let scy = data.background_viewport.y;
        (
            (self.line.fetcher.tile_x.wrapping_add(scx / 8)) & 31,
            (self.line.number.wrapping_add(scy) / 8) & 31,
        )
    }

    /// Window tilemap coordinate computation (page 27).
    /// Uses the window's internal line counter, no scroll offset.
    fn window_tilemap_coords(&self) -> (u8, u8) {
        (self.line.fetcher.tile_x, self.window_line_counter / 8)
    }

    /// Read the tile index from the tilemap for the current fetch position.
    fn read_tile_index(&self, data: &Registers, vram: &Vram) -> u8 {
        let (map_x, map_y) = if self.line.fetcher.fetching_window {
            self.window_tilemap_coords()
        } else {
            self.bg_tilemap_coords(data)
        };

        let map_id = if self.line.fetcher.fetching_window {
            data.control.window_tile_map()
        } else {
            data.control.background_tile_map()
        };
        vram.tile_map(map_id).get_tile(map_x, map_y).0
    }

    /// BG fine Y offset (page 26): which row within the tile, from SCY + LY.
    fn bg_fine_y(&self, data: &Registers) -> u8 {
        self.line.number.wrapping_add(data.background_viewport.y) % 8
    }

    /// Window fine Y offset (page 27): which row within the tile, from
    /// the window's internal line counter.
    fn window_fine_y(&self) -> u8 {
        self.window_line_counter % 8
    }

    /// Read one byte of tile data (low or high bitplane) for the
    /// current BG/window fetch.
    ///
    /// The tile data address combines the tile index (cached from the
    /// tilemap read) with the fine Y offset from the appropriate
    /// address generator. The VRAM interface (page 25) performs the read.
    fn read_tile_data(&self, data: &Registers, vram: &Vram, high: bool) -> u8 {
        let tile_index = TileIndex(self.line.fetcher.tile_index);
        let (block_id, mapped_idx) = data.control.tile_address_mode().tile(tile_index);

        let fine_y = if self.line.fetcher.fetching_window {
            self.window_fine_y()
        } else {
            self.bg_fine_y(data)
        };

        let block = vram.tile_block(block_id);
        block.data[mapped_idx.0 as usize * 16 + fine_y as usize * 2 + high as usize]
    }

    /// Advance the background tile fetcher by one dot.
    fn advance_bg_fetcher(&mut self, data: &Registers, vram: &Vram) {
        match self.line.fetcher.step {
            FetcherStep::GetTile => {
                if self.line.fetcher.dot_in_step == 0 {
                    self.line.fetcher.dot_in_step = 1;
                } else {
                    self.line.fetcher.tile_index = self.read_tile_index(data, vram);
                    self.line.fetcher.dot_in_step = 0;
                    self.line.fetcher.step = FetcherStep::GetTileDataLow;
                }
            }
            FetcherStep::GetTileDataLow => {
                if self.line.fetcher.dot_in_step == 0 {
                    self.line.fetcher.dot_in_step = 1;
                } else {
                    self.line.fetcher.tile_data_low = self.read_tile_data(data, vram, false);
                    self.line.fetcher.dot_in_step = 0;
                    self.line.fetcher.step = FetcherStep::GetTileDataHigh;
                }
            }
            FetcherStep::GetTileDataHigh => {
                if self.line.fetcher.dot_in_step == 0 {
                    self.line.fetcher.dot_in_step = 1;
                } else {
                    self.line.fetcher.tile_data_high = self.read_tile_data(data, vram, true);

                    // Load is instant when the shifter is empty (no
                    // additional dot cost). Otherwise enter the Load
                    // step to wait for it to drain.
                    if self.line.bg_shifter.is_empty() {
                        self.load_bg_tile();
                    } else {
                        self.line.fetcher.dot_in_step = 0;
                        self.line.fetcher.step = FetcherStep::Load;
                    }
                }
            }
            FetcherStep::Load => {
                // Shifter was not empty when DataHigh completed. Wait here
                // until it drains, then load.
                if self.line.bg_shifter.is_empty() {
                    self.load_bg_tile();
                }
            }
        }
    }

    /// Load fetched tile data into the BG shifter and reset the fetcher to
    /// GetTile for the next tile.
    fn load_bg_tile(&mut self) {
        self.line.bg_shifter.load(
            self.line.fetcher.tile_data_low,
            self.line.fetcher.tile_data_high,
        );
        self.line.fetcher.tile_x = self.line.fetcher.tile_x.wrapping_add(1);
        self.line.fetcher.step = FetcherStep::GetTile;
        self.line.fetcher.dot_in_step = 0;
    }

    /// Read one byte of sprite tile data (low or high bitplane).
    ///
    /// On the die, the sprite fetcher (page 29) uses the OAM index
    /// from the sprite store to look up the tile index and attributes,
    /// then generates a VRAM address from the tile index, line offset,
    /// and flip flags. The VRAM interface (page 25) performs the read.
    fn read_sprite_tile_data(
        sf: &SpriteFetch,
        data: &Registers,
        oam: &Oam,
        vram: &Vram,
        high: bool,
    ) -> u8 {
        let sprite = oam.sprite(SpriteId(sf.entry.oam_index));
        let tile_index = if data.control.sprite_size() == SpriteSize::Double {
            TileIndex(sprite.tile.0 & 0xFE)
        } else {
            sprite.tile
        };
        let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

        let flipped_y = if sprite.attributes.flip_y() {
            (data.control.sprite_size().height() as i16 - 1 - sf.entry.line_offset as i16) as u8
        } else {
            sf.entry.line_offset
        };

        let (final_block, final_idx, final_y) = if flipped_y < 8 {
            (block_id, mapped_idx, flipped_y)
        } else {
            (block_id, TileIndex(mapped_idx.0 + 1), flipped_y - 8)
        };

        let block = vram.tile_block(final_block);
        block.data[final_idx.0 as usize * 16 + final_y as usize * 2 + high as usize]
    }

    /// Advance the sprite fetch pipeline by one dot.
    fn advance_sprite_fetch(sf: &mut SpriteFetch, data: &Registers, oam: &Oam, vram: &Vram) {
        match sf.step {
            SpriteStep::GetTile => {
                if sf.dot_in_step == 0 {
                    sf.dot_in_step = 1;
                } else {
                    // Tile index comes from OAM via the sprite store's oam_index
                    sf.dot_in_step = 0;
                    sf.step = SpriteStep::GetTileDataLow;
                }
            }
            SpriteStep::GetTileDataLow => {
                if sf.dot_in_step == 0 {
                    sf.dot_in_step = 1;
                } else {
                    sf.tile_data_low = Self::read_sprite_tile_data(sf, data, oam, vram, false);
                    sf.dot_in_step = 0;
                    sf.step = SpriteStep::GetTileDataHigh;
                }
            }
            SpriteStep::GetTileDataHigh => {
                if sf.dot_in_step == 0 {
                    sf.dot_in_step = 1;
                } else {
                    sf.tile_data_high = Self::read_sprite_tile_data(sf, data, oam, vram, true);
                    // Signal completion. Use dot_in_step = 2 to distinguish
                    // from the initial entry state (dot_in_step = 0).
                    sf.dot_in_step = 2;
                }
            }
        }
    }

    /// Merge fetched sprite pixels into the OBJ shifter.
    fn merge_sprite_into_obj_shifter(
        sf: &SpriteFetch,
        oam: &Oam,
        bg_shifter: &BgShifter,
        obj_shifter: &mut ObjShifter,
    ) {
        let sprite = oam.sprite(SpriteId(sf.entry.oam_index));

        // X-flip: hardware reverses the bit order when loading the shift
        // register. For normal sprites, MSB shifts out first (leftmost pixel).
        // For flipped sprites, LSB shifts out first — achieved by reversing
        // the byte's bit order before loading.
        let sprite_low = if sprite.attributes.flip_x() {
            sf.tile_data_low.reverse_bits()
        } else {
            sf.tile_data_low
        };
        let sprite_high = if sprite.attributes.flip_x() {
            sf.tile_data_high.reverse_bits()
        } else {
            sf.tile_data_high
        };

        let palette_bit = if sprite.attributes.contains(sprites::Attributes::PALETTE) {
            1
        } else {
            0
        };
        let priority_bit = if sprite.attributes.contains(sprites::Attributes::PRIORITY) {
            1
        } else {
            0
        };

        // Sprites partially off-screen left: skip the clipped pixels
        let sprite_screen_x = sf.entry.x as i16 - 8;
        let pixels_clipped_left = if sprite_screen_x < 0 {
            (-sprite_screen_x) as u8
        } else {
            0
        };

        obj_shifter.merge(
            sprite_low,
            sprite_high,
            palette_bit,
            priority_bit,
            pixels_clipped_left,
            bg_shifter.len(),
        );
    }

    /// Pixel mux (page 35 on the die).
    ///
    /// Shifts one bit from each shift register, forms the 2-bit color
    /// indices, applies priority logic, selects the winning pixel, and
    /// maps it through the appropriate palette to the LCD.
    fn shift_pixel_out(&mut self, data: &Registers) {
        // Shift one bit from each BG bitplane
        let (bg_lo, bg_hi) = self.line.bg_shifter.shift();

        // Shift OBJ in lockstep (if it has pixels)
        let obj_bits = self.line.obj_shifter.shift();

        // During fine scroll gating (ROXY active), the pixel clock is
        // frozen on hardware — SACU is held high, PX does not increment,
        // no LCD output. The shifters still advance here (unlike true
        // hardware gating) to keep sprite alignment consistent with the
        // existing sprite fetch model.
        if !self.line.fine_scroll.pixel_clock_active() {
            return;
        }

        // PX 1 through FIRST_VISIBLE_PIXEL-1 are invisible — the first
        // tile shifts through the pipe without writing to the framebuffer.
        // On hardware, the LCD clock gate (WUSA) doesn't open until PX=8.
        if self.line.pixel_counter < FIRST_VISIBLE_PIXEL {
            self.check_window_trigger(data);
            return;
        }

        // Past the visible region — safety guard for dots between WODU
        // and rendering latch clearing.
        if self.line.pixel_counter >= FIRST_VISIBLE_PIXEL + screen::PIXELS_PER_LINE {
            return;
        }

        let x = self.line.pixel_counter - FIRST_VISIBLE_PIXEL;
        let y = self.line.number;

        // Form 2-bit BG color index (0 if BG/window disabled via LCDC.0)
        let bg_color = if data.control.background_and_window_enabled() {
            (bg_hi << 1) | bg_lo
        } else {
            0
        };

        // Sprite priority mixing
        if data.control.sprites_enabled() {
            if let Some((spr_lo, spr_hi, spr_pal, spr_pri)) = obj_bits {
                let spr_color = (spr_hi << 1) | spr_lo;
                if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
                    // Sprite pixel wins
                    let sprite_palette = if spr_pal == 0 {
                        &data.palettes.sprite0
                    } else {
                        &data.palettes.sprite1
                    };
                    let mapped = sprite_palette.map(PaletteIndex(spr_color));
                    self.screen.set_pixel(x, y, mapped);
                    self.check_window_trigger(data);
                    return;
                }
            }
        }

        // Background pixel
        let mapped = data.palettes.background.map(PaletteIndex(bg_color));
        self.screen.set_pixel(x, y, mapped);
        self.check_window_trigger(data);
    }

    /// Check if the window should start rendering at the current pixel position.
    fn check_window_trigger(&mut self, data: &Registers) {
        if self.line.fetcher.fetching_window {
            return;
        }
        if !data.control.window_enabled() {
            return;
        }
        if self.line.number < data.window.y {
            return;
        }
        if self.line.pixel_counter != data.window.x_plus_7 {
            return;
        }

        // Window trigger: clear shifters, reset fine scroll, and restart fetcher
        self.line.bg_shifter.clear();
        self.line.obj_shifter.clear();
        self.line.fine_scroll.reset_for_window();
        self.line.fetcher.step = FetcherStep::GetTile;
        self.line.fetcher.dot_in_step = 0;
        self.line.fetcher.tile_x = 0;
        self.line.fetcher.fetching_window = true;
        self.line.window_rendered = true;
    }

    /// Check if a sprite should start fetching at the current pixel position.
    fn check_sprite_trigger(&mut self, data: &Registers) {
        if !data.control.sprites_enabled() {
            return;
        }

        let match_x = self.line.pixel_counter;

        while (self.line.next_sprite as usize) < self.line.sprites.count as usize {
            let entry = &self.line.sprites.entries[self.line.next_sprite as usize];

            if entry.x < match_x {
                // Sprite already passed — skip it
                self.line.next_sprite += 1;
                continue;
            }

            if entry.x != match_x {
                // Sprite not yet reached
                break;
            }

            if entry.x >= 168 {
                self.line.next_sprite += 1;
                continue;
            }

            self.line.sprite_fetch = Some(SpriteFetch {
                entry: *entry,
                phase: SpriteFetchPhase::WaitingForFetcher,
                step: SpriteStep::GetTile,
                dot_in_step: 0,
                tile_data_low: 0,
                tile_data_high: 0,
            });
            self.line.next_sprite += 1;
            break;
        }
    }
}

// --- PixelPipeline enum ---

pub enum PixelPipeline {
    Rendering(Rendering),
    BetweenFrames(u32),
}

impl PixelPipeline {
    pub fn new() -> Self {
        Self::Rendering(Rendering::new())
    }

    /// Create a PPU for an LCD-on transition (LCDC bit 7 set after being
    /// clear). The first line reports mode 0 in STAT until the OAM scan
    /// begins internally.
    pub fn new_lcd_on() -> Self {
        Self::Rendering(Rendering::new_lcd_on())
    }

    pub fn current_line(&self) -> u8 {
        match self {
            PixelPipeline::Rendering(Rendering {
                line: Line { number, dots, .. },
                ..
            }) => {
                if *dots >= SCANLINE_TOTAL_DOTS - 4 {
                    number + 1
                } else {
                    *number
                }
            }
            PixelPipeline::BetweenFrames(dots) => {
                screen::NUM_SCANLINES + (dots / SCANLINE_TOTAL_DOTS) as u8
            }
        }
    }

    /// True on the exact dot where LY increments early (4 dots before
    /// standard scanline end).
    pub fn ly_transitioning(&self) -> bool {
        match self {
            PixelPipeline::Rendering(Rendering {
                line: Line { dots, .. },
                ..
            }) => *dots == SCANLINE_TOTAL_DOTS - 4,
            PixelPipeline::BetweenFrames(dots) => {
                dots % SCANLINE_TOTAL_DOTS == SCANLINE_TOTAL_DOTS - 4
            }
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            PixelPipeline::Rendering(rendering) => rendering.mode(),
            PixelPipeline::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn stat_mode(&self) -> Mode {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => Mode::BetweenLines,
            PixelPipeline::Rendering(rendering) => rendering.stat_mode(),
            PixelPipeline::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn interrupt_mode(&self) -> Mode {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => Mode::BetweenLines,
            PixelPipeline::Rendering(rendering) => rendering.interrupt_mode(),
            // On hardware, Mode 1 STAT fires at clock 4 of line 144, not clock 0.
            // The internal mode-for-interrupt doesn't transition to Mode 1 until
            // 4 dots after VBlank entry.
            PixelPipeline::BetweenFrames(dots) if *dots >= 4 => Mode::BetweenFrames,
            PixelPipeline::BetweenFrames(_) => Mode::BetweenLines,
        }
    }

    pub fn mode2_interrupt_active(&self) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelPipeline::Rendering(rendering) => rendering.mode2_interrupt_active(),
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    pub fn oam_locked(&self) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelPipeline::Rendering(rendering) => rendering.oam_locked(),
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    pub fn vram_locked(&self) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelPipeline::Rendering(rendering) => rendering.vram_locked(),
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    pub fn oam_write_locked(&self) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelPipeline::Rendering(rendering) => rendering.oam_write_locked(),
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    pub fn vram_write_locked(&self) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelPipeline::Rendering(rendering) => rendering.vram_write_locked(),
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    pub fn is_rendering(&self) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) => rendering.rendering,
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    /// Diagnostic: return (line_number, dots) if rendering, None otherwise.
    pub fn diag_line_dots(&self) -> Option<(u8, u32)> {
        match self {
            PixelPipeline::Rendering(r) => Some((r.line.number, r.line.dots)),
            PixelPipeline::BetweenFrames(_) => None,
        }
    }

    pub fn accessed_oam_row(&self) -> Option<u8> {
        match self {
            PixelPipeline::Rendering(rendering) => {
                if rendering.line.dots < SCANLINE_PREPARING_DOTS {
                    Some(((rendering.line.dots / 4 + 1) * 8) as u8)
                } else {
                    None
                }
            }
            PixelPipeline::BetweenFrames(_) => None,
        }
    }

    /// Advance the PPU by one dot (T-cycle). Returns a completed screen
    /// when a full frame finishes rendering.
    pub fn tcycle(&mut self, data: &Registers, oam: &Oam, vram: &Vram) -> Option<Screen> {
        let mut screen = None;

        match self {
            PixelPipeline::Rendering(rendering) => {
                if rendering.dot(data, oam, vram) {
                    screen = Some(rendering.screen.clone());
                    *self = PixelPipeline::BetweenFrames(0);
                }
            }
            PixelPipeline::BetweenFrames(dots) => {
                *dots += 1;
                if *dots >= BETWEEN_FRAMES_DOTS {
                    *self = PixelPipeline::Rendering(Rendering::new());
                }
            }
        };

        screen
    }
}
