//! Texas Instruments TMS9918A-family Video Display Processor.
//!
//! The digital core of the TMS9918A (NTSC) / TMS9929A (PAL): the register
//! file, the VRAM engine with its read-ahead buffer and CPU-access memory
//! schedule, the status flags and interrupt line, and the per-line sprite
//! scanner. Consumers wire the two CPU-facing ports (mode low = data,
//! mode high = control) and advance the raster with `tick`, one crystal
//! period at a time.
//!
//! Ground truth is the hardware-endorsed test corpus run by
//! `tests/accuracy/` — see `AGENTS.md` for the hierarchy and the stated
//! abstractions.

const VRAM_SIZE: usize = 0x4000;
/// The crystal is the board's master grid: 3 XTAL periods per CPU
/// T-state, 2 per dot, 684 per line.
pub const XTALS_PER_TSTATE: u32 = 3;
const XTALS_PER_LINE: u32 = 684;
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

/// One DRAM memory cycle is two dots = four XTAL periods; 171 per line.
const CYCLES_PER_LINE: usize = 171;
/// The eleven runs of consecutive CPU-access memory cycles on an active
/// non-text line, as run-start positions — starts spaced
/// 16,16,16,16,15,16,15,13,16,16,16 cycles (sum 171), the lattice both
/// silicon maps trace out. The rotation against hsync is unmeasured, so
/// the origin is a free convention.
const RUN_START_CYCLES: [usize; 11] = [0, 16, 32, 48, 64, 79, 95, 110, 123, 139, 155];
/// Run lengths, shortest of the map-equivalent family (19 CPU cycles per
/// line, the documented CPU-cycle budget).
const RUN_LENGTH_CYCLES: [usize; 11] = [1, 1, 1, 1, 1, 2, 5, 4, 1, 1, 1];
/// The servicing cycle samples the transfer register this long after its
/// own start...
const TRANSFER_LOCK_XTALS: u64 = 17;
/// ...and the request flag clears this long after, so a request landing
/// in the gap both supplies the in-flight data and queues itself.
const FLAG_RELEASE_XTALS: u64 = 15;
/// A port write needs this long to settle; a lock sampling sooner sees
/// the register mid-transition and each bit resolves as old AND new.
const TRANSFER_SETTLE_XTALS: u64 = 2;

/// Whether each memory cycle of a rendering line is a CPU-access cycle.
const ACCESS_CYCLES: [bool; CYCLES_PER_LINE] = {
    let mut map = [false; CYCLES_PER_LINE];
    let mut run = 0;
    while run < RUN_START_CYCLES.len() {
        let mut offset = 0;
        while offset < RUN_LENGTH_CYCLES[run] {
            map[RUN_START_CYCLES[run] + offset] = true;
            offset += 1;
        }
        run += 1;
    }
    map
};

/// Direction and data of a CPU port transfer.
#[derive(Clone, Copy)]
enum PortTransfer {
    Write(u8),
    Refill,
}

/// The access a CPU-access cycle has claimed: the address latched at
/// claim, and the cycle's lock and release instants.
struct InFlightAccess {
    address: u16,
    lock_at: u64,
    release_at: u64,
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
    /// Re-latched by every CPU request; the servicing cycle samples it at
    /// its lock instant, so a newer request steals an in-flight cycle.
    port_transfer: PortTransfer,
    /// The transfer being replaced, and when: a lock sampling within the
    /// settle window merges the two.
    prior_transfer: PortTransfer,
    transfer_written_at: u64,
    /// Latched by the request that raised the flag; replacements leave it
    /// holding — a superseded access keeps the first address.
    pending_address: u16,
    pending_flag: bool,
    in_flight: Option<InFlightAccess>,

    frame_flag: bool,
    coincidence_flag: bool,
    fifth_sprite_flag: bool,
    /// XTAL instant of the most recent F / 5S set. A clearing read whose
    /// access strobe contains the instant resolves clear-dominant: the
    /// read reports the pre-set value and its clear swallows the set.
    frame_flag_set_at: u64,
    fifth_sprite_set_at: u64,
    /// Status low five bits: latched fifth-sprite index while the flag is
    /// set, otherwise the scanner's stop position from the last line.
    sprite_field: u8,

    xtal_in_line: u32,
    line: u16,
    xtal_total: u64,
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
            port_transfer: PortTransfer::Refill,
            prior_transfer: PortTransfer::Refill,
            transfer_written_at: 0,
            pending_address: 0,
            pending_flag: false,
            in_flight: None,
            frame_flag: false,
            coincidence_flag: false,
            fifth_sprite_flag: false,
            frame_flag_set_at: 0,
            fifth_sprite_set_at: 0,
            sprite_field: 0,
            xtal_in_line: 0,
            line: 0,
            xtal_total: 0,
        }
    }

    pub fn interrupt_asserted(&self) -> bool {
        self.frame_flag && self.registers[1] & r1::INTERRUPT_ENABLE != 0
    }

    pub fn registers(&self) -> &[u8; 8] {
        &self.registers
    }

    /// The status byte as it stands, disturbing nothing — no flag clear, no
    /// latch reset, and no read strobe for a set to lose to.
    pub fn peek_status(&self) -> u8 {
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
        value
    }

    pub fn line(&self) -> u16 {
        self.line
    }

    pub fn dot(&self) -> u16 {
        (self.xtal_in_line / 2) as u16
    }

    pub fn address(&self) -> u16 {
        self.address
    }

    pub fn awaiting_second_byte(&self) -> bool {
        self.awaiting_second_byte
    }

    pub fn read_buffer(&self) -> u8 {
        self.read_buffer
    }

    /// Advance the raster by `xtals` crystal periods.
    pub fn tick(&mut self, xtals: u32) {
        for _ in 0..xtals {
            self.xtal_total += 1;
            self.xtal_in_line += 1;
            if self.xtal_in_line == XTALS_PER_LINE {
                self.xtal_in_line = 0;
                self.line += 1;
                if self.line == self.standard.lines_per_frame() {
                    self.line = 0;
                }
                self.enter_line();
            }
            self.service_port();
        }
    }

    /// The claim → release → lock events of the CPU port at this instant.
    fn service_port(&mut self) {
        if self.xtal_in_line.is_multiple_of(4)
            && self.pending_flag
            && self.in_flight.is_none()
            && self.cycle_accessible((self.xtal_in_line / 4) as usize)
        {
            self.in_flight = Some(InFlightAccess {
                address: self.pending_address,
                lock_at: self.xtal_total + TRANSFER_LOCK_XTALS,
                release_at: self.xtal_total + FLAG_RELEASE_XTALS,
            });
        }
        if let Some(access) = &self.in_flight {
            if self.xtal_total == access.release_at {
                self.pending_flag = false;
            }
            if self.xtal_total == access.lock_at {
                let address = access.address;
                self.in_flight = None;
                self.complete(address);
            }
        }
    }

    /// Rendering lines confine CPU access to the run schedule; everywhere
    /// else every memory cycle is claimable.
    fn cycle_accessible(&self, cycle: usize) -> bool {
        let rendering = self.registers[1] & r1::DISPLAY_ENABLE != 0
            && self.registers[1] & r1::M1 == 0
            && self.line < ACTIVE_LINES;
        !rendering || ACCESS_CYCLES[cycle]
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
            self.frame_flag_set_at = self.xtal_total;
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
        self.request(address, PortTransfer::Write(value));
        self.increment_address();
    }

    pub fn read_data(&mut self) -> u8 {
        self.awaiting_second_byte = false;
        let value = self.read_buffer;
        let address = self.address;
        self.request(address, PortTransfer::Refill);
        self.increment_address();
        value
    }

    pub fn read_status(&mut self) -> u8 {
        self.awaiting_second_byte = false;
        let mut value = self.sprite_field & 0x1F;
        // F and 5S sets landing inside the read's strobe lose to its
        // clear (dynamic-node contention): the read reports the pre-set
        // value and the clear below swallows the set. C is set-dominant.
        if self.frame_flag && !self.set_in_read_strobe(self.frame_flag_set_at) {
            value |= status::FRAME;
        }
        if self.fifth_sprite_flag && !self.set_in_read_strobe(self.fifth_sprite_set_at) {
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

    /// Whether a set at `set_at` landed inside this read's access strobe
    /// (the T-state whose crystal periods have just elapsed).
    fn set_in_read_strobe(&self, set_at: u64) -> bool {
        self.xtal_total - set_at < XTALS_PER_TSTATE as u64
    }

    fn fill_read_buffer(&mut self) {
        let address = self.address;
        self.request(address, PortTransfer::Refill);
        self.increment_address();
    }

    /// Every CPU access re-latches direction and data; the address only
    /// latches with the request that raises the flag, so a superseded
    /// access carries the first address with the last value.
    fn request(&mut self, address: u16, transfer: PortTransfer) {
        self.prior_transfer = self.port_transfer;
        self.port_transfer = transfer;
        self.transfer_written_at = self.xtal_total;
        if !self.pending_flag {
            self.pending_address = address;
            self.pending_flag = true;
        }
    }

    /// The transfer as the lock samples it: settled, the latest value;
    /// mid-transition, writes merge bitwise old AND new. A direction bit
    /// mid-flip is unmeasured — the latest transfer wins.
    fn locked_transfer(&self) -> PortTransfer {
        if self.xtal_total - self.transfer_written_at >= TRANSFER_SETTLE_XTALS {
            return self.port_transfer;
        }
        match (self.prior_transfer, self.port_transfer) {
            (PortTransfer::Write(old), PortTransfer::Write(new)) => PortTransfer::Write(old & new),
            (_, latest) => latest,
        }
    }

    fn complete(&mut self, address: u16) {
        match self.locked_transfer() {
            PortTransfer::Write(value) => {
                self.vram[self.physical(address)] = value;
            }
            PortTransfer::Refill => {
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
                    self.fifth_sprite_set_at = self.xtal_total;
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
