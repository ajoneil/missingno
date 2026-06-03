// 8-bit per-bitplane SACU-clocked shift registers; parallel-loaded by SEKO at tile boundaries.

/// Two 8-bit BgwPipeA/BgwPipeB shifters; zero fills in from bit 0 on every SACU edge.
/// `cell` is the per-tile BG attribute (CGB) held across the tile's 8 pixels — the
/// bitplanes shift, the cell does not. `()` on the DMG carries nothing.
pub(in crate::ppu) struct BgShifter<C> {
    low: u8,
    high: u8,
    cell: C,
}

impl<C: Copy + Default> BgShifter<C> {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            low: 0,
            high: 0,
            cell: C::default(),
        }
    }

    /// SEKO async parallel-load. The planes are loaded pre-flipped; the cell rides
    /// the tile.
    pub(in crate::ppu) fn load(&mut self, low: u8, high: u8, cell: C) {
        self.low = low;
        self.high = high;
        self.cell = cell;
    }

    pub(in crate::ppu) fn read(&self) -> (u8, u8, C) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        (lo, hi, self.cell)
    }

    pub(in crate::ppu) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
    }

    pub(in crate::ppu) fn registers(&self) -> (u8, u8) {
        (self.low, self.high)
    }
}

/// Four parallel 8-bit dffsr chains (plane A, plane B, palette, priority) collapsed into u8 fields.
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

    /// Stage-7 Q outputs.
    pub(in crate::ppu) fn read(&self) -> (u8, u8, u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (self.palette >> 7) & 1;
        let pri = (self.priority >> 7) & 1;
        (lo, hi, pal, pri)
    }

    pub(in crate::ppu) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
        self.palette <<= 1;
        self.priority <<= 1;
    }

    pub(in crate::ppu) fn registers(&self) -> (u8, u8, u8, u8) {
        (self.low, self.high, self.palette, self.priority)
    }

    /// WUTY-pulse parallel-load, transparency-gated per stage.
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
