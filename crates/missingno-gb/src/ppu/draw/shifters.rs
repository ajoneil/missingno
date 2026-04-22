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
/// Four parallel 8-bit shifters collapsed into four u8 fields:
/// - `low` — sprite plane A: NYLU → PEFU → NATY → PYJO → VARE → WEBA →
///   VANU → VUPY (stage 0 → 7 via dffsr `d` pin; stage-0 d=const0).
/// - `high` — sprite plane B: NURO → MASO → LEFE → LESU → WYHO → WORA →
///   VAFO → WUFY (same shift-chain structure).
/// - `palette` — per-pixel OBJ palette selection shifter.
/// - `priority` — mask pipe (VAVA MSB → VEZO LSB), BG-over-OBJ priority
///   bit per pixel loaded from DEPO's capture of OAM attribute bit 7.
///
/// All 32 dffsr cells are clocked by SACU (pixel pump); shift enters via
/// the dffsr `d` pin from the previous stage's Q. Parallel-load fires
/// asynchronously via NAND2-pair-driven s_n/r_n. Shift and parallel-load
/// are independent mechanisms.
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

    /// Read the MSB data — the stage-7 dffsr Q outputs (sprite_px_a7 /
    /// sprite_px_b7 plus palette / mask-pipe MSBs). When all four planes
    /// at MSB are 0, the color index is 0 and the pixel mux treats it as
    /// transparent (NULY NOR gate, XYLO-gated at `woxa` / `xula`).
    pub(in crate::ppu) fn read(&self) -> (u8, u8, u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (self.palette >> 7) & 1;
        let pri = (self.priority >> 7) & 1;
        (lo, hi, pal, pri)
    }

    /// Advance each dffsr's d input through the `d → clk → Q` rising-edge
    /// capture. All four planes shift unconditionally on each SACU edge;
    /// zero fills in from bit 0 because stage-0 cells NYLU / NURO (and
    /// their palette / mask-pipe counterparts) have `d = const0`.
    pub(in crate::ppu) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
        self.palette <<= 1;
        self.priority <<= 1;
    }

    pub(in crate::ppu) fn registers(&self) -> (u8, u8, u8, u8) {
        (self.low, self.high, self.palette, self.priority)
    }

    /// Parallel-load at wuty pulse — transparency-conditional per stage.
    ///
    /// Collapses the per-stage sprite_onN gate chain
    /// (`sprite_onN = NOR3(xefy, sprite_px_aN, sprite_px_bN)`, where
    /// xefy = NOT(wuty)) and the NAND2 pair at each dffsr's s_n / r_n.
    /// Fires once per sprite-fetch completion; at each stage N the load
    /// asserts only when the current shifter position is transparent
    /// (both planes bit = 0) — the first-fetched sprite's opaque pixels
    /// are preserved, later sprites fill only still-transparent positions.
    /// The sprite-to-sprite overlap priority rule (lower OAM index wins
    /// at same X) emerges from this gate combined with OAM-scan order
    /// and the §6.8 X-match fetch sequencing.
    ///
    /// The emulator's two-check form (incoming-transparent short-circuit
    /// + existing-opaque gate) is observation-equivalent: when incoming
    /// is transparent and existing is transparent, hardware would reset
    /// the cell to 0 (already 0 — no visible change); all other combinations
    /// match the written code path exactly.
    ///
    /// The mask-pipe and palette-pipe per-stage loads are modelled with
    /// the same transparency gating as planes A/B (honest-abstraction —
    /// at transparent sprite-plane positions these bits are not consumed
    /// at the pixel mux, so their values there are moot).
    ///
    /// `sprite_low`/`sprite_high` are the sprite tile bitplane bytes
    /// captured by the sprite temp latches (PEFO/ROKA/MYTU/.. for plane A;
    /// REWO/PEBA/MOFO/.. for plane B); x-flip reversal already applied.
    /// `palette_bit` / `priority_bit` are broadcast uniformly from the
    /// sprite's OAM attributes (DEPO capture of bit 7 for priority).
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
