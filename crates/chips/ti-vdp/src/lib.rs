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

mod port;
mod registers;
mod render;
mod scan;
mod sprites;
mod standard;
mod status;
mod vram;

pub use registers::Mode;
pub use render::{Frame, PALETTE};
pub use sprites::SPRITE_TERMINATOR;
pub use standard::{
    ACTIVE_LINES, ACTIVE_WIDTH, LEFT_BORDER, RIGHT_BORDER, Standard, VISIBLE_WIDTH,
    XTALS_PER_TSTATE,
};
pub use vram::VRAM_SIZE;

use port::Port;
use render::Segment;
use scan::Scanner;
use standard::XTALS_PER_LINE;
use status::Status;

/// The chip: its DRAM and register file, the units that read them, and the
/// raster counter every one of them is timed against.
pub struct Vdp {
    standard: Standard,
    vram: [u8; VRAM_SIZE],
    registers: [u8; 8],

    port: Port,
    status: Status,
    scanner: Scanner,

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
    /// A powered-on part cut for `standard`, its DRAM cleared.
    pub fn new(standard: Standard) -> Self {
        Vdp {
            standard,
            vram: [0; VRAM_SIZE],
            registers: [0; 8],
            port: Port::POWER_ON,
            status: Status::POWER_ON,
            scanner: Scanner::POWER_ON,
            frame: Frame::blank(standard),
            frames_completed: 0,
            line_pixels: [0; VISIBLE_WIDTH as usize],
            sprite_line: [0; 256],
            segment: Segment::BLANK,
            xtal_in_line: 0,
            line: 0,
            xtal_total: 0,
        }
    }

    /// Back to power-on state, keeping the DRAM and frame allocations — a
    /// board's /RESET is indistinguishable from power-on here.
    pub fn reset(&mut self) {
        self.vram.fill(0);
        self.registers = [0; 8];
        self.port = Port::POWER_ON;
        self.status = Status::POWER_ON;
        self.scanner = Scanner::POWER_ON;
        self.frame.pixels.fill(0);
        self.frames_completed = 0;
        self.line_pixels.fill(0);
        self.sprite_line.fill(0);
        self.segment = Segment::BLANK;
        self.xtal_in_line = 0;
        self.line = 0;
        self.xtal_total = 0;
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
        let row = self.emitting_row();
        if row < ACTIVE_LINES {
            self.sprite_line = [0; 256];
            self.paint_sprites(row);
        }
        if self.line == ACTIVE_LINES {
            self.status.frame = true;
            self.status.frame_set_at = self.xtal_total;
        }
    }
}
