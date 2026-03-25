// --- Pixel shift registers ---
//
// On hardware (pages 32-34), each pixel layer uses separate 8-bit shift
// registers for each bitplane. Tile data is loaded in parallel and shifted
// out one bit per dot. The 2-bit color index is only formed at the pixel
// mux (page 35) by combining the two bitplane outputs.

/// Background pixel shift register (page 32 on the die).
///
/// Two 8-bit shift registers, one per bitplane (BgwPipeA/BgwPipeB).
/// On hardware, this is always 8 DFF22 flip-flops that shift on every
/// SACU clock edge unconditionally. Zero shifts in from bit 0. Tile
/// data is loaded in parallel via async SET/RST (SEKO signal).
///
pub(in crate::ppu) struct BgShifter {
    low: u8,
    high: u8,
}

impl BgShifter {
    pub(in crate::ppu) fn new() -> Self {
        Self { low: 0, high: 0 }
    }

    /// Parallel load from a tile fetch. On hardware, the DFF22 shift
    /// register cells use async SET/RST pins, so a load unconditionally
    /// overwrites the current contents (SEKO pre-load at tile boundaries).
    pub(in crate::ppu) fn load(&mut self, low: u8, high: u8) {
        self.low = low;
        self.high = high;
    }

    /// Read the MSB bitplane values — the shift register's output pins.
    /// On hardware, bit 7 is always readable regardless of pipe state.
    pub(in crate::ppu) fn read(&self) -> (u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        (lo, hi)
    }

    /// Shift the register left by one position (SACU clock edge).
    /// On hardware, the BG pipe shifts unconditionally — zero fills
    /// in from bit 0 on every clock edge.
    pub(in crate::ppu) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
    }

    pub(in crate::ppu) fn registers(&self) -> (u8, u8) {
        (self.low, self.high)
    }
}

/// Sprite pixel shift register (pages 33-34 on the die).
///
/// Four parallel 8-bit shift registers matching the hardware's DFF22 cells:
/// - `low`/`high`: sprite bitplanes (SprPipeA/SprPipeB, page 33)
/// - `palette`: palette selection bit per pixel (PalPipe, page 34)
/// - `priority`: BG-over-OBJ priority bit per pixel (MaskPipe, page 26)
///
/// On hardware, this is always 8 flip-flops that shift on every SACU clock
/// edge unconditionally. Zero shifts in from bit 0. When no sprite has been
/// loaded, all bits are 0 (transparent). The pixel mux determines
/// transparency by checking if the color index is 0 (NULY NOR gate).
///
/// Sprites are merged via async SET/RST (same DFF22 mechanism as BG tile
/// loads). The transparency mask prevents new sprite data from overwriting
/// positions that already contain opaque pixels from a higher-priority sprite.
pub(in crate::ppu) struct ObjShifter {
    low: u8,
    high: u8,
    palette: u8,
    priority: u8,
}

impl ObjShifter {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            low: 0,
            high: 0,
            palette: 0,
            priority: 0,
        }
    }

    /// Read the MSB data — the shift register's output pins.
    /// On hardware, bit 7 is always readable. When the pipe contains
    /// all zeros (no sprite loaded or transparent pixel), the color
    /// index is 0 and the pixel mux treats it as transparent.
    pub(in crate::ppu) fn read(&self) -> (u8, u8, u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (self.palette >> 7) & 1;
        let pri = (self.priority >> 7) & 1;
        (lo, hi, pal, pri)
    }

    /// Shift the register left by one position (SACU clock edge).
    /// On hardware, the OBJ pipe shifts unconditionally — zero fills
    /// in from bit 0 on every clock edge regardless of pipe contents.
    pub(in crate::ppu) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
        self.palette <<= 1;
        self.priority <<= 1;
    }

    pub(in crate::ppu) fn registers(&self) -> (u8, u8, u8, u8) {
        (self.low, self.high, self.palette, self.priority)
    }

    /// Merge sprite tile data into the shifter at fixed positions
    /// (tile bit N → pipe bit N), with a per-bit transparency mask.
    ///
    /// On hardware, the merge is a DFF22 async SET/RST that fires on
    /// sfetch_done. The SPRITE_MASK signals (OR of existing pipe bits
    /// at each position) prevent new sprite data from overwriting
    /// positions with existing opaque pixels (first-sprite-wins priority).
    ///
    /// `sprite_low`/`sprite_high` are the raw bitplane bytes from the
    /// sprite tile fetch (already X-flipped if needed). `palette_bit`
    /// and `priority_bit` are uniform for all 8 pixels of this sprite.
    pub(in crate::ppu) fn merge(
        &mut self,
        sprite_low: u8,
        sprite_high: u8,
        palette_bit: u8,
        priority_bit: u8,
    ) {
        for bit_pos in 0..8u8 {
            let lo = (sprite_low >> bit_pos) & 1;
            let hi = (sprite_high >> bit_pos) & 1;
            let color = (hi << 1) | lo;
            if color == 0 {
                continue; // Transparent sprite pixel — don't overwrite
            }

            let existing_lo = (self.low >> bit_pos) & 1;
            let existing_hi = (self.high >> bit_pos) & 1;
            let existing_color = (existing_hi << 1) | existing_lo;
            if existing_color != 0 {
                continue; // Existing opaque pixel wins (DMG priority)
            }

            // Write this sprite's pixel into the slot
            let mask = 1 << bit_pos;
            self.low = (self.low & !mask) | (lo << bit_pos);
            self.high = (self.high & !mask) | (hi << bit_pos);
            self.palette = (self.palette & !mask) | (palette_bit << bit_pos);
            self.priority = (self.priority & !mask) | (priority_bit << bit_pos);
        }
    }
}
