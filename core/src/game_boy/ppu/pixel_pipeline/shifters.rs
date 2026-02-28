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
pub(super) struct BgShifter {
    low: u8,
    high: u8,
    len: u8,
}

impl BgShifter {
    pub(super) fn new() -> Self {
        Self {
            low: 0,
            high: 0,
            len: 0,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(super) fn len(&self) -> u8 {
        self.len
    }

    pub(super) fn clear(&mut self) {
        self.len = 0;
    }

    /// Parallel load from a tile fetch. On hardware, the DFF22 shift
    /// register cells use async SET/RST pins, so a load unconditionally
    /// overwrites the current contents (SEKO pre-load at tile boundaries).
    pub(super) fn load(&mut self, low: u8, high: u8) {
        self.low = low;
        self.high = high;
        self.len = 8;
    }

    /// Read the MSB bitplane values — the shift register's output pins.
    /// On hardware, bit 7 is always readable regardless of pipe state.
    pub(super) fn read(&self) -> (u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        (lo, hi)
    }

    /// Shift the register left by one position (SACU clock edge).
    /// Pure side effect — use `read()` afterward to get the post-shift output.
    pub(super) fn shift(&mut self) {
        debug_assert!(self.len > 0);
        self.low <<= 1;
        self.high <<= 1;
        self.len -= 1;
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
pub(super) struct ObjShifter {
    low: u8,
    high: u8,
    palette: u8,
    priority: u8,
    len: u8,
}

impl ObjShifter {
    pub(super) fn new() -> Self {
        Self {
            low: 0,
            high: 0,
            palette: 0,
            priority: 0,
            len: 0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.low = 0;
        self.high = 0;
        self.palette = 0;
        self.priority = 0;
        self.len = 0;
    }

    /// Read the MSB data — the shift register's output pins.
    /// Returns None if the pipe has no sprite data loaded.
    pub(super) fn read(&self) -> Option<(u8, u8, u8, u8)> {
        if self.len == 0 {
            return None;
        }
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (self.palette >> 7) & 1;
        let pri = (self.priority >> 7) & 1;
        Some((lo, hi, pal, pri))
    }

    /// Shift the register left by one position (SACU clock edge).
    /// Pure side effect — use `read()` afterward to get the post-shift output.
    pub(super) fn shift(&mut self) {
        if self.len == 0 {
            return;
        }
        self.low <<= 1;
        self.high <<= 1;
        self.palette <<= 1;
        self.priority <<= 1;
        self.len -= 1;
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
    pub(super) fn merge(
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
