//! Texas Instruments TMS9918A-family Video Display Processor.
//!
//! The digital core of the TMS9918A (NTSC) / TMS9929A (PAL): the register
//! file, the VRAM engine with its read-ahead buffer, the status flags and
//! interrupt line, and the per-line sprite scanner. Consumers wire the two
//! CPU-facing ports (mode low = data, mode high = control) and advance the
//! raster with `tick`; one dot is one pixel clock (342 dots per line).
//!
//! Ground truth is the hardware-endorsed test corpus run by
//! `tests/accuracy/` — see `AGENTS.md` for the hierarchy and the stated
//! abstractions (instruction-granular interleaving; digital core only).

const VRAM_SIZE: usize = 0x4000;
pub const DOTS_PER_LINE: u32 = 342;
/// Display lines per frame (the sprite scanner's active range).
const ACTIVE_LINES: u16 = 192;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standard {
    Ntsc,
    Pal,
}

impl Standard {
    pub fn lines_per_frame(self) -> u16 {
        match self {
            Standard::Ntsc => 262,
            Standard::Pal => 313,
        }
    }
}

mod r1 {
    pub const RAM_16K: u8 = 0x80;
    pub const DISPLAY_ENABLE: u8 = 0x40;
    pub const INTERRUPT_ENABLE: u8 = 0x20;
    pub const M1: u8 = 0x10;
    pub const SIZE_16: u8 = 0x02;
    pub const MAG: u8 = 0x01;
}

mod status {
    pub const FRAME: u8 = 0x80;
    pub const FIFTH_SPRITE: u8 = 0x40;
    pub const COINCIDENCE: u8 = 0x20;
}

/// Sprite attribute Y value that terminates the scan.
const SPRITE_TERMINATOR: u8 = 0xD0;

/// Dots between CPU-access slots in the memory schedule of an active
/// non-text display line (one slot per 16 memory cycles of 2 dots).
const SLOT_PERIOD: u64 = 32;

/// A CPU-port VRAM access waiting for its memory slot.
enum PendingAccess {
    Write { address: u16, value: u8 },
    Refill { address: u16 },
}

pub struct Vdp {
    standard: Standard,
    vram: [u8; VRAM_SIZE],
    registers: [u8; 8],

    /// 14-bit auto-incrementing VRAM pointer.
    address: u16,
    /// Set between the first and second control-port bytes.
    awaiting_second_byte: bool,
    read_buffer: u8,
    /// The in-flight CPU access and the dot it will be serviced at; a new
    /// request before then replaces it — the first byte dies.
    pending: Option<(u64, PendingAccess)>,

    frame_flag: bool,
    coincidence_flag: bool,
    fifth_sprite_flag: bool,
    /// Status low five bits: latched fifth-sprite index while the flag is
    /// set, otherwise the scanner's stop position from the last line.
    sprite_field: u8,

    dot: u32,
    line: u16,
    dots_total: u64,
}

impl Vdp {
    pub fn new(standard: Standard) -> Self {
        Vdp {
            standard,
            vram: [0; VRAM_SIZE],
            registers: [0; 8],
            address: 0,
            awaiting_second_byte: false,
            read_buffer: 0,
            pending: None,
            frame_flag: false,
            coincidence_flag: false,
            fifth_sprite_flag: false,
            sprite_field: 0,
            dot: 0,
            line: 0,
            dots_total: 0,
        }
    }

    pub fn interrupt_asserted(&self) -> bool {
        self.frame_flag && self.registers[1] & r1::INTERRUPT_ENABLE != 0
    }

    /// Advance the raster by `dots` pixel clocks.
    pub fn tick(&mut self, dots: u32) {
        for _ in 0..dots {
            self.dots_total += 1;
            if let Some((ready, _)) = self.pending
                && self.dots_total >= ready
            {
                let (_, access) = self.pending.take().unwrap();
                self.complete(access);
            }
            self.dot += 1;
            if self.dot == DOTS_PER_LINE {
                self.dot = 0;
                self.line += 1;
                if self.line == self.standard.lines_per_frame() {
                    self.line = 0;
                }
                self.enter_line();
            }
        }
    }

    fn enter_line(&mut self) {
        // The scanner runs on every display line plus a phantom pass on the
        // line after the last (counts, never renders); F rises at the same
        // boundary, after that pass, so the phantom scan sees the old flag.
        if self.line <= ACTIVE_LINES {
            self.scan_sprites();
        }
        if self.line == ACTIVE_LINES {
            self.frame_flag = true;
        }
    }

    pub fn write_control(&mut self, value: u8) {
        if !self.awaiting_second_byte {
            // The first byte lands in the pointer immediately.
            self.address = (self.address & 0x3F00) | value as u16;
            self.awaiting_second_byte = true;
        } else {
            self.awaiting_second_byte = false;
            // The second byte always lands in the pointer's high bits — a
            // register write leaves the pointer pointing at (byte2 & $3F)
            // << 8 | byte1, which is what "destroys" a pending address.
            self.address = (self.address & 0x00FF) | ((value as u16 & 0x3F) << 8);
            match value & 0xC0 {
                0x00 => self.fill_read_buffer(),
                0x40 => {}
                _ => {
                    let data = (self.address & 0xFF) as u8;
                    self.registers[(value & 0x07) as usize] = data;
                }
            }
        }
    }

    pub fn write_data(&mut self, value: u8) {
        self.awaiting_second_byte = false;
        // A write loads the read-ahead buffer with the written value.
        self.read_buffer = value;
        let address = self.address;
        self.request(PendingAccess::Write { address, value });
        self.increment_address();
    }

    pub fn read_data(&mut self) -> u8 {
        self.awaiting_second_byte = false;
        let value = self.read_buffer;
        let address = self.address;
        self.request(PendingAccess::Refill { address });
        self.increment_address();
        value
    }

    pub fn read_status(&mut self) -> u8 {
        self.awaiting_second_byte = false;
        let mut value = self.sprite_field & 0x1F;
        if self.frame_flag {
            value |= status::FRAME;
        }
        if self.fifth_sprite_flag {
            value |= status::FIFTH_SPRITE;
        }
        if self.coincidence_flag {
            value |= status::COINCIDENCE;
        }
        self.frame_flag = false;
        self.fifth_sprite_flag = false;
        self.coincidence_flag = false;
        value
    }

    fn fill_read_buffer(&mut self) {
        let address = self.address;
        self.request(PendingAccess::Refill { address });
        self.increment_address();
    }

    /// CPU accesses on an active non-text display line wait for the next
    /// CPU slot in the memory schedule; everywhere else the port is free.
    fn request(&mut self, access: PendingAccess) {
        if let Some((ready, pending)) = &mut self.pending
            && self.dots_total < *ready
        {
            // The first request's address stays latched; the newcomer
            // re-latches only data and direction, so the serviced access
            // carries the first address with the last value.
            let address = match *pending {
                PendingAccess::Write { address, .. } | PendingAccess::Refill { address } => address,
            };
            *pending = match access {
                PendingAccess::Write { value, .. } => PendingAccess::Write { address, value },
                PendingAccess::Refill { .. } => PendingAccess::Refill { address },
            };
            return;
        }
        let restricted = self.registers[1] & r1::DISPLAY_ENABLE != 0
            && self.registers[1] & r1::M1 == 0
            && self.line < ACTIVE_LINES;
        if restricted {
            // Slots sit at fixed positions in the line (10 slot periods
            // plus the schedule's short gap: 171 cycles = 10x16 + 11), so
            // the wait is to the next in-line slot, line-locked.
            let next_slot = (self.dot as u64 / SLOT_PERIOD + 1) * SLOT_PERIOD;
            let wait = next_slot.min(DOTS_PER_LINE as u64) - self.dot as u64;
            self.pending = Some((self.dots_total + wait, access));
        } else {
            self.complete(access);
        }
    }

    fn complete(&mut self, access: PendingAccess) {
        match access {
            PendingAccess::Write { address, value } => {
                self.vram[self.physical(address)] = value;
            }
            PendingAccess::Refill { address } => {
                self.read_buffer = self.vram[self.physical(address)];
            }
        }
    }

    fn increment_address(&mut self) {
        self.address = (self.address + 1) & 0x3FFF;
    }

    /// The DRAM cell a pointer value reaches. In 4K mode the address pins
    /// multiplex differently, permuting the pointer's bits (measured by
    /// vram/4k-mode).
    fn physical(&self, address: u16) -> usize {
        let address = address & 0x3FFF;
        if self.registers[1] & r1::RAM_16K != 0 {
            address as usize
        } else {
            ((address & 0x2000)
                | ((address & 0x1000) >> 6)
                | ((address & 0x0FC0) << 1)
                | (address & 0x003F)) as usize
        }
    }

    fn vram_cell(&self, address: u16) -> u8 {
        self.vram[self.physical(address)]
    }

    fn sprite_attribute_base(&self) -> u16 {
        (self.registers[5] as u16 & 0x7F) * 0x80
    }

    fn sprite_pattern_base(&self) -> u16 {
        (self.registers[6] as u16 & 0x07) * 0x800
    }

    fn scan_sprites(&mut self) {
        let r1 = self.registers[1];
        if r1 & r1::DISPLAY_ENABLE == 0 {
            return;
        }
        let phantom = self.line == ACTIVE_LINES;
        // M1 gates sprite rendering (never the scanner) in every mode
        // combination that includes it.
        let rendered = !phantom && r1 & r1::M1 == 0;

        let magnified = r1 & r1::MAG != 0;
        let size16 = r1 & r1::SIZE_16 != 0;
        let pattern_rows = if size16 { 16u8 } else { 8 };
        let height = pattern_rows << (magnified as u8);

        let attributes = self.sprite_attribute_base();
        let mut occupied = [false; 256];
        let mut matched = 0u8;
        let mut stop_index = 31u8;

        for index in 0..32u8 {
            let entry = attributes + index as u16 * 4;
            let y = self.vram_cell(entry);
            if y == SPRITE_TERMINATOR {
                stop_index = index;
                break;
            }
            let row = (self.line as u8).wrapping_sub(y.wrapping_add(1));
            if row >= height {
                continue;
            }
            matched += 1;
            if matched == 5 {
                // The data manual's gate is real: 5S only latches while F
                // is clear, and the first capture holds until a read.
                if !self.frame_flag && !self.fifth_sprite_flag {
                    self.fifth_sprite_flag = true;
                    self.sprite_field = index;
                }
            }
            if matched <= 4 && rendered {
                self.render_sprite_row(entry, row >> (magnified as u8), &mut occupied);
            }
        }

        if !self.fifth_sprite_flag {
            self.sprite_field = stop_index;
        }
    }

    fn render_sprite_row(&mut self, entry: u16, row: u8, occupied: &mut [bool; 256]) {
        let r1 = self.registers[1];
        let magnified = r1 & r1::MAG != 0;
        let size16 = r1 & r1::SIZE_16 != 0;

        let x = self.vram_cell(entry + 1);
        let name = self.vram_cell(entry + 2);
        let tag = self.vram_cell(entry + 3);
        let early_clock = tag & 0x80 != 0;
        let origin = x as i32 - if early_clock { 32 } else { 0 };

        let pattern = self.sprite_pattern_base();
        let row_bits: u16 = if size16 {
            let base = pattern + (name as u16 & 0xFC) * 8 + row as u16;
            u16::from_be_bytes([self.vram_cell(base), self.vram_cell(base + 16)])
        } else {
            (self.vram_cell(pattern + name as u16 * 8 + row as u16) as u16) << 8
        };

        let width = if size16 { 16 } else { 8 };
        let scale = if magnified { 2 } else { 1 };
        for bit in 0..width {
            if row_bits & (0x8000 >> bit) == 0 {
                continue;
            }
            for sub in 0..scale {
                let px = origin + bit * scale + sub;
                if !(0..256).contains(&px) {
                    continue;
                }
                let cell = &mut occupied[px as usize];
                if *cell {
                    // Coincidence counts every sprite pixel, transparent
                    // colour included, and is not gated by F.
                    self.coincidence_flag = true;
                }
                *cell = true;
            }
        }
    }
}
