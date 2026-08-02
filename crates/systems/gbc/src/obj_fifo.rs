/// The CGB object FIFO: colour planes, a 3-bit palette (OBP0-7), priority, and a
/// per-pixel source slot (the OAM-scan store index = OAM-priority rank). When OPRI
/// selects CGB priority, a lower-slot object's pixel overwrites a higher one;
/// otherwise stages fill only when transparent (DMG fetch-order).
#[derive(Default)]
pub struct CgbObjShifter {
    low: u8,
    high: u8,
    palette: [u8; 3],
    priority: u8,
    slot: [u8; 8],
}

impl CgbObjShifter {
    pub(crate) fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
        for plane in &mut self.palette {
            *plane <<= 1;
        }
        self.priority <<= 1;
        self.slot.copy_within(0..7, 1);
        self.slot[0] = 0;
    }

    pub(crate) fn pixel(&self) -> (u8, u8, u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (0..3).fold(0, |acc, p| acc | (((self.palette[p] >> 7) & 1) << p));
        let pri = (self.priority >> 7) & 1;
        (lo, hi, pal, pri)
    }

    pub(crate) fn registers(&self) -> (u8, u8, u8, u8) {
        (self.low, self.high, self.palette[0], self.priority)
    }

    pub(crate) fn cells(&self) -> [missingno_gb::ppu::ObjFifoCell; 8] {
        missingno_gb::ppu::obj_fifo_cells_from(self.low, self.high, self.palette, self.priority)
    }

    pub(crate) fn merge(
        &mut self,
        low: u8,
        high: u8,
        palette: u8,
        priority_bit: u8,
        slot: u8,
        by_index: bool,
    ) {
        for bit_pos in 0..8u8 {
            let lo = (low >> bit_pos) & 1;
            let hi = (high >> bit_pos) & 1;
            let color = (hi << 1) | lo;
            if color == 0 {
                continue;
            }

            let existing_lo = (self.low >> bit_pos) & 1;
            let existing_hi = (self.high >> bit_pos) & 1;
            let existing_color = (existing_hi << 1) | existing_lo;
            let pos = bit_pos as usize;
            if existing_color != 0 && !(by_index && slot < self.slot[pos]) {
                continue;
            }

            let mask = 1 << bit_pos;
            self.low = (self.low & !mask) | (lo << bit_pos);
            self.high = (self.high & !mask) | (hi << bit_pos);
            for (p, plane) in self.palette.iter_mut().enumerate() {
                *plane = (*plane & !mask) | (((palette >> p) & 1) << bit_pos);
            }
            self.priority = (self.priority & !mask) | (priority_bit << bit_pos);
            self.slot[pos] = slot;
        }
    }
}
