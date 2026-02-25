use core::fmt;

use crate::game_boy::ppu::{
    PipelineRegisters, VideoControl,
    memory::{Oam, Vram},
    palette::{PaletteIndex, PaletteMap},
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
/// Hardware pixel counter value at which WODU fires (hblank gate).
/// XUGU = NAND5(PX0, PX1, PX2, PX5, PX7) decodes 128+32+4+2+1 = 167.
const WODU_PIXEL_COUNT: u8 = 167;
/// First pixel counter value that produces a visible LCD pixel.
/// On hardware, the LCD X coordinate is `pix_count - 8`. Pixels at
/// PX 0–7 shift the first tile's data through the pipe invisibly.
const FIRST_VISIBLE_PIXEL: u8 = 8;
/// Dot at which the RUTU line-end signal fires (LX=113 × 4 dots/M-cycle = 452).
/// This clocks the LY register and triggers line-end processing.
const RUTU_LINE_END_DOT: u32 = SCANLINE_TOTAL_DOTS - 4;
const BETWEEN_FRAMES_DOTS: u32 = SCANLINE_TOTAL_DOTS * 10;
const MAX_SPRITES_PER_LINE: usize = 10;

/// Pixel pipeline rendering phase, modeling the XYMU (rendering latch)
/// and WODU (hblank gate) hardware signals on page 21.
///
/// On hardware, WODU fires combinationally when the pixel counter reaches
/// 167, then VOGA latches WODU on the next even phase to clear XYMU.
/// The STAT mode 0 interrupt condition (TARU) uses WODU directly, so it
/// sees HBlank one phase before XYMU clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderPhase {
    /// Not drawing — before Mode 3 starts or after line-end reset.
    /// On line 0, the OAM scan runs with `Idle` render phase (BESU
    /// is never set on line 0, so STAT reads mode 0).
    Idle,
    /// Mode 2: BESU set, OAM scanner active. ACYL_SCANNINGp drives
    /// STAT register mode bit 1. Set by CATU_LINE_ENDp at dot 1
    /// for lines 1+, cleared by AVAP when the scan completes.
    /// Line 0 skips this phase (BESU never set on first line).
    Scanning,
    /// Mode 3: XYMU set, fetcher running. Covers the entire rendering
    /// period from AVAP (scan done) through WODU (PX≥167). During
    /// startup, the `StartupFetch` cascade gates the pixel clock until
    /// the first tile fetch completes and POKY latches.
    Drawing,
    /// WODU fired (PX≥167, no sprite match): STAT sees mode=0 via TARU,
    /// pixel clock stops, VRAM/OAM unlocked. XYMU clears next dot.
    /// Hardware: XYMU set, WODU set. Lasts 1 dot.
    WoduFired,
    /// Mode 0 (HBlank): XYMU cleared via VOGA latch. Rendering fully stopped.
    /// Hardware: XYMU clear, WODU set.
    HBlank,
}

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

// --- Window hit (RYDY pixel clock gate) ---

/// RYDY NOR latch state — the window hit signal.
///
/// On hardware, RYDY is SET when the window X match fires (NUKO_WX_MATCHp)
/// and RESET when the window fetch completes (SUZU/MOSU path clears it).
/// While active, RYDY gates TYFA (via SOCY_WIN_HITn = not1(TOMU_WIN_HITp)),
/// freezing the entire pixel clock chain:
///   TYFA=0 → ROXO=0 (fine counter clock frozen)
///           → SEGU=1 → SACU=1 (pixel counter clock frozen)
///
/// The BG fetcher is NOT gated — it runs on LEBO (the half-cycle clock),
/// independent of TYFA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowHit {
    /// RYDY=0: no active window fetch stall. The pixel clock chain
    /// runs normally (subject to other gates like ROXY and POKY).
    Inactive,
    /// RYDY=1: window fetch in progress. The pixel clock chain is
    /// frozen — fine counter and pixel counter do not advance.
    /// Cleared when the fetcher reaches Idle (SUZU fires).
    Active,
    /// RYDY just cleared (SUZU fired): pipe is loaded with window tile data,
    /// but `clkpipe_gate` still reads the old RYDY=1 value. Pixel clock
    /// remains frozen for this 1 tick. Transitions to Inactive on next tick.
    Clearing,
}

// --- Fine scroll (ROXY pixel clock gate) ---

/// ROXY NOR latch state. On hardware, ROXY gates the pixel clock
/// (SACU = or2(SEGU, ROXY)) until the fine scroll counter matches
/// SCX & 7. SET between lines (PAHA_RENDERINGn), RESET on fine
/// scroll match (POVA_FINE_MATCH_TRIGp_evn). One-shot per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Roxy {
    /// ROXY=1: pixel clock gated. The fine counter is still counting
    /// toward the SCX & 7 target.
    Gating,
    /// ROXY=0: pixel clock active. Fine scroll discard is complete
    /// for this line.
    Done,
}

/// Hardware fine scroll counter (RYKU/ROGA/RUBU) and pixel clock
/// gate (ROXY). The ROXY latch gates the pixel clock (SACU) until
/// the counter matches SCX & 7, implementing sub-tile fine scrolling.
struct FineScroll {
    /// 3-bit counter (0–7).
    count: u8,
    /// ROXY NOR latch — gates SACU until fine scroll match fires.
    roxy: Roxy,
}

impl FineScroll {
    fn new() -> Self {
        Self {
            count: 0,
            roxy: Roxy::Gating,
        }
    }

    /// Whether the pixel clock is active (SACU ungated).
    fn pixel_clock_active(&self) -> bool {
        self.roxy == Roxy::Done
    }

    /// Advance the fine counter by one dot (PECU clock).
    fn tick(&mut self) {
        self.count = (self.count + 1) & 7;
    }

    /// Check and clear the gating latch if count matches SCX & 7.
    /// One-shot: once cleared, stays cleared for the rest of the line.
    fn check_scroll_match(&mut self, scx: u8) {
        if self.roxy == Roxy::Gating && self.count == (scx & 7) {
            self.roxy = Roxy::Done;
        }
    }

    /// Reset for window trigger — counter resets, gating clears
    /// (window has no fine scroll).
    fn reset_for_window(&mut self) {
        self.count = 0;
        self.roxy = Roxy::Done;
    }
}

// --- Background fetcher ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetcherStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
    /// The fetcher has completed all three VRAM reads and is frozen,
    /// waiting for the SEKO-triggered reload (fine_count == 7).
    Idle,
}

/// Mode 3 starts with one BG tile fetch before any pixels shift out.
/// On hardware, AVAP fires at Mode 3 entry and the fetcher begins
/// immediately. After the first tile fetch completes, the LYRY
/// combinational signal and NYKA→PORY→POKY DFF cascade propagate the
/// "fetch done" signal across alternating clock phases before enabling
/// the pixel clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupFetch {
    /// First tile fetch in progress. The fetcher runs on DELTA_EVEN only
    /// (LEBO clock). When the fetcher fills the BG shifter, LYRY fires
    /// combinationally → transitions to LyryFired.
    FirstTile,

    /// LYRY_BFETCH_DONEn has fired (combinational — the fetcher filled
    /// the shifter this DELTA_EVEN). NYKA will capture it on the *next*
    /// DELTA_EVEN.
    LyryFired,

    /// NYKA_FETCH_DONEp_evn has captured LYRY. PORY will capture NYKA
    /// on the next DELTA_ODD.
    NykaFired,

    /// PORY_FETCH_DONEp_odd has captured NYKA. POKY will latch on the
    /// next DELTA_EVEN, enabling the pixel clock (startup_fetch → None).
    PoryFired,
}

/// Which half of LEBO's 2-dot clock cycle the fetcher is in.
/// The fetcher (and OAM scanner) are clocked at half the dot rate.
/// T1 is the first dot (LEBO low → high edge); T2 is the second
/// (LEBO high → low edge, when the actual work fires).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetcherTick {
    T1,
    T2,
}

struct TileFetcher {
    step: FetcherStep,
    /// Which half of the 2-dot fetcher clock cycle we're in.
    tick: FetcherTick,
    /// Window tile X counter (hardware's win_x.map). Increments per
    /// window tile fetched. Reset to 0 on window trigger.
    window_tile_x: u8,
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
            tick: FetcherTick::T1,
            window_tile_x: 0,
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
    /// Bitmask of which store slots have been fetched during Mode 3.
    /// Bit N set = slot N already consumed. On hardware, each slot has
    /// an independent reset flag (EBOJ-FONO). Reset at line start.
    fetched: u16,
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
            fetched: 0,
        }
    }
}

// --- OAM scanner ---

/// Hardware OAM scanner (YFEL-FONY scan counter + comparison logic).
/// Processes one OAM entry every 2 dots during Mode 2, reading Y and X
/// from OAM, comparing Y against LY, and writing matches into the
/// sprite store.
struct OamScanner {
    /// Which OAM entry to process next (0-39). Increments every 2 dots.
    entry: u8,
    /// Which half of the 2-dot scanner clock cycle we're in.
    tick: FetcherTick,
}

impl OamScanner {
    fn new() -> Self {
        Self {
            entry: 0,
            tick: FetcherTick::T1,
        }
    }

    /// Process one dot of OAM scanning. On even dots, the scan counter
    /// drives the OAM address and OAM outputs data; on odd dots, the Y
    /// comparison fires and matches are written to the sprite store.
    ///
    /// Only bytes 0–1 (Y, X) are read from OAM during scanning — the
    /// hardware's 16-bit OAM bus provides both in a single access. Tile
    /// index and attributes (bytes 2–3) are not accessed until Mode 3.
    fn scan_next_entry(
        &mut self,
        line_number: u8,
        sprites: &mut SpriteStore,
        regs: &PipelineRegisters,
        oam: &Oam,
    ) {
        if self.tick == FetcherTick::T1 {
            self.tick = FetcherTick::T2;
        } else {
            if (sprites.count as usize) < MAX_SPRITES_PER_LINE {
                // OAM bus read: only Y (byte 0) and X (byte 1).
                let (y_plus_16, x_plus_8) = oam.sprite_position(SpriteId(self.entry));

                // Y comparison (hardware subtractor ERUC–WUHU):
                // Computes delta = LY + 16 - sprite_Y using wrapping
                // arithmetic (matching the 8-bit hardware subtractor).
                // Match when delta < height (8 or 16 per LCDC.2).
                // Bits 0–3 of delta are the sprite line offset — the
                // same value drives the sprite store's line register.
                let delta = line_number.wrapping_add(16).wrapping_sub(y_plus_16);
                let height = regs.control.sprite_size().height();
                if delta < height {
                    let line_offset = delta;
                    sprites.entries[sprites.count as usize] = SpriteStoreEntry {
                        oam_index: self.entry,
                        line_offset,
                        x: x_plus_8,
                    };
                    sprites.count += 1;
                }
            }
            self.entry += 1;
            self.tick = FetcherTick::T1;
        }
    }

    /// Hardware FETO_SCAN_DONE signal. Fires when the scan counter
    /// has processed all 40 OAM entries.
    fn done(&self) -> bool {
        self.entry >= 40
    }

    /// The byte address the scanner is currently driving on the OAM bus.
    /// Hardware: OAM_A[7:2] = scan_counter, OAM_A[1:0] = 0.
    fn oam_address(&self) -> u8 {
        self.entry * 4
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
    tick: FetcherTick,
    tile_data_low: u8,
    tile_data_high: u8,
}

/// Sprite fetch lifecycle. On hardware, FEPO (sprite X match) freezes
/// the pixel clock, the fetch runs, then the pixel clock resumes with
/// PX still at the trigger value so the composited pixel outputs first.
enum SpriteState {
    /// No sprite activity. Pixel clock runs normally.
    Idle,
    /// Sprite fetch in progress (wait + data phases).
    Fetching(SpriteFetch),
    /// One-dot post-fetch: pixel clock resumes but PX stays frozen
    /// so the first pixel at the trigger position includes the sprite.
    /// Cleared after shift_pixel_out on this dot.
    Resuming,
}

// --- Rendering ---

pub struct Rendering {
    screen: Screen,
    window_line_counter: u8,
    /// After LCD enable, the first line's Mode 2 doesn't begin at dot 0.
    /// The STAT mode bits read as 0 until Mode 2 actually starts.
    lcd_turning_on: bool,
    /// Pixel pipeline phase — models XYMU (rendering latch) and WODU
    /// (hblank gate). See `RenderPhase` for hardware signal mapping.
    render_phase: RenderPhase,
    /// Dot position within the current scanline (0..456).
    dot: u32,
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
    /// Sprite fetch lifecycle — Idle, Fetching, or Resuming.
    sprite_state: SpriteState,
    /// Window reactivation zero pixel (DMG only). Set when WX re-matches
    /// while the window is active with specific fetcher/FIFO conditions.
    /// Causes the next pixel output to use bg_color=0 without popping
    /// the BG shifter. The OBJ shifter is popped normally.
    window_zero_pixel: bool,
}

impl Rendering {
    fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            window_line_counter: 0,
            lcd_turning_on: false,
            render_phase: RenderPhase::Idle,
            dot: 0,
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
        }
    }

    fn new_lcd_on() -> Self {
        Rendering {
            screen: Screen::new(),
            window_line_counter: 0,
            lcd_turning_on: true,
            render_phase: RenderPhase::Idle,
            dot: 0,
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
        }
    }

    fn mode(&self) -> Mode {
        match self.render_phase {
            RenderPhase::Drawing | RenderPhase::WoduFired => Mode::DrawingPixels,
            RenderPhase::Scanning => Mode::PreparingScanline,
            _ if self.scanner.is_some() => Mode::PreparingScanline,
            _ => Mode::BetweenLines,
        }
    }

    /// Mode as seen by the STAT register (ACYL/XYMU/POPU-derived).
    /// Scanning maps to mode 2 via the BESU/ACYL signal path.
    fn stat_mode(&self) -> Mode {
        match self.render_phase {
            RenderPhase::WoduFired | RenderPhase::HBlank => Mode::BetweenLines,
            RenderPhase::Drawing => Mode::DrawingPixels,
            RenderPhase::Scanning => Mode::PreparingScanline,
            RenderPhase::Idle => Mode::BetweenLines,
        }
    }

    /// Mode for STAT interrupt edge detection. Mode 0 fires from
    /// WODU (hblank_gate) directly — one phase before XYMU clears.
    fn interrupt_mode(&self) -> Mode {
        match self.render_phase {
            RenderPhase::WoduFired | RenderPhase::HBlank => Mode::BetweenLines,
            RenderPhase::Drawing => Mode::DrawingPixels,
            RenderPhase::Scanning => Mode::PreparingScanline,
            RenderPhase::Idle if self.scanner.is_some() => Mode::PreparingScanline,
            RenderPhase::Idle => Mode::BetweenLines,
        }
    }

    /// Whether the mode 2 STAT interrupt condition is active.
    fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        // On hardware, lines 1+ get an early Mode 2 pre-trigger at clock 0
        // from the previous HBlank pre-setting mode_for_interrupt. Line 0
        // has no previous HBlank, so Mode 2 STAT fires at clock 4 instead.
        self.mode() == Mode::PreparingScanline && (video.ly() != 0 || self.dot >= 4)
    }

    fn oam_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::Scanning | RenderPhase::Drawing
        )
    }

    fn vram_locked(&self) -> bool {
        // Hardware: VRAM blocked by XYMU_RENDERINGp, cleared when WODU fires.
        matches!(self.render_phase, RenderPhase::Drawing)
    }

    fn oam_write_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::Scanning | RenderPhase::Drawing
        )
    }

    fn vram_write_locked(&self) -> bool {
        matches!(
            self.render_phase,
            RenderPhase::Drawing | RenderPhase::WoduFired
        )
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
        // BESU_SCAN_DONEn NOR latch → RenderPhase::Scanning.
        // BESU is never set on line 0 (hardware special case).
        if self.dot == 1 && video.ly() != 0 {
            self.render_phase = RenderPhase::Scanning;
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
        if self.render_phase == RenderPhase::WoduFired {
            self.render_phase = RenderPhase::HBlank;
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
    /// Returns true when a full frame is complete.
    pub(super) fn half_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &mut VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) -> bool {
        if let Some(ref mut scanner) = self.scanner {
            // Mode 2: OAM scan — process one entry every 2 dots
            scanner.scan_next_entry(video.ly(), &mut self.sprites, regs, oam);
            self.dot += 1;
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
                self.render_phase = RenderPhase::WoduFired;
            }

            self.dot += 1;

            // RUTU line-end event: LY register increments (MUWY-LAFO
            // ripple counter clocked by RUTU_LINE_ENDp).
            if self.dot == RUTU_LINE_END_DOT {
                video.write_ly(video.ly() + 1);
            }

            if self.dot == SCANLINE_TOTAL_DOTS {
                self.render_phase = RenderPhase::Idle;
                if self.window_rendered {
                    self.window_line_counter += 1;
                }

                // Scanline boundary — reset per-line state.
                // LY on the bus already holds the next line's value
                // (incremented at RUTU_LINE_END_DOT).
                self.dot = 0;
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

                if video.ly() == screen::NUM_SCANLINES {
                    return true;
                }
            }
        }

        false
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
            self.advance_bg_fetcher(regs, video, vram);
        }

        // TAVE preload: when the startup fetch first reaches Idle
        // (GetTileDataHigh complete), load the pipe immediately. This is
        // the one-shot preload trigger (TAVE on hardware) — it fires once
        // during startup and never again after POKY latches.
        if self.startup_fetch == Some(StartupFetch::FirstTile)
            && self.fetcher.step == FetcherStep::Idle
            && self.bg_shifter.is_empty()
        {
            self.load_bg_tile();
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
                .check_scroll_match(regs.background_viewport.x);
        }

        // Window WX match fires on DELTA_EVEN (PYCO_WIN_MATCHp).
        // Active during both startup fetch and normal rendering.
        self.check_window_trigger(regs, video);
    }

    /// DELTA_ODD Mode 3 pixel pipeline processing.
    fn mode3_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        match self.startup_fetch {
            Some(StartupFetch::FirstTile) | Some(StartupFetch::LyryFired) => {
                // During fetch/cascade phase, no pixel processing.
                return;
            }
            Some(StartupFetch::NykaFired) => {
                // PORY captures NYKA on DELTA_ODD.
                self.startup_fetch = Some(StartupFetch::PoryFired);
                return;
            }
            Some(StartupFetch::PoryFired) => {
                // Waiting for POKY on next EVEN. No pixel processing yet.
                return;
            }
            None => {}
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
                        self.advance_bg_fetcher(regs, video, vram);

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
                            Self::advance_sprite_fetch(sf, regs, oam, vram);
                        }
                    }
                    SpriteFetchPhase::FetchingData => {
                        // BG fetcher is frozen. Advance the sprite data pipeline.
                        let done = Self::advance_sprite_fetch(sf, regs, oam, vram);
                        if done {
                            Self::merge_sprite_into_obj_shifter(
                                sf,
                                oam,
                                &self.bg_shifter,
                                &mut self.obj_shifter,
                            );
                            self.sprite_state = SpriteState::Resuming;
                        }
                    }
                }
            }
            SpriteState::Idle | SpriteState::Resuming => {
                // Clearing → Inactive: on the tick after SUZU fires, the pixel
                // clock gate sees RYDY=0 and resumes normal operation.
                if self.window_hit == WindowHit::Clearing {
                    self.window_hit = WindowHit::Inactive;
                }

                // SUZU/MOSU: when the window fetch completes (fetcher reaches Idle
                // while RYDY is active), load the first window tile and clear the
                // window hit signal. This is the hardware's dedicated window tile
                // load path — independent of fine_count.
                if self.window_hit == WindowHit::Active && self.fetcher.step == FetcherStep::Idle {
                    self.load_bg_tile();
                    self.window_hit = WindowHit::Clearing;
                }

                // SEKO reload (async). On hardware, the SEKO-triggered pipe
                // load is asynchronous — it fires before the clock-driven pipe
                // shift and pixel counter increment on the same dot. Evaluating
                // it first ensures the shifter is non-empty for the subsequent
                // clock-driven operations when fine_count wraps from 7→0.
                if self.fine_scroll.count == 7 && self.fetcher.step == FetcherStep::Idle {
                    self.load_bg_tile();
                }

                // Pixel counter increment. On hardware, SACU (pixel clock) is
                // gated by TYFA (window hit via SEGU), ROXY (fine scroll), FIFO
                // readiness (shifter non-empty), and sprite resumption (PX stays
                // frozen on the first dot after a sprite fetch so the pixel at
                // PX=N is output before PX advances).
                let resuming = matches!(self.sprite_state, SpriteState::Resuming);
                if self.window_hit == WindowHit::Inactive
                    && self.fine_scroll.pixel_clock_active()
                    && !self.bg_shifter.is_empty()
                    && !resuming
                {
                    self.pixel_counter += 1;
                }

                // Sprite trigger check.
                if !matches!(self.sprite_state, SpriteState::Fetching(_)) {
                    self.check_sprite_trigger(regs);
                }

                // On hardware, the pixel clock freezes when a sprite triggers
                // (FEPO match). No pixel is output on the trigger dot — PX
                // stays at the trigger value and the first pixel at that PX is
                // the sprite-composited pixel on the resumption dot.
                if !self.bg_shifter.is_empty()
                    && !matches!(self.sprite_state, SpriteState::Fetching(_))
                {
                    self.shift_pixel_out(regs, video);
                    self.sprite_state = SpriteState::Idle;
                }

                self.advance_bg_fetcher(regs, video, vram);

                // PECU (fine counter clock) derives from ROXO, which derives from
                // TYFA. TYFA is gated by RYDY (window hit), so the fine counter
                // freezes during window fetch stalls. It does NOT freeze during
                // ROXY fine scroll discard (different gating point in the chain).
                if self.window_hit == WindowHit::Inactive {
                    self.fine_scroll.tick();
                }
            }
        }
    }

    // --- Address generation (pages 26-27) ---
    //
    // On the die, BG and window have separate address generators:
    //   Page 26 (BACKGROUND): tilemap coords from pixel_counter, SCX, SCY, LY
    //   Page 27 (WINDOW MAP LOOKUP): tilemap coords from window_tile_x, window_line_counter
    // Both feed into the shared VRAM interface (page 25).

    /// BG tilemap coordinate computation (page 26).
    /// Applies SCX/SCY scroll offsets and wraps at 32-tile boundaries.
    fn bg_tilemap_coords(&self, regs: &PipelineRegisters, video: &VideoControl) -> (u8, u8) {
        let scx = regs.background_viewport.x;
        let scy = regs.background_viewport.y.output();
        (
            ((self.pixel_counter.wrapping_add(scx)) >> 3) & 31,
            (video.ly().wrapping_add(scy) / 8) & 31,
        )
    }

    /// Window tilemap coordinate computation (page 27).
    /// Uses the window's internal line counter, no scroll offset.
    fn window_tilemap_coords(&self) -> (u8, u8) {
        (self.fetcher.window_tile_x, self.window_line_counter / 8)
    }

    /// Read the tile index from the tilemap for the current fetch position.
    fn read_tile_index(&self, regs: &PipelineRegisters, video: &VideoControl, vram: &Vram) -> u8 {
        let (map_x, map_y) = if self.fetcher.fetching_window {
            self.window_tilemap_coords()
        } else {
            self.bg_tilemap_coords(regs, video)
        };

        let map_id = if self.fetcher.fetching_window {
            regs.control.window_tile_map()
        } else {
            regs.control.background_tile_map()
        };
        vram.tile_map(map_id).get_tile(map_x, map_y).0
    }

    /// BG fine Y offset (page 26): which row within the tile, from SCY + LY.
    fn bg_fine_y(&self, regs: &PipelineRegisters, video: &VideoControl) -> u8 {
        video.ly().wrapping_add(regs.background_viewport.y.output()) % 8
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
    fn read_tile_data(
        &self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
        high: bool,
    ) -> u8 {
        let tile_index = TileIndex(self.fetcher.tile_index);
        let (block_id, mapped_idx) = regs.control.tile_address_mode().tile(tile_index);

        let fine_y = if self.fetcher.fetching_window {
            self.window_fine_y()
        } else {
            self.bg_fine_y(regs, video)
        };

        let block = vram.tile_block(block_id);
        block.data[mapped_idx.0 as usize * 16 + fine_y as usize * 2 + high as usize]
    }

    /// Advance the background tile fetcher by one dot.
    fn advance_bg_fetcher(&mut self, regs: &PipelineRegisters, video: &VideoControl, vram: &Vram) {
        match self.fetcher.step {
            FetcherStep::GetTile => {
                if self.fetcher.tick == FetcherTick::T1 {
                    self.fetcher.tick = FetcherTick::T2;
                } else {
                    self.fetcher.tile_index = self.read_tile_index(regs, video, vram);
                    self.fetcher.tick = FetcherTick::T1;
                    self.fetcher.step = FetcherStep::GetTileDataLow;
                }
            }
            FetcherStep::GetTileDataLow => {
                if self.fetcher.tick == FetcherTick::T1 {
                    self.fetcher.tick = FetcherTick::T2;
                } else {
                    self.fetcher.tile_data_low = self.read_tile_data(regs, video, vram, false);
                    self.fetcher.tick = FetcherTick::T1;
                    self.fetcher.step = FetcherStep::GetTileDataHigh;
                }
            }
            FetcherStep::GetTileDataHigh => {
                if self.fetcher.tick == FetcherTick::T1 {
                    self.fetcher.tick = FetcherTick::T2;
                } else {
                    self.fetcher.tile_data_high = self.read_tile_data(regs, video, vram, true);
                    self.fetcher.tick = FetcherTick::T1;
                    self.fetcher.step = FetcherStep::Idle;
                }
            }
            FetcherStep::Idle => {
                // The fetcher is frozen — it waits here until the
                // SEKO-triggered reload (fine_count == 7) fires from
                // mode3_odd, which calls load_bg_tile() and resets
                // the fetcher to GetTile.
            }
        }
    }

    /// Load fetched tile data into the BG shifter and reset the fetcher to
    /// GetTile for the next tile.
    fn load_bg_tile(&mut self) {
        self.bg_shifter
            .load(self.fetcher.tile_data_low, self.fetcher.tile_data_high);
        if self.fetcher.fetching_window {
            self.fetcher.window_tile_x = self.fetcher.window_tile_x.wrapping_add(1);
        }
        self.fetcher.step = FetcherStep::GetTile;
        self.fetcher.tick = FetcherTick::T1;
    }

    /// Read one byte of sprite tile data (low or high bitplane).
    ///
    /// On the die, the sprite fetcher (page 29) uses the OAM index
    /// from the sprite store to look up the tile index and attributes,
    /// then generates a VRAM address from the tile index, line offset,
    /// and flip flags. The VRAM interface (page 25) performs the read.
    fn read_sprite_tile_data(
        sf: &SpriteFetch,
        regs: &PipelineRegisters,
        oam: &Oam,
        vram: &Vram,
        high: bool,
    ) -> u8 {
        let sprite = oam.sprite(SpriteId(sf.entry.oam_index));
        let tile_index = if regs.control.sprite_size() == SpriteSize::Double {
            TileIndex(sprite.tile.0 & 0xFE)
        } else {
            sprite.tile
        };
        let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

        let flipped_y = if sprite.attributes.flip_y() {
            (regs.control.sprite_size().height() as i16 - 1 - sf.entry.line_offset as i16) as u8
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

    /// Advance the sprite fetch pipeline by one dot. Returns `true` when
    /// the fetch is complete (GetTileDataHigh T2 has fired).
    fn advance_sprite_fetch(
        sf: &mut SpriteFetch,
        regs: &PipelineRegisters,
        oam: &Oam,
        vram: &Vram,
    ) -> bool {
        match sf.step {
            SpriteStep::GetTile => {
                if sf.tick == FetcherTick::T1 {
                    sf.tick = FetcherTick::T2;
                } else {
                    // Tile index comes from OAM via the sprite store's oam_index
                    sf.tick = FetcherTick::T1;
                    sf.step = SpriteStep::GetTileDataLow;
                }
            }
            SpriteStep::GetTileDataLow => {
                if sf.tick == FetcherTick::T1 {
                    sf.tick = FetcherTick::T2;
                } else {
                    sf.tile_data_low = Self::read_sprite_tile_data(sf, regs, oam, vram, false);
                    sf.tick = FetcherTick::T1;
                    sf.step = SpriteStep::GetTileDataHigh;
                }
            }
            SpriteStep::GetTileDataHigh => {
                if sf.tick == FetcherTick::T1 {
                    sf.tick = FetcherTick::T2;
                } else {
                    sf.tile_data_high = Self::read_sprite_tile_data(sf, regs, oam, vram, true);
                    return true;
                }
            }
        }
        false
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
    fn shift_pixel_out(&mut self, regs: &PipelineRegisters, video: &VideoControl) {
        // Window reactivation zero pixel: substitute color 0 for the BG
        // pixel without popping the BG shifter. The OBJ shifter is still
        // popped so sprite pixels mix against the zero pixel.
        if self.window_zero_pixel {
            self.window_zero_pixel = false;
            let obj_bits = self.obj_shifter.shift();

            if !self.fine_scroll.pixel_clock_active() {
                return;
            }
            if self.pixel_counter < FIRST_VISIBLE_PIXEL {
                return;
            }
            if self.pixel_counter >= FIRST_VISIBLE_PIXEL + screen::PIXELS_PER_LINE {
                return;
            }

            let x = self.pixel_counter - FIRST_VISIBLE_PIXEL;
            let y = video.ly();
            let bg_color: u8 = 0;

            if regs.control.sprites_enabled() {
                if let Some((spr_lo, spr_hi, spr_pal, spr_pri)) = obj_bits {
                    let spr_color = (spr_hi << 1) | spr_lo;
                    if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
                        let sprite_palette = if spr_pal == 0 {
                            PaletteMap(regs.palettes.sprite0.output())
                        } else {
                            PaletteMap(regs.palettes.sprite1.output())
                        };
                        let mapped = sprite_palette.map(PaletteIndex(spr_color));
                        self.screen.set_pixel(x, y, mapped);
                        return;
                    }
                }
            }

            let mapped = PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color));
            self.screen.set_pixel(x, y, mapped);
            return;
        }

        // Shift one bit from each BG bitplane
        let (bg_lo, bg_hi) = self.bg_shifter.shift();

        // Shift OBJ in lockstep (if it has pixels)
        let obj_bits = self.obj_shifter.shift();

        // During fine scroll gating (ROXY active), the pixel clock is
        // frozen on hardware — SACU is held high, PX does not increment,
        // no LCD output. The shifters still advance here (unlike true
        // hardware gating) to keep sprite alignment consistent with the
        // existing sprite fetch model.
        if !self.fine_scroll.pixel_clock_active() {
            return;
        }

        // PX 1 through FIRST_VISIBLE_PIXEL-1 are invisible — the first
        // tile shifts through the pipe without writing to the framebuffer.
        // On hardware, the LCD clock gate (WUSA) doesn't open until PX=8.
        if self.pixel_counter < FIRST_VISIBLE_PIXEL {
            return;
        }

        // Past the visible region — safety guard for dots between WODU
        // and rendering latch clearing.
        if self.pixel_counter >= FIRST_VISIBLE_PIXEL + screen::PIXELS_PER_LINE {
            return;
        }

        let x = self.pixel_counter - FIRST_VISIBLE_PIXEL;
        let y = video.ly();

        // Form 2-bit BG color index (0 if BG/window disabled via LCDC.0)
        let bg_color = if regs.control.background_and_window_enabled() {
            (bg_hi << 1) | bg_lo
        } else {
            0
        };

        // Sprite priority mixing
        if regs.control.sprites_enabled() {
            if let Some((spr_lo, spr_hi, spr_pal, spr_pri)) = obj_bits {
                let spr_color = (spr_hi << 1) | spr_lo;
                if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
                    // Sprite pixel wins
                    let sprite_palette = if spr_pal == 0 {
                        PaletteMap(regs.palettes.sprite0.output())
                    } else {
                        PaletteMap(regs.palettes.sprite1.output())
                    };
                    let mapped = sprite_palette.map(PaletteIndex(spr_color));
                    self.screen.set_pixel(x, y, mapped);
                    return;
                }
            }
        }

        // Background pixel
        let mapped = PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color));
        self.screen.set_pixel(x, y, mapped);
    }

    /// Check if the window should start rendering at the current pixel position.
    /// Also detects window reactivation zero pixel conditions when the window
    /// is already active.
    fn check_window_trigger(&mut self, regs: &PipelineRegisters, video: &VideoControl) {
        if !regs.control.window_enabled() {
            return;
        }
        if video.ly() < regs.window.y {
            return;
        }
        if self.pixel_counter != regs.window.x_plus_7.output() {
            return;
        }

        // Window already active — check for reactivation zero pixel (DMG only).
        // The hardware condition is GetTile T1 (first tick). Since our WX check
        // runs after advance_bg_fetcher in mode3_even, the fetcher has already
        // been ticked: what was dot=0 (T1) is now dot=1. So we check dot=1.
        if self.fetcher.fetching_window {
            if self.startup_fetch.is_none()
                && self.fetcher.step == FetcherStep::GetTile
                && self.fetcher.tick == FetcherTick::T2
                && self.bg_shifter.len() == 8
            {
                self.window_zero_pixel = true;
            }
            return;
        }

        // Window trigger: clear shifters, reset fine scroll, restart fetcher,
        // and reset cascade DFFs so a new startup fetch begins.
        self.bg_shifter.clear();
        self.obj_shifter.clear();
        self.fine_scroll.reset_for_window();
        self.window_hit = WindowHit::Active;
        self.fetcher.step = FetcherStep::GetTile;
        self.fetcher.tick = FetcherTick::T1;
        self.fetcher.window_tile_x = 0;
        self.fetcher.fetching_window = true;
        if self.startup_fetch.is_some() {
            self.startup_fetch = Some(StartupFetch::FirstTile);
        }
        self.window_rendered = true;
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
            self.sprite_state = SpriteState::Fetching(SpriteFetch {
                entry: *entry,
                phase: SpriteFetchPhase::WaitingForFetcher,
                step: SpriteStep::GetTile,
                tick: FetcherTick::T1,
                tile_data_low: 0,
                tile_data_high: 0,
            });
            break; // Only one sprite fetch at a time
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

    pub fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        match self {
            PixelPipeline::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelPipeline::Rendering(rendering) => rendering.mode2_interrupt_active(video),
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
            PixelPipeline::Rendering(rendering) => {
                matches!(
                    rendering.render_phase,
                    RenderPhase::Drawing | RenderPhase::WoduFired
                )
            }
            PixelPipeline::BetweenFrames(_) => false,
        }
    }

    pub fn scanner_oam_address(&self) -> Option<u8> {
        match self {
            PixelPipeline::Rendering(rendering) => {
                rendering.scanner.as_ref().map(|s| s.oam_address())
            }
            PixelPipeline::BetweenFrames(_) => None,
        }
    }

    /// DELTA_EVEN half of a dot tick: fetcher control, mode transitions.
    pub fn tcycle_even(&mut self, regs: &PipelineRegisters, video: &VideoControl, vram: &Vram) {
        match self {
            PixelPipeline::Rendering(rendering) => {
                rendering.half_even(regs, video, vram);
            }
            PixelPipeline::BetweenFrames(_) => {}
        }
    }

    /// DELTA_ODD half of a dot tick: pixel output, counter increment.
    /// Returns a completed screen when a full frame finishes rendering.
    pub fn tcycle_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &mut VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) -> Option<Screen> {
        let mut screen = None;
        match self {
            PixelPipeline::Rendering(rendering) => {
                if rendering.half_odd(regs, video, oam, vram) {
                    screen = Some(rendering.screen.clone());
                    *self = PixelPipeline::BetweenFrames(0);
                }
            }
            PixelPipeline::BetweenFrames(dots) => {
                *dots += 1;
                // RUTU line-end event within VBlank scanlines:
                // LY increments at the same dot offset as during Rendering.
                if *dots % SCANLINE_TOTAL_DOTS == RUTU_LINE_END_DOT {
                    video.write_ly(video.ly() + 1);
                }
                if *dots >= BETWEEN_FRAMES_DOTS {
                    video.write_ly(0);
                    *self = PixelPipeline::Rendering(Rendering::new());
                }
            }
        }
        screen
    }

    /// Advance the PPU by one dot (T-cycle). Returns a completed screen
    /// when a full frame finishes rendering.
    pub fn tcycle(
        &mut self,
        regs: &PipelineRegisters,
        video: &mut VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) -> Option<Screen> {
        self.tcycle_even(regs, video, vram);
        self.tcycle_odd(regs, video, oam, vram)
    }
}
