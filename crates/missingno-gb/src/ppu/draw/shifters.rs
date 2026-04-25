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

/// Sprite pixel shift register.
///
/// Four parallel 8-bit dffsr chains collapsed into u8 fields:
/// - `low` — plane A: NYLU → … → VUPY
/// - `high` — plane B: NURO → … → WUFY
/// - `palette` — per-pixel OBJ palette selector
/// - `priority` — mask pipe VAVA → VEZO (DEPO captures OAM-A bit 7)
///
/// SACU-clocked shift; parallel-load via NAND2 pair at s_n / r_n.
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

    /// Stage-7 Q outputs: sprite_px_a7 / sprite_px_b7 + palette / mask-pipe MSBs.
    pub(in crate::ppu) fn read(&self) -> (u8, u8, u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (self.palette >> 7) & 1;
        let pri = (self.priority >> 7) & 1;
        (lo, hi, pal, pri)
    }

    /// SACU rising edge — all four planes shift; stage-0 d=const0 fills 0.
    pub(in crate::ppu) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
        self.palette <<= 1;
        self.priority <<= 1;
    }

    pub(in crate::ppu) fn registers(&self) -> (u8, u8, u8, u8) {
        (self.low, self.high, self.palette, self.priority)
    }

    /// wuty-pulse parallel-load — transparency-gated per stage.
    ///
    /// Collapses `sprite_onN = NOR3(xefy, sprite_px_aN, sprite_px_bN)`
    /// (xefy = NOT(wuty)) and the NAND2 pair at each dffsr's s_n / r_n.
    /// Fires only where the current shifter position is transparent.
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
                continue;
            }

            let existing_lo = (self.low >> bit_pos) & 1;
            let existing_hi = (self.high >> bit_pos) & 1;
            let existing_color = (existing_hi << 1) | existing_lo;
            if existing_color != 0 {
                continue;
            }

            let mask = 1 << bit_pos;
            self.low = (self.low & !mask) | (lo << bit_pos);
            self.high = (self.high & !mask) | (hi << bit_pos);
            self.palette = (self.palette & !mask) | (palette_bit << bit_pos);
            self.priority = (self.priority & !mask) | (priority_bit << bit_pos);
        }
    }
}
