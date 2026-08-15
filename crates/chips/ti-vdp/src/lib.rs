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

pub const VRAM_SIZE: usize = 0x4000;
/// The crystal is the board's master grid: 3 XTAL periods per CPU
/// T-state, 2 per dot, 684 per line.
pub const XTALS_PER_TSTATE: u32 = 3;
const XTALS_PER_LINE: u32 = 684;

/// The display area: the window the planes are resolved in.
pub const ACTIVE_WIDTH: u16 = 256;
pub const ACTIVE_LINES: u16 = 192;
/// The backdrop border around it, side dots per the Data Manual.
pub const LEFT_BORDER: u16 = 13;
pub const RIGHT_BORDER: u16 = 15;
pub const VISIBLE_WIDTH: u16 = LEFT_BORDER + ACTIVE_WIDTH + RIGHT_BORDER;

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

    /// Border lines above the display area; the 9929A's split is derived,
    /// no TI document giving its 313-line breakdown.
    pub fn top_border(self) -> u16 {
        match self {
            Standard::Ntsc => 27,
            Standard::Pal => 51,
        }
    }

    pub fn bottom_border(self) -> u16 {
        match self {
            Standard::Ntsc => 24,
            Standard::Pal => 51,
        }
    }

    pub fn visible_lines(self) -> u16 {
        self.top_border() + ACTIVE_LINES + self.bottom_border()
    }
}

mod r0 {
    pub const M3: u8 = 0x02;
}

mod r1 {
    pub const RAM_16K: u8 = 0x80;
    pub const DISPLAY_ENABLE: u8 = 0x40;
    pub const INTERRUPT_ENABLE: u8 = 0x20;
    pub const M1: u8 = 0x10;
    pub const M2: u8 = 0x08;
    pub const SIZE_16: u8 = 0x02;
    pub const MAG: u8 = 0x01;
}

/// The M1/M2/M3 selection, the community-documented combinations included.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    GraphicsI,
    GraphicsII,
    Multicolor,
    Text,
    BitmapText,
    BitmapMulticolor,
    TextMulticolor,
}

mod status {
    pub const FRAME: u8 = 0x80;
    pub const FIFTH_SPRITE: u8 = 0x40;
    pub const COINCIDENCE: u8 = 0x20;
}

/// Sprite attribute Y value that terminates the scan.
pub const SPRITE_TERMINATOR: u8 = 0xD0;

/// Text-family modes show 40 six-pixel cells inside backdrop margins,
/// split 6 left / 10 right — the Data Manual's asymmetric split, measured
/// on silicon (5.84/9.93); the community's symmetric 8/8 is refuted.
const TEXT_MARGIN: usize = 6;

/// The visible raster — the display area inside its backdrop border — as
/// composited colour indices, row-major. 0 survives only where every plane
/// is transparent (the external-video pass-through) and presents as black.
#[derive(Clone)]
pub struct Frame {
    pub pixels: Vec<u8>,
    pub width: u16,
    pub height: u16,
}

/// Raster placement — the counter-to-picture alignment, the same freedom
/// as the schedule rotation, calibrated against midline-name's silicon
/// seam (row 98, source column 16 at that ROM's write phase): picture row
/// N emits during counter line N-1 (row 0 during the frame's last line,
/// coherent with the schedule already running before display line 0),
/// pixel 0 at this XTAL offset. The seam's cell quantisation pins the
/// offset to [24, 40); midframe-m2's independent seam validates it.
const ACTIVE_START_XTALS: u32 = 32;
/// The whole visible span sits inside one counter line, so a counter line
/// carries a complete scanline: left border, display area, right border.
const VISIBLE_START_XTALS: u32 = ACTIVE_START_XTALS - LEFT_BORDER as u32 * 2;
const XTALS_PER_VISIBLE: u32 = VISIBLE_WIDTH as u32 * 2;

/// One latched fetch: `end_x - start_x` pixels drawn MSB-first from
/// `bits`, lit pixels in `fg`, unlit in `bg`; colour 0 falls through to
/// the live backdrop at emission.
#[derive(Clone, Copy)]
struct Segment {
    bits: u8,
    fg: u8,
    bg: u8,
    start_x: usize,
    end_x: usize,
}

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
/// The fetch schedule wakes this many lines before display line 0 (the
/// measured turn-on seam sits ~2.6 lines before; the line boundary is the
/// model's quantum while the schedule rotation stays a free convention).
const SCHEDULE_WARM_UP_LINES: u16 = 3;

/// The sprite pre-processing lattice, locked to the run schedule: entry 0
/// lands with the counter reset at the length-4 run's start, entries 1-7
/// burst one per memory cycle behind it, and entries 8-31 step three per
/// 16-cycle run period across the eight regular length-1 runs — 9 entries
/// per 48 cycles exactly.
const SCAN_RESET_CYCLE: usize = 110;
const SCAN_BURST_CYCLES: std::ops::RangeInclusive<usize> = 112..=118;
/// Burst steps land one XTAL later in their cycle than steady steps and
/// present the counter immediately — the silicon map ramps cleanly
/// through the burst, boundary texture appearing only from 7-to-8 on.
const SCAN_BURST_XTAL: u32 = 3;
const SCAN_STEP_RUNS: [usize; 8] = [123, 139, 155, 0, 16, 32, 48, 64];
const SCAN_STEP_OFFSET_CYCLES: [usize; 3] = [0, 4, 8];
/// Which memory cycles advance the scanner in the steady regime.
const SCAN_STEP_CYCLES: [bool; CYCLES_PER_LINE] = {
    let mut map = [false; CYCLES_PER_LINE];
    let mut run = 0;
    while run < SCAN_STEP_RUNS.len() {
        let mut offset = 0;
        while offset < SCAN_STEP_OFFSET_CYCLES.len() {
            map[SCAN_STEP_RUNS[run] + SCAN_STEP_OFFSET_CYCLES[offset]] = true;
            offset += 1;
        }
        run += 1;
    }
    map
};
/// The lattice instant within its memory cycle; silicon pins it only to a
/// 5-XTAL window, this value reproduces the measured boundary cells.
const SCAN_STEP_XTAL: u32 = 2;
/// The fifth-match hold releases here — between the counter's 13th and
/// 14th steps; the two measured plateau lengths (fifth matches at entries
/// 15 and 31) both pin this cycle, the sub-cycle instant being free.
const SCAN_HOLD_RELEASE_CYCLE: usize = 153;
/// After an increment the field spends this long not presenting the
/// counter: bits 4/3 read 0 throughout; bits 2..0 hold the old value's low
/// bits at the first instant, all-ones through the middle, and the new
/// value's low bits at the last.
const SCAN_WINDOW_XTALS: u64 = 5;

/// Where a line's pre-processing ramp ends and why: the full 32-entry
/// walk, a terminator's own index, or the fifth match's — only the
/// fifth-match halt arms the field hold.
#[derive(Clone, Copy)]
enum ScanStop {
    FullWalk,
    Terminator(u8),
    FifthMatch(u8),
}

impl ScanStop {
    fn index(self) -> u8 {
        match self {
            ScanStop::FullWalk => 31,
            ScanStop::Terminator(index) | ScanStop::FifthMatch(index) => index,
        }
    }
}

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
    /// Latched fifth-sprite index, presented while 5S is set.
    sprite_field: u8,
    /// The pre-processing scanner's live progress: last SAT entry handled,
    /// where this line's ramp ends, and the latest step (instant + the
    /// value it replaced) for the boundary window.
    scan_counter: u8,
    scan_stop: ScanStop,
    scan_stepped_at: u64,
    scan_step_from: u8,
    /// The fifth-match event arms a hold on the presented field; it
    /// survives the next reset and drops at the release cycle of the
    /// first scan with no event.
    field_hold: Option<u8>,
    fifth_match_this_scan: bool,

    frame: Frame,
    frames_completed: u64,
    /// The in-flight row: pixels composited as the raster passes, the
    /// line-latched sprite plane of the row being emitted (0 = no sprite
    /// pixel), and the current fetch segment.
    line_pixels: [u8; VISIBLE_WIDTH as usize],
    sprite_line: [u8; 256],
    segment: Segment,

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
            scan_counter: 31,
            scan_stop: ScanStop::FullWalk,
            scan_stepped_at: 0,
            scan_step_from: 31,
            field_hold: None,
            fifth_match_this_scan: false,
            frame: Frame {
                pixels: vec![0; VISIBLE_WIDTH as usize * standard.visible_lines() as usize],
                width: VISIBLE_WIDTH,
                height: standard.visible_lines(),
            },
            frames_completed: 0,
            line_pixels: [0; VISIBLE_WIDTH as usize],
            sprite_line: [0; 256],
            segment: Segment {
                bits: 0,
                fg: 0,
                bg: 0,
                start_x: 0,
                end_x: 0,
            },
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

    /// The DRAM as it stands, disturbing nothing — cells in physical order, so
    /// a logical pointer value reaches its byte through
    /// [`vram_cell`](Self::vram_cell) rather than by indexing this.
    pub fn vram(&self) -> &[u8; VRAM_SIZE] {
        &self.vram
    }

    /// The status byte as it stands, disturbing nothing — no flag clear, no
    /// latch reset, and no read strobe for a set to lose to.
    pub fn peek_status(&self) -> u8 {
        let mut value = self.scanned_field();
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

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Count of completed visible rasters — increments as the raster leaves
    /// the bottom border, when every row of `frame` is this frame's.
    pub fn frames_completed(&self) -> u64 {
        self.frames_completed
    }

    pub fn mode(&self) -> Mode {
        let m1 = self.registers[1] & r1::M1 != 0;
        let m2 = self.registers[1] & r1::M2 != 0;
        let m3 = self.registers[0] & r0::M3 != 0;
        match (m1, m2, m3) {
            (false, false, false) => Mode::GraphicsI,
            (false, false, true) => Mode::GraphicsII,
            (false, true, false) => Mode::Multicolor,
            (true, false, false) => Mode::Text,
            (true, false, true) => Mode::BitmapText,
            (false, true, true) => Mode::BitmapMulticolor,
            (true, true, _) => Mode::TextMulticolor,
        }
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
            self.scan_lattice();
            self.service_port();
            self.raster_dot();
        }
    }

    /// Emit the dot under the raster: the fetch segment latches from the
    /// live registers at its own instant, transparency resolves against
    /// the live backdrop per dot — mid-line writes land at their
    /// silicon-measured granularities (R7 per pixel, tables and mode bits
    /// per cell; sprites stay line-latched).
    fn raster_dot(&mut self) {
        let Some(offset) = self.xtal_in_line.checked_sub(VISIBLE_START_XTALS) else {
            return;
        };
        if offset >= XTALS_PER_VISIBLE || !offset.is_multiple_of(2) {
            return;
        }
        let visible_x = (offset / 2) as usize;
        let picture_row = if self.line + 1 == self.standard.lines_per_frame() {
            0
        } else {
            self.line + 1
        };
        let Some(frame_row) = self.frame_row(picture_row) else {
            return;
        };

        let active_x = (picture_row < ACTIVE_LINES)
            .then(|| visible_x.wrapping_sub(LEFT_BORDER as usize))
            .filter(|&x| x < ACTIVE_WIDTH as usize);
        self.line_pixels[visible_x] = match active_x {
            // The backdrop is the only plane that reaches the border, and
            // no fetch belongs to a border dot.
            None => self.registers[7] & 0x0F,
            Some(x) => {
                if x == 0 || x >= self.segment.end_x {
                    self.segment = self.latch_segment(picture_row, x);
                }
                if self.registers[1] & r1::DISPLAY_ENABLE == 0 {
                    self.registers[7] & 0x0F
                } else if self.sprite_line[x] != 0 {
                    self.sprite_line[x]
                } else {
                    let bit = x - self.segment.start_x;
                    let lit = bit < 8 && self.segment.bits & (0x80 >> bit) != 0;
                    let colour = if lit {
                        self.segment.fg
                    } else {
                        self.segment.bg
                    };
                    self.over_backdrop(colour)
                }
            }
        };
        if visible_x == VISIBLE_WIDTH as usize - 1 {
            let start = frame_row as usize * VISIBLE_WIDTH as usize;
            self.frame.pixels[start..start + VISIBLE_WIDTH as usize]
                .copy_from_slice(&self.line_pixels);
            if frame_row == self.frame.height - 1 {
                self.frames_completed += 1;
            }
        }
    }

    /// Where a picture row lands in the visible raster: the top border rides
    /// the counter wrap, then the display area, then the bottom border;
    /// everything else is blanking.
    fn frame_row(&self, picture_row: u16) -> Option<u16> {
        let top = self.standard.top_border();
        let first_top_row = self.standard.lines_per_frame() - top;
        if picture_row >= first_top_row {
            Some(picture_row - first_top_row)
        } else if picture_row < ACTIVE_LINES + self.standard.bottom_border() {
            // The display area and the bottom border run contiguously.
            Some(top + picture_row)
        } else {
            None
        }
    }

    /// Advance the pre-processing scanner at its lattice instants. M1 gates
    /// rendering, never the scanner, so only blanking stops it.
    fn scan_lattice(&mut self) {
        if self.registers[1] & r1::DISPLAY_ENABLE == 0 {
            return;
        }
        let sub = self.xtal_in_line % 4;
        if sub != SCAN_STEP_XTAL && sub != SCAN_BURST_XTAL {
            return;
        }
        let cycle = (self.xtal_in_line / 4) as usize;
        // From the reset on, a line's lattice tail belongs to the NEXT
        // line's scan; the scanner serves display lines plus the phantom
        // pass, so the counter holds its stop through the border.
        let scanned_line = if cycle >= SCAN_RESET_CYCLE {
            self.line < ACTIVE_LINES || self.line == self.standard.lines_per_frame() - 1
        } else {
            self.line <= ACTIVE_LINES
        };
        if !scanned_line {
            return;
        }
        if sub == SCAN_BURST_XTAL {
            if SCAN_BURST_CYCLES.contains(&cycle) && self.scan_counter < self.scan_stop.index() {
                self.scan_step_from = self.scan_counter;
                self.scan_counter += 1;
                self.arm_hold_at_stop();
            }
        } else if cycle == SCAN_RESET_CYCLE {
            self.scan_step_from = self.scan_counter;
            self.scan_counter = 0;
            self.scan_stepped_at = self.xtal_total;
            self.fifth_match_this_scan = false;
        } else if cycle == SCAN_HOLD_RELEASE_CYCLE {
            if !self.fifth_match_this_scan {
                self.field_hold = None;
            }
        } else if SCAN_STEP_CYCLES[cycle] && self.scan_counter < self.scan_stop.index() {
            self.scan_step_from = self.scan_counter;
            self.scan_counter += 1;
            self.scan_stepped_at = self.xtal_total;
            self.arm_hold_at_stop();
        }
    }

    fn arm_hold_at_stop(&mut self) {
        if let ScanStop::FifthMatch(index) = self.scan_stop
            && self.scan_counter == index
        {
            self.field_hold = Some(index);
            self.fifth_match_this_scan = true;
        }
    }

    /// Status low five bits: the latched fifth-sprite index while 5S is
    /// set, the armed fifth-match hold next, otherwise the scanner's
    /// counter — live, except inside the boundary window around each step.
    /// The 7-to-8 step reads inverted mid-window and whole at the trailing
    /// edge (measured; cause open).
    fn scanned_field(&self) -> u8 {
        if self.fifth_sprite_flag {
            return self.sprite_field & 0x1F;
        }
        if let Some(held) = self.field_hold {
            return held;
        }
        let elapsed = self.xtal_total - self.scan_stepped_at;
        if elapsed >= SCAN_WINDOW_XTALS {
            return self.scan_counter;
        }
        let carry_step = self.scan_step_from == 7 && self.scan_counter == 8;
        if elapsed == 0 {
            self.scan_step_from & 7
        } else if elapsed == SCAN_WINDOW_XTALS - 1 {
            if carry_step {
                self.scan_counter
            } else {
                self.scan_counter & 7
            }
        } else if carry_step {
            0b11000
        } else {
            7
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

    /// Rendering lines and the pre-display warm-up tail confine CPU access
    /// to the run schedule; everywhere else every memory cycle is claimable.
    fn cycle_accessible(&self, cycle: usize) -> bool {
        let scheduled_line = self.line < ACTIVE_LINES
            || self.line >= self.standard.lines_per_frame() - SCHEDULE_WARM_UP_LINES;
        let rendering = self.registers[1] & r1::DISPLAY_ENABLE != 0
            && self.registers[1] & r1::M1 == 0
            && scheduled_line;
        !rendering || ACCESS_CYCLES[cycle]
    }

    fn enter_line(&mut self) {
        // The scanner runs on every display line plus a phantom pass on the
        // line after the last (counts, never renders); F rises at the same
        // boundary, after that pass, so the phantom scan sees the old flag.
        if self.line <= ACTIVE_LINES {
            self.scan_sprites();
        }
        // The row emitting during this counter line (the calibrated raster
        // placement) gets its sprite plane painted now; its effects keep
        // their own corroborated boundary above.
        let row = if self.line + 1 == self.standard.lines_per_frame() {
            0
        } else {
            self.line + 1
        };
        if row < ACTIVE_LINES {
            self.sprite_line = [0; 256];
            self.paint_sprites(row);
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
        let mut value = self.scanned_field();
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

    /// The byte a pointer value reaches, disturbing nothing — the renderer's
    /// own fetch, so an inspecting consumer reads what the raster reads.
    pub fn vram_cell(&self, address: u16) -> u8 {
        self.vram[self.physical(address)]
    }

    pub fn name_table_base(&self) -> u16 {
        (self.registers[2] as u16 & 0x0F) * 0x400
    }

    /// The pattern generator base R4 selects, outside the bitmap family.
    pub fn pattern_table_base(&self) -> u16 {
        (self.registers[4] as u16 & 0x07) * 0x800
    }

    /// The colour table base R3 selects in Graphics I, where one byte colours
    /// a group of eight patterns.
    pub fn colour_table_base(&self) -> u16 {
        self.registers[3] as u16 * 0x40
    }

    /// Graphics II's two fetches for a tile row `offset` — pattern byte then
    /// colour byte. R3's AND mask governs both (silicon: gii-mask-pattern,
    /// gii-mask-colour); R4 contributes only the pattern half select.
    pub fn graphics_ii_cells(&self, offset: u16) -> (u8, u8) {
        let mask = ((self.registers[3] as u16 & 0x7F) << 6) | 0x3F;
        let colour_half = ((self.registers[3] as u16) & 0x80) << 6;
        let pattern_half = ((self.registers[4] as u16) & 0x04) << 11;
        (
            self.vram_cell(pattern_half | (offset & mask)),
            self.vram_cell(colour_half | (offset & mask)),
        )
    }

    /// A transparent pixel falls through to the backdrop; a transparent
    /// backdrop stays 0 (the external-video plane, presented black).
    fn over_backdrop(&self, colour: u8) -> u8 {
        if colour != 0 {
            colour
        } else {
            self.registers[7] & 0x0F
        }
    }

    /// The pattern-table base a bitmap-family third selects: the half from
    /// R4's high base bit, the second and third tables gated by its low bits.
    fn bitmap_third_table(&self, third: u16) -> u16 {
        let half = ((self.registers[4] as u16) & 0x04) << 11;
        let table = match third {
            1 if self.registers[4] & 0x01 != 0 => 1,
            2 if self.registers[4] & 0x02 != 0 => 2,
            _ => 0,
        };
        half + table * 0x800
    }

    /// The fetch segment covering pixel `x`, latched from the live
    /// registers and VRAM at this instant — mid-line register writes take
    /// effect from the next segment (silicon: R2 and M2 cell-quantised).
    fn latch_segment(&self, row: u16, x: usize) -> Segment {
        let line = row;
        let cell_row = line / 8;
        let row_in_cell = line % 8;
        let name_base = self.name_table_base();
        let pattern_base = self.pattern_table_base();
        let mode = self.mode();

        if matches!(mode, Mode::Text | Mode::BitmapText | Mode::TextMulticolor) {
            let (left, right) = (TEXT_MARGIN, TEXT_MARGIN + 240);
            if x < left || x >= right {
                let end_x = if x < left { left } else { 256 };
                return Segment {
                    bits: 0,
                    fg: 0,
                    bg: 0,
                    start_x: x,
                    end_x,
                };
            }
            let col = (x - left) / 6;
            let start_x = left + col * 6;
            let end_x = start_x + 6;
            return match mode {
                // No table reads: four text-colour pixels then two of
                // backdrop, all from R7.
                Mode::TextMulticolor => Segment {
                    bits: 0b1111_0000,
                    fg: self.registers[7] >> 4,
                    bg: 0,
                    start_x,
                    end_x,
                },
                _ => {
                    let table = if mode == Mode::BitmapText {
                        self.bitmap_third_table(line / 64)
                    } else {
                        pattern_base
                    };
                    let name = self.vram_cell(name_base + cell_row * 40 + col as u16) as u16;
                    Segment {
                        bits: self.vram_cell(table + name * 8 + row_in_cell),
                        fg: self.registers[7] >> 4,
                        // The 0-pixel colour is R7's low nibble — the
                        // backdrop register itself, so it rides the live
                        // per-dot resolution.
                        bg: 0,
                        start_x,
                        end_x,
                    }
                }
            };
        }

        let col = (x / 8) as u16;
        let start_x = col as usize * 8;
        let end_x = start_x + 8;
        let name = self.vram_cell(name_base + cell_row * 32 + col) as u16;
        let (bits, fg, bg) = match mode {
            Mode::GraphicsI => {
                let bits = self.vram_cell(pattern_base + name * 8 + row_in_cell);
                let colours = self.vram_cell(self.colour_table_base() + name / 8);
                (bits, colours >> 4, colours & 0x0F)
            }
            Mode::GraphicsII => {
                let offset = ((line / 64) * 256 + name) * 8 + row_in_cell;
                let (bits, colours) = self.graphics_ii_cells(offset);
                (bits, colours >> 4, colours & 0x0F)
            }
            Mode::Multicolor => {
                let byte =
                    self.vram_cell(pattern_base + name * 8 + (cell_row & 3) * 2 + row_in_cell / 4);
                (0b1111_0000, byte >> 4, byte & 0x0F)
            }
            Mode::BitmapMulticolor => {
                // R3's mask governs this fetch too (silicon:
                // undoc-bitmap-multicolor); bitmap text alone is unmasked.
                let table = self.bitmap_third_table(line / 64);
                let mask = (((self.registers[3] as u16 & 0x7F) << 6) | 0x3F) & 0x7FF;
                let offset = (name * 8 + (cell_row & 3) * 2 + row_in_cell / 4) & mask;
                let byte = self.vram_cell(table + offset);
                (0b1111_0000, byte >> 4, byte & 0x0F)
            }
            _ => unreachable!(),
        };
        Segment {
            bits,
            fg,
            bg,
            start_x,
            end_x,
        }
    }

    pub fn sprite_attribute_base(&self) -> u16 {
        (self.registers[5] as u16 & 0x7F) * 0x80
    }

    pub fn sprite_pattern_base(&self) -> u16 {
        (self.registers[6] as u16 & 0x07) * 0x800
    }

    /// Whether R1's SIZE bit selects 16×16 sprites — four consecutive
    /// generators — rather than 8×8.
    pub fn sprites_16x16(&self) -> bool {
        self.registers[1] & r1::SIZE_16 != 0
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
        let mut stop = ScanStop::FullWalk;

        for index in 0..32u8 {
            let entry = attributes + index as u16 * 4;
            let y = self.vram_cell(entry);
            if y == SPRITE_TERMINATOR {
                stop = ScanStop::Terminator(index);
                break;
            }
            let row = (self.line as u8).wrapping_sub(y.wrapping_add(1));
            if row >= height {
                continue;
            }
            matched += 1;
            if matched == 5 {
                // The data manual's gate is real: 5S only latches while F
                // is clear, and the first capture holds until a read. The
                // scan itself halts here whatever the flags say.
                if !self.frame_flag && !self.fifth_sprite_flag {
                    self.fifth_sprite_flag = true;
                    self.fifth_sprite_set_at = self.xtal_total;
                    self.sprite_field = index;
                }
                stop = ScanStop::FifthMatch(index);
                break;
            }
            if rendered {
                self.render_sprite_row(entry, row >> (magnified as u8), &mut occupied, false);
            }
        }

        self.scan_stop = stop;
    }

    /// Paint `row`'s displayed sprites into the emission plane — the same
    /// walk as the effects scan, latching nothing: 5S, C and the stop
    /// latch keep their own corroborated line boundary.
    fn paint_sprites(&mut self, row: u16) {
        let r1 = self.registers[1];
        if r1 & r1::DISPLAY_ENABLE == 0 || r1 & r1::M1 != 0 {
            return;
        }
        let magnified = r1 & r1::MAG != 0;
        let size16 = r1 & r1::SIZE_16 != 0;
        let pattern_rows = if size16 { 16u8 } else { 8 };
        let height = pattern_rows << (magnified as u8);

        let attributes = self.sprite_attribute_base();
        let mut occupied = [false; 256];
        let mut matched = 0u8;

        for index in 0..32u8 {
            let entry = attributes + index as u16 * 4;
            let y = self.vram_cell(entry);
            if y == SPRITE_TERMINATOR {
                break;
            }
            let sprite_row = (row as u8).wrapping_sub(y.wrapping_add(1));
            if sprite_row >= height {
                continue;
            }
            matched += 1;
            if matched <= 4 {
                self.render_sprite_row(entry, sprite_row >> (magnified as u8), &mut occupied, true);
            }
        }
    }

    fn render_sprite_row(&mut self, entry: u16, row: u8, occupied: &mut [bool; 256], paint: bool) {
        let r1 = self.registers[1];
        let magnified = r1 & r1::MAG != 0;
        let size16 = r1 & r1::SIZE_16 != 0;

        let x = self.vram_cell(entry + 1);
        let name = self.vram_cell(entry + 2);
        let tag = self.vram_cell(entry + 3);
        let early_clock = tag & 0x80 != 0;
        let colour = tag & 0x0F;
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
                if *cell && !paint {
                    // Coincidence counts every sprite pixel, transparent
                    // colour included, and is not gated by F.
                    self.coincidence_flag = true;
                }
                *cell = true;
                // A transparent sprite pixel collides but masks nothing;
                // among painters the frontmost wins.
                if paint && colour != 0 && self.sprite_line[px as usize] == 0 {
                    self.sprite_line[px as usize] = colour;
                }
            }
        }
    }
}
